//! provider 연결을 만들고 session을 여는 공통 도구. 채팅의 작업 폴더와 환경은 `chat_env`를 쓴다.
//! 설계: docs/design/providers-and-sessions.md

use std::path::PathBuf;

use saturn_core::providers::{ProviderError, SessionHandle, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{
    AgentId, ChatId, Provider, ProviderSessionId, SessionId, SettingsRevision,
};

use crate::flow::LiveSession;
use crate::models::pinned_choice;
use crate::providers::{
    LaunchSpec, PermissionInput, ProviderConnection, ProviderHandle, SaturnDefaults,
    UserProviderConfig,
};
use crate::secrets::HookPolicy;
use crate::settings::ContextMode;
use crate::{Engine, EngineError};

/// 연결을 시작할 때 쓴 값. 연결이 등록될 때 기억한다.
#[derive(Debug)]
pub(crate) struct ConnectionSeed {
    /// 에이전트 질문 기능을 켰다.
    questions: bool,
    /// 번역한 규칙의 지문. 규칙이 연결을 시작할 때 고정되는 어댑터만 있다.
    rules: Option<String>,
}

impl ConnectionSeed {
    pub(crate) fn of(launch: &LaunchSpec) -> Self {
        Self {
            questions: !launch.permission.questions_disabled,
            rules: launch.permission.rules_fingerprint.clone(),
        }
    }
}

impl Engine {
    /// 기록하지 못한 session은 쓰지 않으므로 provider 쪽도 닫는다. 결과를 기다리지 않는다.
    pub(crate) fn close_unregistered(
        &self,
        chat: ChatId,
        provider: Provider,
        handle: &SessionHandle,
    ) {
        if let Some(connection) = self.providers.get(&(chat, provider)) {
            connection.close_session_detached(handle.provider_session.clone());
        }
    }

    pub(crate) fn remember(
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

    /// `model`은 session에 기록한 모델이고, `packet`은 첫 턴으로 보낼 글이다.
    pub(crate) fn session_spec(
        &self,
        record: &QueuedInput,
        agent: AgentId,
        model: Option<String>,
        resume: Option<ProviderSessionId>,
        packet: Option<String>,
    ) -> SessionSpec {
        SessionSpec {
            agent,
            workdir: record.workdir.clone(),
            model,
            settings: record.settings,
            resume,
            packet,
            add_dirs: self.chat_dirs_of(record.chat),
            interrupted_children: Vec::new(),
        }
    }

    /// 입력에 고정한 모델의 provider, 없으면 내부 호출로 정한 provider, 없으면 이어 갈 메인 session의 provider,
    /// 그것도 없으면 설치된 앞쪽 provider.
    ///
    /// # Errors
    /// 모두 정하지 못하면 `NoProvider`.
    pub(crate) fn pick_provider(&self, record: &QueuedInput) -> Result<Provider, EngineError> {
        let chat = record.chat;
        if let Some(choice) = pinned_choice(&self.registry, record) {
            return Ok(choice.provider);
        }
        if let Some(provider) = self.flow.switch_to.get(&chat) {
            return Ok(*provider);
        }
        if let Some(main) = self.sessions.live_main(chat) {
            return Ok(main.provider);
        }
        let env = self
            .chat_env(chat)
            .map(crate::chat_env::ChatEnv::provider_env)
            .unwrap_or_default();
        self.registry
            .first_installed(&env, |provider| {
                self.providers.contains_key(&(chat, provider))
            })
            .ok_or(EngineError::NoProvider)
    }

    /// 맺은 연결을 맡는 작업을 띄우고 채팅에 등록한다.
    pub(crate) fn add_connection(&mut self, chat: ChatId, connection: ProviderConnection) {
        let provider = connection.provider();
        let handle = ProviderHandle::spawn(
            connection,
            chat,
            self.flow.provider_tx.clone(),
            self.masker.clone(),
        );
        self.providers.insert((chat, provider), handle);
    }

    /// 새 연결을 등록하고 그 연결을 시작할 때 쓴 값을 기억한다.
    pub(crate) fn attach_connection(
        &mut self,
        chat: ChatId,
        connection: ProviderConnection,
        seed: ConnectionSeed,
    ) {
        let provider = connection.provider();
        self.add_connection(chat, connection);
        self.flow
            .questions_of_connection
            .insert((chat, provider), seed.questions);
        self.flow.stale_connections.remove(&(chat, provider));
        if let Some(rules) = seed.rules {
            self.flow
                .rules_of_connection
                .insert((chat, provider), rules);
        }
    }

    /// 작업 폴더와 환경은 채팅에 고정한 값이고, router 키 변수는 뺀다. 훅은 에이전트가 키 저장소를 읽지 못하게 막는다.
    /// Saturn 권한 규칙은 provider 실행 설정으로 번역해 넣는다.
    ///
    /// # Errors
    /// 붙은 적 없는 채팅이면 `ChatNotAttached`, 규칙을 번역하지 못하면 `Provider(NotSent)`다.
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
        let questions = self.agent_questions(chat, revision).await?;
        let adapter = self
            .registry
            .get(provider)
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown provider: {provider}"),
            })?;
        let permission = adapter.translate_permission(PermissionInput {
            saturn_home: &self.options.home,
            rules: &settings.permission().rules,
            env: &provider_env,
            questions,
        })?;
        Ok(LaunchSpec {
            provider,
            program: PathBuf::from(adapter.descriptor().program),
            workdir: env.workdir().to_path_buf(),
            settings: revision,
            user_config: UserProviderConfig::default(),
            defaults: SaturnDefaults {
                auto_compact_tokens: (settings.context_mode() == ContextMode::Saturn).then(|| {
                    settings
                        .context_budget(provider, adapter.descriptor().context)
                        .hard_limit(None)
                }),
            },
            env: provider_env,
            hook_settings: Some(hook),
            permission,
            masker: self.masker.clone(),
        })
    }
}
