//! Codex 어댑터: 설명자, 연결 만들기, 권한 규칙을 전용 `CODEX_HOME`으로 번역하기.
//! 설계: docs/design/providers-and-sessions.md#provider-계층과-어댑터

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use saturn_core::permission::Rule;
use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};

use super::config::{relative_home, user_folder};
use super::home::{self, HomeInput};
use super::{CodexClient, direct};
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::{
    Adapter, AdapterConnection, AppliedReader, AppliedSettings, BoxFuture, ContextDefaults,
    Descriptor, DirectInstall, ExtensionLayout, Feature, INTERFACE_VERSION, LaunchSpec,
    PermissionInput, PermissionLaunch, ProviderConnection,
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
    // 훅은 Saturn 권한 판정에 끼어들 수 있는지 정해지기 전이라 주입하지 않는다
    extensions: ExtensionLayout {
        skills_dir: Some("skills"),
        commands: Some(("prompts", "md")),
        mcp_servers: true,
        hooks: false,
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

    /// 사용자 `~/.codex`에 직접 설치된 항목. 읽기만 한다.
    fn direct_installs(&self, env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
        direct::read(env)
    }

    /// 사용자 `~/.codex`는 읽기만 하고 전용 `CODEX_HOME`을 만든다. 사용자 폴더는 환경의 `CODEX_HOME`, 없으면
    /// `HOME/.codex`다.
    fn translate_permission(
        &self,
        input: PermissionInput<'_>,
    ) -> Result<PermissionLaunch, ProviderError> {
        if relative_home(input.env).is_some() {
            return Err(ProviderError::NotSent {
                reason:
                    "CODEX_HOME is a relative path; set an absolute path to use codex with saturn"
                        .to_owned(),
            });
        }
        let user_codex_home = user_folder(input.env).unwrap_or_else(|| PathBuf::from("/.codex"));
        let (prepared, injection_failures) = home::prepare_with(
            HomeInput {
                saturn_home: input.saturn_home,
                user_codex_home: &user_codex_home,
                rules: input.rules,
                questions: input.questions,
            },
            &DESCRIPTOR.extensions,
            input.extensions,
        )
        .map_err(|error| ProviderError::NotSent {
            reason: format!("failed to prepare codex home: {error}"),
        })?;
        Ok(PermissionLaunch {
            injection_failures,
            rules_fingerprint: home::rules_of_home(&prepared.path),
            env: vec![("CODEX_HOME".into(), prepared.path.into_os_string())],
            mcp_servers: prepared.mcp_servers,
            questions_disabled: !input.questions,
            ..PermissionLaunch::default()
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
