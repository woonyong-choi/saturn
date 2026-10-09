//! 로컬 Saturn 모델 연결. 키를 쓰지 않는다.
//! 설계: docs/design/router.md

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
    /// 이미 떠 있는 로컬 API 서버. 루프백 `https`만 받는다(초안).
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
    /// `Server`일 때만 쓰고 키를 붙이지 않는다.
    transport: Option<Arc<dyn Transport>>,
}

impl LocalRouter {
    pub(crate) fn new(source: LocalSource, version: String) -> Self {
        let transport: Option<Arc<dyn Transport>> = match &source {
            LocalSource::Server { .. } => super::remote::ReqwestTransport::new()
                .ok()
                .map(|transport| Arc::new(transport) as Arc<dyn Transport>),
        };
        Self {
            source,
            version,
            transport,
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
            transport: Some(transport),
        }
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    /// 루프백 주소가 아니면 보내지 않는다.
    pub(crate) async fn exchange(&self, request: RouterRequest) -> RouterExchange {
        let started_at = SystemTime::now();
        let clock = Instant::now();
        let body = router_body(&request).to_string();
        let (received, result) = match (&self.source, &self.transport) {
            (LocalSource::Server { endpoint }, Some(transport)) if is_loopback(endpoint) => {
                let url = format!("{}/v1/systemone", endpoint.trim_end_matches('/'));
                let headers = vec![("content-type".to_owned(), "application/json".to_owned())];
                match transport
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
            }
            _ => (None, Err(RouterError::NoResponse)),
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
    let Some(rest) = endpoint.strip_prefix("https://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => authority,
    };
    LOOPBACK_HOSTS.contains(&host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::remote::tests::{ANSWER, FakeTransport, ok, request};

    #[test]
    fn only_loopback_https_is_allowed() {
        for good in [
            "https://127.0.0.1:8080",
            "https://localhost",
            "https://[::1]:9000/x",
        ] {
            assert!(is_loopback(good), "{good}");
        }
        for bad in [
            "http://127.0.0.1",
            "https://user:password@localhost",
            "https://10.0.0.2",
            "https://localhost.evil.com",
            "https://127.0.0.1.nip.io",
        ] {
            assert!(!is_loopback(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn server_source_uses_same_wire_format_without_key() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let source = LocalSource::Server {
            endpoint: "https://127.0.0.1:7777/".to_owned(),
        };
        let router = LocalRouter::with_transport(source, "saturn-v3".to_owned(), transport.clone());

        let exchange = router.exchange(request()).await;

        assert_eq!(exchange.result.unwrap().answers.len(), 3);
        let calls = transport.calls();
        assert_eq!(calls[0].0, "https://127.0.0.1:7777/v1/systemone");
        assert!(calls[0].1.iter().all(|(name, _)| name != "authorization"));
        assert_eq!(router.version(), "saturn-v3");
    }

    #[tokio::test]
    async fn remote_server_is_not_called() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let remote = LocalRouter::with_transport(
            LocalSource::Server {
                endpoint: "https://192.168.0.9:7777".to_owned(),
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
