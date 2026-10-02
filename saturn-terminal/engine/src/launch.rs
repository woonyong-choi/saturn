//! 입력을 보낼 provider와 session을 정하고 연결한다. 채팅의 작업 폴더와 환경은 `chat_env`를 쓴다.
//! 설계: docs/design/providers-and-sessions.md

use std::path::PathBuf;
use std::time::SystemTime;

use saturn_core::providers::{ProviderClient, ProviderError, SessionHandle, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_core::sessions::{AgentRole, SendTarget, SessionRecord};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, SessionId, SettingsRevision};
use saturn_protocol::state::SessionState;

use crate::dispatch::{MAX_SEND_ATTEMPTS, Start};
use crate::flow::LiveSession;
use crate::providers::{
    FIRST_INPUT_ORDER, LaunchSpec, ProviderConnection, SaturnDefaults, UserProviderConfig,
    is_installed, program_name,
};
use crate::secrets::HookPolicy;
use crate::sessions::SendRequest;
use crate::store::IdKind;
use crate::{Engine, EngineError};

impl Engine {
    /// 보낼 session을 연다. 실패하면 사용자에게 보일 원인 한 줄을 돌려준다.
    pub(crate) async fn open_for(
        &mut self,
        record: &QueuedInput,
        start: Start,
    ) -> Result<LiveSession, String> {
        let role = match start {
            Start::Task(task) if !self.queue.is_main_task(task) => AgentRole::Sub,
            _ => AgentRole::Main,
        };
        let provider = self
            .pick_provider(record.chat)
            .map_err(|error| self.failure_line(&error))?;
        self.ensure_connected(provider, record)
            .await
            .map_err(|error| self.failure_line(&error))?;
        let request = SendRequest {
            chat: record.chat,
            provider,
            role,
            packet: 0,
            settings: record.settings,
        };
        let target = self
            .send_target(request, SystemTime::now())
            .await
            .map_err(|error| self.failure_line(&error))?;
        let live = match target {
            SendTarget::Open(id) => self.reopen_if_needed(record, id).await,
            SendTarget::Resume(id) => self.reopen_session(record, id).await,
            // TODO(#168): 패킷 없이 새 session을 연다. provider 전환과 모델 고정이 정해지면 패킷을 넘긴다
            SendTarget::New { provider, role } => {
                self.open_new_session(record, provider, role).await
            }
        };
        live.map_err(|error| self.failure_line(&error))
    }

    /// 이미 이 프로세스에서 열어 둔 session이면 그대로 쓴다.
    async fn reopen_if_needed(
        &mut self,
        record: &QueuedInput,
        id: SessionId,
    ) -> Result<LiveSession, EngineError> {
        let agent = self.sessions.get(id).map(|session| session.agent);
        if let Some(live) = agent.and_then(|agent| self.flow.live.get(&agent)) {
            return Ok(live.clone());
        }
        self.reopen_session(record, id).await
    }

    /// 보관한 provider session id로 다시 연다. 열린 뒤에만 상태를 `Open`으로 바꾼다.
    async fn reopen_session(
        &mut self,
        record: &QueuedInput,
        id: SessionId,
    ) -> Result<LiveSession, EngineError> {
        let stored = self
            .sessions
            .get(id)
            .cloned()
            .ok_or(saturn_core::sessions::SessionError::NotFound(id))?;
        let resume = stored
            .provider_session
            .clone()
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("session {} has no provider session id", id.0),
            })?;
        let spec = self.session_spec(record, stored.agent, Some(resume));
        let handle = self
            .open_with_retries(record.chat, stored.provider, spec)
            .await?;
        if stored.state != SessionState::Open {
            self.resume_main(id).await?;
        }
        Ok(self.remember(stored.agent, id, stored.provider, &handle))
    }

    async fn open_new_session(
        &mut self,
        record: &QueuedInput,
        provider: Provider,
        role: AgentRole,
    ) -> Result<LiveSession, EngineError> {
        let agent = AgentId(self.store.allocate_id(IdKind::Agent).await?);
        let id = SessionId(self.store.allocate_id(IdKind::Session).await?);
        let spec = self.session_spec(record, agent, None);
        let handle = self.open_with_retries(record.chat, provider, spec).await?;
        let registered = self
            .register_session(SessionRecord {
                id,
                chat: record.chat,
                agent,
                role,
                provider,
                provider_session: Some(handle.provider_session.clone()),
                state: SessionState::Open,
                delivered: LedgerSeq(0),
                idle_since: None,
            })
            .await;
        if let Err(error) = registered {
            self.close_unregistered(record.chat, provider, &handle)
                .await;
            return Err(error);
        }
        Ok(self.remember(agent, id, provider, &handle))
    }

    /// 기록하지 못한 session은 쓰지 않으므로 provider 쪽도 닫는다.
    async fn close_unregistered(
        &mut self,
        chat: ChatId,
        provider: Provider,
        handle: &SessionHandle,
    ) {
        let Ok(connection) = self.provider_mut(chat, provider) else {
            return;
        };
        if let Err(error) = connection.close_session(&handle.provider_session).await {
            tracing::warn!(%error, "failed to close unregistered session");
        }
    }

    fn remember(
        &mut self,
        agent: AgentId,
        session: SessionId,
        provider: Provider,
        handle: &SessionHandle,
    ) -> LiveSession {
        let live = LiveSession {
            agent,
            session,
            provider,
            provider_session: handle.provider_session.clone(),
            steer_verified: handle.steer_verified,
        };
        self.flow.live.insert(agent, live.clone());
        live
    }

    /// 모델은 요청의 `pinned_model`을 그대로 넘긴다.
    /// TODO(#168): 모델에서 provider로 가는 대응과 이미 열린 session의 모델 교체
    fn session_spec(
        &self,
        record: &QueuedInput,
        agent: AgentId,
        resume: Option<saturn_protocol::ids::ProviderSessionId>,
    ) -> SessionSpec {
        SessionSpec {
            agent,
            workdir: record.workdir.clone(),
            model: record.pinned_model.clone(),
            settings: record.settings,
            resume,
            packet: None,
        }
    }

    /// 열지 못하는 `NotSent`(재개 실패)만 다시 연다.
    async fn open_with_retries(
        &mut self,
        chat: ChatId,
        provider: Provider,
        spec: SessionSpec,
    ) -> Result<SessionHandle, ProviderError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let opened = self
                .provider_mut(chat, provider)?
                .open_session(spec.clone())
                .await;
            match opened {
                Err(ProviderError::NotSent { .. }) if attempt < MAX_SEND_ATTEMPTS => {
                    tracing::warn!(attempt, "session was not opened, trying again");
                }
                other => return other,
            }
        }
    }

    /// 이어 갈 메인 session이 있으면 그 provider, 없으면 설치된 앞쪽 provider.
    ///
    /// # Errors
    /// 둘 다 설치돼 있지 않으면 `NoProvider`.
    fn pick_provider(&self, chat: ChatId) -> Result<Provider, EngineError> {
        if let Some(main) = self.sessions.live_main(chat) {
            return Ok(main.provider);
        }
        let env = self
            .chat_env(chat)
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        FIRST_INPUT_ORDER
            .into_iter()
            .find(|provider| {
                self.providers.contains_key(&(chat, *provider)) || is_installed(*provider, &env)
            })
            .ok_or(EngineError::NoProvider)
    }

    /// 채팅의 provider 연결이 없으면 그 채팅의 작업 폴더와 환경으로 만든다.
    async fn ensure_connected(
        &mut self,
        provider: Provider,
        record: &QueuedInput,
    ) -> Result<(), EngineError> {
        if self.providers.contains_key(&(record.chat, provider)) {
            return Ok(());
        }
        let launch = self
            .launch_spec(provider, record.chat, record.settings)
            .await?;
        let connection = ProviderConnection::connect(launch, self.supervisor.clone()).await?;
        self.providers.insert((record.chat, provider), connection);
        Ok(())
    }

    /// 작업 폴더와 환경은 채팅에 고정한 값이고, judge 키 변수는 뺀다. 훅은 에이전트가 키 저장소를 읽지 못하게 막는다.
    /// TODO(#232): 권한 규칙을 provider 실행 설정으로 번역한다. 그 전에는 수정을 항상 허용하는 기본값을 쓴다
    pub(crate) async fn launch_spec(
        &self,
        provider: Provider,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<LaunchSpec, EngineError> {
        let env = self
            .chat_env(chat)
            .ok_or(EngineError::ChatNotAttached { chat })?;
        let settings = self.settings.at(&self.store, revision).await?;
        let provider_env = env.provider_env();
        let user_home = provider_env
            .iter()
            .find(|(name, _)| name == "HOME")
            .map_or_else(|| PathBuf::from("/"), |(_, home)| PathBuf::from(home));
        let saturn_bin = std::env::current_exe().map_err(|error| {
            tracing::warn!(%error, "failed to find saturn executable for the key hook");
            ProviderError::ConnectionLost
        })?;
        let hook =
            HookPolicy::new(&self.options.home, &user_home).pre_tool_use_settings(&saturn_bin);
        Ok(LaunchSpec {
            provider,
            program: PathBuf::from(program_name(provider)),
            workdir: env.workdir().to_path_buf(),
            settings: revision,
            user_config: UserProviderConfig::default(),
            defaults: SaturnDefaults {
                allow_edits: true,
                auto_compact_tokens: settings.context_budget(provider).hard_limit(None),
            },
            env: provider_env,
            hook_settings: Some(hook),
            masker: self.masker.clone(),
        })
    }
}
