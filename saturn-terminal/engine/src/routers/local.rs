//! 로컬 Saturn 모델 연결. 키를 쓰지 않는다.
//! 설계: docs/design/router.md
//! TODO(#43): 모델 형식과 실행 방식이 정해지면 `LocalSource`를 확정한다. 그 전에는 `Server`만 동작한다

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use saturn_core::routers::{RouterClient, RouterError, RouterRequest, RouterResponse};

use super::RouterExchange;
use super::remote::{HttpReply, Transport, TransportError, parse_router_reply, router_body};

/// 판단 원문이 기기 밖으로 나가지 않게 루프백만 받는다. 초안 값.
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "localhost", "[::1]"];

/// 초안 값.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LocalSource {
    /// 이미 떠 있는 로컬 API 서버. 루프백 `http`만 받는다(초안).
    Server {
        /// 설정 `router.local.endpoint`(초안).
        endpoint: String,
    },
}

#[derive(Debug)]
pub(crate) struct LocalRouter {
    source: LocalSource,
    /// 판단 기록에 남긴다.
    version: String,
    /// 키를 붙이지 않는다.
    transport: Arc<dyn Transport>,
}

impl LocalRouter {
    /// 아직 모델을 불러오지 않는다.
    pub(crate) fn new(source: LocalSource, version: String) -> Self {
        Self {
            source,
            version,
            transport: Arc::new(PlainTransport::new()),
        }
    }

    /// 테스트가 가짜 서버를 넣는다.
    #[cfg(test)]
    pub(crate) fn with_transport(
        source: LocalSource,
        version: String,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            source,
            version,
            transport,
        }
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    /// 루프백 주소가 아니면 보내지 않고 `NoResponse`로 끝낸다.
    pub(crate) async fn exchange(&self, request: RouterRequest) -> RouterExchange {
        let started_at = SystemTime::now();
        let clock = Instant::now();
        let body = router_body(&request).to_string();
        let LocalSource::Server { endpoint } = &self.source;
        let (received, result) = if is_loopback(endpoint) {
            let url = format!("{}/v1/systemone", endpoint.trim_end_matches('/'));
            let headers = vec![("content-type".to_owned(), "application/json".to_owned())];
            match self
                .transport
                .send(&url, headers, Some(body.clone()), LOCAL_TIMEOUT)
                .await
            {
                Ok(HttpReply {
                    status: 200..=299,
                    body: reply,
                    ..
                }) => {
                    let parsed = parse_router_reply(&request, &reply);
                    (Some(reply), parsed)
                }
                Ok(reply) => (
                    Some(reply.body),
                    Err(RouterError::Invalid {
                        reason: format!("local router returned status {}", reply.status),
                    }),
                ),
                Err(TransportError::BeforeSend) => (None, Err(RouterError::NoResponse)),
                Err(TransportError::AfterSend) => (None, Err(RouterError::TimedOutAfterSend)),
            }
        } else {
            (None, Err(RouterError::NoResponse))
        };
        RouterExchange {
            sent: body,
            received,
            result,
            started_at,
            elapsed: clock.elapsed(),
            unknown_cost_calls: 0,
        }
    }
}

impl RouterClient for LocalRouter {
    async fn check(&self) -> Result<(), RouterError> {
        let request = super::check_request(self.version.clone());
        self.exchange(request).await.result.map(|_| ())
    }

    async fn router(&self, request: RouterRequest) -> Result<RouterResponse, RouterError> {
        self.exchange(request).await.result
    }
}

pub(crate) fn is_loopback(endpoint: &str) -> bool {
    let Some(rest) = endpoint.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    LOOPBACK_HOSTS.contains(&host)
}

/// 키를 붙이지 않고 리다이렉트를 따르지 않는다.
#[derive(Debug)]
struct PlainTransport {
    client: reqwest::Client,
}

impl PlainTransport {
    fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default(); // 설정 없는 빌더는 TLS 초기화 실패 때만 실패한다. 평문 루프백이라 기본 클라이언트로 충분하다
        Self { client }
    }
}

impl Transport for PlainTransport {
    fn send<'a>(
        &'a self,
        url: &'a str,
        headers: Vec<(String, String)>,
        body: Option<String>,
        timeout: Duration,
    ) -> super::remote::TransportFuture<'a> {
        super::remote::send_with(&self.client, url, headers, body, timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::remote::tests::{ANSWER, FakeTransport, ok, request};

    #[test]
    fn only_loopback_http_is_allowed() {
        for good in [
            "http://127.0.0.1:8080",
            "http://localhost",
            "http://[::1]:9000/x",
        ] {
            assert!(is_loopback(good), "{good}");
        }
        for bad in [
            "https://127.0.0.1",
            "http://10.0.0.2",
            "http://localhost.evil.com",
            "http://127.0.0.1.nip.io",
        ] {
            assert!(!is_loopback(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn server_source_uses_same_wire_format_without_key() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let source = LocalSource::Server {
            endpoint: "http://127.0.0.1:7777/".to_owned(),
        };
        let router = LocalRouter::with_transport(source, "saturn-v3".to_owned(), transport.clone());

        let exchange = router.exchange(request()).await;

        assert_eq!(exchange.result.unwrap().answers.len(), 3);
        let calls = transport.calls();
        assert_eq!(calls[0].0, "http://127.0.0.1:7777/v1/systemone");
        assert!(calls[0].1.iter().all(|(name, _)| name != "authorization"));
        assert_eq!(router.version(), "saturn-v3");
    }

    #[tokio::test]
    async fn remote_server_is_not_called() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let remote = LocalRouter::with_transport(
            LocalSource::Server {
                endpoint: "http://192.168.0.9:7777".to_owned(),
            },
            "v".to_owned(),
            transport.clone(),
        );

        assert!(matches!(
            remote.exchange(request()).await.result,
            Err(RouterError::NoResponse)
        ));
        assert!(matches!(remote.check().await, Err(RouterError::NoResponse)));
        assert!(transport.calls().is_empty());
    }
}
