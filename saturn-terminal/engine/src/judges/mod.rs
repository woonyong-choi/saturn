//! judge 연결 구현과 engine 쪽 판단 흐름: judge 선택, 시작 확인, 키 재확인, 호출, 연속 실패 집계, 판단 기록.
//! 설계: docs/design/judge.md

mod local;
mod remote;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use saturn_core::judges::{
    AnswerKind, JudgeClient, JudgeError, JudgeRequest, JudgeResponse, Method, Question,
    QuestionSetId,
};
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::Alert;
use tokio::sync::Mutex;

use crate::secrets::{KeyInfo, KeyInput, Masker, SecretStore, SecretsError, acquire};
use crate::settings::{Settings, SettingsError, SettingsManager};
use crate::store::{JudgmentOutcome, NewJudgment, Store, StoreError};

pub use local::{LocalJudge, LocalSource};
pub use remote::{
    ALLOWED_HOST, MAX_CHOICES, REQUEST_SPLIT_LIMIT, RemoteJudge, RetryPolicy, STATE_SPLIT_LIMIT,
};

/// 기록용 중립 이름. 초안 값.
pub const REMOTE_JUDGE_ID: &str = "jev";

/// 기록용 중립 이름. 초안 값.
pub const LOCAL_JUDGE_ID: &str = "saturn-local";

const CHECK_QUESTION: &str = "saturn_check";

/// 로컬 judge 버전 설정이 없을 때. 초안 값.
const UNVERSIONED_LOCAL: &str = "unversioned";

/// 끝 이름만 남긴다. 초안 값.
const ABSOLUTE_PATH_MARK: &str = "[abs]/";

/// 이만큼 쌓이면 새 입력 접수를 멈추고 연결 복구를 안내한다.
pub const CONSECUTIVE_FAILURE_LIMIT: u32 = 3;

/// 키를 메모리에 들고 있는 곳은 `SecretStore` 하나뿐이라 `RemoteJudge`도 이것을 빌려 쓴다.
pub type SharedSecrets = Arc<Mutex<SecretStore>>;

/// 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum JudgesError {
    /// HTTPS가 아니거나 허용 호스트가 아니라 키를 보내지 않는다.
    #[error("judge endpoint is not allowed: {endpoint}")]
    DisallowedEndpoint {
        /// 설정의 judge 주소.
        endpoint: String,
    },
    /// 예: `saturn` 방식인데 로컬 모델이 없다.
    #[error("no judge configured for method {method:?}")]
    NotConfigured {
        /// 판단 방식.
        method: Method,
    },
    /// 호출자는 키를 받아 `accept_key`로 다시 확인하거나 실행하지 않는다.
    #[error("judge check failed")]
    Check(#[source] JudgeError),
    #[error("judge key handling failed")]
    Secrets(#[from] SecretsError),
    #[error("failed to record judge key info")]
    Settings(#[from] SettingsError),
    #[error("failed to record judgment")]
    Store(#[from] StoreError),
}

/// `JudgeClient`는 `impl Future`를 돌려 dyn으로 못 쓰므로 enum으로 나눈다.
#[derive(Debug)]
pub enum ActiveJudge {
    /// `jev` 방식.
    Remote(RemoteJudge),
    /// `saturn` 방식.
    Local(LocalJudge),
}

impl ActiveJudge {
    /// 실제 모델과 버전은 설정 매핑과 `judge_manifest`에만 둔다.
    pub fn judge_id(&self) -> &str {
        match self {
            Self::Remote(_) => REMOTE_JUDGE_ID,
            Self::Local(_) => LOCAL_JUDGE_ID,
        }
    }

    /// 판단 기록에 원문이 필요해 engine은 trait의 `judge` 대신 이것을 쓴다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        match self {
            Self::Remote(judge) => judge.exchange(request).await,
            Self::Local(judge) => judge.exchange(request).await,
        }
    }

    /// 외부는 고정 모델, 로컬은 judge 버전.
    pub fn model(&self) -> &str {
        match self {
            Self::Remote(judge) => judge.model(),
            Self::Local(judge) => judge.version(),
        }
    }
}

impl JudgeClient for ActiveJudge {
    async fn check(&self) -> Result<(), JudgeError> {
        match self {
            Self::Remote(judge) => judge.check().await,
            Self::Local(judge) => judge.check().await,
        }
    }

    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        match self {
            Self::Remote(judge) => judge.judge(request).await,
            Self::Local(judge) => judge.judge(request).await,
        }
    }
}

/// 원문은 가리기 전 값이라 로그에 남기지 않고 `record`에서만 가린 뒤 쓴다.
pub struct JudgeExchange {
    /// 나눠 보냈으면 조각을 순서대로 이은 것. Authorization 헤더는 넣지 않는다.
    pub sent: String,
    /// 응답이 없으면 `None`.
    pub received: Option<String>,
    /// 형식 검사는 호출자가 한다.
    pub result: Result<JudgeResponse, JudgeError>,
    pub started_at: SystemTime,
    /// 응답이 없으면 포기할 때까지.
    pub elapsed: Duration,
}

impl std::fmt::Debug for JudgeExchange {
    /// 원문은 쓰지 않고 길이, 결과 종류, 걸린 시간만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result = match &self.result {
            Ok(_) => "ok",
            Err(JudgeError::NoResponse) => "NoResponse",
            Err(JudgeError::TimedOutAfterSend) => "TimedOutAfterSend",
            Err(JudgeError::Unauthorized) => "Unauthorized",
            Err(JudgeError::RateLimited) => "RateLimited",
            Err(JudgeError::Invalid { .. }) => "Invalid",
            Err(JudgeError::Superseded) => "Superseded",
        };
        f.debug_struct("JudgeExchange")
            .field("sent_len", &self.sent.len())
            .field("received_len", &self.received.as_ref().map(String::len))
            .field("result", &result)
            .field("elapsed", &self.elapsed)
            .finish()
    }
}

#[derive(Debug)]
pub enum StartCheck {
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

/// 원문과 결과는 `JudgeExchange`에서 온다.
#[derive(Debug, Clone)]
pub struct RecordContext {
    /// `/record off`인지 `store`가 이것으로 본다.
    pub chat: ChatId,
    /// 입력과 무관한 호출(`compact`, `loop` 등)은 `None`.
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
    /// 한도 전 실패는 `JudgePaused`, 한도에 닿으면 `IntakeStopped`, 성공이면 `None`.
    fn observe(&mut self, ok: bool) -> Option<Alert> {
        if ok {
            self.consecutive = 0;
            return None;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        if self.intake_stopped() {
            Some(Alert::IntakeStopped)
        } else {
            Some(Alert::JudgePaused)
        }
    }

    fn intake_stopped(&self) -> bool {
        self.consecutive >= CONSECUTIVE_FAILURE_LIMIT
    }
}

#[derive(Debug)]
pub struct Judges {
    active: ActiveJudge,
    method: Method,
    failures: FailureTracker,
    masker: Masker,
}

impl Judges {
    /// 외부 judge 모델 기본값과 재시도 정책, 로컬 설정 키 이름은 초안이다.
    /// TODO(#40): `collect` 방식이 어떤 judge를 쓰는지 미정. 정해지기 전에는 `NotConfigured`
    ///
    /// # Errors
    /// 주소가 HTTPS나 허용 호스트가 아니면 `DisallowedEndpoint`, 방식에 맞는 설정이 없으면 `NotConfigured`.
    pub fn select(
        settings: &Settings,
        secrets: SharedSecrets,
        masker: Masker,
    ) -> Result<Self, JudgesError> {
        let method = settings.method();
        let active = match method {
            Method::Jev => {
                let model = settings
                    .get("judge.model")
                    .and_then(|value| value.as_str())
                    .unwrap_or("jev-1.13.0")
                    .to_owned();
                ActiveJudge::Remote(RemoteJudge::new(
                    settings.judge_endpoint(),
                    model,
                    secrets,
                    RetryPolicy::default(),
                )?)
            }
            Method::Saturn => {
                let endpoint = settings
                    .get("judge.local.endpoint")
                    .and_then(|value| value.as_str())
                    .ok_or(JudgesError::NotConfigured { method })?;
                let version = settings
                    .get("judge.local.version")
                    .and_then(|value| value.as_str())
                    .unwrap_or(UNVERSIONED_LOCAL);
                ActiveJudge::Local(LocalJudge::new(
                    LocalSource::Server {
                        endpoint: endpoint.to_owned(),
                    },
                    version.to_owned(),
                ))
            }
            Method::Collect => return Err(JudgesError::NotConfigured { method }),
        };
        Ok(Self::with_active(active, method, masker))
    }

    pub(crate) fn with_active(active: ActiveJudge, method: Method, masker: Masker) -> Self {
        Self {
            active,
            method,
            failures: FailureTracker::default(),
            masker,
        }
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn active(&self) -> &ActiveJudge {
        &self.active
    }

    /// 실패는 오류가 아니라 원인을 가린 한 줄과 함께 `KeyRequired`로 돌려준다.
    pub async fn check(&self, settings: &Settings) -> StartCheck {
        let skip = settings
            .get("judge.skip_check")
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
    pub async fn accept_key(
        &mut self,
        input: KeyInput,
        secrets: &SharedSecrets,
        settings: &SettingsManager,
    ) -> Result<(), JudgesError> {
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
            return Err(JudgesError::Check(error));
        }
        secrets.lock().await.persist_current()?;
        settings.record_key_info(&info).await?;
        Ok(())
    }

    /// 형식 오류(`Invalid`)와 revision 변경은 judge가 답한 것이라 연속 실패로 세지 않는다.
    pub async fn call(&mut self, request: JudgeRequest) -> (JudgeExchange, Option<Alert>) {
        let exchange = self.active.exchange(request).await;
        let answered = matches!(
            exchange.result,
            Ok(_) | Err(JudgeError::Invalid { .. } | JudgeError::Superseded)
        );
        let alert = self.failures.observe(answered);
        (exchange, alert)
    }

    /// 참이면 engine은 `SubmitInput`을 접수하지 않는다.
    pub fn intake_stopped(&self) -> bool {
        self.failures.intake_stopped()
    }

    /// 원문은 `Masker`로 가린 뒤 넘기고, `/record off` 채팅이면 `store`가 쓰지 않는다.
    pub async fn record(
        &self,
        store: &Store,
        context: RecordContext,
        exchange: &JudgeExchange,
    ) -> Result<Option<JudgmentId>, JudgesError> {
        let judgment = new_judgment(self, context, exchange);
        Ok(store.record_judgment(&judgment).await?)
    }
}

pub(crate) fn check_request(model: String) -> JudgeRequest {
    JudgeRequest {
        model,
        state: "Saturn judge start check.".to_owned(),
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
pub fn sanitize_state(state: &str, masker: &Masker) -> String {
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
pub fn outcome_of(result: &Result<JudgeResponse, JudgeError>) -> JudgmentOutcome {
    match result {
        Ok(_) => JudgmentOutcome::Ok,
        Err(JudgeError::TimedOutAfterSend) => JudgmentOutcome::CostUnknown,
        Err(JudgeError::NoResponse | JudgeError::RateLimited | JudgeError::Unauthorized) => {
            JudgmentOutcome::NoResponse
        }
        Err(JudgeError::Invalid { .. }) => JudgmentOutcome::Invalid,
        Err(JudgeError::Superseded) => JudgmentOutcome::Superseded,
    }
}

fn new_judgment(judges: &Judges, context: RecordContext, exchange: &JudgeExchange) -> NewJudgment {
    let response = exchange.result.as_ref().ok();
    let tokens = match context.outcome {
        JudgmentOutcome::CostUnknown | JudgmentOutcome::NoResponse => None,
        _ => response.map(|response| response.tokens),
    };
    let model = judges.active.model().to_owned();
    NewJudgment {
        chat: context.chat,
        input: context.input,
        method: judges.method,
        judge: judges.active.judge_id().to_owned(),
        model: (
            model.clone(),
            response.map(|response| response.model.clone()),
        ),
        question_sets: context.question_sets,
        settings: context.settings,
        sent: judges.masker.mask(&exchange.sent),
        received: exchange
            .received
            .as_ref()
            .map(|received| judges.masker.mask(received)),
        answers: response
            .map(|response| response.answers.clone())
            .unwrap_or_default(),
        fallbacks: context.fallbacks,
        tokens,
        started_at: exchange.started_at,
        elapsed: exchange.elapsed,
        outcome: context.outcome,
        judge_version: model,
        thresholds: context.thresholds,
        asked_with: context.asked_with,
    }
}

/// 다른 모듈 테스트가 가짜 전송으로 judge를 만든다.
#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use super::remote::tests::{FakeTransport, KEY, judge, ok, status};
    pub(crate) use super::remote::{HttpReply, TransportError};
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_core::judges::Answer;

    use super::*;
    use crate::judges::remote::tests::{
        ANSWER, FakeTransport, KEY, judge, ok, request, secrets_with_key, status,
    };
    use crate::secrets::StorageMode;

    fn judges(secrets: SharedSecrets, transport: Arc<FakeTransport>) -> Judges {
        let active = ActiveJudge::Remote(judge(secrets, transport));
        Judges::with_active(active, Method::Jev, Masker::new(vec![KEY.to_owned()]))
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
        let judges = judges(
            secrets_with_key(dir.path()).await,
            FakeTransport::new(Vec::new()),
        );
        let exchange = JudgeExchange {
            sent: String::new(),
            received: None,
            result: Err(JudgeError::NoResponse),
            started_at: SystemTime::now(),
            elapsed: Duration::ZERO,
        };

        let judgment = new_judgment(
            &judges,
            context(ChatId(1), JudgmentOutcome::NoResponse),
            &exchange,
        );

        assert_eq!(judgment.thresholds, vec![("keep_current".to_owned(), 0.8)]);
        assert_eq!(judgment.asked_with, Some(0.25));
    }

    #[test]
    fn three_failures_stop_intake_and_success_resets() {
        let mut tracker = FailureTracker::default();

        assert_eq!(tracker.observe(false), Some(Alert::JudgePaused));
        assert_eq!(tracker.observe(false), Some(Alert::JudgePaused));
        assert_eq!(tracker.observe(false), Some(Alert::IntakeStopped));
        assert!(tracker.intake_stopped());
        assert_eq!(tracker.observe(true), None);
        assert!(!tracker.intake_stopped());
    }

    #[tokio::test]
    async fn call_counts_only_unanswered_failures() {
        let dir = tempfile::tempdir().unwrap();
        let invalid = r#"{"answers":{"keep_current":{"noul":-1}}}"#;
        let transport = FakeTransport::new(vec![
            ok(invalid),
            Err(remote::TransportError::AfterSend),
            Err(remote::TransportError::AfterSend),
            Err(remote::TransportError::AfterSend),
        ]);
        let mut judges = judges(secrets_with_key(dir.path()).await, transport);

        let (first, alert) = judges.call(request()).await;
        assert!(matches!(first.result, Err(JudgeError::Invalid { .. })));
        assert_eq!(alert, None);
        for _ in 0..2 {
            assert_eq!(judges.call(request()).await.1, Some(Alert::JudgePaused));
        }
        assert_eq!(judges.call(request()).await.1, Some(Alert::IntakeStopped));
        assert!(judges.intake_stopped());
    }

    #[tokio::test]
    async fn judgments_are_recorded_masked_and_skipped_when_off() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(&dir.path().join("home")).await.unwrap();
        let chat = store.create_chat(PathBuf::from("/w")).await.unwrap();
        let reply = ANSWER.replace("\"usage\"", &format!("\"echo\":\"{KEY}\",\"usage\""));
        let transport =
            FakeTransport::new(vec![ok(&reply), Err(remote::TransportError::AfterSend)]);
        let mut judges = judges(secrets_with_key(dir.path()).await, transport);
        let mut leaky = request();
        leaky.state = format!("pasted {KEY}");

        let (exchange, _) = judges.call(leaky).await;
        let outcome = outcome_of(&exchange.result);
        let id = judges
            .record(&store, context(chat, outcome), &exchange)
            .await
            .unwrap();
        let (timeout, _) = judges.call(request()).await;
        let timeout_outcome = outcome_of(&timeout.result);
        judges
            .record(&store, context(chat, timeout_outcome), &timeout)
            .await
            .unwrap();
        store.set_recording(chat, false).await.unwrap();
        let skipped = judges
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
        assert_eq!(lines[0]["judge"], "jev");
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
        let key_file = dir.path().join("judge.key");
        let empty = SecretStore::with_key_file(key_file.clone(), StorageMode::Standard);
        let secrets: SharedSecrets = Arc::new(Mutex::new(empty));
        let check_ok = r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"noul":0.6}},"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let transport = FakeTransport::new(vec![
            status(401, "{}", Vec::new()),
            ok(r#"{"models":[]}"#),
            ok(check_ok),
        ]);
        let mut judges = Judges::with_active(
            ActiveJudge::Remote(judge(Arc::clone(&secrets), transport)),
            Method::Jev,
            Masker::default(),
        );

        let rejected = judges
            .accept_key(
                KeyInput::Hidden("sk-wrong-0000".to_owned()),
                &secrets,
                &settings,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            rejected,
            JudgesError::Check(JudgeError::Unauthorized)
        ));
        assert!(!key_file.exists());

        judges
            .accept_key(KeyInput::Hidden(format!("{KEY}\n")), &secrets, &settings)
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&key_file).unwrap(), KEY);
        let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains("last4 = \"6789\""));
        assert!(!config.contains(KEY));
        assert_eq!(judges.masker.mask(KEY).as_str(), "[redacted]");
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
        let judges = judges(secrets_with_key(dir.path()).await, transport);

        let failed = judges.check(&settings).await;

        assert!(
            matches!(failed, StartCheck::KeyRequired { ref reason } if reason == "judge rejected the key")
        );
        std::fs::write(home.join("config.toml"), "[judge]\nskip_check = true\n").unwrap();
        let skip = manager.apply_user(&store).await.unwrap().revision;
        let skipping = manager.at(&store, skip).await.unwrap();
        assert!(matches!(judges.check(&skipping).await, StartCheck::Skipped));
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
        let jev = load(&mut manager, &store, &home, "").await;
        let chosen = Judges::select(&jev, Arc::clone(&secrets), Masker::default()).unwrap();
        assert_eq!(chosen.active().judge_id(), "jev");
        let evil = load(
            &mut manager,
            &store,
            &home,
            "[judge]\nendpoint = \"https://evil.example\"\n",
        )
        .await;
        assert!(matches!(
            Judges::select(&evil, Arc::clone(&secrets), Masker::default()),
            Err(JudgesError::DisallowedEndpoint { .. })
        ));
        let saturn = load(
            &mut manager,
            &store,
            &home,
            "[judge]\nmethod = \"saturn\"\n",
        )
        .await;
        assert!(matches!(
            Judges::select(&saturn, Arc::clone(&secrets), Masker::default()),
            Err(JudgesError::NotConfigured { .. })
        ));
        let local = load(
            &mut manager,
            &store,
            &home,
            "[judge]\nmethod = \"saturn\"\n[judge.local]\nendpoint = \"http://127.0.0.1:9\"\nversion = \"v2\"\n",
        )
        .await;
        let chosen = Judges::select(&local, secrets, Masker::default()).unwrap();
        assert_eq!(chosen.active().judge_id(), "saturn-local");
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
        let ok = Ok(JudgeResponse {
            model: "m".to_owned(),
            answers: vec![("a".to_owned(), Answer::Noul(0.5))],
            tokens: (1, 1),
        });

        assert_eq!(outcome_of(&ok), JudgmentOutcome::Ok);
        assert_eq!(
            outcome_of(&Err(JudgeError::TimedOutAfterSend)),
            JudgmentOutcome::CostUnknown
        );
        assert_eq!(
            outcome_of(&Err(JudgeError::RateLimited)),
            JudgmentOutcome::NoResponse
        );
        assert_eq!(
            outcome_of(&Err(JudgeError::Invalid {
                reason: String::new()
            })),
            JudgmentOutcome::Invalid
        );
        assert_eq!(
            outcome_of(&Err(JudgeError::Superseded)),
            JudgmentOutcome::Superseded
        );
    }

    #[test]
    fn exchange_debug_hides_text() {
        let exchange = JudgeExchange {
            sent: format!("state {KEY}"),
            received: None,
            result: Err(JudgeError::Invalid {
                reason: KEY.to_owned(),
            }),
            started_at: SystemTime::now(),
            elapsed: Duration::from_millis(5),
        };

        let debug = format!("{exchange:?}");

        assert!(!debug.contains(KEY));
        assert!(debug.contains("sent_len"));
        assert!(debug.contains("Invalid"));
    }
}
