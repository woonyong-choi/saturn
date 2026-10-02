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

mod chat_env;
mod control;
mod dispatch;
mod flow;
mod intake;
mod launch;
mod outcomes;
mod requests;
mod sessions;
mod usage;

#[cfg(test)]
mod lifecycle;

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use saturn_core::agents::AgentTracker;
use saturn_core::providers::ProviderError;
use saturn_core::queue::{Queue, QueueError};
use saturn_core::sessions::{SessionError, SessionManager};
use saturn_protocol::envelope::{
    INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND, RequestId, Response,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, Provider, RunId, TaskId};
use saturn_protocol::rpc::Request;
use tokio::sync::Mutex;

use crate::chat_env::ChatEnv;
use crate::judges::{ActiveJudge, Judges, JudgesError, SharedSecrets, StartCheck};
use crate::processes::{NESTED_MARKER_ENV, ProcessError, Supervisor};
use crate::providers::ProviderConnection;
use crate::rpc::{ClientId, EngineLock, RpcError, RpcEvent, RpcServer};
use crate::secrets::{KeyInput, Masker, SecretStore, SecretsError, input_order};
use crate::settings::{FolderTrustPrompt, Settings, SettingsError, SettingsManager};
use crate::store::{MigrationNotice, RunRecord, Store, StoreError};
use crate::training::{TrainPlan, TrainingError};

/// 초안. judge 키를 기다리는 동안 거절한 요청의 오류 번호(JSON-RPC 서버 오류 범위).
pub const JUDGE_KEY_REQUIRED: i32 = -32001;

/// 초안. 붙을 때 보내는 기록 수. TUI `HISTORY_PAGE`와 같다.
const ATTACH_HISTORY: u32 = 50;

/// 시작 단계 오류면 원인 한 줄을 stderr에 보이고 소켓을 열지 않고 끝난다.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// 자식 Saturn을 부모에 잇는 방식이 정해질 때까지 에이전트 작업 안의 실행을 거절한다.
    #[error("nested saturn is not allowed inside an agent task")]
    Nested,
    /// 판단 방식에 맞는 judge를 만들 수 없어 키를 받아도 확인할 수 없다.
    #[error("judge is not available: {reason}")]
    JudgeUnavailable {
        /// 가린 원인 한 줄.
        reason: String,
    },
    /// judge 확인 전에는 `SubmitJudgeKey`, `Attach`, `Detach`만 받는다.
    #[error("judge key required: {reason}")]
    JudgeKeyRequired {
        /// 가린 원인 한 줄.
        reason: String,
    },
    /// 맡은 이슈가 아직 구현하지 않은 요청이라 처리하지 않는다.
    #[error("request not supported yet: {method}")]
    Unsupported { method: &'static str },
    /// 묻지 않은 창의 답이라 적용하지 않는다.
    #[error("no pending {what} for this answer")]
    UnexpectedAnswer { what: &'static str },
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

impl EngineError {
    fn code(&self) -> i32 {
        match self {
            Self::JudgeKeyRequired { .. } => JUDGE_KEY_REQUIRED,
            Self::Unsupported { .. } => METHOD_NOT_FOUND,
            Self::UnexpectedAnswer { .. }
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

/// 시작 단계가 프로세스 환경, 키 저장소, judge API와 닿는 자리. 테스트는 가짜를 넣는다.
#[derive(Debug, Default)]
struct StartEnv {
    nested_marker: Option<OsString>,
    /// `None`이면 사용자 설정의 저장 방식으로 열고 저장된 키를 읽는다.
    secrets: Option<SharedSecrets>,
    /// `None`이면 `secrets::input_order`를 따른다.
    key_inputs: Option<Vec<KeyInput>>,
    /// `None`이면 설정의 판단 방식으로 고른다.
    judge: Option<ActiveJudge>,
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
enum JudgeGate {
    Open,
    KeyRequired {
        /// 가린 원인 한 줄.
        reason: String,
    },
}

#[derive(Debug)]
struct VerifiedJudge {
    judges: Judges,
    secrets: SharedSecrets,
    masker: Masker,
    gate: JudgeGate,
}

/// 첫 TUI에 한 번 보낸다.
#[derive(Debug, Default)]
struct StartNotices {
    migration: Option<MigrationNotice>,
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
    /// 연결은 채팅마다 둔다. 작업 폴더와 환경이 채팅마다 달라서다.
    providers: HashMap<(ChatId, Provider), ProviderConnection>,
    judges: Judges,
    judge_gate: JudgeGate,
    rpc: RpcServer,
    attachments: HashMap<ClientId, Attachment>,
    /// 채팅마다 가장 나중에 붙은 TUI가 넘긴 작업 폴더와 환경.
    chats: HashMap<ChatId, ChatEnv>,
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

    /// 앞 단계가 실패하면 뒤 단계를 하지 않는다. judge 키가 없거나 틀려도 소켓은 열고 키를 기다린다.
    ///
    /// # Errors
    /// 판단 방식에 맞는 judge를 만들 수 없으면 `JudgeUnavailable`.
    async fn start(options: EngineOptions) -> Result<Self, EngineError> {
        Self::start_with(options, StartEnv::from_process()).await
    }

    async fn start_with(options: EngineOptions, env: StartEnv) -> Result<Self, EngineError> {
        Self::ensure_not_nested(env.nested_marker.as_deref())?;
        let lock = Self::acquire_lock(&options)?;
        let (store, migration) = Self::open_store(&options).await?;
        let sessions = sessions::restore_sessions(&store).await?;
        let settings = Self::merge_settings(&options, &store).await?;
        let verified = Self::verify_judge(&options, &store, &settings, env).await?;
        let rpc = Self::listen(&options, lock).await?;
        Ok(Self {
            options,
            store,
            settings,
            secrets: verified.secrets,
            masker: verified.masker,
            supervisor: Supervisor::new(),
            providers: HashMap::new(),
            judges: verified.judges,
            judge_gate: verified.gate,
            rpc,
            attachments: HashMap::new(),
            chats: HashMap::new(),
            notices: StartNotices { migration },
            queue: Queue::new(),
            sessions,
            signals: outcomes::SignalWatch::default(),
            agents: AgentTracker::new(),
            runs: Runs::default(),
            flow: flow::FlowState::default(),
            presence: Presence::Background { idle_since: None },
            pending_train: None,
        })
    }

    /// 판정 기준은 cli와 같다: 중첩 표지 변수가 있으면 거절한다.
    /// TODO(#33): 자식 Saturn을 부모 engine에 붙일지, 독립 engine으로 띄울지
    fn ensure_not_nested(marker: Option<&OsStr>) -> Result<(), EngineError> {
        match marker {
            Some(_) => Err(EngineError::Nested),
            None => Ok(()),
        }
    }

    /// 이미 잡혀 있으면 `Rpc(AlreadyRunning)`이고 cli는 기존 소켓에 붙는다.
    fn acquire_lock(options: &EngineOptions) -> Result<EngineLock, EngineError> {
        Ok(EngineLock::acquire(&options.home)?)
    }

    /// 이관했으면 안내 한 줄을 stderr에 쓰고 첫 TUI에도 보낸다.
    async fn open_store(
        options: &EngineOptions,
    ) -> Result<(Store, Option<MigrationNotice>), EngineError> {
        let (store, notice) = Store::open(&options.home).await?;
        if let Some(notice) = &notice {
            tracing::warn!(notice = %notice.line(), "record store migrated");
        }
        Ok((store, notice))
    }

    /// 채팅이 붙기 전이라 폴더 층과 채팅 층 없이 병합한다. 그 층은 TUI가 붙을 때 채팅마다 병합한다.
    ///
    /// # Errors
    /// 검사 실패이고 이전 설정 번호도 없으면 `Settings(NoPreviousRevision)`.
    async fn merge_settings(
        options: &EngineOptions,
        store: &Store,
    ) -> Result<SettingsManager, EngineError> {
        let mut settings =
            SettingsManager::new(options.home.clone(), options.run_overrides.clone(), store)
                .await?;
        let applied = settings.apply_user(store).await?;
        if let Some(warning) = &applied.warning {
            tracing::warn!(%warning, "settings applied with warning");
        }
        Ok(settings)
    }

    /// 확인이 실패하면 환경 변수, 비밀번호 관리자 명령 순서로 키를 받아 다시 확인하고,
    /// 그래도 실패하면 TUI가 `SubmitJudgeKey`로 키를 보낼 때까지 일반 요청을 막는다.
    ///
    /// # Errors
    /// 판단 방식에 맞는 judge를 만들 수 없으면 `JudgeUnavailable`.
    async fn verify_judge(
        options: &EngineOptions,
        store: &Store,
        settings: &SettingsManager,
        env: StartEnv,
    ) -> Result<VerifiedJudge, EngineError> {
        let revision = settings
            .current()
            .ok_or(SettingsError::NoPreviousRevision)?;
        let current = settings.at(store, revision).await?;
        let secrets = match env.secrets {
            Some(secrets) => secrets,
            None => open_secrets(&options.home, &current).await,
        };
        let masker = Masker::new(secrets.lock().await.mask_needles());
        let mut judges =
            match env.judge {
                Some(active) => Judges::with_active(active, current.method(), masker.clone()),
                None => Judges::select(&current, Arc::clone(&secrets), masker.clone()).map_err(
                    |error| EngineError::JudgeUnavailable {
                        reason: masked_chain(&masker, &error),
                    },
                )?,
            };
        let reason = match judges.check(&current).await {
            StartCheck::Ready | StartCheck::Skipped => None,
            StartCheck::KeyRequired { reason } => Some(reason),
        };
        let Some(reason) = reason else {
            return Ok(VerifiedJudge {
                judges,
                secrets,
                masker,
                gate: JudgeGate::Open,
            });
        };
        let inputs = env
            .key_inputs
            .unwrap_or_else(|| input_order(current.key_command()));
        for input in inputs {
            match judges.accept_key(input, &secrets, settings).await {
                Ok(()) => {
                    let masker = Masker::new(secrets.lock().await.mask_needles());
                    return Ok(VerifiedJudge {
                        judges,
                        secrets,
                        masker,
                        gate: JudgeGate::Open,
                    });
                }
                Err(error) => tracing::debug!(error = %error, "judge key input failed"),
            }
        }
        tracing::warn!(%reason, "judge check failed, waiting for judge key from tui");
        Ok(VerifiedJudge {
            judges,
            secrets,
            masker,
            gate: JudgeGate::KeyRequired { reason },
        })
    }

    async fn listen(options: &EngineOptions, lock: EngineLock) -> Result<RpcServer, EngineError> {
        Ok(RpcServer::bind(&options.home, lock).await?)
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
        let mut tick = tokio::time::interval(outcomes::SETTLE_TICK);
        loop {
            tokio::select! {
                event = self.rpc.next_event() => {
                    let Some(event) = event else { break };
                    self.handle_event(event).await?;
                }
                Some(done) = self.flow.judge_rx.recv() => {
                    self.on_judged(done).await;
                }
                _ = tick.tick() => {
                    if let Err(error) = self.settle_signals(Instant::now()).await {
                        tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to settle judgment signals");
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: RpcEvent) -> Result<(), EngineError> {
        match event {
            RpcEvent::Connected(_) => {}
            RpcEvent::Request(client, id, request) => {
                self.handle_request(client, id, request).await?;
            }
            RpcEvent::Disconnected(client) => {
                self.attachments.remove(&client);
            }
            // TODO(#150): 마지막 TUI가 떨어진 뒤 `on_last_detach`와 유예 시계
            RpcEvent::LastDetached => {}
        }
        Ok(())
    }

    /// 요청마다 응답 하나를 돌려준다. `SubmitJudgeKey` 메시지는 기록하지 않는다.
    async fn handle_request(
        &mut self,
        client: ClientId,
        id: RequestId,
        request: Request,
    ) -> Result<(), EngineError> {
        let response = match self.route(client, request).await {
            Ok(()) => Response::ok(id),
            Err(error) => {
                let message = masked_chain(&self.masker, &error);
                tracing::warn!(client = client.0, error = %message, "request failed");
                Response::error(Some(id), error.code(), message)
            }
        };
        let _ = self.rpc.respond(client, response).await; // 이미 끊긴 클라이언트에는 응답할 곳이 없다
        Ok(())
    }

    async fn route(&mut self, client: ClientId, request: Request) -> Result<(), EngineError> {
        if let JudgeGate::KeyRequired { reason } = &self.judge_gate
            && !matches!(
                request,
                Request::Attach { .. } | Request::SubmitJudgeKey { .. }
            )
        {
            return Err(EngineError::JudgeKeyRequired {
                reason: reason.clone(),
            });
        }
        match request {
            Request::Attach {
                chat,
                workdir,
                env,
                overrides,
            } => {
                self.attach(client, chat, PathBuf::from(workdir), env, overrides)
                    .await
            }
            Request::LoadHistory {
                chat,
                before,
                limit,
            } => self.load_history(client, chat, before, limit).await,
            // TODO(#161): 채팅 이름과 묶음
            Request::RenameChat { .. } => Err(unsupported("RenameChat")),
            Request::SetChatGroup { .. } => Err(unsupported("SetChatGroup")),
            // 서버가 응답하고 끊김으로 바꿔 여기까지 오지 않는다.
            Request::Detach => Ok(()),
            Request::SubmitInput {
                chat,
                client_ref,
                text,
                pinned_model,
                skip_relation,
            } => {
                self.submit_input(client, chat, client_ref, text, pinned_model, skip_relation)
                    .await
            }
            Request::RunAsNewTask { input } => self.run_as_new_task(client, input).await,
            Request::SendNow { input } => self.send_now(client, input).await,
            Request::CancelInput { input } => self.cancel_input(client, input).await,
            Request::Stop { chat } => self.stop_chat(chat).await,
            Request::Continue { chat, task } => self.continue_held(chat, task).await,
            // TODO(#149): 보류 입력 하나 재개, 보류 닫기, 허가 답 전달
            Request::ContinueInput { .. } => Err(unsupported("ContinueInput")),
            Request::CloseHeld { .. } => Err(unsupported("CloseHeld")),
            Request::AnswerPermission { .. } => Err(unsupported("AnswerPermission")),
            Request::AnswerFeedback { judgment, correct } => {
                self.answer_feedback(judgment, correct).await
            }
            Request::SubmitJudgeKey { key } => self.submit_judge_key(client, key).await,
            Request::AnswerFolderTrust {
                path,
                fingerprint,
                apply,
            } => {
                self.answer_folder_trust(client, path, fingerprint, apply)
                    .await
            }
            Request::SetRecording { chat, on } => Ok(self.store.set_recording(chat, on).await?),
            Request::Usage { scope } => self.send_usage(client, scope).await,
            // TODO(#161): 작업 목록
            Request::ListTasks => Err(unsupported("ListTasks")),
            // TODO(#91): 학습과 judge 버전
            Request::Train { .. } => Err(unsupported("Train")),
            Request::ConfirmTrain { .. } => Err(unsupported("ConfirmTrain")),
            Request::ListJudgeVersions => Err(unsupported("ListJudgeVersions")),
            Request::UseJudgeVersion { .. } => Err(unsupported("UseJudgeVersion")),
            // TODO(#161): 기록 정리 미리보기
            Request::Prune { .. } => Err(unsupported("Prune")),
            Request::ExportJudgments { path } => {
                let count = self.store.export_judgments(Path::new(&path)).await?;
                tracing::info!(count, "judgments exported");
                Ok(())
            }
        }
    }

    /// 순서: `StartInfo` → `HistoryChunk` → 답을 기다리는 허가 요청 → 시작 안내와 키·신뢰 창.
    /// 새 채팅은 `workdir`로 만들어 그 폴더에 고정한다. 있는 채팅은 TUI가 다른 폴더를 넘겨도
    /// 처음 폴더로 폴더 설정 층과 신뢰를 판단하고, 환경 `env`만 가장 최근 TUI의 것으로 바꾼다.
    /// `overrides`는 이 접속의 입력에만 적용하는 실행 층이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `Store(NotFound)`.
    async fn attach(
        &mut self,
        client: ClientId,
        chat: Option<ChatId>,
        workdir: PathBuf,
        env: Vec<(String, String)>,
        overrides: Vec<(String, String)>,
    ) -> Result<(), EngineError> {
        let (chat, workdir) = match chat {
            Some(chat) => {
                self.store.chat_layer(chat).await?;
                (chat, self.store.chat_workdir(chat).await?)
            }
            None => (self.store.create_chat(workdir.clone()).await?, workdir),
        };
        let chat_env = ChatEnv::new(workdir, env);
        let (applied, folder_trust) = self
            .settings
            .apply_trusted(&self.store, Some(chat), chat_env.workdir())
            .await?;
        let start = self.start_info(chat_env.workdir());
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
        self.send_start_notices(client, applied).await;
        Ok(())
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
    /// TODO(#235): 완료 알림 설정 키. TODO(#151): 알림 보내는 방법
    fn enter_background(&mut self) {
        todo!("#90")
    }

    /// 트리 유휴 뒤 `sessions::IDLE_GRACE`가 지났고 그사이 TUI가 붙지 않았으면 참.
    fn background_expired(&self, now: Instant) -> bool {
        todo!("#90")
    }

    /// provider session id는 기록 저장소에 남아 있어 따로 보관하지 않는다.
    async fn shutdown(self) -> Result<(), EngineError> {
        for (group, result) in self.supervisor.stop_all().await {
            if let Err(error) = result {
                tracing::warn!(group = group.0, %error, "failed to stop process group");
            }
        }
        self.rpc.close().await;
        tracing::info!("engine stopped");
        Ok(())
    }
}

/// 강화 방식이면 키체인 암호를 한 번 요청한다. 키가 없거나 잠겨 있으면 시작 확인이 실패해 키를 받는다.
async fn open_secrets(home: &Path, settings: &Settings) -> SharedSecrets {
    let mut store = SecretStore::open(home, settings.storage_mode());
    if let Err(error) = store.unlock(Instant::now()).await {
        tracing::warn!(%error, "failed to unlock judge key");
    }
    match store.load().await {
        Ok(_) | Err(SecretsError::NotFound) => {}
        Err(error) => tracing::warn!(%error, "failed to load judge key"),
    }
    Arc::new(Mutex::new(store))
}

/// 원인까지 `: `로 이은 한 줄. judge 키와 같은 문자열은 가린다.
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
