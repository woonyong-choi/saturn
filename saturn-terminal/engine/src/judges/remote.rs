//! 외부 judge API 연결(`jev` 방식의 기준 judge).
//!
//! 설계: docs/design/judge.md(judge 시작 확인, judge 호출, 오류 처리), docs/design/judge-key-security.md(전송, 출력 마스킹).
//! 전송 규칙:
//! - HTTPS만 쓰고 허용 호스트는 `api.typesafe.ai` 하나다. 주소 검사는 만들 때 한 번 하고 요청마다 다시 본다.
//! - 리다이렉트 때 인증 헤더를 지운다. 허용 호스트 밖으로 가는 리다이렉트는 따르지 않는다.
//! - TLS 검증을 끄는 설정은 두지 않는다.
//! - Authorization 헤더는 어디에도 기록하지 않는다. 키는 요청 직전에 `SecretStore::key`로 빌리고 들고 있지 않는다.
//! - 모델은 버전을 고정한 이름으로 부르고, 별칭이면 응답의 `model`을 기록한다.
//!
//! 재시도 규칙(보내기 전에 확정된 실패만 다시 보낸다):
//! | 실패 | 처리 |
//! |---|---|
//! | 보내기 전 실패(연결 실패, DNS 등) | `RetryPolicy::pre_send_attempts`번까지 다시 보낸다 |
//! | 보낸 뒤 시간 초과 | `JudgeError::TimedOutAfterSend`(`cost-unknown`). 다시 보내지 않는다 |
//! | 속도 제한 | 기다렸다가 다시 보낸다 |
//! | 후보 밖 선택, NaN, 확률 누락 | `JudgeError::Invalid`. 다시 보내지 않는다 |
//!
//! HTTP 클라이언트는 `reqwest`(rustls, HTTPS 전용, 자동 리다이렉트 끔)다. 리다이렉트는 한 번만 손으로 따르고,
//! 대상이 `validate_endpoint`를 통과할 때만 `redirect_headers`로 인증 헤더를 뺀 채 다시 보낸다.
//! TLS 검증을 끄는 옵션은 만들지 않는다.
//!
//! API 형식(docs.typesafe.ai `api.md`, `models.md`, 2026-10-01 확인):
//! | 항목 | 값 |
//! |---|---|
//! | 판단 | `POST /v1/systemone`, 본문 `{"model","state","questions":{id:{"type","instructions","criteria"}}}` |
//! | 답 | `answers[id]`: `noul`은 `{"noul":p}`, `choice`는 `{"probabilities":{선택지:p}}`, `score`는 `{"probabilities":{"0":p,...}}` |
//! | 사용량 | `usage.input_tokens`, `usage.output_tokens`, 실제 모델은 `model` |
//! | 모델 목록 | `GET /v1/models`, `{"models":[{"name",...}]}` |
//! | 오류 | 401 키 거절, 422 형식 오류, 429 속도 제한, 529 과부하(속도 제한과 같이 기다렸다 다시 보낸다) |

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use saturn_core::judges::{
    Answer, AnswerKind, JudgeClient, JudgeError, JudgeRequest, JudgeResponse, Question,
};
use serde_json::{Map, Value, json};

use super::{JudgeExchange, JudgesError, SharedSecrets};
use crate::secrets::is_sensitive_header;

/// 허용 호스트. judge 키는 이 호스트로만 간다.
pub const ALLOWED_HOST: &str = "api.typesafe.ai";

/// 요청 하나의 크기 한도. 넘으면 나눠 보낸다. API는 64k 토큰을 한도로 두지만 토큰을 셀 수 없어 본문 바이트로 잰다.
/// 바이트 수는 토큰 수보다 크거나 같아 한도를 넘지 않는다(초안).
pub const REQUEST_SPLIT_LIMIT: usize = 64 * 1024;

/// `state`와 가장 긴 질문의 합 한도. 넘으면 나눠 보낸다. 단위는 `REQUEST_SPLIT_LIMIT`와 같다(초안).
pub const STATE_SPLIT_LIMIT: usize = 32 * 1024;

/// `choice` 선택지 최대 수. 넘으면 계층 선택으로 나눈다.
pub const MAX_CHOICES: usize = 255;

/// 판단 경로.
const JUDGE_PATH: &str = "/v1/systemone";

/// 모델 목록 경로.
const MODELS_PATH: &str = "/v1/models";

/// 속도 제한 응답을 받고 다시 보내는 최대 횟수. 넘으면 `RateLimited`로 포기한다. 값은 초안이다.
const RATE_LIMIT_ATTEMPTS: u32 = 3;

/// 계층 선택 조각의 "이 조각에 없음" 선택지 이름.
const OTHER_CHUNK_OPTION: &str = "none of these";

/// 재시도 설정. 값은 설정 층에서 읽는다. TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 보내기 전 실패를 다시 보내는 최대 횟수(설정된 횟수).
    pub pre_send_attempts: u32,
    /// 응답을 기다리는 시간. 보낸 뒤 이 시간이 지나면 `TimedOutAfterSend`.
    pub response_timeout: Duration,
    /// 속도 제한 응답이 기다릴 시간을 주지 않을 때 기다리는 시간.
    pub rate_limit_wait: Duration,
}

impl Default for RetryPolicy {
    /// 초안 기본값: 보내기 전 실패 3회, 응답 30초, 속도 제한 대기 2초.
    fn default() -> Self {
        Self {
            pre_send_attempts: 3,
            response_timeout: Duration::from_secs(30),
            rate_limit_wait: Duration::from_secs(2),
        }
    }
}

/// HTTP 응답 하나. 헤더 이름은 소문자.
#[derive(Debug, Clone)]
pub(crate) struct HttpReply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

/// 전송 실패. 보내기 전과 보낸 뒤를 나눈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportError {
    /// 연결을 열지 못했다. 요청은 나가지 않았다.
    BeforeSend,
    /// 보낸 뒤 응답 시간을 넘겼거나 응답 중 끊겼다.
    AfterSend,
}

/// 전송 결과 future.
pub(crate) type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HttpReply, TransportError>> + Send + 'a>>;

/// HTTP 전송. 운영은 `ReqwestTransport`, 테스트는 가짜 전송을 쓴다. 헤더는 이 안에서만 다루고 기록하지 않는다.
pub(crate) trait Transport: Debug + Send + Sync {
    /// 요청 하나. `body`가 `None`이면 GET, 있으면 POST. 리다이렉트는 따르지 않는다.
    fn send<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Option<String>,
        timeout: Duration,
    ) -> TransportFuture<'a>;
}

/// `reqwest` 전송. HTTPS만, 자동 리다이렉트 끔, TLS 검증은 기본값 그대로(끄는 옵션 없음).
#[derive(Debug)]
pub(crate) struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client })
    }
}

impl Transport for ReqwestTransport {
    fn send<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Option<String>,
        timeout: Duration,
    ) -> TransportFuture<'a> {
        send_with(&self.client, url, headers, body, timeout)
    }
}

/// `reqwest` 클라이언트로 한 번 보낸다. 연결 실패만 보내기 전 실패로 본다.
pub(crate) fn send_with<'a>(
    client: &'a reqwest::Client,
    url: &'a str,
    headers: Vec<(String, String)>,
    body: Option<String>,
    timeout: Duration,
) -> TransportFuture<'a> {
    Box::pin(async move {
        let mut request = match body {
            Some(body) => client.post(url).body(body),
            None => client.get(url),
        };
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.timeout(timeout).send().await.map_err(|error| {
            if error.is_connect() {
                TransportError::BeforeSend
            } else {
                TransportError::AfterSend
            }
        })?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                Some((
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().ok()?.to_owned(),
                ))
            })
            .collect();
        let body = response
            .text()
            .await
            .map_err(|_| TransportError::AfterSend)?;
        Ok(HttpReply {
            status,
            headers,
            body,
        })
    })
}

/// 외부 judge 연결. `JudgeClient`로 engine에만 붙는다. `Debug`는 주소, 모델, 재시도 설정만 쓴다(키 보관소와 전송은 쓰지 않는다).
pub struct RemoteJudge {
    /// 검사를 통과한 judge 주소(`https://api.typesafe.ai/...`).
    endpoint: String,
    /// 버전을 고정한 모델 이름.
    model: String,
    /// 키 보관소. 요청 직전에만 키를 빌린다.
    secrets: SharedSecrets,
    /// 재시도 설정.
    retry: RetryPolicy,
    /// HTTP 전송. HTTPS 전용, 리다이렉트 때 인증 헤더 제거, TLS 검증 고정.
    transport: Arc<dyn Transport>,
}

impl Debug for RemoteJudge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteJudge")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl RemoteJudge {
    /// 주소를 검사하고 연결을 만든다. 아직 네트워크를 쓰지 않는다.
    ///
    /// # Errors
    /// 주소가 HTTPS가 아니거나 호스트가 `ALLOWED_HOST`가 아니면 `DisallowedEndpoint`.
    pub fn new(
        endpoint: &str,
        model: String,
        secrets: SharedSecrets,
        retry: RetryPolicy,
    ) -> Result<Self, JudgesError> {
        validate_endpoint(endpoint)?;
        let transport = ReqwestTransport::new().map_err(|_| JudgesError::DisallowedEndpoint {
            endpoint: endpoint.to_owned(),
        })?;
        Ok(Self::with_transport(
            endpoint,
            model,
            secrets,
            retry,
            Arc::new(transport),
        ))
    }

    /// 전송을 받아 만든다. 주소 검사는 호출자가 했다.
    pub(crate) fn with_transport(
        endpoint: &str,
        model: String,
        secrets: SharedSecrets,
        retry: RetryPolicy,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_owned(),
            model,
            secrets,
            retry,
            transport,
        }
    }

    /// 고정한 모델 이름.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// `GET /v1/models`. 키가 맞는지와 고정한 모델이 목록에 있는지 본다.
    /// 목록에는 별칭만 오고 버전 고정 이름은 목록에 없어도 받으므로, 목록에 없다는 이유로는 실패하지 않는다.
    ///
    /// # Errors
    /// 인증 실패(키 없음, 거절)는 `Unauthorized`, 연결 실패는 `NoResponse`.
    pub async fn list_models(&self) -> Result<Vec<String>, JudgeError> {
        let url = format!("{}{MODELS_PATH}", self.endpoint);
        let reply = self
            .authorized(&url, None)
            .await
            .map_err(|failure| match failure {
                SendFailure::Rejected { status: 401, .. } => JudgeError::Unauthorized,
                _ => JudgeError::NoResponse,
            })?;
        let parsed: Value = serde_json::from_str(&reply).map_err(|_| JudgeError::NoResponse)?;
        Ok(parsed["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| model["name"].as_str().map(str::to_owned))
            .collect())
    }

    /// 요청과 응답 원문을 함께 돌려주는 판단 호출. 흐름:
    /// 1. `split_request`로 크기 한도에 맞게 나누고, 선택지가 255개를 넘는 질문은 `split_choices`로 계층 선택으로 나눈다.
    /// 2. 조각마다 `send_with_retry`로 보낸다. 한 조각이라도 실패하면 그 실패를 결과로 한다.
    /// 3. `merge_responses`로 합친다. 형식 검사는 호출자가 한다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        let started_at = SystemTime::now();
        let clock = Instant::now();
        let expanded = expand_choices(&request);
        let mut sent = Vec::new();
        let mut received = Vec::new();
        let mut parts = Vec::new();
        let mut failure = None;
        for part in split_request(expanded) {
            let (body, reply, result) = self.send_with_retry(&part).await;
            sent.push(body);
            received.extend(reply);
            match result {
                Ok(response) => parts.push(response),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let result = match failure {
            Some(error) => Err(error),
            None => Ok(merge_responses(&request, parts)),
        };
        JudgeExchange {
            sent: sent.join("\n"),
            received: (!received.is_empty()).then(|| received.join("\n")),
            result,
            started_at,
            elapsed: clock.elapsed(),
        }
    }

    /// 조각 하나를 보낸다. 보내기 전 실패만 `RetryPolicy::pre_send_attempts`번까지 다시 보내고,
    /// 속도 제한은 기다렸다가 다시 보낸다. 보낸 뒤 시간 초과는 다시 보내지 않는다.
    /// 돌려주는 값은 (보낸 본문, 받은 본문, 결과)다.
    async fn send_with_retry(
        &self,
        request: &JudgeRequest,
    ) -> (String, Option<String>, Result<JudgeResponse, JudgeError>) {
        let body = judge_body(request).to_string();
        let mut pre_send_failures = 0;
        let mut rate_limited = 0;
        loop {
            let result = self.send_once(&body).await;
            let error = match result {
                Ok(reply) => {
                    let parsed = parse_judge_reply(request, &reply);
                    return (body, Some(reply), parsed);
                }
                Err(SendFailure::BeforeSend) => {
                    pre_send_failures += 1;
                    if pre_send_failures > self.retry.pre_send_attempts {
                        JudgeError::NoResponse
                    } else {
                        continue;
                    }
                }
                Err(SendFailure::TimedOutAfterSend) => JudgeError::TimedOutAfterSend,
                Err(SendFailure::RateLimited { retry_after }) => {
                    rate_limited += 1;
                    if rate_limited > RATE_LIMIT_ATTEMPTS {
                        JudgeError::RateLimited
                    } else {
                        tokio::time::sleep(retry_after.unwrap_or(self.retry.rate_limit_wait)).await;
                        continue;
                    }
                }
                Err(SendFailure::Rejected {
                    status,
                    body: reply,
                }) => {
                    let error = match status {
                        401 | 403 => JudgeError::Unauthorized,
                        _ => JudgeError::Invalid {
                            reason: format!("judge returned status {status}"),
                        },
                    };
                    return (body, Some(reply), Err(error));
                }
            };
            return (body, None, Err(error));
        }
    }

    /// HTTPS POST 한 번. 인증 헤더는 요청 직전에 `SecretStore::key`로 붙이고 기록하지 않는다.
    /// 실패는 보내기 전 실패와 보낸 뒤 실패로 나눠 돌려준다.
    async fn send_once(&self, body: &str) -> Result<String, SendFailure> {
        let url = format!("{}{JUDGE_PATH}", self.endpoint);
        self.authorized(&url, Some(body.to_owned())).await
    }

    /// 인증 헤더를 붙여 한 번 보낸다. 3xx면 대상이 허용 주소일 때만 인증 헤더를 뺀 채 한 번 따른다.
    async fn authorized(&self, url: &str, body: Option<String>) -> Result<String, SendFailure> {
        validate_endpoint(url).map_err(|_| SendFailure::BeforeSend)?;
        let mut headers = vec![("content-type".to_owned(), "application/json".to_owned())];
        {
            let mut secrets = self.secrets.lock().await;
            let key =
                secrets
                    .key(std::time::Instant::now())
                    .map_err(|_| SendFailure::Rejected {
                        status: 401,
                        body: String::new(),
                    })?;
            headers.push((
                "authorization".to_owned(),
                format!("Bearer {}", key.expose()),
            ));
        }
        let timeout = self.retry.response_timeout;
        let reply = self
            .transport
            .send(url, headers.clone(), body.clone(), timeout)
            .await
            .map_err(SendFailure::from)?;
        let reply = match location(&reply) {
            Some(target) if validate_endpoint(&target).is_ok() => self
                .transport
                .send(&target, redirect_headers(headers), body, timeout)
                .await
                .map_err(SendFailure::from)?,
            _ => reply,
        };
        classify(reply)
    }
}

impl JudgeClient for RemoteJudge {
    /// `list_models`로 키와 모델을 확인한 뒤 실제 판단 1건을 보낸다.
    async fn check(&self) -> Result<(), JudgeError> {
        self.list_models().await?;
        let request = super::check_request(self.model.clone());
        self.exchange(request).await.result.map(|_| ())
    }

    /// `exchange`의 결과만 돌려준다.
    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        self.exchange(request).await.result
    }
}

/// 전송 한 번의 실패 구분. 재시도 판단에만 쓴다.
#[derive(Debug)]
enum SendFailure {
    /// 보내기 전에 실패했다. 전달되지 않은 것이 확정이라 다시 보내도 된다.
    BeforeSend,
    /// 보낸 뒤 응답 시간 초과. 다시 보내지 않는다.
    TimedOutAfterSend,
    /// 속도 제한. `retry_after`가 있으면 그만큼, 없으면 `RetryPolicy::rate_limit_wait`만큼 기다린다.
    RateLimited { retry_after: Option<Duration> },
    /// 응답은 받았으나 오류 상태다. 본문은 가리기 전 값이다.
    Rejected { status: u16, body: String },
}

impl From<TransportError> for SendFailure {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::BeforeSend => Self::BeforeSend,
            TransportError::AfterSend => Self::TimedOutAfterSend,
        }
    }
}

/// 상태 코드로 응답을 나눈다. 2xx만 본문을 돌려준다.
fn classify(reply: HttpReply) -> Result<String, SendFailure> {
    match reply.status {
        200..=299 => Ok(reply.body),
        429 | 529 => Err(SendFailure::RateLimited {
            retry_after: header(&reply, "retry-after")
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs),
        }),
        status => Err(SendFailure::Rejected {
            status,
            body: reply.body,
        }),
    }
}

/// 3xx 응답의 `location`.
fn location(reply: &HttpReply) -> Option<String> {
    (300..400)
        .contains(&reply.status)
        .then(|| header(reply, "location").map(str::to_owned))
        .flatten()
}

fn header<'a>(reply: &'a HttpReply, name: &str) -> Option<&'a str> {
    reply
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

/// 주소 검사. `https` 체계이고 호스트가 `ALLOWED_HOST`와 정확히 같아야 한다(하위 도메인, 포트 우회 불가).
///
/// # Errors
/// 조건을 어기면 `DisallowedEndpoint`.
pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), JudgesError> {
    let disallowed = || JudgesError::DisallowedEndpoint {
        endpoint: endpoint.to_owned(),
    };
    let rest = endpoint.strip_prefix("https://").ok_or_else(disallowed)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority != ALLOWED_HOST {
        return Err(disallowed());
    }
    Ok(())
}

/// 리다이렉트를 따를 때 넘길 헤더. `secrets::is_sensitive_header`인 헤더(Authorization 등)를 모두 뺀다.
/// 리다이렉트 대상이 `validate_endpoint`를 통과하지 못하면 호출자는 따르지 않는다.
pub(crate) fn redirect_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, _)| !is_sensitive_header(name))
        .collect()
}

/// 요청을 나눈다. 전체가 `REQUEST_SPLIT_LIMIT`를 넘거나 `state`와 가장 긴 질문의 합이 `STATE_SPLIT_LIMIT`를 넘으면
/// 질문 세트를 여러 요청에 나눠 담는다. 조각마다 `state`는 그대로 싣는다. 나눌 필요가 없으면 하나를 돌려준다.
/// 질문 하나와 `state`만으로 한도를 넘으면 그 질문 하나만 담아 보낸다(더 나눌 수 없다).
pub(crate) fn split_request(request: JudgeRequest) -> Vec<JudgeRequest> {
    let base = request.state.len() + request.model.len();
    let mut parts: Vec<JudgeRequest> = Vec::new();
    let mut size = base;
    let mut longest = 0;
    for (set, questions) in request.sets {
        for question in questions {
            let question_size = question_body(&question).to_string().len();
            let fits = !parts.is_empty()
                && size + question_size <= REQUEST_SPLIT_LIMIT
                && request.state.len() + longest.max(question_size) <= STATE_SPLIT_LIMIT;
            if !fits {
                parts.push(JudgeRequest {
                    model: request.model.clone(),
                    state: request.state.clone(),
                    sets: Vec::new(),
                });
                size = base;
                longest = 0;
            }
            let part = parts.last_mut().expect("a part was pushed above");
            match part.sets.last_mut() {
                Some((last, questions)) if *last == set => questions.push(question),
                _ => part.sets.push((set.clone(), vec![question])),
            }
            size += question_size;
            longest = longest.max(question_size);
        }
    }
    if parts.is_empty() {
        parts.push(JudgeRequest {
            model: request.model,
            state: request.state,
            sets: Vec::new(),
        });
    }
    parts
}

/// 선택지가 `MAX_CHOICES`를 넘는 `choice` 질문을 계층 선택으로 나눈다(묶음 고르기 → 묶음 안에서 고르기).
/// 넘지 않으면 그대로 하나를 돌려준다. TODO(#68): 계층 질문의 묶음 나누기와 2차 질문 방식 미정
/// 초안: 한 번의 요청으로 끝내려고 선택지를 `MAX_CHOICES - 1`개씩 나누고 조각마다 `none of these`를 더한
/// `choice` 질문(`<id>#<n>`)을 만든다. 합칠 때 조각의 `none of these`가 아닌 확률 질량으로 조각 사이 무게를 정한다.
pub(crate) fn split_choices(question: &Question) -> Vec<Question> {
    let AnswerKind::Choice { options } = &question.kind else {
        return vec![question.clone()];
    };
    if options.len() <= MAX_CHOICES {
        return vec![question.clone()];
    }
    options
        .chunks(MAX_CHOICES - 1)
        .enumerate()
        .map(|(index, chunk)| {
            let mut options = chunk.to_vec();
            options.push(OTHER_CHUNK_OPTION.to_owned());
            Question {
                id: format!("{}#{index}", question.id),
                text: question.text.clone(),
                kind: AnswerKind::Choice { options },
            }
        })
        .collect()
}

/// 조각 응답을 질문 id 순서대로 합치고 토큰을 더한다. 계층 선택은 원래 선택지 확률로 되돌린다.
pub(crate) fn merge_responses(request: &JudgeRequest, parts: Vec<JudgeResponse>) -> JudgeResponse {
    let model = parts
        .first()
        .map_or_else(|| request.model.clone(), |part| part.model.clone());
    let tokens = parts.iter().fold((0, 0), |(input, output), part| {
        (input + part.tokens.0, output + part.tokens.1)
    });
    let all: Vec<(String, Answer)> = parts.into_iter().flat_map(|part| part.answers).collect();
    let mut answers = Vec::new();
    for question in request.sets.iter().flat_map(|(_, questions)| questions) {
        let chunks = split_choices(question);
        if chunks.len() == 1 {
            if let Some((_, answer)) = all.iter().find(|(id, _)| *id == question.id) {
                answers.push((question.id.clone(), answer.clone()));
            }
            continue;
        }
        let pieces: Option<Vec<Vec<f64>>> = chunks
            .iter()
            .map(|chunk| match all.iter().find(|(id, _)| *id == chunk.id) {
                Some((_, Answer::Choice(probabilities))) => Some(probabilities.clone()),
                _ => None,
            })
            .collect();
        if let Some(pieces) = pieces {
            answers.push((question.id.clone(), Answer::Choice(combine_chunks(&pieces))));
        }
    }
    JudgeResponse {
        model,
        answers,
        tokens,
    }
}

/// 조각별 확률(마지막이 `none of these`)을 원래 선택지 분포로. 조각 무게는 `1 - P(none)`에 비례한다.
fn combine_chunks(pieces: &[Vec<f64>]) -> Vec<f64> {
    let weights: Vec<f64> = pieces
        .iter()
        .map(|piece| 1.0 - piece.last().copied().unwrap_or(0.0))
        .collect();
    let total: f64 = weights.iter().sum();
    let mut combined = Vec::new();
    for (piece, weight) in pieces.iter().zip(&weights) {
        let inside: f64 = piece[..piece.len() - 1].iter().sum();
        for probability in &piece[..piece.len() - 1] {
            let share = if inside > 0.0 {
                probability / inside
            } else {
                0.0
            };
            let scale = if total > 0.0 {
                weight / total
            } else {
                1.0 / pieces.len() as f64
            };
            combined.push(share * scale);
        }
    }
    combined
}

/// 선택지가 많은 질문을 조각 질문으로 바꾼 요청.
fn expand_choices(request: &JudgeRequest) -> JudgeRequest {
    JudgeRequest {
        model: request.model.clone(),
        state: request.state.clone(),
        sets: request
            .sets
            .iter()
            .map(|(set, questions)| {
                (
                    set.clone(),
                    questions.iter().flat_map(split_choices).collect(),
                )
            })
            .collect(),
    }
}

/// 요청 본문 JSON. 로컬 서버도 같은 본문을 쓴다.
pub(crate) fn judge_body(request: &JudgeRequest) -> Value {
    let questions: Map<String, Value> = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .map(|question| (question.id.clone(), question_body(question)))
        .collect();
    json!({ "model": request.model, "state": request.state, "questions": questions })
}

/// 질문 하나의 JSON. `score` 단계 설명은 질문에 없어 `level 1`..`level N`을 쓴다(초안).
fn question_body(question: &Question) -> Value {
    match &question.kind {
        AnswerKind::Noul => json!({ "type": "noul", "instructions": question.text }),
        AnswerKind::Choice { options } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|option| (option.clone(), Value::Null))
                .collect();
            json!({ "type": "choice", "instructions": question.text, "criteria": criteria })
        }
        AnswerKind::Score { levels } => {
            let criteria: Vec<String> = (1..=*levels)
                .map(|level| format!("level {level}"))
                .collect();
            json!({ "type": "score", "instructions": question.text, "criteria": criteria })
        }
    }
}

/// 응답 본문을 질문 순서의 답으로. 로컬 서버 응답도 같은 형식이다. 확률이 없거나 0~1 밖이면 `Invalid`. 질문 답 누락 검사는 `core::validate`가 한다.
pub(crate) fn parse_judge_reply(
    request: &JudgeRequest,
    body: &str,
) -> Result<JudgeResponse, JudgeError> {
    let invalid = |reason: &str| JudgeError::Invalid {
        reason: reason.to_owned(),
    };
    let parsed: Value = serde_json::from_str(body).map_err(|_| invalid("response is not json"))?;
    let mut answers = Vec::new();
    for question in request.sets.iter().flat_map(|(_, questions)| questions) {
        let Some(answer) = parsed["answers"].get(&question.id) else {
            continue;
        };
        answers.push((question.id.clone(), parse_answer(question, answer)?));
    }
    Ok(JudgeResponse {
        model: parsed["model"]
            .as_str()
            .unwrap_or(&request.model)
            .to_owned(),
        answers,
        tokens: (
            parsed["usage"]["input_tokens"].as_u64().unwrap_or(0),
            parsed["usage"]["output_tokens"].as_u64().unwrap_or(0),
        ),
    })
}

/// 답 하나.
fn parse_answer(question: &Question, answer: &Value) -> Result<Answer, JudgeError> {
    let invalid = |reason: String| JudgeError::Invalid { reason };
    let probability = |value: &Value, what: &str| {
        value
            .as_f64()
            .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
            .ok_or_else(|| invalid(format!("{}: missing or out of range {what}", question.id)))
    };
    match &question.kind {
        AnswerKind::Noul => Ok(Answer::Noul(probability(&answer["noul"], "noul")?)),
        AnswerKind::Choice { options } => {
            let probabilities = options
                .iter()
                .map(|option| probability(&answer["probabilities"][option], option))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Answer::Choice(probabilities))
        }
        AnswerKind::Score { levels } => {
            let probabilities = (0..*levels)
                .map(|level| {
                    let key = level.to_string();
                    probability(&answer["probabilities"][key.as_str()], &key)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Answer::Score(probabilities))
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    use saturn_core::judges::QuestionSetId;

    use super::*;
    use crate::secrets::{JudgeKey, KeySource, SecretStore, StorageMode};

    pub(crate) const KEY: &str = "sk-judge-test-0123456789";

    /// 가짜 전송이 받은 요청 하나(주소, 헤더, 본문).
    pub(crate) type Call = (String, Vec<(String, String)>, Option<String>);

    /// 기록한 응답을 차례로 돌려주는 가짜 전송. 받은 요청(주소, 헤더, 본문)을 남긴다.
    #[derive(Debug, Default)]
    pub(crate) struct FakeTransport {
        replies: StdMutex<VecDeque<Result<HttpReply, TransportError>>>,
        pub(crate) calls: StdMutex<Vec<Call>>,
    }

    impl FakeTransport {
        pub(crate) fn new(replies: Vec<Result<HttpReply, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                replies: StdMutex::new(replies.into()),
                calls: StdMutex::default(),
            })
        }

        pub(crate) fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Transport for FakeTransport {
        fn send<'a>(
            &'a self,
            url: &'a str,
            headers: Vec<(String, String)>,
            body: Option<String>,
            _timeout: Duration,
        ) -> TransportFuture<'a> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_owned(), headers, body));
            let reply = self
                .replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(TransportError::BeforeSend));
            Box::pin(async move { reply })
        }
    }

    pub(crate) fn ok(body: &str) -> Result<HttpReply, TransportError> {
        status(200, body, Vec::new())
    }

    pub(crate) fn status(
        code: u16,
        body: &str,
        headers: Vec<(&str, &str)>,
    ) -> Result<HttpReply, TransportError> {
        Ok(HttpReply {
            status: code,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            body: body.to_owned(),
        })
    }

    pub(crate) async fn secrets_with_key(dir: &std::path::Path) -> SharedSecrets {
        let mut store = SecretStore::with_key_file(dir.join("judge.key"), StorageMode::Standard);
        store
            .save(JudgeKey::new(KEY.to_owned()).unwrap(), KeySource::Stored)
            .await
            .unwrap();
        Arc::new(tokio::sync::Mutex::new(store))
    }

    pub(crate) fn judge(secrets: SharedSecrets, transport: Arc<FakeTransport>) -> RemoteJudge {
        let retry = RetryPolicy {
            pre_send_attempts: 2,
            response_timeout: Duration::from_secs(1),
            rate_limit_wait: Duration::from_millis(1),
        };
        RemoteJudge::with_transport(
            "https://api.typesafe.ai",
            "jev-1.13.0".to_owned(),
            secrets,
            retry,
            transport,
        )
    }

    pub(crate) fn request() -> JudgeRequest {
        JudgeRequest {
            model: "jev-1.13.0".to_owned(),
            state: "user: also translate the error message".to_owned(),
            sets: vec![(
                QuestionSetId {
                    name: "route".to_owned(),
                    major: 3,
                    minor: 1,
                },
                vec![
                    Question {
                        id: "keep_current".to_owned(),
                        text: "Does the input continue the running task?".to_owned(),
                        kind: AnswerKind::Noul,
                    },
                    Question {
                        id: "relation".to_owned(),
                        text: "How does the input relate to the running task?".to_owned(),
                        kind: AnswerKind::Choice {
                            options: vec!["refines".to_owned(), "independent".to_owned()],
                        },
                    },
                    Question {
                        id: "difficulty".to_owned(),
                        text: "How hard is the task?".to_owned(),
                        kind: AnswerKind::Score { levels: 3 },
                    },
                ],
            )],
        }
    }

    pub(crate) const ANSWER: &str = r#"{"model":"jev-1.13.0","answers":{
        "keep_current":{"type":"noul","noul":0.91},
        "relation":{"type":"choice","choice":"refines","probabilities":{"refines":0.8,"independent":0.2},"confidence":0.6},
        "difficulty":{"type":"score","score":0.4,"legend":{"0":"level 1","1":"level 2","2":"level 3"},"probabilities":{"0":0.6,"1":0.4,"2":0.0},"confidence":0.3}},
        "usage":{"input_tokens":310,"output_tokens":24}}"#;

    #[test]
    fn endpoint_must_be_https_allowed_host() {
        for good in [
            "https://api.typesafe.ai",
            "https://api.typesafe.ai/v1/systemone",
        ] {
            assert!(validate_endpoint(good).is_ok(), "{good}");
        }
        for bad in [
            "http://api.typesafe.ai",
            "https://api.typesafe.ai.evil.com",
            "https://evil.api.typesafe.ai",
            "https://api.typesafe.ai:8443",
            "https://user@api.typesafe.ai",
            "ftp://api.typesafe.ai",
        ] {
            assert!(validate_endpoint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn redirect_drops_auth_headers() {
        let headers = vec![
            ("authorization".to_owned(), "Bearer x".to_owned()),
            ("X-Api-Key".to_owned(), "y".to_owned()),
            ("content-type".to_owned(), "application/json".to_owned()),
        ];

        assert_eq!(
            redirect_headers(headers),
            vec![("content-type".to_owned(), "application/json".to_owned())]
        );
    }

    #[tokio::test]
    async fn exchange_sends_bearer_and_parses_all_answer_kinds() {
        let dir = tempfile::tempdir().unwrap();
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let judge = judge(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let exchange = judge.exchange(request()).await;

        let response = exchange.result.unwrap();
        assert_eq!(response.tokens, (310, 24));
        assert_eq!(
            response.answers,
            vec![
                ("keep_current".to_owned(), Answer::Noul(0.91)),
                ("relation".to_owned(), Answer::Choice(vec![0.8, 0.2])),
                ("difficulty".to_owned(), Answer::Score(vec![0.6, 0.4, 0.0])),
            ]
        );
        let calls = transport.calls();
        assert_eq!(calls[0].0, "https://api.typesafe.ai/v1/systemone");
        assert!(
            calls[0]
                .1
                .contains(&("authorization".to_owned(), format!("Bearer {KEY}")))
        );
        let body: Value = serde_json::from_str(calls[0].2.as_deref().unwrap()).unwrap();
        assert_eq!(
            body["questions"]["relation"]["criteria"],
            json!({ "refines": null, "independent": null })
        );
        assert_eq!(
            body["questions"]["difficulty"]["criteria"],
            json!(["level 1", "level 2", "level 3"])
        );
        assert!(!exchange.sent.contains(KEY));
        assert!(!format!("{judge:?}").contains(KEY));
    }

    #[tokio::test]
    async fn only_pre_send_failures_are_retried() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let retried = FakeTransport::new(vec![
            Err(TransportError::BeforeSend),
            Err(TransportError::BeforeSend),
            ok(ANSWER),
        ]);
        let timed_out = FakeTransport::new(vec![Err(TransportError::AfterSend), ok(ANSWER)]);
        let gave_up = FakeTransport::new(vec![Err(TransportError::BeforeSend); 3]);

        let first = judge(Arc::clone(&secrets), Arc::clone(&retried))
            .exchange(request())
            .await;
        let second = judge(Arc::clone(&secrets), Arc::clone(&timed_out))
            .exchange(request())
            .await;
        let third = judge(Arc::clone(&secrets), Arc::clone(&gave_up))
            .exchange(request())
            .await;

        assert!(first.result.is_ok());
        assert_eq!(retried.calls().len(), 3);
        assert!(matches!(second.result, Err(JudgeError::TimedOutAfterSend)));
        assert_eq!(timed_out.calls().len(), 1);
        assert!(second.received.is_none());
        assert!(matches!(third.result, Err(JudgeError::NoResponse)));
        assert_eq!(gave_up.calls().len(), 3);
    }

    #[tokio::test]
    async fn rate_limit_waits_and_auth_failure_is_unauthorized() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let limited = FakeTransport::new(vec![
            status(429, "{}", vec![("retry-after", "0")]),
            status(529, "{}", Vec::new()),
            ok(ANSWER),
        ]);
        let rejected = FakeTransport::new(vec![status(401, "{\"error\":\"bad key\"}", Vec::new())]);

        let first = judge(Arc::clone(&secrets), Arc::clone(&limited))
            .exchange(request())
            .await;
        let second = judge(Arc::clone(&secrets), rejected)
            .exchange(request())
            .await;

        assert!(first.result.is_ok());
        assert_eq!(limited.calls().len(), 3);
        assert!(matches!(second.result, Err(JudgeError::Unauthorized)));
    }

    #[tokio::test]
    async fn bad_probabilities_are_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"model":"jev-1.13.0","answers":{"keep_current":{"noul":1.7}},"usage":{}}"#;
        let transport = FakeTransport::new(vec![ok(body)]);
        let judge = judge(secrets_with_key(dir.path()).await, transport);

        let exchange = judge.exchange(request()).await;

        assert!(matches!(exchange.result, Err(JudgeError::Invalid { .. })));
        assert_eq!(exchange.received.as_deref(), Some(body));
    }

    #[tokio::test]
    async fn redirect_is_followed_only_to_allowed_host_without_auth() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let allowed = FakeTransport::new(vec![
            status(
                307,
                "",
                vec![("location", "https://api.typesafe.ai/v1/systemone2")],
            ),
            ok(ANSWER),
        ]);
        let outside = FakeTransport::new(vec![status(
            307,
            "",
            vec![("location", "https://evil.example/steal")],
        )]);

        judge(Arc::clone(&secrets), Arc::clone(&allowed))
            .exchange(request())
            .await;
        let refused = judge(secrets, Arc::clone(&outside))
            .exchange(request())
            .await;

        let calls = allowed.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1.iter().all(|(name, _)| name != "authorization"));
        assert_eq!(outside.calls().len(), 1);
        assert!(matches!(refused.result, Err(JudgeError::Invalid { .. })));
    }

    #[tokio::test]
    async fn check_lists_models_then_judges_once() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let good = FakeTransport::new(vec![
            ok(r#"{"models":[{"name":"jev-latest"}]}"#),
            ok(
                r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"type":"noul","noul":0.7}},"usage":{"input_tokens":5,"output_tokens":1}}"#,
            ),
        ]);
        let bad_key = FakeTransport::new(vec![status(401, "{}", Vec::new())]);

        judge(Arc::clone(&secrets), Arc::clone(&good))
            .check()
            .await
            .unwrap();
        let error = judge(secrets, bad_key).check().await.unwrap_err();

        let calls = good.calls();
        assert_eq!(calls[0].0, "https://api.typesafe.ai/v1/models");
        assert!(calls[0].2.is_none());
        assert!(matches!(error, JudgeError::Unauthorized));
    }

    #[tokio::test]
    async fn missing_key_is_unauthorized_without_sending() {
        let dir = tempfile::tempdir().unwrap();
        let empty = SecretStore::with_key_file(dir.path().join("none.key"), StorageMode::Standard);
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let judge = judge(
            Arc::new(tokio::sync::Mutex::new(empty)),
            Arc::clone(&transport),
        );

        let exchange = judge.exchange(request()).await;

        assert!(matches!(exchange.result, Err(JudgeError::Unauthorized)));
        assert!(transport.calls().is_empty());
    }

    #[test]
    fn large_requests_are_split_by_question() {
        let mut big = request();
        big.state = "s".repeat(20 * 1024);
        let long_text = "q".repeat(15 * 1024);
        for question in &mut big.sets[0].1 {
            question.text = long_text.clone();
        }

        let parts = split_request(big);
        let small = split_request(request());

        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|part| part.state.len() == 20 * 1024));
        assert_eq!(parts[2].sets[0].1[0].id, "difficulty");
        assert_eq!(small.len(), 1);
        assert_eq!(small[0].sets[0].1.len(), 3);
    }

    #[test]
    fn many_choices_split_and_merge_back() {
        let options: Vec<String> = (0..600).map(|n| format!("model-{n}")).collect();
        let question = Question {
            id: "target_model".to_owned(),
            text: "Which model?".to_owned(),
            kind: AnswerKind::Choice { options },
        };
        let chunks = split_choices(&question);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|chunk| matches!(&chunk.kind, AnswerKind::Choice { options } if options.len() <= MAX_CHOICES)));
        let mut request = request();
        request.sets[0].1 = vec![question];
        let answer = |id: &str, len: usize, hot: Option<usize>| {
            let mut probabilities = vec![0.0; len];
            match hot {
                Some(index) => probabilities[index] = 1.0,
                None => probabilities[len - 1] = 1.0,
            }
            (id.to_owned(), Answer::Choice(probabilities))
        };
        let part = JudgeResponse {
            model: "jev-1.13.0".to_owned(),
            answers: vec![
                answer("target_model#0", 255, None),
                answer("target_model#1", 255, Some(3)),
                answer("target_model#2", 93, None),
            ],
            tokens: (10, 1),
        };

        let merged = merge_responses(&request, vec![part]);

        let Answer::Choice(probabilities) = &merged.answers[0].1 else {
            panic!("choice answer expected");
        };
        assert_eq!(probabilities.len(), 600);
        assert!((probabilities[254 + 3] - 1.0).abs() < 1e-9);
        assert!((probabilities.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }
}
