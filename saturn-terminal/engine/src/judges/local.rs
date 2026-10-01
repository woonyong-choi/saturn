//! 로컬 Saturn 모델 연결(`saturn` 방식).
//!
//! 설계: docs/design/judge.md(판단 방식, judge 시작 확인), docs/design/judge-training.md(judge 버전).
//! 시작 확인은 모델 로드나 로컬 API 서버 응답으로 한다. 키를 쓰지 않는다.
//! `saturn` 방식에서 확신도가 기준보다 낮으면 행동하지 않는 규칙은 `saturn_core::judges::decide_route`가 맡는다.
//! TODO(#43): 모델 형식(공개 체크포인트 이어 학습, 항목별 yes/no 확률)과 실행 방식이 정해지면 `LocalSource`를 확정한다.
//! 정해지기 전에는 `Model`은 확인과 판단 모두 `NoResponse`이고, `Server`만 동작한다

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use saturn_core::judges::{JudgeClient, JudgeError, JudgeRequest, JudgeResponse};

use super::JudgeExchange;
use super::remote::{HttpReply, Transport, TransportError, judge_body, parse_judge_reply};

/// 로컬 서버에 허용하는 호스트. 판단 원문이 기기 밖으로 나가지 않게 루프백만 받는다(초안).
const LOOPBACK_HOSTS: &[&str] = &["127.0.0.1", "localhost", "[::1]"];

/// 로컬 서버 응답을 기다리는 시간(초안).
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);

/// 로컬 모델을 어디서 부르는지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalSource {
    /// 승격된 모델 파일. engine이 불러 쓴다. 경로는 judge 버전의 모델.
    Model {
        /// 모델 경로.
        path: PathBuf,
    },
    /// 이미 떠 있는 로컬 API 서버. 주소는 설정 `judge.local.endpoint`(초안)이고 루프백(`127.0.0.1`, `localhost`, `[::1]`)의
    /// `http`만 받는다(초안). 외부 judge와 같은 `POST /v1/systemone` 본문을 쓴다.
    Server {
        /// 서버 주소.
        endpoint: String,
    },
}

/// 로컬 Saturn 모델 연결. `JudgeClient`로 engine에만 붙는다.
#[derive(Debug)]
pub struct LocalJudge {
    /// 모델 위치.
    source: LocalSource,
    /// 지금 쓰는 judge 버전 이름. 판단 기록에 남긴다.
    version: String,
    /// 로컬 서버 전송. `Server`일 때만 쓴다. 키를 붙이지 않는다.
    transport: Option<Arc<dyn Transport>>,
}

impl LocalJudge {
    /// 연결을 만든다. 아직 모델을 불러오지 않는다.
    pub fn new(source: LocalSource, version: String) -> Self {
        let transport: Option<Arc<dyn Transport>> = match &source {
            LocalSource::Server { .. } => Some(Arc::new(PlainTransport::new())),
            LocalSource::Model { .. } => None,
        };
        Self {
            source,
            version,
            transport,
        }
    }

    /// 전송을 받아 만든다. 테스트가 가짜 서버를 넣는다.
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

    /// 지금 쓰는 judge 버전.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 요청과 응답 원문을 함께 돌려주는 판단 호출. 원문은 모델에 넣은 본문과 모델 출력이다.
    /// `Model`은 실행 방식이 정해지지 않아(#43) 보내지 않고 `NoResponse`다. `Server`는 루프백 주소가 아니면 보내지 않는다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        let started_at = SystemTime::now();
        let clock = Instant::now();
        let body = judge_body(&request).to_string();
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
                        let parsed = parse_judge_reply(&request, &reply);
                        (Some(reply), parsed)
                    }
                    Ok(reply) => (
                        Some(reply.body),
                        Err(JudgeError::Invalid {
                            reason: format!("local judge returned status {}", reply.status),
                        }),
                    ),
                    Err(TransportError::BeforeSend) => (None, Err(JudgeError::NoResponse)),
                    Err(TransportError::AfterSend) => (None, Err(JudgeError::TimedOutAfterSend)),
                }
            }
            _ => (None, Err(JudgeError::NoResponse)),
        };
        JudgeExchange {
            sent: body,
            received,
            result,
            started_at,
            elapsed: clock.elapsed(),
        }
    }
}

impl JudgeClient for LocalJudge {
    /// `Model`이면 모델을 불러오고, `Server`면 서버 응답을 확인한다. 둘 다 실제 판단 1건을 돌려 본다.
    /// `Model`은 실행 방식이 정해질 때까지(#43) 항상 실패한다.
    async fn check(&self) -> Result<(), JudgeError> {
        let request = super::check_request(self.version.clone());
        self.exchange(request).await.result.map(|_| ())
    }

    /// `exchange`의 결과만 돌려준다.
    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        self.exchange(request).await.result
    }
}

/// 루프백 `http` 주소인지.
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

/// 루프백 서버용 평문 HTTP 전송. 키를 붙이지 않고 리다이렉트를 따르지 않는다.
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
    use crate::judges::remote::tests::{ANSWER, FakeTransport, ok, request};

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
        let judge = LocalJudge::with_transport(source, "saturn-v3".to_owned(), transport.clone());

        let exchange = judge.exchange(request()).await;

        assert_eq!(exchange.result.unwrap().answers.len(), 3);
        let calls = transport.calls();
        assert_eq!(calls[0].0, "http://127.0.0.1:7777/v1/systemone");
        assert!(calls[0].1.iter().all(|(name, _)| name != "authorization"));
        assert_eq!(judge.version(), "saturn-v3");
    }

    #[tokio::test]
    async fn model_source_and_remote_server_are_not_called() {
        let transport = FakeTransport::new(vec![ok(ANSWER)]);
        let remote = LocalJudge::with_transport(
            LocalSource::Server {
                endpoint: "http://192.168.0.9:7777".to_owned(),
            },
            "v".to_owned(),
            transport.clone(),
        );
        let model = LocalJudge::new(
            LocalSource::Model {
                path: PathBuf::from("/models/saturn"),
            },
            "v".to_owned(),
        );

        assert!(matches!(
            remote.exchange(request()).await.result,
            Err(JudgeError::NoResponse)
        ));
        assert!(matches!(model.check().await, Err(JudgeError::NoResponse)));
        assert!(transport.calls().is_empty());
    }
}
