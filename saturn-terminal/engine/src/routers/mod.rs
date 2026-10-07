//! router 연결 구현과 engine 쪽 판단 흐름: router 선택, 시작 확인, 키 재확인, 호출, 실패 대체, 연속 실패 집계, 판단 기록.
//! 설계: docs/design/router.md

mod local;
mod remote;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use saturn_core::routers::failure;
use saturn_core::routers::{
    AnswerKind, Method, Question, QuestionSetId, RouteDecision, RouterClient, RouterError,
    RouterRequest, RouterResponse,
};
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::Alert;
use tokio::sync::Mutex;

use crate::secrets::{KeyInfo, KeyInput, Masker, SecretStore, SecretsError, acquire};
use crate::settings::{Settings, SettingsError, SettingsManager};
use crate::store::{JudgmentOutcome, NewJudgment, Store, StoreError};

pub(crate) use local::{LocalRouter, LocalSource};
pub(crate) use remote::{RemoteRouter, RetryPolicy};

/// 기록용 중립 이름. 초안 값.
pub(crate) const REMOTE_ROUTER_ID: &str = "jev";

/// 기록용 중립 이름. 초안 값.
pub(crate) const LOCAL_ROUTER_ID: &str = "saturn-local";

const CHECK_QUESTION: &str = "saturn_check";

/// 로컬 router 버전 설정이 없을 때. 초안 값.
const UNVERSIONED_LOCAL: &str = "unversioned";

/// 끝 이름만 남긴다. 초안 값.
const ABSOLUTE_PATH_MARK: &str = "[abs]/";

/// 이만큼 쌓이면 상태판에 판단 모델 연결 끊김을 보인다. 입력 접수는 멈추지 않는다.
pub(crate) const CONSECUTIVE_FAILURE_LIMIT: u32 = 3;

/// 키를 메모리에 들고 있는 곳은 `SecretStore` 하나뿐이라 `RemoteRouter`도 이것을 빌려 쓴다.
pub(crate) type SharedSecrets = Arc<Mutex<SecretStore>>;

/// 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum RoutersError {
    /// HTTPS가 아니거나 허용 호스트가 아니라 키를 보내지 않는다.
    #[error("router endpoint is not allowed: {endpoint}")]
    DisallowedEndpoint {
        /// 설정의 router 주소.
        endpoint: String,
    },
    /// 예: `saturn` 방식인데 로컬 모델이 없다.
    #[error("no router configured for method {method:?}")]
    NotConfigured {
        /// 판단 방식.
        method: Method,
    },
    /// 호출자는 키를 받아 `accept_key`로 다시 확인하거나 실행하지 않는다.
    #[error("router check failed")]
    Check(#[source] RouterError),
    #[error("router key handling failed")]
    Secrets(#[from] SecretsError),
    #[error("failed to record router key info")]
    Settings(#[from] SettingsError),
    #[error("failed to record judgment")]
    Store(#[from] StoreError),
}

/// `RouterClient`는 `impl Future`를 돌려 dyn으로 못 쓰므로 enum으로 나눈다.
#[derive(Debug)]
pub(crate) enum ActiveRouter {
    /// 판단 모델을 호출하지 않는 사용자 선택 경로.
    Manual,
    /// `jev` 방식.
    Remote(RemoteRouter),
    /// `saturn` 방식.
    Local(LocalRouter),
}

impl ActiveRouter {
    /// 실제 모델과 버전은 설정 매핑과 `router_manifest`에만 둔다.
    pub(crate) fn router_id(&self) -> &str {
        match self {
            Self::Manual => "manual",
            Self::Remote(_) => REMOTE_ROUTER_ID,
            Self::Local(_) => LOCAL_ROUTER_ID,
        }
    }

    /// 판단 기록에 원문이 필요해 engine은 trait의 `router` 대신 이것을 쓴다.
    pub(crate) async fn exchange(&self, request: RouterRequest) -> RouterExchange {
        match self {
            Self::Manual => RouterExchange {
                sent: String::new(),
                received: None,
                result: Err(RouterError::NoResponse),
                started_at: SystemTime::now(),
                elapsed: Duration::ZERO,
                unknown_cost_calls: 0,
            },
            Self::Remote(router) => router.exchange(request).await,
            Self::Local(router) => router.exchange(request).await,
        }
    }

    /// 외부는 고정 모델, 로컬은 router 버전.
    pub(crate) fn model(&self) -> &str {
        match self {
            Self::Manual => "manual",
            Self::Remote(router) => router.model(),
            Self::Local(router) => router.version(),
        }
    }
}

impl RouterClient for ActiveRouter {
    async fn check(&self) -> Result<(), RouterError> {
        match self {
            Self::Manual => Ok(()),
            Self::Remote(router) => router.check().await,
            Self::Local(router) => router.check().await,
        }
    }

    async fn router(&self, request: RouterRequest) -> Result<RouterResponse, RouterError> {
        match self {
            Self::Manual => Err(RouterError::NoResponse),
            Self::Remote(router) => router.router(request).await,
            Self::Local(router) => router.router(request).await,
        }
    }
}

/// 원문은 가리기 전 값이라 로그에 남기지 않고 `record`에서만 가린 뒤 쓴다.
pub(crate) struct RouterExchange {
    /// 나눠 보냈으면 조각을 순서대로 이은 것. Authorization 헤더는 넣지 않는다.
    pub sent: String,
    /// 응답이 없으면 `None`.
    pub received: Option<String>,
    /// 형식 검사는 호출자가 한다.
    pub result: Result<RouterResponse, RouterError>,
    pub started_at: SystemTime,
    /// 응답이 없으면 포기할 때까지.
    pub elapsed: Duration,
    /// 보낸 뒤 시간 초과로 이미 처리됐을 수 있어 비용을 모르는 호출 수. 다시 보내 성공해도 남는다.
    pub unknown_cost_calls: u32,
}

impl std::fmt::Debug for RouterExchange {
    /// 원문은 쓰지 않고 길이, 결과 종류, 걸린 시간, 비용을 모르는 호출 수만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result = match &self.result {
            Ok(_) => "ok",
            Err(RouterError::NoResponse) => "NoResponse",
            Err(RouterError::TimedOutAfterSend) => "TimedOutAfterSend",
            Err(RouterError::Unauthorized) => "Unauthorized",
            Err(RouterError::RateLimited) => "RateLimited",
            Err(RouterError::Invalid { .. }) => "Invalid",
            Err(RouterError::Superseded) => "Superseded",
        };
        f.debug_struct("RouterExchange")
            .field("sent_len", &self.sent.len())
            .field("received_len", &self.received.as_ref().map(String::len))
            .field("result", &result)
            .field("elapsed", &self.elapsed)
            .field("unknown_cost_calls", &self.unknown_cost_calls)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum StartCheck {
    /// 소켓 접속을 받아도 된다.
    Ready,
    /// engine은 키를 받아 `accept_key`로 넘긴다.
    KeyRequired {
        /// 화면과 stderr에 보일 한 줄.
        reason: String,
    },
    /// 도움말에 보이지 않는 설치 검증 전용 설정으로 건너뛰었다.
    Skipped,
}

/// 원문과 결과는 `RouterExchange`에서 온다.
#[derive(Debug, Clone)]
pub(crate) struct RecordContext {
    /// `/record off`인지 `store`가 이것으로 본다.
    pub chat: ChatId,
    /// 입력과 무관한 호출은 `None`.
    pub input: Option<InputId>,
    pub question_sets: Vec<QuestionSetId>,
    pub settings: SettingsRevision,
    pub fallbacks: Vec<(String, String)>,
    /// revision이 바뀌었으면 호출자가 `Superseded`로 바꿔 넘긴다.
    pub outcome: JudgmentOutcome,
    /// 이 판단에 쓴 질문별 기준값.
    pub thresholds: Vec<(String, f64)>,
    /// 피드백 질문을 한 확률 q. 묻지 않았으면 `None`.
    pub asked_with: Option<f64>,
}

/// 성공 한 번이면 0으로 돌아간다.
#[derive(Debug, Default)]
struct FailureTracker {
    consecutive: u32,
}

impl FailureTracker {
    /// 한도 전 실패는 `RouterPaused`, 한도에 닿으면 `RouterDisconnected`, 성공이면 `None`.
    fn observe(&mut self, ok: bool) -> Option<Alert> {
        if ok {
            self.consecutive = 0;
            return None;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        if self.consecutive >= CONSECUTIVE_FAILURE_LIMIT {
            Some(Alert::RouterDisconnected)
        } else {
            Some(Alert::RouterPaused)
        }
    }
}

#[derive(Debug)]
pub(crate) struct Routers {
    /// 호출을 별도 작업으로 보내려고 공유한다.
    active: Arc<ActiveRouter>,
    method: Method,
    failures: FailureTracker,
    masker: Masker,
}

impl Routers {
    /// 외부 router 모델 기본값과 재시도 정책, 로컬 설정 키 이름은 초안이다.
    /// TODO(#40): `collect` 방식이 어떤 router를 쓰는지 미정. 정해지기 전에는 `NotConfigured`
    ///
    /// # Errors
    /// 주소가 HTTPS나 허용 호스트가 아니면 `DisallowedEndpoint`, 방식에 맞는 설정이 없으면 `NotConfigured`.
    pub(crate) fn select(
        settings: &Settings,
        secrets: SharedSecrets,
        masker: Masker,
    ) -> Result<Self, RoutersError> {
        let method = settings.method();
        let active = match method {
            Method::Manual => ActiveRouter::Manual,
            Method::Jev => {
                let model = settings
                    .get("router.model")
                    .and_then(|value| value.as_str())
                    .unwrap_or("jev-1.13.0")
                    .to_owned();
                ActiveRouter::Remote(RemoteRouter::new(
                    settings.router_endpoint(),
                    model,
                    secrets,
                    RetryPolicy::default(),
                )?)
            }
            Method::Saturn => {
                let endpoint = settings
                    .get("router.local.endpoint")
                    .and_then(|value| value.as_str())
                    .ok_or(RoutersError::NotConfigured { method })?;
                let version = settings
                    .get("router.local.version")
                    .and_then(|value| value.as_str())
                    .unwrap_or(UNVERSIONED_LOCAL);
                ActiveRouter::Local(LocalRouter::new(
                    LocalSource::Server {
                        endpoint: endpoint.to_owned(),
                    },
                    version.to_owned(),
                ))
            }
            Method::Collect => return Err(RoutersError::NotConfigured { method }),
        };
        Ok(Self::with_active(active, method, masker))
    }

    pub(crate) fn with_active(active: ActiveRouter, method: Method, masker: Masker) -> Self {
        Self {
            active: Arc::new(active),
            method,
            failures: FailureTracker::default(),
            masker,
        }
    }

    pub(crate) fn method(&self) -> Method {
        self.method
    }

    pub(crate) fn active(&self) -> &ActiveRouter {
        &self.active
    }

    /// 호출만 따로 돌릴 작업에 넘기는 손잡이. 연속 실패 집계는 결과를 받은 쪽이 `observe`로 한다.
    pub(crate) fn shared(&self) -> Arc<ActiveRouter> {
        Arc::clone(&self.active)
    }

    /// 실패는 오류가 아니라 원인을 가린 한 줄과 함께 `KeyRequired`로 돌려준다.
    pub(crate) async fn check(&self, settings: &Settings) -> StartCheck {
        let skip = settings
            .get("router.skip_check")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if skip {
            return StartCheck::Skipped;
        }
        match self.active.check().await {
            Ok(()) => StartCheck::Ready,
            Err(error) => StartCheck::KeyRequired {
                reason: self.masker.mask(&error.to_string()).as_str().to_owned(),
            },
        }
    }

    /// 관리자 명령과 환경 변수 키는 저장하지 않고 메모리에만 둔다.
    ///
    /// # Errors
    /// 키를 못 받으면 `Secrets`, 다시 확인이 실패하면 `Check`, 키 정보 기록 실패면 `Settings`.
    pub(crate) async fn accept_key(
        &mut self,
        input: KeyInput,
        secrets: &SharedSecrets,
        settings: &SettingsManager,
    ) -> Result<(), RoutersError> {
        let (key, source) = acquire(input).await?;
        let info = KeyInfo {
            source,
            last4: key.last4(),
        };
        let previous = std::mem::take(&mut self.masker);
        self.masker = Masker::new(vec![key.expose().to_owned()]);
        secrets.lock().await.hold(key, source);
        if let Err(error) = self.active.check().await {
            secrets.lock().await.drop_current();
            self.masker = previous;
            return Err(RoutersError::Check(error));
        }
        secrets.lock().await.persist_current()?;
        settings.record_key_info(&info).await?;
        Ok(())
    }

    /// 형식 오류(`Invalid`)와 revision 변경은 router가 답한 것이라 연속 실패로 세지 않는다.
    pub(crate) fn observe(&mut self, exchange: &RouterExchange) -> Option<Alert> {
        let answered = matches!(
            exchange.result,
            Ok(_) | Err(RouterError::Invalid { .. } | RouterError::Superseded)
        );
        self.failures.observe(answered)
    }

    /// 입력 처리 판단이 재시도 끝에 실패했을 때 쓴다. 현재 에이전트와 현재 모델로 보내고 입력은 대기로 두지 않는다.
    pub(crate) fn route_after_failure(
        &self,
        request: &RouterRequest,
        revision: ChatRevision,
        settings: SettingsRevision,
    ) -> RouteDecision {
        tracing::warn!("{}", failure::SKIP_MODEL_MESSAGE);
        failure::route_after_failure(request, revision, settings)
    }

    /// 원문은 `Masker`로 가린 뒤 넘기고, `/record off` 채팅이면 `store`가 쓰지 않는다.
    pub(crate) async fn record(
        &self,
        store: &Store,
        context: RecordContext,
        exchange: &RouterExchange,
    ) -> Result<Option<JudgmentId>, RoutersError> {
        let judgment = new_judgment(self, context, exchange);
        Ok(store.record_judgment(&judgment).await?)
    }
}

pub(crate) fn check_request(model: String) -> RouterRequest {
    RouterRequest {
        model,
        state: "Saturn router start check.".to_owned(),
        sets: vec![(
            QuestionSetId {
                name: "check".to_owned(),
                major: 1,
                minor: 0,
            },
            vec![Question {
                id: CHECK_QUESTION.to_owned(),
                text: "Is this text a start check message?".to_owned(),
                kind: AnswerKind::Noul,
            }],
        )],
    }
}

/// 절대 경로는 끝 이름만 남기고, 다른 대화 원문은 `core`가 넣지 않아 찾지 않는다. 초안 규칙.
pub(crate) fn sanitize_state(state: &str, masker: &Masker) -> String {
    let masked = masker.mask(state);
    let mut out = String::with_capacity(masked.as_str().len());
    let mut word = String::new();
    for ch in masked.as_str().chars() {
        if is_word_boundary(ch) {
            out.push_str(&replace_absolute(&word));
            word.clear();
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    out.push_str(&replace_absolute(&word));
    out
}

fn is_word_boundary(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';'
        )
}

/// 아니면 그대로.
fn replace_absolute(word: &str) -> String {
    let path = word.strip_prefix('~').unwrap_or(word);
    let is_absolute = path.starts_with('/') && path.matches('/').count() >= 2;
    if !is_absolute {
        return word.to_owned();
    }
    let name = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    format!("{ABSOLUTE_PATH_MARK}{name}")
}

/// 보낸 뒤 시간 초과는 `CostUnknown`, 무응답과 속도 제한 포기는 `NoResponse`.
pub(crate) fn outcome_of(result: &Result<RouterResponse, RouterError>) -> JudgmentOutcome {
    match result {
        Ok(_) => JudgmentOutcome::Ok,
        Err(RouterError::TimedOutAfterSend) => JudgmentOutcome::CostUnknown,
        Err(RouterError::NoResponse | RouterError::RateLimited | RouterError::Unauthorized) => {
            JudgmentOutcome::NoResponse
        }
        Err(RouterError::Invalid { .. }) => JudgmentOutcome::Invalid,
        Err(RouterError::Superseded) => JudgmentOutcome::Superseded,
    }
}

fn new_judgment(
    routers: &Routers,
    context: RecordContext,
    exchange: &RouterExchange,
) -> NewJudgment {
    let response = exchange.result.as_ref().ok();
    let tokens = match context.outcome {
        JudgmentOutcome::CostUnknown | JudgmentOutcome::NoResponse => None,
        _ => response.map(|response| response.tokens),
    };
    let model = routers.active.model().to_owned();
    NewJudgment {
        chat: context.chat,
        input: context.input,
        method: routers.method,
        router: routers.active.router_id().to_owned(),
        model: (
            model.clone(),
            response.map(|response| response.model.clone()),
        ),
        question_sets: context.question_sets,
        settings: context.settings,
        sent: routers.masker.mask(&exchange.sent),
        received: exchange
            .received
            .as_ref()
            .map(|received| routers.masker.mask(received)),
        answers: response
            .map(|response| response.answers.clone())
            .unwrap_or_default(),
        fallbacks: context.fallbacks,
        tokens,
        started_at: exchange.started_at,
        elapsed: exchange.elapsed,
        outcome: context.outcome,
        router_version: model,
        thresholds: context.thresholds,
        asked_with: context.asked_with,
    }
}

/// 다른 모듈 테스트가 가짜 전송으로 router를 만든다.
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use super::remote::tests::{FakeTransport, KEY, ok, router, status};
    pub(crate) use super::remote::{HttpReply, TransportError};
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_core::routers::Answer;

    use super::*;
    use crate::routers::remote::tests::{
        ANSWER, FakeTransport, KEY, ok, request, router, secrets_with_key, status,
    };
    use crate::secrets::StorageMode;

    fn routers(secrets: SharedSecrets, transport: Arc<FakeTransport>) -> Routers {
        let active = ActiveRouter::Remote(router(secrets, transport));
        Routers::with_active(active, Method::Jev, Masker::new(vec![KEY.to_owned()]))
    }

    async fn call(
        routers: &mut Routers,
        request: RouterRequest,
    ) -> (RouterExchange, Option<Alert>) {
        let exchange = routers.shared().exchange(request).await;
        let alert = routers.observe(&exchange);
        (exchange, alert)
    }

    fn context(chat: ChatId, outcome: JudgmentOutcome) -> RecordContext {
        RecordContext {
            chat,
            input: None,
            question_sets: vec![QuestionSetId {
                name: "route".to_owned(),
                major: 3,
                minor: 1,
            }],
            settings: SettingsRevision(1),
            fallbacks: Vec::new(),
            outcome,
            thresholds: vec![("keep_current".to_owned(), 0.8)],
            asked_with: Some(0.25),
        }
    }

    #[tokio::test]
    async fn new_judgment_carries_caller_thresholds_and_probability() {
        let dir = tempfile::tempdir().unwrap();
        let routers = routers(
            secrets_with_key(dir.path()).await,
            FakeTransport::new(Vec::new()),
        );
        let exchange = RouterExchange {
            sent: String::new(),
            received: None,
            result: Err(RouterError::NoResponse),
            started_at: SystemTime::now(),
            elapsed: Duration::ZERO,
            unknown_cost_calls: 0,
        };

        let judgment = new_judgment(
            &routers,
            context(ChatId(1), JudgmentOutcome::NoResponse),
            &exchange,
        );

        assert_eq!(judgment.thresholds, vec![("keep_current".to_owned(), 0.8)]);
        assert_eq!(judgment.asked_with, Some(0.25));
    }

    #[test]
    fn three_failures_show_disconnected_and_success_resets() {
        let mut tracker = FailureTracker::default();

        assert_eq!(tracker.observe(false), Some(Alert::RouterPaused));
        assert_eq!(tracker.observe(false), Some(Alert::RouterPaused));
        assert_eq!(tracker.observe(false), Some(Alert::RouterDisconnected));
        assert_eq!(tracker.observe(false), Some(Alert::RouterDisconnected));
        assert_eq!(tracker.observe(true), None);
        assert_eq!(tracker.observe(false), Some(Alert::RouterPaused));
    }

    #[tokio::test(start_paused = true)]
    async fn call_counts_only_unanswered_failures_and_keeps_accepting() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = r#"{"answers":{"keep_current":{"noul":-1}}}"#;
        let mut replies = vec![ok(invalid)];
        replies.extend(vec![Err(remote::TransportError::AfterSend); 9]);
        replies.push(ok(ANSWER));
        let transport = FakeTransport::new(replies);
        let mut routers = routers(secrets_with_key(dir.path()).await, transport);

        let (first, alert) = call(&mut routers, request()).await;
        assert!(matches!(first.result, Err(RouterError::Invalid { .. })));
        assert_eq!(alert, None);
        for _ in 0..2 {
            assert_eq!(
                call(&mut routers, request()).await.1,
                Some(Alert::RouterPaused)
            );
        }
        assert_eq!(
            call(&mut routers, request()).await.1,
            Some(Alert::RouterDisconnected)
        );
        let (answered, alert) = call(&mut routers, request()).await;
        assert!(answered.result.is_ok());
        assert_eq!(alert, None);
    }

    /// 테스트 안에서 나온 로그 줄을 모은다.
    #[derive(Clone, Default)]
    struct LogSink(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogSink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
        type Writer = LogSink;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn logged<T>(run: impl FnOnce() -> T) -> (T, String) {
        let sink = LogSink::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, run);
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        (value, text)
    }

    fn routers_without_transport() -> Routers {
        routers(
            Arc::new(Mutex::new(SecretStore::with_key_file(
                PathBuf::from("/nonexistent/none.key"),
                StorageMode::Standard,
            ))),
            FakeTransport::new(Vec::new()),
        )
    }

    #[test]
    fn route_after_failure_logs_skip_message_and_keeps_current_model() {
        let routers = routers_without_transport();

        let (decision, log) = logged(|| {
            routers.route_after_failure(&request(), ChatRevision(1), SettingsRevision(1))
        });

        assert!(log.contains("판단 모델 실패로 모델 선택을 건너뜁니다"));
        assert!(!log.contains(KEY));
        assert_eq!(decision.model, None);
        assert!(decision.keep_current);
    }

    #[tokio::test]
    async fn judgments_are_recorded_masked_and_skipped_when_off() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(&dir.path().join("home")).await.unwrap();
        let chat = store.create_chat(PathBuf::from("/w")).await.unwrap();
        let reply = ANSWER.replace("\"usage\"", &format!("\"echo\":\"{KEY}\",\"usage\""));
        let transport = FakeTransport::new(vec![ok(&reply)]);
        let mut routers = routers(secrets_with_key(dir.path()).await, transport);
        let mut leaky = request();
        leaky.state = format!("pasted {KEY}");

        let (exchange, _) = call(&mut routers, leaky).await;
        let outcome = outcome_of(&exchange.result);
        let id = routers
            .record(&store, context(chat, outcome), &exchange)
            .await
            .unwrap();
        let timeout = RouterExchange {
            sent: "{}".to_owned(),
            received: None,
            result: Err(RouterError::TimedOutAfterSend),
            started_at: SystemTime::now(),
            elapsed: Duration::from_secs(40),
            unknown_cost_calls: 3,
        };
        let timeout_outcome = outcome_of(&timeout.result);
        routers
            .record(&store, context(chat, timeout_outcome), &timeout)
            .await
            .unwrap();
        store.set_recording(chat, false).await.unwrap();
        let skipped = routers
            .record(&store, context(chat, outcome), &exchange)
            .await
            .unwrap();

        assert!(id.is_some());
        assert_eq!(timeout_outcome, JudgmentOutcome::CostUnknown);
        assert!(skipped.is_none());
        let path = dir.path().join("judgments.jsonl");
        store.export_judgments(&path).await.unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(KEY));
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["router"], "jev");
        assert_eq!(
            lines[0]["tokens"],
            serde_json::json!({ "input": 310, "output": 24 })
        );
        assert_eq!(
            lines[0]["answers"][0]["answer"],
            serde_json::json!({ "Noul": 0.91 })
        );
        assert_eq!(lines[1]["outcome"], "CostUnknown");
        assert_eq!(lines[1]["tokens"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn accept_key_rechecks_then_saves_key_and_info() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (store, _) = Store::open(&home).await.unwrap();
        let settings = SettingsManager::new(home.clone(), Vec::new(), &store)
            .await
            .unwrap();
        let key_file = dir.path().join("router.key");
        let empty = SecretStore::with_key_file(key_file.clone(), StorageMode::Standard);
        let secrets: SharedSecrets = Arc::new(Mutex::new(empty));
        let check_ok = r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"noul":0.6}},"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let transport = FakeTransport::new(vec![
            status(401, "{}", Vec::new()),
            ok(r#"{"models":[]}"#),
            ok(check_ok),
        ]);
        let mut routers = Routers::with_active(
            ActiveRouter::Remote(router(Arc::clone(&secrets), transport)),
            Method::Jev,
            Masker::default(),
        );

        let rejected = routers
            .accept_key(
                KeyInput::Hidden("sk-wrong-0000".to_owned()),
                &secrets,
                &settings,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            rejected,
            RoutersError::Check(RouterError::Unauthorized)
        ));
        assert!(!key_file.exists());

        routers
            .accept_key(KeyInput::Hidden(format!("{KEY}\n")), &secrets, &settings)
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&key_file).unwrap(), KEY);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("last4 = \"6789\""));
        assert!(!config.contains(KEY));
        assert_eq!(routers.masker.mask(KEY).as_str(), "[redacted]");
    }

    #[tokio::test]
    async fn start_check_reports_masked_reason_or_skips() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (store, _) = Store::open(&home).await.unwrap();
        let mut manager = SettingsManager::new(home.clone(), Vec::new(), &store)
            .await
            .unwrap();
        let normal = manager.apply_user(&store).await.unwrap().revision;
        let settings = manager.at(&store, normal).await.unwrap();
        let transport = FakeTransport::new(vec![status(401, "{}", Vec::new())]);
        let routers = routers(secrets_with_key(dir.path()).await, transport);

        let failed = routers.check(&settings).await;

        assert!(
            matches!(failed, StartCheck::KeyRequired { ref reason } if reason == "router rejected the key")
        );
        std::fs::write(home.join("config.toml"), "[router]\nskip_check = true\n").unwrap();
        let skip = manager.apply_user(&store).await.unwrap().revision;
        let skipping = manager.at(&store, skip).await.unwrap();
        assert!(matches!(
            routers.check(&skipping).await,
            StartCheck::Skipped
        ));
    }

    #[tokio::test]
    async fn select_follows_method_and_endpoint_rules() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let (store, _) = Store::open(&home).await.unwrap();
        let mut manager = SettingsManager::new(home.clone(), Vec::new(), &store)
            .await
            .unwrap();
        let secrets = secrets_with_key(dir.path()).await;
        let jev = load(&mut manager, &store, &home, "[router]\nmode = \"jev\"\n").await;
        let chosen = Routers::select(&jev, Arc::clone(&secrets), Masker::default()).unwrap();
        assert_eq!(chosen.active().router_id(), "jev");
        let evil = load(
            &mut manager,
            &store,
            &home,
            "[router]\nmode = \"jev\"\nendpoint = \"https://evil.example\"\n",
        )
        .await;
        assert!(matches!(
            Routers::select(&evil, Arc::clone(&secrets), Masker::default()),
            Err(RoutersError::DisallowedEndpoint { .. })
        ));
        let saturn = load(
            &mut manager,
            &store,
            &home,
            "[router]\nmethod = \"saturn\"\n",
        )
        .await;
        assert!(matches!(
            Routers::select(&saturn, Arc::clone(&secrets), Masker::default()),
            Err(RoutersError::NotConfigured { .. })
        ));
        let collect = load(
            &mut manager,
            &store,
            &home,
            "[router]\nmode = \"collect\"\n",
        )
        .await;
        assert!(matches!(
            Routers::select(&collect, Arc::clone(&secrets), Masker::default()),
            Err(RoutersError::NotConfigured {
                method: Method::Collect
            })
        ));
        let local = load(
            &mut manager,
            &store,
            &home,
            "[router]\nmethod = \"saturn\"\n[router.local]\nendpoint = \"http://127.0.0.1:9\"\nversion = \"v2\"\n",
        )
        .await;
        let chosen = Routers::select(&local, secrets, Masker::default()).unwrap();
        assert_eq!(chosen.active().router_id(), "saturn-local");
        assert_eq!(chosen.method(), Method::Saturn);
    }

    async fn load(
        manager: &mut SettingsManager,
        store: &Store,
        home: &std::path::Path,
        content: &str,
    ) -> Settings {
        std::fs::write(home.join("config.toml"), content).unwrap();
        let revision = manager.apply_user(store).await.unwrap().revision;
        manager.at(store, revision).await.unwrap()
    }

    #[test]
    fn state_drops_secrets_and_absolute_paths() {
        let masker = Masker::new(vec![KEY.to_owned()]);

        let clean = sanitize_state(
            &format!(
                "edit /Users/me/proj/src/main.rs and ~/notes/todo.md (key {KEY}) but keep src/lib.rs, /tmp"
            ),
            &masker,
        );

        assert_eq!(
            clean,
            "edit [abs]/main.rs and [abs]/todo.md (key [redacted]) but keep src/lib.rs, /tmp"
        );
    }

    #[test]
    fn outcomes_follow_error_kinds() {
        let ok = Ok(RouterResponse {
            model: "m".to_owned(),
            answers: vec![("a".to_owned(), Answer::Noul(0.5))],
            tokens: (1, 1),
        });

        assert_eq!(outcome_of(&ok), JudgmentOutcome::Ok);
        assert_eq!(
            outcome_of(&Err(RouterError::TimedOutAfterSend)),
            JudgmentOutcome::CostUnknown
        );
        assert_eq!(
            outcome_of(&Err(RouterError::RateLimited)),
            JudgmentOutcome::NoResponse
        );
        assert_eq!(
            outcome_of(&Err(RouterError::Invalid {
                reason: String::new()
            })),
            JudgmentOutcome::Invalid
        );
        assert_eq!(
            outcome_of(&Err(RouterError::Superseded)),
            JudgmentOutcome::Superseded
        );
    }

    #[test]
    fn exchange_debug_hides_text() {
        let exchange = RouterExchange {
            sent: format!("state {KEY}"),
            received: None,
            result: Err(RouterError::Invalid {
                reason: KEY.to_owned(),
            }),
            started_at: SystemTime::now(),
            elapsed: Duration::from_millis(5),
            unknown_cost_calls: 0,
        };

        let debug = format!("{exchange:?}");

        assert!(!debug.contains(KEY));
        assert!(debug.contains("sent_len"));
        assert!(debug.contains("Invalid"));
    }
}
