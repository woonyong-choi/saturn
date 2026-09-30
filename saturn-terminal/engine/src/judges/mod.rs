//! judge 연결 구현과 engine 쪽 판단 흐름: judge 선택(판단 방식), 시작 확인, 키 요청과 재확인, 호출, 연속 실패 집계, 판단 기록.
//!
//! 설계: docs/design/judge.md(판단 방식, judge 시작 확인, judge 호출, 판단 기록, 오류 처리),
//! docs/design/judge-key-security.md(키 입력, 전송, 출력 마스킹).
//! 규칙:
//! - judge는 engine만 부른다. 모든 judge는 `saturn_core::judges::JudgeClient` 구현(`RemoteJudge`, `LocalJudge`)으로만 붙는다.
//! - 질문 구성과 답 해석은 `saturn_core::judges`가 한다. 여기는 전송, 확인, 기록만 한다.
//! - 판단 기록은 판단 방식과 관계없이 전부 남긴다. 원문은 `secrets::Masker`로 가린 뒤 `store`에 넘기고, `/record off` 채팅은 `store`가 거른다.
//! - 보내기 전에 확정된 실패만 다시 보낸다. 보낸 뒤 시간 초과는 `cost-unknown`으로 기록하고 다시 보내지 않는다.
//!
//! 시작 확인 흐름(engine 시작 4단계):
//! 1. `Judges::select`가 판단 방식으로 judge를 고른다.
//! 2. `Judges::check`로 확인한다. 외부는 `GET /v1/models`와 실제 판단 1건, 로컬은 모델 로드나 API 서버 응답.
//! 3. 실패하면 engine이 키를 받아 `Judges::accept_key`를 부른다: 키 받기 → 다시 확인 → `SecretStore::save` → `KeyInfo` 기록.
//! 4. 그래도 실패하면 engine은 원인 한 줄을 보이고 실행하지 않는다.

mod local;
mod remote;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use saturn_core::judges::{
    JudgeClient, JudgeError, JudgeRequest, JudgeResponse, Method, QuestionSetId,
};
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::Alert;
use tokio::sync::Mutex;

use crate::secrets::{KeyInput, Masker, SecretStore, SecretsError};
use crate::settings::{Settings, SettingsError, SettingsManager};
use crate::store::{JudgmentOutcome, NewJudgment, Store, StoreError};

pub use local::{LocalJudge, LocalSource};
pub use remote::{
    ALLOWED_HOST, MAX_CHOICES, REQUEST_SPLIT_LIMIT, RemoteJudge, RetryPolicy, STATE_SPLIT_LIMIT,
};

/// 연속 호출 실패가 이만큼 쌓이면 새 입력 접수를 멈추고 연결 복구를 안내한다(`Alert::IntakeStopped`).
pub const CONSECUTIVE_FAILURE_LIMIT: u32 = 3;

/// engine과 judge가 함께 쓰는 키 보관소. 키를 메모리에 들고 있는 곳은 `SecretStore` 하나뿐이라 `RemoteJudge`도 이것을 빌려 쓴다.
pub type SharedSecrets = Arc<Mutex<SecretStore>>;

/// judge 선택, 시작 확인, 키 처리, 판단 기록 오류. 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum JudgesError {
    /// judge 주소가 HTTPS가 아니거나 허용 호스트(`api.typesafe.ai`)가 아니다. 키를 보내지 않는다.
    #[error("judge endpoint is not allowed: {endpoint}")]
    DisallowedEndpoint {
        /// 설정의 judge 주소.
        endpoint: String,
    },
    /// 판단 방식에 맞는 judge 설정이 없다(`saturn` 방식인데 로컬 모델이 없는 등).
    #[error("no judge configured for method {method:?}")]
    NotConfigured {
        /// 판단 방식.
        method: Method,
    },
    /// 시작 확인 실패. 호출자는 키를 받아 `accept_key`로 다시 확인하거나 실행하지 않는다.
    #[error("judge check failed")]
    Check(#[source] JudgeError),
    /// 키 받기, 저장, 조회 실패.
    #[error("judge key handling failed")]
    Secrets(#[from] SecretsError),
    /// 키 정보(`KeyInfo`)를 사용자 설정에 쓰지 못했다.
    #[error("failed to record judge key info")]
    Settings(#[from] SettingsError),
    /// 판단 기록 저장 실패.
    #[error("failed to record judgment")]
    Store(#[from] StoreError),
}

/// 판단 방식이 고른 judge. `JudgeClient`는 `impl Future`를 돌려 dyn으로 못 쓰므로 enum으로 나눈다.
#[derive(Debug)]
pub enum ActiveJudge {
    /// 기준 judge(외부 API). `jev` 방식.
    Remote(RemoteJudge),
    /// Saturn 모델(로컬). `saturn` 방식.
    Local(LocalJudge),
}

impl ActiveJudge {
    /// 기록에 쓰는 중립 이름(`judge_id`). 예: `jev`, `saturn-local`. 실제 모델과 버전은 설정 매핑과 `judge_manifest`에만 둔다.
    /// TODO(#88): `judge_id` 값 미정
    pub fn judge_id(&self) -> &str {
        todo!("#88")
    }

    /// 요청과 응답 원문을 함께 돌려주는 판단 호출. 판단 기록에 원문이 필요해 trait의 `judge` 대신 engine은 이것을 쓴다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        todo!("#88")
    }
}

impl JudgeClient for ActiveJudge {
    /// 고른 judge에 그대로 넘긴다.
    async fn check(&self) -> Result<(), JudgeError> {
        todo!("#88")
    }

    /// 고른 judge에 그대로 넘긴다.
    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        todo!("#88")
    }
}

/// 판단 호출 한 건의 원문과 결과. 원문은 가리기 전 값이라 로그에 남기지 않고 `record`에서만 가린 뒤 쓴다.
pub struct JudgeExchange {
    /// 보낸 요청 본문(나눠 보냈으면 조각을 순서대로 이은 것). Authorization 헤더는 넣지 않는다.
    pub sent: String,
    /// 받은 응답 본문. 응답이 없으면 `None`.
    pub received: Option<String>,
    /// 해석한 결과. 형식 검사(`saturn_core::judges::validate`)는 호출자가 한다.
    pub result: Result<JudgeResponse, JudgeError>,
    /// 요청 시작 시각.
    pub started_at: SystemTime,
    /// 걸린 시간. 응답이 없으면 포기할 때까지.
    pub elapsed: Duration,
}

impl std::fmt::Debug for JudgeExchange {
    /// 원문은 쓰지 않고 길이, 결과 종류, 걸린 시간만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("#88")
    }
}

/// 시작 확인 결과.
#[derive(Debug)]
pub enum StartCheck {
    /// 확인 통과. 소켓 접속을 받아도 된다.
    Ready,
    /// 확인 실패. engine은 키를 받아 `accept_key`로 넘긴다. `reason`은 화면과 stderr에 보일 한 줄.
    KeyRequired {
        /// 실패 원인 한 줄.
        reason: String,
    },
    /// 설치 검증 전용 설정으로 확인을 건너뛰었다. 도움말에 보이지 않는 설정이다. TODO(#49): 설정 키 이름
    Skipped,
}

/// 판단 기록 한 건에 붙는 문맥. 원문과 결과는 `JudgeExchange`에서 온다.
#[derive(Debug, Clone)]
pub struct RecordContext {
    /// 채팅. `/record off`인지 `store`가 이것으로 본다.
    pub chat: ChatId,
    /// 판단 대상 입력. 입력과 무관한 호출(`compact`, `loop` 등)은 `None`.
    pub input: Option<InputId>,
    /// 물은 질문 세트와 버전.
    pub question_sets: Vec<QuestionSetId>,
    /// 판단 때 쓴 설정 번호.
    pub settings: SettingsRevision,
    /// 대체 규칙을 적용한 질문 id와 사유.
    pub fallbacks: Vec<(String, String)>,
    /// 결과 분류. revision이 바뀌었으면 호출자가 `Superseded`로 바꿔 넘긴다.
    pub outcome: JudgmentOutcome,
}

/// 연속 호출 실패 집계. 성공 한 번이면 0으로 돌아간다.
#[derive(Debug, Default)]
struct FailureTracker {
    /// 연속 실패 수.
    consecutive: u32,
}

impl FailureTracker {
    /// 호출 결과를 반영하고 알릴 경고를 돌려준다. 실패가 `CONSECUTIVE_FAILURE_LIMIT`번 이어지면 `IntakeStopped`,
    /// 그 전 실패는 `JudgePaused`(질문별 대체 규칙 적용 중), 성공이면 `None`.
    fn observe(&mut self, ok: bool) -> Option<Alert> {
        todo!("#88")
    }

    /// 새 입력 접수를 멈춘 상태인지.
    fn intake_stopped(&self) -> bool {
        todo!("#88")
    }
}

/// engine이 쓰는 judge 묶음. 판단 방식이 고른 judge 하나와 연속 실패 집계, 기록용 가림을 가진다.
#[derive(Debug)]
pub struct Judges {
    active: ActiveJudge,
    method: Method,
    failures: FailureTracker,
    masker: Masker,
}

impl Judges {
    /// 판단 방식으로 judge를 고른다. `jev`는 `RemoteJudge`(주소는 `settings.judge_endpoint()`, 허용 호스트 검사),
    /// `saturn`은 `LocalJudge`. TODO(#40): `collect` 방식이 어떤 judge를 쓰는지 미정
    ///
    /// # Errors
    /// 주소가 HTTPS나 허용 호스트가 아니면 `DisallowedEndpoint`, 방식에 맞는 설정이 없으면 `NotConfigured`.
    pub fn select(
        settings: &Settings,
        secrets: SharedSecrets,
        masker: Masker,
    ) -> Result<Self, JudgesError> {
        todo!("#88")
    }

    /// 판단 방식.
    pub fn method(&self) -> Method {
        self.method
    }

    /// 고른 judge.
    pub fn active(&self) -> &ActiveJudge {
        &self.active
    }

    /// 시작 확인 1회. 실패하면 원인을 가린 한 줄과 함께 `KeyRequired`를 돌려준다(오류가 아니다).
    /// 설치 검증 전용 설정이 켜져 있으면 확인 없이 `Skipped`.
    pub async fn check(&self, settings: &Settings) -> StartCheck {
        todo!("#88")
    }

    /// 받은 키로 다시 확인하고 저장한다.
    /// 1. `secrets::acquire`로 키를 받는다(숨김 입력, 표준 입력, 환경 변수, 관리자 명령 네 방법만).
    /// 2. 가림 대상을 새 키로 다시 만든다.
    /// 3. `JudgeClient::check`로 다시 확인한다.
    /// 4. 통과하면 `SecretStore::save`(관리자 명령과 환경 변수 키는 저장하지 않고 메모리에만), `SettingsManager::record_key_info`.
    ///
    /// # Errors
    /// 키를 못 받으면 `Secrets`, 다시 확인이 실패하면 `Check`, 키 정보 기록 실패면 `Settings`.
    pub async fn accept_key(
        &mut self,
        input: KeyInput,
        secrets: &SharedSecrets,
        settings: &SettingsManager,
    ) -> Result<(), JudgesError> {
        todo!("#88")
    }

    /// 판단 호출 한 건. 결과로 연속 실패를 집계하고, 알릴 경고가 있으면 함께 돌려준다.
    /// 무응답이면 호출자가 입력을 대기로 보내고, 실행 중 일시 실패면 질문별 대체 규칙을 적용한다.
    pub async fn call(&mut self, request: JudgeRequest) -> (JudgeExchange, Option<Alert>) {
        todo!("#88")
    }

    /// 연속 3회 실패로 새 입력 접수를 멈춘 상태인지. 참이면 engine은 `SubmitInput`을 접수하지 않는다.
    pub fn intake_stopped(&self) -> bool {
        self.failures.intake_stopped()
    }

    /// 판단 기록을 쓴다.
    /// 1. 보낸 원문과 받은 원문을 `Masker`로 가린다.
    /// 2. `store.record_judgment`에 넘긴다. `/record off` 채팅이면 `store`가 쓰지 않고 `None`을 돌려준다.
    /// 비용 칸은 `CostUnknown`이거나 보고되지 않았으면 NULL이다.
    ///
    /// # Errors
    /// 저장 실패면 `Store`.
    pub async fn record(
        &self,
        store: &Store,
        context: RecordContext,
        exchange: &JudgeExchange,
    ) -> Result<Option<JudgmentId>, JudgesError> {
        todo!("#88")
    }
}

/// 요청 `state`에서 비밀값, 절대 경로, 다른 대화 원문을 뺀다. 비밀값은 `Masker`로 가린다.
/// TODO(#88): 절대 경로와 다른 대화 원문을 찾는 규칙 미정
pub fn sanitize_state(state: &str, masker: &Masker) -> String {
    todo!("#88")
}

/// 호출 결과를 판단 기록 분류로 바꾼다. 보낸 뒤 시간 초과는 `CostUnknown`, 무응답과 속도 제한 포기는 `NoResponse`,
/// 형식 오류는 `Invalid`, revision 변경은 `Superseded`.
pub fn outcome_of(result: &Result<JudgeResponse, JudgeError>) -> JudgmentOutcome {
    todo!("#88")
}

/// 판단 기록 한 건을 만든다. 원문은 여기서 가린다. `record`가 쓴다.
fn new_judgment(judges: &Judges, context: RecordContext, exchange: &JudgeExchange) -> NewJudgment {
    todo!("#88")
}
