//! Codex 어댑터: 설명자, 연결 만들기, 권한 규칙을 전용 `CODEX_HOME`으로 번역하기.
//! 설계: docs/design/providers-and-sessions.md#provider-계층과-어댑터

use std::path::PathBuf;
use std::sync::Arc;

use saturn_core::permission::Rule;
use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};

use super::CodexClient;
use super::home::{self, HomeInput};
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::{
    Adapter, AdapterConnection, AppliedReader, AppliedSettings, BoxFuture, ContextDefaults,
    Descriptor, Feature, INTERFACE_VERSION, LaunchSpec, PermissionInput, PermissionLaunch,
    ProviderConnection,
};

/// 설정 키 `provider.codex.*`와 모델 고정 글 `codex/<model>`의 앞부분이다.
pub(crate) const ID: Provider = Provider::from_static("codex");

static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    display_name: "codex",
    program: "codex",
    order: 20,
    features: &[Feature::Steer, Feature::Compact],
    instruction_doc: "AGENTS.md",
    interface_version: INTERFACE_VERSION,
    context: ContextDefaults {
        window: 272_000,
        cache_write: 1.0,
    },
};

#[derive(Debug)]
struct CodexAdapter;

pub(crate) fn adapter() -> Arc<dyn Adapter> {
    Arc::new(CodexAdapter)
}

impl Adapter for CodexAdapter {
    fn descriptor(&self) -> &Descriptor {
        &DESCRIPTOR
    }

    fn connect(
        &self,
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> BoxFuture<'_, Result<ProviderConnection, ProviderError>> {
        Box::pin(async move {
            let client = CodexClient::start(launch, supervisor).await?;
            Ok(ProviderConnection::new(ID, client))
        })
    }

    /// 사용자 `~/.codex`는 읽기만 하고 전용 `CODEX_HOME`을 만든다. 사용자 폴더는 환경의 `CODEX_HOME`, 없으면
    /// `HOME/.codex`다.
    fn translate_permission(
        &self,
        input: PermissionInput<'_>,
    ) -> Result<PermissionLaunch, ProviderError> {
        let value = |name: &str| {
            input
                .env
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| PathBuf::from(value))
        };
        let user_codex_home = value("CODEX_HOME")
            .or_else(|| value("HOME").map(|home| home.join(".codex")))
            .unwrap_or_else(|| PathBuf::from("/.codex"));
        let prepared = home::prepare(HomeInput {
            saturn_home: input.saturn_home,
            user_codex_home: &user_codex_home,
            rules: input.rules,
            questions: input.questions,
        })
        .map_err(|error| ProviderError::NotSent {
            reason: format!("failed to prepare codex home: {error}"),
        })?;
        Ok(PermissionLaunch {
            rules_fingerprint: home::rules_of_home(&prepared.path),
            env: vec![("CODEX_HOME".into(), prepared.path.into_os_string())],
            mcp_servers: prepared.mcp_servers,
            questions_disabled: !input.questions,
        })
    }

    /// 규칙은 전용 `CODEX_HOME`에 들어가 연결을 시작할 때 고정된다.
    fn rules_fingerprint(&self, rules: &[Rule]) -> Option<String> {
        Some(home::rules_fingerprint(rules))
    }
}

impl AdapterConnection for CodexClient {
    /// 모든 session이 app-server 묶음 하나를 같이 쓴다.
    fn process_group(&self, _session: &ProviderSessionId) -> Option<ProcessGroupId> {
        Some(CodexClient::process_group(self))
    }

    fn shared_group(&self) -> Option<ProcessGroupId> {
        Some(CodexClient::process_group(self))
    }

    fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        CodexClient::applied_settings(self, session)
    }

    fn applied_reader(&self, session: &ProviderSessionId) -> Option<AppliedReader> {
        Some(CodexClient::applied_reader(self, session))
    }

    fn start_queued_turn(&mut self, agent: AgentId) -> impl Future<Output = ()> + Send {
        CodexClient::start_queued_turn(self, agent)
    }
}
