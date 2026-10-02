//! 외부 router API 연결. HTTPS와 허용 호스트만 쓰고 TLS 검증을 끄는 옵션은 두지 않는다.
//! 설계: docs/design/router.md

use std::fmt::Debug;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use saturn_core::routers::failure::{remaining_until_deadline, retry_delay};
use saturn_core::routers::{RouterClient, RouterError, RouterRequest, RouterResponse};
use serde_json::Value;
use tokio::time::Instant as TokioInstant;

use super::{RouterExchange, RoutersError, SharedSecrets};
use crate::secrets::is_sensitive_header;

mod transport;
mod wire;

use transport::ReqwestTransport;
pub(crate) use transport::{
    HttpReply, RetryPolicy, Transport, TransportError, TransportFuture, send_with,
};
use wire::{expand_choices, merge_responses, split_request};
pub(crate) use wire::{parse_router_reply, router_body};

/// router 키는 이 호스트로만 간다.
pub(crate) const ALLOWED_HOST: &str = "api.typesafe.ai";

/// 토큰을 셀 수 없어 본문 바이트로 잰다(바이트 수 ≥ 토큰 수라 API 한도 64k 토큰을 넘지 않는다). 초안 값.
pub(crate) const REQUEST_SPLIT_LIMIT: usize = 64 * 1024;

/// `state`와 가장 긴 질문의 합 한도(바이트). 초안 값.
pub(crate) const STATE_SPLIT_LIMIT: usize = 32 * 1024;

/// 넘으면 계층 선택으로 나눈다.
pub(crate) const MAX_CHOICES: usize = 255;

const ROUTER_PATH: &str = "/v1/systemone";

const MODELS_PATH: &str = "/v1/models";

/// 계층 선택 조각의 "이 조각에 없음" 선택지 이름.
const OTHER_CHUNK_OPTION: &str = "none of these";

/// `Debug`는 키 보관소와 전송을 빼고 주소, 모델, 재시도 설정만 쓴다.
pub(crate) struct RemoteRouter {
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
    pub(crate) fn new(
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

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    /// 별칭만 목록에 오고 버전 고정 이름은 없어도 받으므로, 목록에 없다는 이유로는 실패하지 않는다.
    ///
    /// # Errors
    /// 인증 실패(키 없음, 거절)는 `Unauthorized`, 연결 실패는 `NoResponse`.
    pub(crate) async fn list_models(&self) -> Result<Vec<String>, RouterError> {
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
    pub(crate) async fn exchange(&self, request: RouterRequest) -> RouterExchange {
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

#[cfg(test)]
pub(crate) mod tests;
