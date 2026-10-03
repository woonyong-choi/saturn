//! Saturn engine: `saturn-core` 규칙을 실제 연결과 조립해 화면 없이 돌리는 상주 프로세스.
//! 설계: docs/architecture.md

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

pub mod engine_log;
pub(crate) mod processes;
pub(crate) mod providers;
pub(crate) mod routers;
pub(crate) mod rpc;
pub(crate) mod secrets;
pub(crate) mod settings;
pub(crate) mod store;
pub(crate) mod training;

pub use processes::Supervisor;
pub use providers::{
    HookInputError, LaunchSpec, PermissionLaunch, ProviderConnection, SaturnDefaults,
    UserProviderConfig, run_pre_tool_use,
};
pub use secrets::{Masker, pre_tool_use_hook_settings};

mod add_dir;
mod chat_env;
mod control;
mod dispatch;
mod events;
mod exit;
mod flow;
mod handoff;
mod inputs;
mod intake;
mod launch;
mod models;
mod outcomes;
mod permission;
mod requests;
mod serve;
mod sessions;
mod startup;
mod stop;
mod switch;
mod turn_end;
mod usage;

#[cfg(test)]
mod lifecycle;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use saturn_core::agents::AgentTracker;
use saturn_core::providers::ProviderError;
use saturn_core::queue::{Queue, QueueError};
use saturn_core::sessions::{SessionError, SessionManager};
use saturn_protocol::envelope::{INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use saturn_protocol::ids::{AgentId, ChatId, Provider, RunId, TaskId};

use crate::chat_env::ChatEnv;
use crate::processes::{NESTED_MARKER_ENV, ProcessError};
use crate::routers::{ActiveRouter, Routers, RoutersError, SharedSecrets};
use crate::rpc::{ClientId, RpcError, RpcServer};
use crate::secrets::{KeyInput, SecretsError};
use crate::settings::{FolderTrustPrompt, SettingsError, SettingsManager};
use crate::store::{MigrationNotice, RunRecord, Store, StoreError};
use crate::training::{TrainPlan, TrainingError};

/// 초안. router 키를 기다리는 동안 거절한 요청의 오류 번호(JSON-RPC 서버 오류 범위).
pub const ROUTER_KEY_REQUIRED: i32 = -32001;

/// 초안. 붙을 때 보내는 기록 수. TUI `HISTORY_PAGE`와 같다.
const ATTACH_HISTORY: u32 = 50;

/// 시작 단계 오류면 원인 한 줄을 stderr에 보이고 소켓을 열지 않고 끝난다.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 자식 Saturn을 부모에 잇는 방식이 정해질 때까지 에이전트 작업 안의 실행을 거절한다.
    #[error("nested saturn is not allowed inside an agent task")]
    Nested,
    /// 판단 방식에 맞는 router를 만들 수 없어 키를 받아도 확인할 수 없다.
    #[error("router is not available: {reason}")]
    RouterUnavailable {
        /// 가린 원인 한 줄.
        reason: String,
    },
    /// router 확인 전에는 `SubmitRouterKey`, `Attach`, `Detach`만 받는다.
    #[error("router key required: {reason}")]
    RouterKeyRequired {
        /// 가린 원인 한 줄.
        reason: String,
    },
    /// 맡은 이슈가 아직 구현하지 않은 요청이라 처리하지 않는다.
    #[error("request not supported yet: {method}")]
    Unsupported { method: &'static str },
    /// 묻지 않은 창의 답이라 적용하지 않는다.
    #[error("no pending {what} for this answer")]
    UnexpectedAnswer { what: &'static str },
    /// 채팅에 더하려는 경로가 이미 있는 폴더의 절대 경로가 아니다.
    #[error("invalid folder {path}: {reason}")]
    InvalidFolder { path: String, reason: &'static str },
    /// `ask`, `edit`, `read-only`, `full`이 아닌 권한 모드 이름이다.
    #[error("unknown permission mode: {mode}")]
    UnknownPermissionMode { mode: String },
    /// 이벤트를 붙일 실행이 없어 기록하지 못한다.
    #[error("no run to record the event of agent {}", agent.0)]
    NoRun { agent: AgentId },
    /// 이 클라이언트가 붙지 않은 채팅이라 입력을 받지 않는다.
    #[error("chat {} is not attached to this client", chat.0)]
    ChatNotAttached { chat: ChatId },
    /// 고정 모델도 현재 provider도 없는 첫 입력인데 설치된 provider가 없다.
    #[error("no provider is installed")]
    NoProvider,
    #[error("rpc failed")]
    Rpc(#[from] RpcError),
    #[error("record store failed")]
    Store(#[from] StoreError),
    #[error("settings failed")]
    Settings(#[from] SettingsError),
    #[error("router failed")]
    Routers(#[from] RoutersError),
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

impl EngineError {
    fn code(&self) -> i32 {
        match self {
            Self::RouterKeyRequired { .. } => ROUTER_KEY_REQUIRED,
            Self::Unsupported { .. } => METHOD_NOT_FOUND,
            Self::UnexpectedAnswer { .. }
            | Self::UnknownPermissionMode { .. }
            | Self::InvalidFolder { .. }
            | Self::ChatNotAttached { .. }
            | Self::Store(StoreError::NotFound { .. })
            | Self::Queue(
                QueueError::NotFound(_)
                | QueueError::AlreadySent
                | QueueError::InvalidTransition { .. },
            ) => INVALID_PARAMS,
            _ => INTERNAL_ERROR,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    pub home: PathBuf,
    pub run_overrides: Vec<String>,
}

/// 시작 단계가 프로세스 환경, 키 저장소, router API와 닿는 자리. 테스트는 가짜를 넣는다.
#[derive(Debug, Default)]
struct StartEnv {
    nested_marker: Option<OsString>,
    /// `None`이면 사용자 설정의 저장 방식으로 열고 저장된 키를 읽는다.
    secrets: Option<SharedSecrets>,
    /// `None`이면 `secrets::input_order`를 따른다.
    key_inputs: Option<Vec<KeyInput>>,
    /// `None`이면 설정의 판단 방식으로 고른다.
    router: Option<ActiveRouter>,
}

impl StartEnv {
    fn from_process() -> Self {
        Self {
            nested_marker: std::env::var_os(NESTED_MARKER_ENV),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RouterGate {
    Open,
    KeyRequired {
        /// 가린 원인 한 줄.
        reason: String,
    },
}

/// 첫 TUI에 한 번 보낸다.
#[derive(Debug, Default)]
struct StartNotices {
    migration: Option<MigrationNotice>,
}

/// `Request::Attach`의 값.
#[derive(Debug)]
struct AttachRequest {
    chat: Option<ChatId>,
    workdir: PathBuf,
    env: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
    add_dirs: Vec<String>,
}

#[derive(Debug)]
struct Attachment {
    chat: ChatId,
    /// 이 접속의 입력에만 적용하는 실행 층.
    overrides: Vec<(String, String)>,
    /// 이 TUI에 묻고 답을 기다리는 폴더 설정. 폴더는 채팅마다 달라 TUI마다 따로 묻는다.
    folder_trust: Option<FolderTrustPrompt>,
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

impl Runs {
    /// 에이전트의 채팅과 작업 연결을 지우고, 끝내지 않은 실행이 있으면 돌려준다.
    fn forget(&mut self, agent: AgentId) -> Option<RunId> {
        self.chat_of.remove(&agent);
        self.task_of.remove(&agent);
        self.active.remove(&agent)
    }
}

#[derive(Debug)]
pub struct Engine {
    options: EngineOptions,
    /// 쓰는 쪽은 이 engine 하나.
    store: Store,
    settings: SettingsManager,
    /// `RemoteRouter`와 함께 쓴다.
    secrets: SharedSecrets,
    masker: Masker,
    supervisor: Supervisor,
    /// 연결은 채팅마다 둔다. 작업 폴더와 환경이 채팅마다 달라서다.
    providers: HashMap<(ChatId, Provider), ProviderConnection>,
    routers: Routers,
    router_gate: RouterGate,
    rpc: RpcServer,
    attachments: HashMap<ClientId, Attachment>,
    /// 채팅마다 가장 나중에 붙은 TUI가 넘긴 작업 폴더와 환경.
    chats: HashMap<ChatId, ChatEnv>,
    /// 채팅마다 더한 폴더. 기록 저장소가 정본이고 session을 열 때 읽는 빠른 사본이다.
    chat_dirs: HashMap<ChatId, Vec<PathBuf>>,
    notices: StartNotices,
    queue: Queue,
    sessions: SessionManager,
    /// 결과 신호를 아직 확정하지 않은 판단.
    signals: outcomes::SignalWatch,
    agents: AgentTracker,
    runs: Runs,
    /// 입력 접수부터 전송까지 메모리에 두는 값.
    flow: flow::FlowState,
    presence: Presence,
    /// `ConfirmTrain`을 기다린다.
    pending_train: Option<TrainPlan>,
}

impl Engine {
    pub async fn run(options: EngineOptions) -> Result<(), EngineError> {
        let mut engine = Self::start(options).await?;
        // TODO(#150): 시작 직후 크래시 복구
        let served = engine.serve().await;
        engine.shutdown().await?;
        served
    }

    /// 크래시 전에 보낸 패킷은 어느 경우에도 다시 보내지 않는다.
    /// TODO(#66): 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 정리할지, 끊김 표시만 할지
    #[expect(clippy::todo, reason = "#90 뼈대")]
    async fn recover_after_crash(&mut self) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 파일 상태를 확인한 뒤 그 상태로 만든 새 입력을 접수해 보낸다.
    /// TODO(#65): 수정 파일 목록을 실행 경계의 파일 상태 차이로 셀지, provider 이벤트로 셀지
    #[expect(clippy::todo, reason = "#90 뼈대")]
    async fn resume_proven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    #[expect(clippy::todo, reason = "#90 뼈대")]
    async fn hold_unproven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        todo!("#90")
    }

    /// 순서: `StartInfo` → `HistoryChunk` → 답을 기다리는 허가 요청 → 고정 모델(`ModelPinned`, 고정했을 때만) → 시작 안내와 키·신뢰 창.
    /// 새 채팅은 `workdir`로 만들어 그 폴더에 고정한다. 있는 채팅은 TUI가 다른 폴더를 넘겨도
    /// 처음 폴더로 폴더 설정 층과 신뢰를 판단하고, 환경 `env`만 가장 최근 TUI의 것으로 바꾼다.
    /// `overrides`는 이 접속의 입력에만 적용하는 실행 층이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `Store(NotFound)`.
    async fn attach(
        &mut self,
        client: ClientId,
        request: AttachRequest,
    ) -> Result<(), EngineError> {
        let AttachRequest {
            chat,
            workdir,
            env,
            overrides,
            add_dirs,
        } = request;
        let add_dirs = add_dir::resolve_folders(&add_dirs)?;
        let (chat, workdir) = match chat {
            Some(chat) => {
                self.store.chat_layer(chat).await?;
                (chat, self.store.chat_workdir(chat).await?)
            }
            None => (self.store.create_chat(workdir.clone()).await?, workdir),
        };
        self.load_chat_dirs(chat).await?;
        for dir in add_dirs {
            self.register_dir(chat, dir).await?;
        }
        let chat_env = ChatEnv::new(workdir, env);
        let (applied, folder_trust) = self
            .settings
            .apply_trusted(&self.store, Some(chat), chat_env.workdir())
            .await?;
        let start = self.start_info(chat_env.workdir(), &self.chat_dirs_of(chat));
        let history = self.history_chunk(chat, ATTACH_HISTORY).await?;
        self.rpc.greet(client, chat, start, history).await?;
        self.chats.insert(chat, chat_env);
        self.attachments.insert(
            client,
            Attachment {
                chat,
                overrides,
                folder_trust,
            },
        );
        self.presence = Presence::Attached;
        self.send_chat_model(client, chat).await?;
        self.send_start_notices(client, applied).await;
        Ok(())
    }

    /// 보류는 자동으로 이어 가지 않는다.
    /// TODO(#235): 완료 알림 설정 키. TODO(#151): 알림 보내는 방법
    #[expect(clippy::todo, reason = "#90 뼈대")]
    fn enter_background(&mut self) {
        todo!("#90")
    }

    /// 트리 유휴 뒤 `sessions::IDLE_GRACE`가 지났고 그사이 TUI가 붙지 않았으면 참.
    #[expect(clippy::todo, reason = "#90 뼈대")]
    fn background_expired(&self, now: Instant) -> bool {
        todo!("#90")
    }

    /// provider session id는 기록 저장소에 남아 있어 따로 보관하지 않는다.
    async fn shutdown(self) -> Result<(), EngineError> {
        self.stop_process_groups().await;
        self.rpc.close().await;
        tracing::info!("engine stopped");
        Ok(())
    }

    async fn stop_process_groups(&self) {
        for (group, result) in self.supervisor.stop_all().await {
            if let Err(error) = result {
                tracing::warn!(group = group.0, %error, "failed to stop process group");
            }
        }
    }
}

/// 원인까지 `: `로 이은 한 줄. router 키와 같은 문자열은 가린다.
fn masked_chain(masker: &Masker, error: &dyn std::error::Error) -> String {
    let mut line = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        line.push_str(": ");
        line.push_str(&cause.to_string());
        source = cause.source();
    }
    masker.mask(&line).as_str().to_owned()
}

fn unsupported(method: &'static str) -> EngineError {
    EngineError::Unsupported { method }
}
