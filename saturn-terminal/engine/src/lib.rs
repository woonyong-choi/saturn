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
    Frame, HookInputError, InjectedPart, InjectionFailure, LaunchSpec, PermissionLaunch,
    ProviderConnection, ProviderTrace, RawTap, ReadScope, Registry, SaturnDefaults,
    UserProviderConfig, run_pre_tool_use,
};
pub use secrets::{Masker, pre_tool_use_hook_settings, with_read_scope};

mod add_dir;
mod calls;
mod chat_env;
mod chat_labels;
mod children;
mod commands;
mod constraint_change;
mod constraints;
mod control;
mod delivery;
mod dispatch;
mod events;
mod exit;
mod extensions;
mod flow;
mod handoff;
mod idle_close;
mod inputs;
mod intake;
mod judge_context;
mod launch;
mod model_catalog;
mod models;
mod outcomes;
mod packets;
mod passes;
mod permission;
mod policy;
mod prune;
mod recover;
mod requests;
mod run_changes;
mod serve;
mod sessions;
mod settings_watch;
mod shadow;
mod startup;
mod stop;
mod switch;
mod tasks;
mod turn_end;
mod usage;
mod versions;
mod workspace;

#[cfg(test)]
mod lifecycle;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use saturn_core::agents::AgentTracker;
use saturn_core::providers::ProviderError;
use saturn_core::queue::{Queue, QueueError};
use saturn_core::sessions::{SessionError, SessionManager};
use saturn_protocol::envelope::{ErrorKind, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use saturn_protocol::ids::{AgentId, ChatId, Provider, RunId, TaskId};

use crate::chat_env::ChatEnv;
use crate::children::ChildLink;
use crate::passes::PassGate;
use crate::processes::{NESTED_MARKER_ENV, ProcessError};
use crate::routers::{ActiveRouter, Routers, RoutersError, SharedSecrets};
use crate::rpc::{ClientId, RpcError, RpcServer};
use crate::secrets::{KeyInput, SecretsError};
use crate::settings::{FolderTrustPrompt, SettingsError, SettingsManager};
use crate::store::{MigrationNotice, Store, StoreError};
use crate::training::{TrainPlan, TrainingError};

/// 초안. router 키를 기다리는 동안 거절한 요청의 오류 번호(JSON-RPC 서버 오류 범위).
pub const ROUTER_KEY_REQUIRED: i32 = -32001;

/// 초안. 붙을 때 보내는 기록 수. TUI `HISTORY_PAGE`와 같다.
const ATTACH_HISTORY: u32 = 50;

/// 시작 단계 오류면 원인 한 줄을 stderr에 보이고 소켓을 열지 않고 끝난다.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 에이전트 작업 안에서는 engine을 띄우지 않는다. 하위 Saturn은 떠 있는 engine에 출입증으로 접속한다.
    #[error("engine must not be started inside an agent task")]
    Nested,
    /// 하위 접속을 받지 않았다. 부모에 실행 중인 작업이 없거나 요청 모드가 부모 모드를 넘는다.
    #[error("child session rejected: {reason}")]
    ChildRejected {
        /// 원인 한 줄. 출입증 글자는 담지 않는다.
        reason: String,
    },
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
    /// 화면이 본 제약 revision이 지금과 다르거나 대상이 유효 제약이 아니다. 목록을 새로 읽어야 한다.
    #[error("stale constraint request: read the constraints again")]
    StaleConstraint,
    /// 채팅에 더하려는 경로가 이미 있는 폴더의 절대 경로가 아니다.
    #[error("invalid folder {path}: {reason}")]
    InvalidFolder { path: String, reason: &'static str },
    /// 채팅 이름이나 묶음에 줄바꿈 같은 제어 문자가 들어 있다.
    #[error("invalid chat {what}: contains a control character")]
    InvalidLabel { what: &'static str },
    /// 정리 확인 번호가 없거나 만료됐거나 이미 썼다.
    #[error("unknown prune plan: preview again")]
    PrunePlanUnknown,
    /// `yes`에 미리보기 번호도 `all`도 없다. 미리보기 확인을 모르는 옛 클라이언트의 요청이라 지우지 않는다.
    #[error("prune confirmation needs the plan of a preview or an explicit all: update the client")]
    PruneNeedsPlan,
    /// 정리 기준 `retention.max_age_days`가 없어 어떤 채팅이 오래됐는지 정할 수 없다.
    #[error("no retention.max_age_days setting to decide which chats to prune")]
    NoRetention,
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
    #[error("failed to listen for SIGTERM")]
    Signal(#[source] std::io::Error),
    #[error("training failed")]
    Training(#[from] TrainingError),
}

impl EngineError {
    fn code(&self) -> i32 {
        match self {
            Self::RouterKeyRequired { .. } => ROUTER_KEY_REQUIRED,
            Self::ChildRejected { .. } => saturn_protocol::rpc::CHILD_REJECTED,
            Self::Unsupported { .. } => METHOD_NOT_FOUND,
            Self::UnexpectedAnswer { .. }
            | Self::StaleConstraint
            | Self::UnknownPermissionMode { .. }
            | Self::InvalidFolder { .. }
            | Self::InvalidLabel { .. }
            | Self::NoRetention
            | Self::PrunePlanUnknown
            | Self::PruneNeedsPlan
            | Self::ChatNotAttached { .. }
            | Self::Store(StoreError::NotFound { .. })
            | Self::Queue(
                QueueError::NotFound(_)
                | QueueError::AlreadySent
                | QueueError::NotAwaitingStop(_)
                | QueueError::InvalidTransition { .. },
            ) => INVALID_PARAMS,
            _ => INTERNAL_ERROR,
        }
    }

    /// `cli`가 종료 코드를 가르는 원인 종류. 없으면 engine 내부 오류로 본다.
    fn kind(&self) -> Option<ErrorKind> {
        match self {
            Self::RouterKeyRequired { .. }
            | Self::Secrets(_)
            | Self::Routers(RoutersError::Check(_) | RoutersError::Secrets(_)) => {
                Some(ErrorKind::RouterKey)
            }
            Self::NoRetention
            | Self::Settings(_)
            | Self::Routers(
                RoutersError::DisallowedEndpoint { .. }
                | RoutersError::NotConfigured { .. }
                | RoutersError::Settings(_),
            )
            | Self::Training(TrainingError::NoGrader) => Some(ErrorKind::Config),
            Self::Store(StoreError::NotFound { .. })
            | Self::PrunePlanUnknown
            | Self::Training(TrainingError::UnknownVersion { .. }) => Some(ErrorKind::NotFound),
            Self::Training(TrainingError::NotEnough { .. }) => Some(ErrorKind::RetryLater),
            Self::NoProvider
            | Self::ChildRejected { .. }
            | Self::Provider(_)
            | Self::Training(TrainingError::Grader(_) | TrainingError::Trainer { .. }) => {
                Some(ErrorKind::Failed)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineOptions {
    pub home: PathBuf,
    pub run_overrides: Vec<String>,
    /// 업데이트를 적용하려고 옛 engine을 끝내고 띄운 engine이면 참. 첫 TUI에 `Alert::EngineRestarted`를 한 번 보낸다.
    pub after_upgrade: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoPruneNotice {
    Deleted { chats: u32, rows: u64 },
    Failed,
}

/// 첫 TUI에 한 번 보낸다.
#[derive(Debug, Default)]
struct StartNotices {
    migration: Option<MigrationNotice>,
    /// 업데이트로 다시 시작했다는 알림. 첫 TUI에 보내고 지운다.
    restarted: bool,
    /// 시작 때 자동 정리의 결과. 지운 채팅이 있거나 실패했을 때만 둔다.
    auto_prune: Option<AutoPruneNotice>,
    /// 크래시 복구가 보류한 작업. 그 채팅에 처음 붙는 TUI에 `/continue`를 제안하고 지운다.
    resume_suggested: HashMap<ChatId, Vec<TaskId>>,
    /// 시작 때 읽은 provider CLI 버전이 마지막으로 확인한 버전과 달랐던 것. 첫 TUI에 알리고 지운다.
    provider_updates: Vec<versions::VersionChange>,
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

impl Attachment {
    /// 설정 병합에 넘기는 `키=값` 목록.
    fn run_layer(&self) -> Vec<String> {
        self.overrides
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect()
    }
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
    /// provider 원시 메시지 관측 기록(`debug.provider_events`)을 연결마다 나눠 주는 곳.
    trace: providers::TraceHub,
    supervisor: Supervisor,
    /// 연결은 채팅마다 둔다. 작업 폴더와 환경이 채팅마다 달라서다.
    providers: HashMap<(ChatId, Provider), providers::ProviderHandle>,
    /// 붙은 어댑터. 설명자와 연결 만들기는 모두 여기서 찾는다.
    registry: Registry,
    routers: Routers,
    /// 배포에 묶인 모델 평가 근거 목록. 시작 때 정하고 끝까지 쓴다.
    catalog: saturn_core::models::ModelCatalog,
    router_gate: RouterGate,
    rpc: RpcServer,
    /// 하위 접속 출입증과 상한. 연결 작업이 요청 처리 루프를 거치지 않고 함께 쓴다.
    passes: PassGate,
    /// 하위 접속으로 만든 채팅과 그 부모 작업.
    children: HashMap<ChatId, ChildLink>,
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
    /// 백그라운드에서 모든 작업이 끝난 뒤 engine이 끝나기까지 기다리는 시간.
    idle_grace: Duration,
    /// `ConfirmTrain`을 기다린다.
    pending_train: Option<TrainPlan>,
    /// `Shutdown`을 받았다. 요청 처리 루프는 붙은 TUI에 알린 뒤 끝난다.
    upgrade_requested: bool,
    /// 종료 신호(`SIGTERM`)를 받았다. `serve`가 끝나고 `shutdown`이 provider 프로세스 묶음을 정리한다.
    terminate: Arc<tokio::sync::Notify>,
}

impl Engine {
    pub async fn run(options: EngineOptions) -> Result<(), EngineError> {
        // 시작과 복구 중에 온 신호도 놓치지 않게 provider를 띄우기 전에 등록한다
        let terminate = Arc::new(tokio::sync::Notify::new());
        watch_terminate(&terminate)?;
        let mut engine = Self::start(options).await?;
        engine.terminate = terminate;
        engine.finish_start().await?;
        engine
            .detect_provider_versions(&std::env::vars_os().collect::<Vec<_>>())
            .await;
        let served = engine.serve().await;
        engine.shutdown().await?;
        served
    }

    /// 소켓을 연 뒤 요청을 처리하기 전에 한다. 크래시 복구가 열린 입력을 되살린 뒤에 자동 정리를 해
    /// 되살아난 채팅을 지우지 않게 한다.
    ///
    /// # Errors
    /// 크래시 복구가 기록 저장소를 읽지 못하면 `Store`. 자동 정리 실패는 오류로 끝내지 않는다.
    async fn finish_start(&mut self) -> Result<(), EngineError> {
        self.recover_after_crash().await?;
        self.rejudge_unknown_extensions().await;
        self.auto_prune_on_start().await;
        Ok(())
    }

    /// 순서: `StartInfo` → `HistoryChunk` → 답을 기다리는 허가 요청 → 고정 모델(`ModelPinned`, 고정했을 때만) → 맥락 사용량(`ContextSize`, 알 때만) → 시작 안내와 키·신뢰 창.
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
        self.register_attach_dirs(chat, add_dirs).await?;
        let chat_env = ChatEnv::new(workdir, env);
        let run: Vec<String> = overrides
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();
        let (applied, folder_trust) = self
            .settings
            .apply_trusted(&self.store, Some(chat), chat_env.workdir(), &run)
            .await?;
        let start = self.start_info(chat_env.workdir(), &self.chat_dirs_of(chat));
        let history = self.history_chunk(chat, None, ATTACH_HISTORY).await?;
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
        self.send_chat_status(client, chat).await;
        self.send_start_notices(client, applied).await;
        self.send_resume_suggestions(chat).await;
        Ok(())
    }

    /// 접속 때 보내는 명령 목록과 맥락 사용량. 알 수 없는 값은 보내지 않는다.
    async fn send_chat_status(&self, client: ClientId, chat: ChatId) {
        self.send_chat_commands(client, chat).await;
        self.send_chat_context(client, chat).await;
    }

    /// 모든 작업이 끝났는지 본다. 실행 중인 작업, 보내기 전에 판단하거나 기다리는 입력, 멈추는 중인 채팅, 응답을 기다리는
    /// 전달, 끝나지 않은 subagent가 없을 때만 참이다. 보류와 TUI 확인을 기다리는 요청은 세지 않는다.
    fn is_all_idle(&self) -> bool {
        self.continuing_work_total() == 0
            && self.flow.stopping.is_empty()
            && self.flow.deliveries.is_empty()
            && self
                .flow
                .live
                .keys()
                .all(|agent| self.agents.is_tree_idle(*agent))
    }

    /// 백그라운드에서 모든 작업이 끝난 시각을 기록하고, 다시 일이 생기면 지운다.
    fn refresh_idle(&mut self, now: Instant) {
        let is_idle = self.is_all_idle();
        if let Presence::Background { idle_since } = &mut self.presence {
            *idle_since = match (is_idle, *idle_since) {
                (true, None) => Some(now),
                (true, since) => since,
                (false, _) => None,
            };
        }
    }

    /// TUI가 없고 트리 유휴가 된 뒤 유예(`sessions::IDLE_GRACE`)가 지났으면 참. 그사이 TUI가 붙거나 일이 생기면 거짓이다.
    fn background_expired(&self, now: Instant) -> bool {
        match self.presence {
            Presence::Background {
                idle_since: Some(since),
            } => now.saturating_duration_since(since) >= self.idle_grace,
            Presence::Background { idle_since: None } | Presence::Attached => false,
        }
    }

    /// provider session id는 기록 저장소에 남아 있어 따로 보관하지 않는다.
    async fn shutdown(mut self) -> Result<(), EngineError> {
        // 소켓을 먼저 닫아 종료 중에 새 접속이 붙지 않게 한다. 잠금은 provider 프로세스를 정리한 뒤에 푼다
        self.rpc.close_connections().await;
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

/// `SIGTERM`이 오면 `terminate`를 깨운다. 등록은 이 함수가 돌아오기 전에 끝난다.
fn watch_terminate(terminate: &Arc<tokio::sync::Notify>) -> Result<(), EngineError> {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(EngineError::Signal)?;
    let terminate = Arc::clone(terminate);
    tokio::spawn(async move {
        term.recv().await;
        tracing::info!("received SIGTERM, engine is ending");
        terminate.notify_one();
    });
    Ok(())
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

#[cfg(test)]
mod error_kind_tests {
    use super::*;

    #[test]
    fn engine_errors_carry_the_kind_the_cli_turns_into_an_exit_code() {
        let cases = [
            (
                EngineError::RouterKeyRequired {
                    reason: "no key".to_owned(),
                },
                Some(ErrorKind::RouterKey),
            ),
            (EngineError::NoRetention, Some(ErrorKind::Config)),
            (EngineError::PrunePlanUnknown, Some(ErrorKind::NotFound)),
            (
                EngineError::Training(TrainingError::NotEnough { have: 1, need: 200 }),
                Some(ErrorKind::RetryLater),
            ),
            (
                EngineError::Training(TrainingError::UnknownVersion {
                    version: "v9".to_owned(),
                }),
                Some(ErrorKind::NotFound),
            ),
            (EngineError::NoProvider, Some(ErrorKind::Failed)),
            (EngineError::InvalidLabel { what: "name" }, None),
        ];

        for (error, kind) in cases {
            assert_eq!(error.kind(), kind, "{error}");
        }
    }
}
