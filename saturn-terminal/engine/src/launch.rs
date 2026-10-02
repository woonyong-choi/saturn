//! provider 연결을 만들고 session을 여는 공통 도구. 채팅의 작업 폴더와 환경은 `chat_env`를 쓴다.
//! 설계: docs/design/providers-and-sessions.md

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use saturn_core::permission::Rule;
use saturn_core::providers::{ProviderClient, ProviderError, SessionHandle, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{
    AgentId, ChatId, Provider, ProviderSessionId, SessionId, SettingsRevision,
};

use crate::dispatch::MAX_SEND_ATTEMPTS;
use crate::flow::LiveSession;
use crate::models::pinned_choice;
use crate::providers::{
    FIRST_INPUT_ORDER, HomeInput, LaunchSpec, PermissionLaunch, ProviderConnection, SaturnDefaults,
    UserProviderConfig, is_installed, prepare_codex_home, program_name,
};
use crate::secrets::HookPolicy;
use crate::{Engine, EngineError};

impl Engine {
    /// 기록하지 못한 session은 쓰지 않으므로 provider 쪽도 닫는다.
    pub(crate) async fn close_unregistered(
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
        }
    }

    /// 열지 못하는 `NotSent`(재개 실패)만 다시 연다.
    pub(crate) async fn open_with_retries(
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

    /// 입력에 고정한 모델의 provider, 없으면 내부 호출로 정한 provider, 없으면 이어 갈 메인 session의 provider,
    /// 그것도 없으면 설치된 앞쪽 provider.
    ///
    /// # Errors
    /// 모두 정하지 못하면 `NoProvider`.
    pub(crate) fn pick_provider(&self, record: &QueuedInput) -> Result<Provider, EngineError> {
        let chat = record.chat;
        if let Some(choice) = pinned_choice(record) {
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
        FIRST_INPUT_ORDER
            .into_iter()
            .find(|provider| {
                self.providers.contains_key(&(chat, *provider)) || is_installed(*provider, &env)
            })
            .ok_or(EngineError::NoProvider)
    }

    /// 채팅의 provider 연결이 없으면 그 채팅의 작업 폴더와 환경으로 만든다.
    pub(crate) async fn ensure_connected(
        &mut self,
        provider: Provider,
        chat: ChatId,
        settings: SettingsRevision,
    ) -> Result<(), EngineError> {
        if self.providers.contains_key(&(chat, provider)) {
            return Ok(());
        }
        let launch = self.launch_spec(provider, chat, settings).await?;
        let rules = launch
            .permission
            .codex_home
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned());
        let connection = ProviderConnection::connect(launch, self.supervisor.clone()).await?;
        self.providers.insert((chat, provider), connection);
        self.remember_models(provider, chat).await;
        if let Some(rules) = rules {
            self.flow.rules_of_connection.insert(chat, rules);
            self.flow.rules_stale.remove(&chat);
        }
        Ok(())
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
        let permission = match provider {
            Provider::Codex => {
                self.codex_permission(&settings.permission().rules, &provider_env)?
            }
            Provider::Claude => PermissionLaunch::default(),
        };
        Ok(LaunchSpec {
            provider,
            program: PathBuf::from(program_name(provider)),
            workdir: env.workdir().to_path_buf(),
            settings: revision,
            user_config: UserProviderConfig::default(),
            defaults: SaturnDefaults {
                auto_compact_tokens: settings.context_budget(provider).hard_limit(None),
            },
            env: provider_env,
            hook_settings: Some(hook),
            permission,
            masker: self.masker.clone(),
        })
    }

    /// 사용자 `~/.codex`는 읽기만 하고 전용 `CODEX_HOME`을 만든다. 사용자 폴더는 환경의 `CODEX_HOME`, 없으면
    /// `HOME/.codex`다.
    fn codex_permission(
        &self,
        rules: &[Rule],
        env: &[(OsString, OsString)],
    ) -> Result<PermissionLaunch, ProviderError> {
        let value = |name: &str| {
            env.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        let user_codex_home = value("CODEX_HOME")
            .or_else(|| value("HOME").map(|home| home.join(".codex")))
            .unwrap_or_else(|| PathBuf::from("/.codex"));
        let prepared = prepare_codex_home(HomeInput {
            saturn_home: &self.options.home,
            user_codex_home: &user_codex_home,
            rules,
        })
        .map_err(|error| ProviderError::NotSent {
            reason: format!("failed to prepare codex home: {error}"),
        })?;
        Ok(PermissionLaunch {
            codex_home: Some(prepared.path),
            mcp_servers: prepared.mcp_servers,
        })
    }
}
