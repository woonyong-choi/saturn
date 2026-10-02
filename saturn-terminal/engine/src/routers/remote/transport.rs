use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// 재시도 횟수와 간격은 `saturn_core::routers::failure`가 정한다.
/// TODO(#235): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetryPolicy {
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
    pub(super) fn new() -> Result<Self, reqwest::Error> {
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
