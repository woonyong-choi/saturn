//! Saturn engine: `saturn-core` 규칙을 실제 연결(provider, judge, SQLite, 프로세스, 키체인)과 조립해 화면 없이 돌린다.
//!
//! 설계: docs/architecture.md(실행 흐름, 불변 조건), docs/design/engine-lifecycle.md, docs/design/input-handling.md.
//! 사용자당 하나이고 잠금으로 지킨다. `Engine`이 모든 구성 요소를 소유하고 흐름 함수가 순서대로 부른다.
//!
//! 흐름 함수 목록(이름 순서가 곧 흐름이다):
//! - 시작: `run` → `start`(`ensure_not_nested` → `acquire_lock` → `open_store` → `merge_settings` → `verify_judge` → `listen`)
//!   → `recover_after_crash` → `serve` → `shutdown`
//! - 입력 하나: `submit_input` → `judge_next` → `apply_decision` → `dispatch_next` → `deliver` → `on_provider_event` → `on_turn_end`
//! - 멈춤과 재개: `stop_chat`, `continue_held`
//! - TUI 종료 뒤: `on_last_detach` → `enter_background` → `background_expired`
//!
//! 불변 조건(docs/architecture.md): 입력은 저장소 접수 뒤에만 보낸다, 보내기 전 확정된 실패만 다시 보낸다,
//! 판단 결과는 적용 직전 revision을 비교한다, judge는 engine만 부른다.

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

pub mod judges;
pub mod processes;
pub mod providers;
pub mod rpc;
pub mod secrets;
pub mod settings;
pub mod store;
pub mod training;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use saturn_core::agents::AgentTracker;
use saturn_core::judges::RouteDecision;
use saturn_core::providers::ProviderError;
use saturn_core::queue::{Queue, QueueError, SendAction};
use saturn_core::sessions::{SessionError, SessionManager};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, InputId, Provider, RunId, TaskId};
use saturn_protocol::rpc::Request;

use crate::judges::{Judges, JudgesError, SharedSecrets};
use crate::processes::{ProcessError, Supervisor};
use crate::providers::ProviderConnection;
use crate::rpc::{ClientId, EngineLock, RpcError, RpcServer};
use crate::secrets::{Masker, SecretsError};
use crate::settings::{SettingsError, SettingsManager};
use crate::store::{MigrationNotice, RunRecord, Store, StoreError};
use crate::training::{TrainPlan, TrainingError};

/// engine 오류. 시작 단계 오류면 engine은 원인 한 줄을 stderr에 보이고 소켓을 열지 않고 끝난다.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 에이전트가 작업 중에 실행한 engine이다. 자식 Saturn을 부모에 잇는 방식이 정해질 때까지 거절한다.
    #[error("nested saturn is not allowed inside an agent task")]
    Nested,
    /// judge 시작 확인과 키 재확인이 모두 실패했다. 실행하지 않는다.
    #[error("judge is not available: {reason}")]
    JudgeUnavailable {
        /// 원인 한 줄(가린 값).
        reason: String,
    },
    /// 잠금, 소켓, 접속 오류.
    #[error("rpc failed")]
    Rpc(#[from] RpcError),
    /// 기록 저장소 오류.
    #[error("record store failed")]
    Store(#[from] StoreError),
    /// 설정 오류.
    #[error("settings failed")]
    Settings(#[from] SettingsError),
    /// judge 선택, 확인, 기록 오류.
    #[error("judge failed")]
    Judges(#[from] JudgesError),
    /// 키 받기, 저장 오류.
    #[error("secrets failed")]
    Secrets(#[from] SecretsError),
    /// provider 연결 오류.
    #[error("provider failed")]
    Provider(#[from] ProviderError),
    /// 대기열 규칙 위반.
    #[error("queue rule violated")]
    Queue(#[from] QueueError),
    /// session 규칙 위반.
    #[error("session rule violated")]
    Session(#[from] SessionError),
    /// 프로세스 실행·중지 오류.
    #[error("process supervision failed")]
    Process(#[from] ProcessError),
    /// 학습 오류.
    #[error("training failed")]
    Training(#[from] TrainingError),
}

/// engine 실행 선택. `main`이 명령 인자에서 만든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    /// Saturn 폴더(`~/.saturn`).
    pub home: PathBuf,
    /// engine을 띄운 작업 폴더. 폴더 설정 층 검색의 시작점.
    pub workdir: PathBuf,
    /// 실행 층 `-c key=value` 원문.
    pub run_overrides: Vec<String>,
}

/// TUI 접속 상태. `on_exit`와 background 유예를 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// TUI가 하나 이상 붙어 있다.
    Attached,
    /// TUI가 없다. 접수된 입력은 계속 처리하고, 허가 요청은 보관하고, 피드백 질문은 건너뛴다.
    /// `idle_since`는 모든 에이전트가 트리 유휴가 된 시각이고, `IDLE_GRACE`(5분)가 지나면 engine을 끝낸다.
    Background { idle_since: Option<Instant> },
}

/// 실행 중 연결 정보. 기록 저장소가 정본이고 이것은 빠른 조회용 사본이다.
#[derive(Debug, Default)]
struct Runs {
    /// 에이전트별 진행 중인 실행.
    active: HashMap<AgentId, RunId>,
    /// 에이전트가 속한 채팅.
    chat_of: HashMap<AgentId, ChatId>,
    /// 에이전트가 맡은 작업.
    task_of: HashMap<AgentId, TaskId>,
}

/// engine 하나. 모든 구성 요소를 소유한다.
#[derive(Debug)]
pub struct Engine {
    /// 실행 선택.
    options: EngineOptions,
    /// 기록 저장소. 쓰는 쪽은 이 engine 하나.
    store: Store,
    /// 설정 병합과 설정 번호.
    settings: SettingsManager,
    /// judge 키 보관소. `RemoteJudge`와 함께 쓴다.
    secrets: SharedSecrets,
    /// 로그와 판단 기록 저장 전 가림.
    masker: Masker,
    /// provider 프로세스 감시.
    supervisor: Supervisor,
    /// provider별 연결.
    providers: HashMap<Provider, ProviderConnection>,
    /// 판단 방식이 고른 judge.
    judges: Judges,
    /// TUI 접속과 잠금.
    rpc: RpcServer,
    /// 채팅별 대기열(채팅 id로 나뉜다).
    queue: Queue,
    /// 채팅별 session 기록.
    sessions: SessionManager,
    /// subagent 트리와 사용량.
    agents: AgentTracker,
    /// 실행 중 연결 정보.
    runs: Runs,
    /// TUI 접속 상태.
    presence: Presence,
    /// 확인 창에 보낸 학습 계획. `ConfirmTrain`을 기다린다.
    pending_train: Option<TrainPlan>,
}

impl Engine {
    /// engine 전체 수명.
    /// 1. `start`로 시작 순서를 밟는다.
    /// 2. `recover_after_crash`로 끝나지 않은 실행을 나눈다.
    /// 3. `serve`로 접속, 요청, provider 이벤트, 유예 시계를 처리한다.
    /// 4. `shutdown`으로 session을 닫고 프로세스를 멈추고 잠금을 푼다.
    ///
    /// # Errors
    /// 시작 단계 오류면 그대로 돌려준다. `main`이 원인 한 줄을 stderr에 쓴다.
    pub async fn run(options: EngineOptions) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 1단계. 시작 순서. 앞 단계가 실패하면 뒤 단계를 하지 않는다.
    /// 0. `ensure_not_nested`: 에이전트 안에서 실행됐으면 거절.
    /// 1. `acquire_lock`: 사용자당 engine 잠금.
    /// 2. `open_store`: 스키마 이관(직전 백업, 14일 지난 백업 삭제).
    /// 3. `merge_settings`: 설정 층 병합과 설정 번호 확정, 자동 정리가 켜져 있으면 `prune_on_start` 한 번.
    /// 4. `verify_judge`: 판단 방식의 judge 확인, 실패하면 키 요청 → 재확인 → 저장.
    /// 5. `listen`: 소켓 접속 수락 시작.
    ///
    /// # Errors
    /// 단계별 오류. judge를 확인하지 못하면 `JudgeUnavailable`.
    async fn start(options: EngineOptions) -> Result<Self, EngineError> {
        todo!("#90")
    }

    /// 시작 0단계. 자식 환경 표지(`processes::NESTED_MARKER_ENV`)가 있으면 거절한다. 판정 기준은 cli와 같다.
    /// TODO(#33): 자식 Saturn을 부모 engine에 붙일지, 독립 engine으로 띄울지 정해지면 거절 대신 연결한다
    ///
    /// # Errors
    /// 에이전트 안에서 실행됐으면 `Nested`.
    fn ensure_not_nested() -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 시작 1단계. `home` 아래 잠금을 잡는다. 이미 잡혀 있으면 두 번째 engine은 끝나고 cli는 기존 소켓에 붙는다.
    ///
    /// # Errors
    /// 다른 engine이 있으면 `Rpc(AlreadyRunning)`.
    fn acquire_lock(options: &EngineOptions) -> Result<EngineLock, EngineError> {
        todo!("#90")
    }

    /// 시작 2단계. 기록 저장소를 열고 스키마를 맞춘다. 이관했으면 안내 한 줄을 stderr에 쓰고 첫 TUI에도 보낸다.
    ///
    /// # Errors
    /// 파일 버전이 더 높거나 백업·이관 실패면 `Store`.
    async fn open_store(
        options: &EngineOptions,
    ) -> Result<(Store, Option<MigrationNotice>), EngineError> {
        todo!("#90")
    }

    /// 시작 3단계. 설정을 병합하고 설정 번호를 확정한다. 폴더 설정을 신뢰하지 않았으면 그 층을 빼고 시작하고
    /// 첫 TUI가 붙을 때 신뢰 창을 연다. 자동 정리 정책이 켜져 있으면 `Store::prune_on_start`를 한 번 부른다.
    ///
    /// # Errors
    /// 검사 실패이고 이전 설정 번호도 없으면 `Settings(NoPreviousRevision)`. 실행하지 않는다.
    async fn merge_settings(
        options: &EngineOptions,
        store: &Store,
    ) -> Result<SettingsManager, EngineError> {
        todo!("#90")
    }

    /// 시작 4단계. judge 시작 확인.
    /// 1. `SecretStore::open`(설정의 저장 방식)과 `Judges::select`(판단 방식).
    /// 2. `Judges::check`. `Ready`나 `Skipped`면 끝.
    /// 3. `KeyRequired`면 키를 받는다: 네 방법 중 쓸 수 있는 것. 순서는 `secrets::input_order`(초안, 설계에 없음)로
    ///    환경 변수 → 관리자 명령 설정 → 표준 입력 → 숨김 입력이다.
    ///    입력할 수 없는 환경(`secrets::can_prompt`가 거짓)이면 환경 변수와 표준 입력 방식을 안내하고 끝낸다.
    /// 4. `Judges::accept_key`: 다시 확인하고 저장한다.
    ///
    /// TODO(#93): cli가 띄운 engine이 숨김 입력을 받을 터미널을 넘겨받는 방식
    ///
    /// # Errors
    /// 다시 확인도 실패하거나 키 입력을 거부하면 `JudgeUnavailable`.
    async fn verify_judge(
        store: &Store,
        settings: &SettingsManager,
    ) -> Result<(Judges, SharedSecrets, Masker), EngineError> {
        todo!("#90")
    }

    /// 시작 5단계. 소켓을 열어 접속을 받기 시작한다. judge 확인 전에는 부르지 않는다.
    ///
    /// # Errors
    /// 소켓을 열지 못하면 `Rpc(Bind)`.
    async fn listen(options: &EngineOptions, lock: EngineLock) -> Result<RpcServer, EngineError> {
        todo!("#90")
    }

    /// 2단계. 크래시 뒤 복구.
    /// 1. `Store::unfinished_runs`로 끝나지 않은 실행과 `effect_scope`를 읽는다.
    /// 2. `EffectScope::allows_auto_resume`(`proven-by-config`, `proven-by-observation`)이면 `resume_proven`.
    /// 3. 그 밖(`network-possible`, `unobserved`)이면 `hold_unproven`.
    /// 크래시 전에 보낸 패킷은 어느 경우에도 다시 보내지 않는다.
    /// TODO(#66): 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 정리할지, 끊김 표시만 할지
    ///
    /// # Errors
    /// 조회나 기록 실패면 `Store`.
    async fn recover_after_crash(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 복구 2단계. 증명된 실행을 자동으로 이어 간다. 파일 상태를 확인한 뒤 그 상태로 만든 새 입력을 접수해 보낸다.
    /// TODO(#65): 파일 상태 확인에 쓰는 수정 파일 목록을 실행 경계의 파일 상태 차이로 셀지, provider 이벤트로 셀지
    ///
    /// # Errors
    /// 접수나 기록 실패면 `Store`, 전송 실패면 `Provider`.
    async fn resume_proven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 복구 3단계. 증명되지 않은 실행을 보류로 기록하고 session도 보류(`SessionState::Held`)로 둔다.
    /// 보류마다 `/continue <이름표>` 제안 한 줄(`ChatNotice::ResumeSuggested`)을 채팅 기록에 남겨 TUI가 붙으면 보이게 한다.
    ///
    /// # Errors
    /// 기록 실패면 `Store`.
    async fn hold_unproven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 3단계. 주 반복. 아래 중 먼저 온 것 하나를 처리하고 되돌아간다.
    /// - `RpcServer::next_event`: 접속 → 대기, 요청 → `handle_request`, 마지막 끊김 → `on_last_detach`.
    /// - provider별 `next_event` → `on_provider_event`.
    /// - 유예 시계: `SessionManager::due_for_close`로 session 닫기, `background_expired`면 반복을 끝낸다.
    ///
    /// # Errors
    /// 복구할 수 없는 오류(저장소 쓰기 실패 등)만 돌려준다. 요청 하나의 오류는 그 클라이언트에 알리고 계속한다.
    async fn serve(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 요청 하나를 흐름 함수로 나눈다. `SubmitInput` → `submit_input`, `Stop` → `stop_chat`, `Continue` → `continue_held`,
    /// `Attach` → `attach`, `Detach` → 끊김 처리, `SubmitJudgeKey` → `Judges::accept_key`(메시지를 기록하지 않는다),
    /// `Train`·`ConfirmTrain` → `training`, 나머지 조회·정리 요청은 `store`·`settings`로 보낸다.
    /// TODO(#46): RPC 메서드 이름과 목록이 확정되면 여기 분배표를 맞춘다
    ///
    /// # Errors
    /// 처리 흐름의 오류.
    async fn handle_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 채팅에 붙는다. `chat`이 없으면 새 채팅을 만든다. 폴더 설정 신뢰가 필요하면 먼저 묻고,
    /// `RpcServer::greet`로 `StartInfo` → 최근 기록 → 보관한 허가 요청 순서로 보낸다. `Presence`를 `Attached`로 바꾼다.
    /// `overrides`는 이 접속의 입력에만 적용하는 실행 층이다.
    ///
    /// # Errors
    /// 채팅 생성이나 기록 조회 실패면 `Store`, 접속이 끊겼으면 `Rpc`.
    async fn attach(
        &mut self,
        client: ClientId,
        chat: Option<ChatId>,
        overrides: Vec<(String, String)>,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 1단계. 접수(ACK).
    /// 1. judge 연속 3회 실패로 접수를 멈춘 상태면 거절하고 `Alert::IntakeStopped`를 보낸다.
    /// 2. 설정 파일이 바뀌었으면 먼저 `SettingsManager::apply`(폴더 설정이 바뀌었으면 신뢰 창).
    /// 3. `Store::accept_input`으로 기록 저장소에 접수한다. 설정 번호와 권한을 이때 고정한다.
    /// 4. `Queue::accept`에 넣고 보낸 클라이언트에 `InputAccepted`, 채팅의 모든 클라이언트에 `InputChanged`를 보낸다.
    /// 5. `judge_next`로 판단 차례를 돌린다.
    ///
    /// # Errors
    /// 접수 기록 실패면 `Store`. 이때 입력은 접수되지 않았으므로 어디에도 보내지 않는다.
    async fn submit_input(
        &mut self,
        client: ClientId,
        chat: ChatId,
        client_ref: u64,
        text: String,
        pinned_model: Option<String>,
        skip_relation: bool,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 2단계. 판단 차례와 judge 호출.
    /// 1. `Queue::next_to_judge`로 같은 채팅의 다음 입력과 판단 시점 revision을 받는다(접수 순서로 하나씩).
    /// 2. 모델을 고정한 입력이면 judge를 부르지 않는다. 그 밖은 `questions_for_input`으로 질문을 고르고,
    ///    앞 입력의 판단 결과를 넣은 `state`(`judges::sanitize_state`로 거른 값)로 요청 한 건을 만든다.
    /// 3. `Judges::call` → `validate` → `decide_route`. 무응답이면 대기, 일시 실패면 질문별 대체 규칙.
    /// 4. `Judges::record`로 판단 기록을 쓴다(방식과 관계없이 전부, `/record off` 채팅 제외).
    /// 5. `apply_decision`으로 넘긴다.
    ///
    /// # Errors
    /// 판단 기록 실패면 `Judges`.
    async fn judge_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 3단계. 판단 적용(CAS).
    /// 1. `Queue::apply`가 적용 직전 revision을 비교한다.
    /// 2. `RevisionConflict`면 한 번 다시 판단한다(`retried`가 거짓일 때만). 또 어긋나면 대기로 둔다.
    ///    판단 중 revision이 바뀐 판단은 `superseded`로 기록한다.
    /// 3. `resume_held` 판단을 `Queue::note_resume_signal`에 넘긴다. 무시가 3번 쌓이면 보류를 종료한다.
    /// 4. 결과(끼워 넣기, 새 작업, 대기)를 `InputChanged`로 알리고 `dispatch_next`를 부른다.
    ///
    /// # Errors
    /// 없는 입력이면 `Queue`, 상태 기록 실패면 `Store`.
    async fn apply_decision(
        &mut self,
        input: InputId,
        decision: RouteDecision,
        retried: bool,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 4단계. 보낼 입력 고르기.
    /// 1. `Queue::next_to_send`가 대기열 맨 앞부터 쓰기 규칙을 적용해 `SendAction`을 준다.
    /// 2. 보내는 순간 `SessionManager::target_for_send`로 대상 session을 고른다
    ///    (열린 session, 닫힌 session 재개, provider 전환이나 compaction 재시작이면 새 session과 패킷).
    /// 3. 끼워 넣기인데 `providers::steer_route`가 `Queue`면 대기로 바꾸고 `바로 반영: 준비 중`을 보인다.
    /// 4. `deliver`로 보낸다. 보낼 것이 없을 때까지 반복한다.
    ///
    /// # Errors
    /// session 규칙 위반이면 `Session`, 연결 실패면 `Provider`.
    async fn dispatch_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 5단계. provider 전달.
    /// 1. `Store::start_run`으로 실행을 먼저 기록하고 입력 상태를 `Delivering`으로 바꾼다.
    /// 2. `Steer`면 `ProviderClient::steer`, `NewTurn`·`NewTask`면 `send_turn`.
    /// 3. 끼워 넣기가 `NoActiveTurn`이면 `steer_as_new_turn`으로 넘긴다.
    /// 4. `NotSent`(보내기 전 확정 실패)만 다시 보낸다. `Unknown`이면 다시 보내지 않고 작업을 `NeedsCheck`로 두어 사용자 확인으로 넘긴다.
    /// 5. 받았으면 `Applied`, 거절이면 `Rejected`로 기록하고 알린다.
    ///
    /// # Errors
    /// 기록 실패면 `Store`, 재전송할 수 없는 연결 실패면 `Provider`.
    async fn deliver(&mut self, chat: ChatId, action: SendAction) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 끼워 넣기 실패 뒤 새 턴. 활성 턴 없음은 확정 미전달이므로 기록하고, 다시 판단하지 않고
    /// 같은 session에 새 턴(`send_turn`)으로 한 번 보낸다.
    ///
    /// # Errors
    /// 새 턴 전송이 `Unknown`이면 다시 보내지 않고 `Provider`를 돌려준다.
    async fn steer_as_new_turn(
        &mut self,
        input: InputId,
        agent: AgentId,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 6단계. provider 이벤트 하나.
    /// 1. `Usage`면 `Store::record_usage`, 그 밖은 `Store::append_event`로 먼저 기록한다.
    /// 2. `AgentTracker::on_event`로 subagent 트리를 갱신한다. `StreamLost`면 실행의 `effect_scope`를 `Unobserved`로 바꾼다.
    /// 3. `PermissionRequested`는 `RpcServer::offer_permission`(TUI가 없으면 보관).
    /// 4. 채팅의 TUI에 `TaskEvent`, `TaskChanged`, `ContextSize`를 보낸다.
    /// 5. `TurnCompleted`이고 트리 유휴면 `on_turn_end`.
    ///
    /// # Errors
    /// 기록 실패면 `Store`.
    async fn on_provider_event(
        &mut self,
        provider: Provider,
        event: ProviderEvent,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 입력 7단계. 턴 끝(트리 유휴).
    /// 1. `Store::finish_run`, 쓰기 잠금 해제(`WriteGate::release`).
    /// 2. `context::decide`로 compaction 판정. `Restart`면 `build_packet`으로 패킷을 만들고 새 session을 열어
    ///    `SessionManager::replace`(턴 경계에서만)로 교체하고 `ChatNotice::Compacted`를 보낸다. `Defer`면 다음 턴 경계로 미룬다.
    /// 3. TUI가 붙어 있으면 `calibration::ask_probability`로 피드백 질문 여부를 정한다. background면 건너뛴다.
    /// 4. 모든 작업이 끝났으면 `ChatNotice::RequestSummary`, background면 완료 알림(켠 경우만).
    /// 5. `SessionManager::mark_idle`, `dispatch_next`로 대기 입력을 이어 보낸다.
    ///
    /// # Errors
    /// 기록 실패면 `Store`, session 교체 실패면 `Session`.
    async fn on_turn_end(&mut self, chat: ChatId, agent: AgentId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 멈춤(트리 전체 중지).
    /// 1. `Store::begin_stop`으로 멈춤 시작을 기록한다.
    /// 2. `Queue::stop`이 실행 중 작업과 보내지 않은 입력을 보류로 바꾼다.
    /// 3. `AgentTracker::interrupt_order` 순서(추적된 subagent 깊은 것부터, 마지막이 메인)로 `ProviderClient::interrupt`.
    /// 4. `Supervisor::stop_tree`(10초 뒤 SIGTERM, 그래도 남으면 SIGKILL).
    /// 5. 트리 유휴와 `StopOutcome::Stopped`를 모두 확인한 뒤에만 `Store::end_stop`과 `ChatNotice::Stopped`.
    ///    남은 프로세스가 있으면 완료라 하지 않고 `ChatNotice::StopUnconfirmed`.
    ///
    /// # Errors
    /// 기록 실패면 `Store`, 신호 실패면 `Process`.
    async fn stop_chat(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 보류 재개(`/continue`). `task`가 있으면 그 작업만, 없으면 채팅의 보류 전부를 접수 순서로.
    /// 1. `Queue::resume`이 재개할 작업을 쓰기 규칙에 따라 하나씩 내준다.
    /// 2. 확인된 상태로 만든 새 입력을 접수한다. 같은 패킷은 다시 보내지 않는다.
    /// 3. `dispatch_next`.
    ///
    /// # Errors
    /// 접수 실패면 `Store`, 전송 실패면 `Provider`.
    async fn continue_held(
        &mut self,
        chat: ChatId,
        task: Option<TaskId>,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 마지막 TUI가 떨어졌다. 설정 `on_exit`를 본다. `Background`(기본)면 `enter_background`.
    /// TODO(#70): `stop`과 `ask`의 동작 미정. 정해지기 전에는 `Background`와 같이 처리한다
    ///
    /// # Errors
    /// 설정 조회 실패면 `Settings`.
    async fn on_last_detach(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// TUI 없이 계속한다.
    /// 1. 접수된 대기 입력을 순서대로 계속 보낸다.
    /// 2. 허가 요청은 답하지 않고 보관한다(`RpcServer::offer_permission`).
    /// 3. 피드백 질문을 건너뛴다.
    /// 4. 완료 알림(macOS)은 켠 경우에만 보낸다. TODO(#49): 설정 키, TODO(#90): 알림 보내는 방법
    /// 5. 보류는 그대로 둔다(자동으로 이어 가지 않는다).
    /// 6. 모든 에이전트가 트리 유휴가 되면 그 시각을 `Presence::Background`에 적는다.
    fn enter_background(&mut self) {
        todo!("#90")
    }

    /// background 유예가 끝났는지. 트리 유휴 뒤 `sessions::IDLE_GRACE`(5분)가 지났고 그사이 TUI가 붙지 않았으면 참.
    /// 참이면 `serve`가 반복을 끝내고 `shutdown`으로 간다.
    fn background_expired(&self, now: Instant) -> bool {
        todo!("#90")
    }

    /// 4단계. 종료. 열린 session을 닫고(provider session id는 보관), `Supervisor::stop_all`로 남은 프로세스를 멈추고,
    /// `RpcServer::close`로 소켓을 지우고 잠금을 푼다.
    ///
    /// # Errors
    /// session 기록 실패면 `Store`.
    async fn shutdown(self) -> Result<(), EngineError> {
        todo!("#90")
    }
}
