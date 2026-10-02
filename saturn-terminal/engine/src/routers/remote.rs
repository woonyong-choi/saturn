//! 외부 router API 연결. HTTPS와 허용 호스트만 쓰고 TLS 검증을 끄는 옵션은 두지 않는다.
//! 설계: docs/design/router.md

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use saturn_core::routers::failure::{remaining_until_deadline, retry_delay};
use saturn_core::routers::{
    Answer, AnswerKind, Question, RouterClient, RouterError, RouterRequest, RouterResponse,
};
use serde_json::{Map, Value, json};
use tokio::time::Instant as TokioInstant;

use super::{RouterExchange, RoutersError, SharedSecrets};
use crate::secrets::is_sensitive_header;

/// router 키는 이 호스트로만 간다.
pub const ALLOWED_HOST: &str = "api.typesafe.ai";

/// 토큰을 셀 수 없어 본문 바이트로 잰다(바이트 수 ≥ 토큰 수라 API 한도 64k 토큰을 넘지 않는다). 초안 값.
pub const REQUEST_SPLIT_LIMIT: usize = 64 * 1024;

/// `state`와 가장 긴 질문의 합 한도(바이트). 초안 값.
pub const STATE_SPLIT_LIMIT: usize = 32 * 1024;

/// 넘으면 계층 선택으로 나눈다.
pub const MAX_CHOICES: usize = 255;

const ROUTER_PATH: &str = "/v1/systemone";

const MODELS_PATH: &str = "/v1/models";

/// 계층 선택 조각의 "이 조각에 없음" 선택지 이름.
const OTHER_CHUNK_OPTION: &str = "none of these";

/// 재시도 횟수와 간격은 `saturn_core::routers::failure`가 정한다.
/// TODO(#235): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 시도마다(첫 시도 포함) 보낸 뒤 이 시간이 지나면 `TimedOutAfterSend`.
    pub response_timeout: Duration,
}

impl Default for RetryPolicy {
    /// 초안 값.
    fn default() -> Self {
        Self {
            response_timeout: Duration::from_secs(5),
        }
    }
}

/// 헤더 이름은 소문자.
#[derive(Clone)]
pub(crate) struct HttpReply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

impl Debug for HttpReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpReply")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportError {
    /// 연결을 열지 못해 요청이 나가지 않았다.
    BeforeSend,
    /// 보낸 뒤 응답 시간을 넘겼거나 응답 중 끊겼다.
    AfterSend,
}

pub(crate) type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HttpReply, TransportError>> + Send + 'a>>;

/// 헤더는 이 안에서만 다루고 기록하지 않는다.
pub(crate) trait Transport: Debug + Send + Sync {
    /// `body`가 `None`이면 GET, 있으면 POST. 리다이렉트는 따르지 않는다.
    fn send<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Option<String>,
        timeout: Duration,
    ) -> TransportFuture<'a>;
}

/// HTTPS만, 자동 리다이렉트 끔, TLS 검증은 기본값 그대로 둔다.
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

/// 연결 실패만 보내기 전 실패로 본다.
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

/// `Debug`는 키 보관소와 전송을 빼고 주소, 모델, 재시도 설정만 쓴다.
pub struct RemoteRouter {
    endpoint: String,
    /// 버전을 고정한 이름.
    model: String,
    /// 요청 직전에만 키를 빌린다.
    secrets: SharedSecrets,
    retry: RetryPolicy,
    /// 리다이렉트 때 인증 헤더 제거, TLS 검증 고정.
    transport: Arc<dyn Transport>,
}

impl Debug for RemoteRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteRouter")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl RemoteRouter {
    /// 아직 네트워크를 쓰지 않는다.
    ///
    /// # Errors
    /// 주소가 HTTPS가 아니거나 호스트가 `ALLOWED_HOST`가 아니면 `DisallowedEndpoint`.
    pub fn new(
        endpoint: &str,
        model: String,
        secrets: SharedSecrets,
        retry: RetryPolicy,
    ) -> Result<Self, RoutersError> {
        validate_endpoint(endpoint)?;
        let transport = ReqwestTransport::new().map_err(|_| RoutersError::DisallowedEndpoint {
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

    /// 주소 검사는 호출자가 했다.
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

    pub fn model(&self) -> &str {
        &self.model
    }

    /// 별칭만 목록에 오고 버전 고정 이름은 없어도 받으므로, 목록에 없다는 이유로는 실패하지 않는다.
    ///
    /// # Errors
    /// 인증 실패(키 없음, 거절)는 `Unauthorized`, 연결 실패는 `NoResponse`.
    pub async fn list_models(&self) -> Result<Vec<String>, RouterError> {
        let url = format!("{}{MODELS_PATH}", self.endpoint);
        let reply = self
            .authorized(&url, None)
            .await
            .map_err(|failure| match failure {
                SendFailure::Rejected { status: 401, .. } => RouterError::Unauthorized,
                _ => RouterError::NoResponse,
            })?;
        let parsed: Value = serde_json::from_str(&reply).map_err(|_| RouterError::NoResponse)?;
        Ok(parsed["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|model| model["name"].as_str().map(str::to_owned))
            .collect())
    }

    /// 한 조각이라도 실패하면 그 실패를 결과로 한다. 형식 검사는 호출자가 한다.
    pub async fn exchange(&self, request: RouterRequest) -> RouterExchange {
        let started_at = SystemTime::now();
        let clock = Instant::now();
        let expanded = expand_choices(&request);
        let mut sent = Vec::new();
        let mut received = Vec::new();
        let mut parts = Vec::new();
        let mut failure = None;
        let mut unknown_cost_calls = 0;
        for part in split_request(expanded) {
            let attempt = self.send_with_retry(&part).await;
            sent.push(attempt.body);
            received.extend(attempt.reply);
            unknown_cost_calls += attempt.unknown_cost_calls;
            match attempt.result {
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
        RouterExchange {
            sent: sent.join("\n"),
            received: (!received.is_empty()).then(|| received.join("\n")),
            result,
            started_at,
            elapsed: clock.elapsed(),
            unknown_cost_calls,
        }
    }

    // cost: time O(r·t), heap O(b), stack O(1), io r
    // vars: r = 시도 수(최대 3), t = 시도 하나의 응답 대기(5초), b = 요청과 응답 본문 크기
    // basis: estimate
    /// 보내기 전 실패, 응답 없음, 속도 제한은 5초 간격으로 두 번까지 다시 보낸다. router 판단은 부작용이 없는 조회라 보낸 뒤 시간 초과도 다시 보내고, 그 호출의 비용은 모른다고 센다.
    ///
    /// 첫 실패 시각부터 10초가 전체 마감이다. 시도는 응답을 5초까지 기다리되 마감까지 남은 시간을 넘기지 않고, 마감에 걸린 시도는 끊는다.
    async fn send_with_retry(&self, request: &RouterRequest) -> SentPart {
        let body = router_body(request).to_string();
        let mut failures = 0;
        let mut first_failure = None;
        let mut unknown_cost_calls = 0;
        loop {
            let since_first_failure =
                first_failure.map_or(Duration::ZERO, |at: TokioInstant| at.elapsed());
            let wait = self
                .retry
                .response_timeout
                .min(remaining_until_deadline(since_first_failure));
            let outcome = tokio::time::timeout(wait, self.send_once(&body))
                .await
                .unwrap_or(Err(SendFailure::TimedOutAfterSend));
            let error = match outcome {
                Ok(reply) => {
                    let result = parse_router_reply(request, &reply);
                    return SentPart::new(body, Some(reply), result, unknown_cost_calls);
                }
                Err(SendFailure::Rejected {
                    status,
                    body: reply,
                }) => {
                    let error = match status {
                        401 | 403 => RouterError::Unauthorized,
                        _ => RouterError::Invalid {
                            reason: format!("router returned status {status}"),
                        },
                    };
                    return SentPart::new(body, Some(reply), Err(error), unknown_cost_calls);
                }
                Err(SendFailure::BeforeSend) => RouterError::NoResponse,
                Err(SendFailure::TimedOutAfterSend) => {
                    unknown_cost_calls += 1;
                    RouterError::TimedOutAfterSend
                }
                Err(SendFailure::RateLimited) => RouterError::RateLimited,
            };
            failures += 1;
            let first = *first_failure.get_or_insert_with(TokioInstant::now);
            let Some(delay) = retry_delay(failures, first.elapsed()) else {
                return SentPart::new(body, None, Err(error), unknown_cost_calls);
            };
            tracing::warn!(failures, error = %error, "router call failed, retrying");
            tokio::time::sleep(delay).await;
        }
    }

    /// 인증 헤더는 요청 직전에 붙이고 기록하지 않는다.
    async fn send_once(&self, body: &str) -> Result<String, SendFailure> {
        let url = format!("{}{ROUTER_PATH}", self.endpoint);
        self.authorized(&url, Some(body.to_owned())).await
    }

    /// 3xx면 대상이 허용 주소일 때만 인증 헤더를 뺀 채 한 번 따른다.
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

impl RouterClient for RemoteRouter {
    /// 키와 모델을 확인한 뒤 실제 판단 1건을 보낸다.
    async fn check(&self) -> Result<(), RouterError> {
        self.list_models().await?;
        let request = super::check_request(self.model.clone());
        self.exchange(request).await.result.map(|_| ())
    }

    async fn router(&self, request: RouterRequest) -> Result<RouterResponse, RouterError> {
        self.exchange(request).await.result
    }
}

/// 조각 하나를 재시도까지 마친 결과.
struct SentPart {
    body: String,
    reply: Option<String>,
    result: Result<RouterResponse, RouterError>,
    /// 보낸 뒤 시간 초과로 비용을 모르는 호출 수.
    unknown_cost_calls: u32,
}

impl SentPart {
    fn new(
        body: String,
        reply: Option<String>,
        result: Result<RouterResponse, RouterError>,
        unknown_cost_calls: u32,
    ) -> Self {
        Self {
            body,
            reply,
            result,
            unknown_cost_calls,
        }
    }
}

/// 재시도 판단에만 쓴다.
enum SendFailure {
    /// 전달되지 않은 것이 확정이다.
    BeforeSend,
    /// 이미 처리됐을 수 있다. 판단은 조회라 다시 보낸다.
    TimedOutAfterSend,
    /// 429, 529. 기다림은 응답의 `retry-after`와 무관하게 재시도 간격으로 통일한다.
    RateLimited,
    /// 본문은 가리기 전 값이다.
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

/// 2xx만 본문을 돌려준다.
fn classify(reply: HttpReply) -> Result<String, SendFailure> {
    match reply.status {
        200..=299 => Ok(reply.body),
        429 | 529 => Err(SendFailure::RateLimited),
        status => Err(SendFailure::Rejected {
            status,
            body: reply.body,
        }),
    }
}

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

/// 호스트가 `ALLOWED_HOST`와 정확히 같아야 한다(하위 도메인, 포트 우회 불가).
///
/// # Errors
/// 조건을 어기면 `DisallowedEndpoint`.
pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), RoutersError> {
    let disallowed = || RoutersError::DisallowedEndpoint {
        endpoint: endpoint.to_owned(),
    };
    let rest = endpoint.strip_prefix("https://").ok_or_else(disallowed)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority != ALLOWED_HOST {
        return Err(disallowed());
    }
    Ok(())
}

/// 대상이 `validate_endpoint`를 통과하지 못하면 호출자는 따르지 않는다.
pub(crate) fn redirect_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, _)| !is_sensitive_header(name))
        .collect()
}

/// 조각마다 `state`는 그대로 싣고, 질문 하나와 `state`만으로 한도를 넘으면 그 질문만 담아 보낸다.
pub(crate) fn split_request(request: RouterRequest) -> Vec<RouterRequest> {
    let base = request.state.len() + request.model.len();
    let mut parts: Vec<RouterRequest> = Vec::new();
    let mut size = base;
    let mut longest = 0;
    for (set, questions) in request.sets {
        for question in questions {
            let question_size = question_body(&question).to_string().len();
            let fits = !parts.is_empty()
                && size + question_size <= REQUEST_SPLIT_LIMIT
                && request.state.len() + longest.max(question_size) <= STATE_SPLIT_LIMIT;
            if !fits {
                parts.push(RouterRequest {
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
        parts.push(RouterRequest {
            model: request.model,
            state: request.state,
            sets: Vec::new(),
        });
    }
    parts
}

/// 조각 사이 무게는 조각의 `none of these`가 아닌 확률 질량으로 정한다. 초안 방식.
/// TODO(#68): 계층 질문의 묶음 나누기와 2차 질문 방식 미정
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

/// 계층 선택은 원래 선택지 확률로 되돌린다.
pub(crate) fn merge_responses(
    request: &RouterRequest,
    parts: Vec<RouterResponse>,
) -> RouterResponse {
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
    RouterResponse {
        model,
        answers,
        tokens,
    }
}

/// 조각 무게는 `1 - P(none)`에 비례한다.
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

fn expand_choices(request: &RouterRequest) -> RouterRequest {
    RouterRequest {
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

/// 로컬 서버도 같은 본문을 쓴다.
pub(crate) fn router_body(request: &RouterRequest) -> Value {
    let questions: Map<String, Value> = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .map(|question| (question.id.clone(), question_body(question)))
        .collect();
    json!({ "model": request.model, "state": request.state, "questions": questions })
}

/// `score` 단계 설명은 질문에 없어 `level 1`..`level N`을 쓴다. 초안.
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

/// 확률이 없거나 0~1 밖이면 `Invalid`. 답 누락 검사는 `core::validate`가 한다.
pub(crate) fn parse_router_reply(
    request: &RouterRequest,
    body: &str,
) -> Result<RouterResponse, RouterError> {
    let invalid = |reason: &str| RouterError::Invalid {
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
    Ok(RouterResponse {
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

fn parse_answer(question: &Question, answer: &Value) -> Result<Answer, RouterError> {
    let invalid = |reason: String| RouterError::Invalid { reason };
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

    use saturn_core::routers::QuestionSetId;

    use super::*;
    use crate::secrets::{KeySource, RouterKey, SecretStore, StorageMode};

    pub(crate) const KEY: &str = "sk-router-test-0123456789";

    /// 주소, 헤더, 본문.
    pub(crate) type Call = (String, Vec<(String, String)>, Option<String>);

    /// 기록한 응답을 차례로 돌려주고 받은 요청을 남긴다.
    #[derive(Debug, Default)]
    pub(crate) struct FakeTransport {
        replies: StdMutex<VecDeque<Result<HttpReply, TransportError>>>,
        pub(crate) calls: StdMutex<Vec<Call>>,
        /// 다음 호출 하나가 열릴 때까지 답을 미룬다.
        gate: StdMutex<Option<Arc<tokio::sync::Notify>>>,
    }

    impl FakeTransport {
        pub(crate) fn new(replies: Vec<Result<HttpReply, TransportError>>) -> Arc<Self> {
            Arc::new(Self {
                replies: StdMutex::new(replies.into()),
                calls: StdMutex::default(),
                gate: StdMutex::default(),
            })
        }

        /// 다음 호출은 돌려받은 `Notify`에 `notify_one`을 부를 때까지 답하지 않는다.
        pub(crate) fn hold_next_call(&self) -> Arc<tokio::sync::Notify> {
            let gate = Arc::new(tokio::sync::Notify::new());
            *self.gate.lock().unwrap() = Some(Arc::clone(&gate));
            gate
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
            let gate = self.gate.lock().unwrap().take();
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                reply
            })
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

    #[test]
    fn reply_debug_hides_body_and_headers() {
        let reply = status(401, KEY, vec![("authorization", KEY)]).unwrap();
        let debug = format!("{reply:?}");
        assert!(!debug.contains(KEY));
        assert!(debug.contains("body_len"));
    }

    pub(crate) async fn secrets_with_key(dir: &std::path::Path) -> SharedSecrets {
        let mut store = SecretStore::with_key_file(dir.join("router.key"), StorageMode::Standard);
        store
            .save(RouterKey::new(KEY.to_owned()).unwrap(), KeySource::Stored)
            .await
            .unwrap();
        Arc::new(tokio::sync::Mutex::new(store))
    }

    pub(crate) fn router(secrets: SharedSecrets, transport: Arc<FakeTransport>) -> RemoteRouter {
        let retry = RetryPolicy {
            response_timeout: Duration::from_secs(1),
        };
        RemoteRouter::with_transport(
            "https://api.typesafe.ai",
            "jev-1.13.0".to_owned(),
            secrets,
            retry,
            transport,
        )
    }

    pub(crate) fn request() -> RouterRequest {
        RouterRequest {
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
        let router = router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let exchange = router.exchange(request()).await;

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
        assert!(!format!("{router:?}").contains(KEY));
    }

    #[tokio::test(start_paused = true)]
    async fn failures_retry_twice_five_seconds_apart_then_give_up() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let recovered = FakeTransport::new(vec![
            Err(TransportError::BeforeSend),
            Err(TransportError::BeforeSend),
            ok(ANSWER),
        ]);
        let gave_up = FakeTransport::new(vec![Err(TransportError::BeforeSend); 3]);

        let started = tokio::time::Instant::now();
        let first = router(Arc::clone(&secrets), Arc::clone(&recovered))
            .exchange(request())
            .await;
        let after_recovery = started.elapsed();
        let second = router(Arc::clone(&secrets), Arc::clone(&gave_up))
            .exchange(request())
            .await;
        let after_give_up = started.elapsed() - after_recovery;

        assert!(first.result.is_ok());
        assert_eq!(recovered.calls().len(), 3);
        assert_eq!(after_recovery, Duration::from_secs(10));
        assert!(matches!(second.result, Err(RouterError::NoResponse)));
        assert_eq!(gave_up.calls().len(), 3);
        assert_eq!(after_give_up, Duration::from_secs(10));
    }

    #[tokio::test(start_paused = true)]
    async fn first_retry_waits_five_seconds_before_sending() {
        let dir = tempfile::tempdir().unwrap();
        let transport = FakeTransport::new(vec![Err(TransportError::BeforeSend), ok(ANSWER)]);
        let router = router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let started = tokio::time::Instant::now();
        let exchange = router.exchange(request()).await;

        assert!(exchange.result.is_ok());
        assert_eq!(transport.calls().len(), 2);
        assert_eq!(started.elapsed(), Duration::from_secs(5));
    }

    /// 답을 정한 순서대로 주되 `None`인 호출은 끝내 응답하지 않는다. 호출 시각을 남긴다.
    #[derive(Debug)]
    struct ScriptedTransport {
        script: StdMutex<VecDeque<Option<Result<HttpReply, TransportError>>>>,
        started: tokio::time::Instant,
        called_at: StdMutex<Vec<Duration>>,
    }

    impl ScriptedTransport {
        fn new(script: Vec<Option<Result<HttpReply, TransportError>>>) -> Arc<Self> {
            Arc::new(Self {
                script: StdMutex::new(script.into()),
                started: tokio::time::Instant::now(),
                called_at: StdMutex::default(),
            })
        }

        fn called_at_seconds(&self) -> Vec<u64> {
            let times = self.called_at.lock().unwrap();
            times.iter().map(Duration::as_secs).collect()
        }
    }

    impl Transport for ScriptedTransport {
        fn send<'a>(
            &'a self,
            _url: &'a str,
            _headers: Vec<(String, String)>,
            _body: Option<String>,
            _timeout: Duration,
        ) -> TransportFuture<'a> {
            self.called_at.lock().unwrap().push(self.started.elapsed());
            let step = self.script.lock().unwrap().pop_front().flatten();
            Box::pin(async move {
                match step {
                    Some(reply) => reply,
                    None => std::future::pending().await,
                }
            })
        }
    }

    fn scripted_router(secrets: SharedSecrets, transport: Arc<ScriptedTransport>) -> RemoteRouter {
        RemoteRouter::with_transport(
            "https://api.typesafe.ai",
            "jev-1.13.0".to_owned(),
            secrets,
            RetryPolicy::default(),
            transport,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn no_response_at_all_gives_up_within_fifteen_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let transport = ScriptedTransport::new(vec![None, None, None]);
        let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let started = tokio::time::Instant::now();
        let exchange = router.exchange(request()).await;

        assert!(matches!(
            exchange.result,
            Err(RouterError::TimedOutAfterSend)
        ));
        assert_eq!(started.elapsed(), Duration::from_secs(15));
        assert_eq!(transport.called_at_seconds(), vec![0, 10]);
        assert_eq!(exchange.unknown_cost_calls, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn immediate_failure_retries_at_five_and_ten_seconds_then_gives_up_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let transport = ScriptedTransport::new(vec![
            Some(Err(TransportError::BeforeSend)),
            Some(Err(TransportError::BeforeSend)),
            None,
        ]);
        let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let started = tokio::time::Instant::now();
        let exchange = router.exchange(request()).await;

        assert!(matches!(
            exchange.result,
            Err(RouterError::TimedOutAfterSend)
        ));
        assert_eq!(started.elapsed(), Duration::from_secs(10));
        assert_eq!(transport.called_at_seconds(), vec![0, 5, 10]);
        assert_eq!(exchange.unknown_cost_calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn attempt_started_near_the_deadline_waits_only_the_remaining_time() {
        let dir = tempfile::tempdir().unwrap();
        let transport =
            ScriptedTransport::new(vec![Some(Err(TransportError::BeforeSend)), None, None]);
        let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let started = tokio::time::Instant::now();
        let exchange = router.exchange(request()).await;

        assert!(matches!(
            exchange.result,
            Err(RouterError::TimedOutAfterSend)
        ));
        assert_eq!(started.elapsed(), Duration::from_secs(10));
        assert_eq!(transport.called_at_seconds(), vec![0, 5]);
        assert_eq!(exchange.unknown_cost_calls, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn attempt_after_a_slow_first_failure_is_cut_at_the_new_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let transport =
            ScriptedTransport::new(vec![None, Some(Err(TransportError::BeforeSend)), None]);
        let router = scripted_router(secrets_with_key(dir.path()).await, Arc::clone(&transport));

        let started = tokio::time::Instant::now();
        let exchange = router.exchange(request()).await;

        assert!(matches!(
            exchange.result,
            Err(RouterError::TimedOutAfterSend)
        ));
        assert_eq!(started.elapsed(), Duration::from_secs(15));
        assert_eq!(transport.called_at_seconds(), vec![0, 10, 15]);
        assert_eq!(exchange.unknown_cost_calls, 2);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_after_send_is_retried_and_counted_as_unknown_cost() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let recovered = FakeTransport::new(vec![Err(TransportError::AfterSend), ok(ANSWER)]);
        let timed_out = FakeTransport::new(vec![Err(TransportError::AfterSend); 3]);

        let first = router(Arc::clone(&secrets), Arc::clone(&recovered))
            .exchange(request())
            .await;
        let second = router(secrets, Arc::clone(&timed_out))
            .exchange(request())
            .await;

        assert!(first.result.is_ok());
        assert_eq!(first.unknown_cost_calls, 1);
        assert_eq!(recovered.calls().len(), 2);
        assert!(matches!(second.result, Err(RouterError::TimedOutAfterSend)));
        assert_eq!(second.unknown_cost_calls, 3);
        assert_eq!(timed_out.calls().len(), 3);
        assert!(second.received.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_retries_on_the_same_interval_ignoring_retry_after() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let limited = FakeTransport::new(vec![
            status(429, "{}", vec![("retry-after", "60")]),
            status(529, "{}", Vec::new()),
            ok(ANSWER),
        ]);
        let exhausted = FakeTransport::new(vec![status(429, "{}", Vec::new()); 3]);

        let started = tokio::time::Instant::now();
        let first = router(Arc::clone(&secrets), Arc::clone(&limited))
            .exchange(request())
            .await;
        let after_recovery = started.elapsed();
        let second = router(secrets, Arc::clone(&exhausted))
            .exchange(request())
            .await;

        assert!(first.result.is_ok());
        assert_eq!(limited.calls().len(), 3);
        assert_eq!(after_recovery, Duration::from_secs(10));
        assert!(matches!(second.result, Err(RouterError::RateLimited)));
        assert_eq!(exhausted.calls().len(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn auth_failure_and_invalid_status_are_not_retried() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let rejected = FakeTransport::new(vec![status(401, "{\"error\":\"bad key\"}", Vec::new())]);
        let broken = FakeTransport::new(vec![status(500, "{}", Vec::new())]);

        let first = router(Arc::clone(&secrets), Arc::clone(&rejected))
            .exchange(request())
            .await;
        let second = router(secrets, Arc::clone(&broken))
            .exchange(request())
            .await;

        assert!(matches!(first.result, Err(RouterError::Unauthorized)));
        assert_eq!(rejected.calls().len(), 1);
        assert!(matches!(second.result, Err(RouterError::Invalid { .. })));
        assert_eq!(broken.calls().len(), 1);
    }

    #[tokio::test]
    async fn bad_probabilities_are_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"model":"jev-1.13.0","answers":{"keep_current":{"noul":1.7}},"usage":{}}"#;
        let transport = FakeTransport::new(vec![ok(body)]);
        let router = router(secrets_with_key(dir.path()).await, transport);

        let exchange = router.exchange(request()).await;

        assert!(matches!(exchange.result, Err(RouterError::Invalid { .. })));
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

        router(Arc::clone(&secrets), Arc::clone(&allowed))
            .exchange(request())
            .await;
        let refused = router(secrets, Arc::clone(&outside))
            .exchange(request())
            .await;

        let calls = allowed.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1.iter().all(|(name, _)| name != "authorization"));
        assert_eq!(outside.calls().len(), 1);
        assert!(matches!(refused.result, Err(RouterError::Invalid { .. })));
    }

    #[tokio::test]
    async fn check_lists_models_then_routers_once() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let good = FakeTransport::new(vec![
            ok(r#"{"models":[{"name":"jev-latest"}]}"#),
            ok(
                r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"type":"noul","noul":0.7}},"usage":{"input_tokens":5,"output_tokens":1}}"#,
            ),
        ]);
        let bad_key = FakeTransport::new(vec![status(401, "{}", Vec::new())]);

        router(Arc::clone(&secrets), Arc::clone(&good))
            .check()
            .await
            .unwrap();
        let error = router(secrets, bad_key).check().await.unwrap_err();

        let calls = good.calls();
        assert_eq!(calls[0].0, "https://api.typesafe.ai/v1/models");
        assert!(calls[0].2.is_none());
        assert!(matches!(error, RouterError::Unauthorized));
    }

    #[tokio::test]
    async fn missing_key_is_unauthorized_without_sending() {
        let dir = tempfile::tempdir().unwrap();
        let empty = SecretStore::with_key_file(dir.path().join("none.key"), StorageMode::Standard);
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let router = router(
            Arc::new(tokio::sync::Mutex::new(empty)),
            Arc::clone(&transport),
        );

        let exchange = router.exchange(request()).await;

        assert!(matches!(exchange.result, Err(RouterError::Unauthorized)));
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
        let part = RouterResponse {
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
