//! Saturn engine: `saturn-core` 규칙을 실제 연결과 조립해 화면 없이 돌리는 상주 프로세스.
//! 설계: docs/architecture.md

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

/// 시작 단계 오류면 원인 한 줄을 stderr에 보이고 소켓을 열지 않고 끝난다.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 자식 Saturn을 부모에 잇는 방식이 정해질 때까지 에이전트 작업 안의 실행을 거절한다.
    #[error("nested saturn is not allowed inside an agent task")]
    Nested,
    /// judge 시작 확인과 키 재확인이 모두 실패해 실행하지 않는다.
    #[error("judge is not available: {reason}")]
    JudgeUnavailable {
        /// 가린 원인 한 줄.
        reason: String,
    },
    #[error("rpc failed")]
    Rpc(#[from] RpcError),
    #[error("record store failed")]
    Store(#[from] StoreError),
    #[error("settings failed")]
    Settings(#[from] SettingsError),
    #[error("judge failed")]
    Judges(#[from] JudgesError),
    #[error("secrets failed")]
    Secrets(#[from] SecretsError),
    #[error("provider failed")]
    Provider(#[from] ProviderError),
    #[error("queue rule violated")]
    Queue(#[from] QueueError),
    #[error("session rule violated")]
    Session(#[from] SessionError),
    #[error("process supervision failed")]
    Process(#[from] ProcessError),
    #[error("training failed")]
    Training(#[from] TrainingError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    pub home: PathBuf,
    /// 폴더 설정 층 검색의 시작점.
    pub workdir: PathBuf,
    pub run_overrides: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    /// TUI가 하나 이상 붙어 있다.
    Attached,
    /// `idle_since`는 모든 에이전트가 트리 유휴가 된 시각이다.
    Background { idle_since: Option<Instant> },
}

/// 기록 저장소가 정본이고 이것은 빠른 조회용 사본이다.
#[derive(Debug, Default)]
struct Runs {
    active: HashMap<AgentId, RunId>,
    chat_of: HashMap<AgentId, ChatId>,
    task_of: HashMap<AgentId, TaskId>,
}

#[derive(Debug)]
pub struct Engine {
    options: EngineOptions,
    /// 쓰는 쪽은 이 engine 하나.
    store: Store,
    settings: SettingsManager,
    /// `RemoteJudge`와 함께 쓴다.
    secrets: SharedSecrets,
    masker: Masker,
    supervisor: Supervisor,
    providers: HashMap<Provider, ProviderConnection>,
    judges: Judges,
    rpc: RpcServer,
    queue: Queue,
    sessions: SessionManager,
    agents: AgentTracker,
    runs: Runs,
    presence: Presence,
    /// `ConfirmTrain`을 기다린다.
    pending_train: Option<TrainPlan>,
}

impl Engine {
    pub async fn run(options: EngineOptions) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 앞 단계가 실패하면 뒤 단계를 하지 않는다.
    ///
    /// # Errors
    /// judge를 확인하지 못하면 `JudgeUnavailable`.
    async fn start(options: EngineOptions) -> Result<Self, EngineError> {
        todo!("#90")
    }

    /// 판정 기준은 cli와 같다.
    /// TODO(#33): 자식 Saturn을 부모 engine에 붙일지, 독립 engine으로 띄울지
    fn ensure_not_nested() -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 이미 잡혀 있으면 `Rpc(AlreadyRunning)`이고 cli는 기존 소켓에 붙는다.
    fn acquire_lock(options: &EngineOptions) -> Result<EngineLock, EngineError> {
        todo!("#90")
    }

    /// 이관했으면 안내 한 줄을 stderr에 쓰고 첫 TUI에도 보낸다.
    async fn open_store(
        options: &EngineOptions,
    ) -> Result<(Store, Option<MigrationNotice>), EngineError> {
        todo!("#90")
    }

    /// 신뢰하지 않은 폴더 설정은 빼고 시작하고 첫 TUI가 붙을 때 신뢰 창을 연다.
    ///
    /// # Errors
    /// 검사 실패이고 이전 설정 번호도 없으면 `Settings(NoPreviousRevision)`.
    async fn merge_settings(
        options: &EngineOptions,
        store: &Store,
    ) -> Result<SettingsManager, EngineError> {
        todo!("#90")
    }

    /// 키 받는 순서는 `secrets::input_order`를 따른다.
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

    /// judge 확인 전에는 부르지 않는다.
    async fn listen(options: &EngineOptions, lock: EngineLock) -> Result<RpcServer, EngineError> {
        todo!("#90")
    }

    /// 크래시 전에 보낸 패킷은 어느 경우에도 다시 보내지 않는다.
    /// TODO(#66): 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 정리할지, 끊김 표시만 할지
    async fn recover_after_crash(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 파일 상태를 확인한 뒤 그 상태로 만든 새 입력을 접수해 보낸다.
    /// TODO(#65): 수정 파일 목록을 실행 경계의 파일 상태 차이로 셀지, provider 이벤트로 셀지
    async fn resume_proven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    async fn hold_unproven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// # Errors
    /// 복구할 수 없는 오류만 돌려주고, 요청 하나의 오류는 그 클라이언트에 알리고 계속한다.
    async fn serve(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// `SubmitJudgeKey` 메시지는 기록하지 않는다.
    /// TODO(#46): RPC 메서드 이름과 목록이 확정되면 여기 분배표를 맞춘다
    async fn handle_request(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// `overrides`는 이 접속의 입력에만 적용하는 실행 층이다.
    async fn attach(
        &mut self,
        client: ClientId,
        chat: Option<ChatId>,
        overrides: Vec<(String, String)>,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 설정 번호와 권한은 접수 때 고정한다.
    ///
    /// # Errors
    /// 접수 기록 실패면 `Store`이고 입력은 어디에도 보내지 않는다.
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

    /// 모델을 고정한 입력이면 judge를 부르지 않는다.
    async fn judge_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// `RevisionConflict`면 `retried`가 거짓일 때만 한 번 다시 판단하고, 또 어긋나면 대기로 둔다.
    async fn apply_decision(
        &mut self,
        input: InputId,
        decision: RouteDecision,
        retried: bool,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 보낼 것이 없을 때까지 반복한다.
    async fn dispatch_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// `NotSent`만 다시 보내고, `Unknown`이면 작업을 `NeedsCheck`로 두어 사용자 확인으로 넘긴다.
    async fn deliver(&mut self, chat: ChatId, action: SendAction) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 활성 턴 없음은 확정 미전달이라 다시 판단하지 않고 같은 session에 새 턴으로 한 번 보낸다.
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

    /// 이벤트는 처리 전에 먼저 기록한다.
    async fn on_provider_event(
        &mut self,
        provider: Provider,
        event: ProviderEvent,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// compaction 교체는 턴 경계에서만 한다.
    async fn on_turn_end(&mut self, chat: ChatId, agent: AgentId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 트리 유휴와 `StopOutcome::Stopped`를 모두 확인한 뒤에만 멈춤 완료를 알린다.
    async fn stop_chat(&mut self, chat: ChatId) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// `task`가 없으면 채팅의 보류 전부를 접수 순서로 재개하고, 같은 패킷은 다시 보내지 않는다.
    async fn continue_held(
        &mut self,
        chat: ChatId,
        task: Option<TaskId>,
    ) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// TODO(#70): `stop`과 `ask`의 동작이 정해지기 전에는 `Background`와 같이 처리한다
    async fn on_last_detach(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 보류는 자동으로 이어 가지 않는다.
    /// TODO(#49): 완료 알림 설정 키. TODO(#90): 알림 보내는 방법
    fn enter_background(&mut self) {
        todo!("#90")
    }

    /// 트리 유휴 뒤 `sessions::IDLE_GRACE`가 지났고 그사이 TUI가 붙지 않았으면 참.
    fn background_expired(&self, now: Instant) -> bool {
        todo!("#90")
    }

    /// provider session id는 보관한다.
    async fn shutdown(self) -> Result<(), EngineError> {
        todo!("#90")
    }
}
