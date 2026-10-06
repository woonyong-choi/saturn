use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use saturn_core::agents::AgentTracker;
use saturn_core::passes::PassLimits;
use saturn_core::queue::Queue;
use tokio::sync::Mutex;

use super::{
    Engine, EngineError, EngineOptions, Presence, RouterGate, Runs, StartEnv, StartNotices,
    masked_chain,
};
use crate::passes::PassGate;
use crate::processes::Supervisor;
use crate::routers::{ActiveRouter, Routers, SharedSecrets, StartCheck};
use crate::rpc::{EngineLock, RpcServer};
use crate::secrets::{KeyInput, Masker, SecretStore, SecretsError, input_order};
use crate::settings::{Settings, SettingsError, SettingsManager};
use crate::store::{MigrationNotice, Store};
use crate::{flow, outcomes, sessions};

#[derive(Debug)]
struct VerifiedRouter {
    routers: Routers,
    secrets: SharedSecrets,
    masker: Masker,
    gate: RouterGate,
}

impl Engine {
    /// 앞 단계가 실패하면 뒤 단계를 하지 않는다. router 키가 없거나 틀려도 소켓은 열고 키를 기다린다.
    ///
    /// # Errors
    /// 판단 방식에 맞는 router를 만들 수 없으면 `RouterUnavailable`.
    pub(super) async fn start(options: EngineOptions) -> Result<Self, EngineError> {
        Self::start_with(options, StartEnv::from_process()).await
    }

    pub(super) async fn start_with(
        options: EngineOptions,
        env: StartEnv,
    ) -> Result<Self, EngineError> {
        Self::ensure_not_nested(env.nested_marker.as_deref())?;
        let lock = Self::acquire_lock(&options)?;
        let (store, migration) = Self::open_store(&options).await?;
        let sessions = sessions::restore_sessions(&store).await?;
        let settings = Self::merge_settings(&options, &store).await?;
        let verified = Self::verify_router(&options, &store, &settings, env).await?;
        let passes = PassGate::new(Self::child_limits(&settings, &store).await);
        let supervisor = Supervisor::new();
        let rpc = Self::listen(&options, lock, passes.clone(), supervisor.clone()).await?;
        let restarted = options.after_upgrade;
        let packet_capture = crate::packets::capture_dir(
            &options.home,
            std::env::var_os(crate::packets::CAPTURE_ENV),
            std::env::var_os(saturn_protocol::home::HOME_ENV),
            std::env::var_os("HOME"),
        );
        Ok(Self {
            trace: crate::providers::TraceHub::new(&options.home),
            packet_capture,
            options,
            store,
            settings,
            secrets: verified.secrets,
            masker: verified.masker,
            supervisor,
            providers: HashMap::new(),
            registry: crate::Registry::builtin(),
            routers: verified.routers,
            catalog: crate::model_catalog::builtin(),
            router_gate: verified.gate,
            rpc,
            passes,
            children: HashMap::new(),
            attachments: HashMap::new(),
            chats: HashMap::new(),
            chat_dirs: HashMap::new(),
            notices: StartNotices {
                migration,
                restarted,
                auto_prune: None,
                resume_suggested: HashMap::new(),
                provider_updates: Vec::new(),
            },
            queue: Queue::new(),
            sessions,
            signals: outcomes::SignalWatch::default(),
            agents: AgentTracker::new(),
            runs: Runs::default(),
            flow: flow::FlowState::default(),
            presence: Presence::Background { idle_since: None },
            idle_grace: saturn_core::sessions::IDLE_GRACE,
            pending_train: None,
            upgrade_requested: false,
            terminate: Arc::default(),
        })
    }

    /// 설정의 하위 접속 상한. 읽지 못하면 기본값이다.
    pub(super) async fn child_limits(settings: &SettingsManager, store: &Store) -> PassLimits {
        let Some(revision) = settings.current() else {
            return PassLimits::DEFAULT;
        };
        match settings.at(store, revision).await {
            Ok(current) => current.child_limits(),
            Err(error) => {
                tracing::warn!(%error, "child limits not read, using the defaults");
                PassLimits::DEFAULT
            }
        }
    }

    /// 에이전트 작업 안에서는 engine을 띄우지 않는다. 하위 Saturn은 떠 있는 engine에 출입증으로 접속해 부탁만 하고,
    /// engine은 사용자당 하나다.
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
            tracing::warn!(?warning, "settings applied with warning");
        }
        Ok(settings)
    }

    /// 확인이 실패하면 환경 변수, 비밀번호 관리자 명령 순서로 키를 받아 다시 확인하고,
    /// 그래도 실패하면 TUI가 `SubmitRouterKey`로 키를 보낼 때까지 일반 요청을 막는다.
    ///
    /// # Errors
    /// 판단 방식에 맞는 router를 만들 수 없으면 `RouterUnavailable`.
    async fn verify_router(
        options: &EngineOptions,
        store: &Store,
        settings: &SettingsManager,
        env: StartEnv,
    ) -> Result<VerifiedRouter, EngineError> {
        let revision = settings
            .current()
            .ok_or(SettingsError::NoPreviousRevision)?;
        let current = settings.at(store, revision).await?;
        let secrets = match env.secrets {
            Some(secrets) => secrets,
            None => open_secrets(&options.home, &current).await,
        };
        let masker = Masker::new(secrets.lock().await.mask_needles());
        let mut routers = Self::select_router(env.router, &current, &secrets, &masker)?;
        let reason = match routers.check(&current).await {
            StartCheck::Ready | StartCheck::Skipped => None,
            StartCheck::KeyRequired { reason } => Some(reason),
        };
        let Some(reason) = reason else {
            return Ok(VerifiedRouter {
                routers,
                secrets,
                masker,
                gate: RouterGate::Open,
            });
        };
        let inputs = env
            .key_inputs
            .unwrap_or_else(|| input_order(current.key_command()));
        if accept_first_key(&mut routers, inputs, &secrets, settings).await {
            let masker = Masker::new(secrets.lock().await.mask_needles());
            return Ok(VerifiedRouter {
                routers,
                secrets,
                masker,
                gate: RouterGate::Open,
            });
        }
        tracing::warn!(%reason, "router check failed, waiting for router key from tui");
        Ok(VerifiedRouter {
            routers,
            secrets,
            masker,
            gate: RouterGate::KeyRequired { reason },
        })
    }

    fn select_router(
        active: Option<ActiveRouter>,
        current: &Settings,
        secrets: &SharedSecrets,
        masker: &Masker,
    ) -> Result<Routers, EngineError> {
        match active {
            Some(active) => Ok(Routers::with_active(
                active,
                current.method(),
                masker.clone(),
            )),
            None => {
                Routers::select(current, Arc::clone(secrets), masker.clone()).map_err(|error| {
                    EngineError::RouterUnavailable {
                        reason: masked_chain(masker, &error),
                    }
                })
            }
        }
    }

    async fn listen(
        options: &EngineOptions,
        lock: EngineLock,
        passes: PassGate,
        supervisor: Supervisor,
    ) -> Result<RpcServer, EngineError> {
        Ok(RpcServer::bind(&options.home, lock, passes, supervisor).await?)
    }
}

/// 앞선 입력이 실패하면 다음 입력을 시도한다. 하나라도 받으면 참.
async fn accept_first_key(
    routers: &mut Routers,
    inputs: Vec<KeyInput>,
    secrets: &SharedSecrets,
    settings: &SettingsManager,
) -> bool {
    for input in inputs {
        match routers.accept_key(input, secrets, settings).await {
            Ok(()) => return true,
            Err(error) => tracing::debug!(error = %error, "router key input failed"),
        }
    }
    false
}

/// 강화 방식이면 키체인 암호를 한 번 요청한다. 키가 없거나 잠겨 있으면 시작 확인이 실패해 키를 받는다.
async fn open_secrets(home: &Path, settings: &Settings) -> SharedSecrets {
    let mut store = SecretStore::open(home, settings.storage_mode());
    let unlocked = store.unlock(Instant::now()).await;
    warn_failed("failed to unlock router key", unlocked);
    let loaded = store.load().await;
    if !matches!(loaded, Ok(_) | Err(SecretsError::NotFound)) {
        warn_failed("failed to load router key", loaded);
    }
    Arc::new(Mutex::new(store))
}

fn warn_failed<T>(what: &str, result: Result<T, impl std::fmt::Display>) {
    if let Err(error) = result {
        tracing::warn!(%error, "{what}");
    }
}
