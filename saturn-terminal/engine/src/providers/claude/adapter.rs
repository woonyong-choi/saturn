//! Claude Code 어댑터: 설명자와 연결 만들기. 권한 규칙은 실행 인자와 훅 설정으로 `ClaudeClient`가 번역한다.
//! 설계: docs/design/providers-and-sessions.md#provider-계층과-어댑터

use std::ffi::OsString;
use std::sync::Arc;

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};

use super::{ClaudeClient, direct, extensions};
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::{
    Adapter, AdapterConnection, AppliedReader, AppliedSettings, BoxFuture, ContextDefaults,
    Descriptor, DirectInstall, ExtensionLayout, Feature, INTERFACE_VERSION, LaunchSpec,
    PermissionInput, PermissionLaunch, ProviderConnection, read_deny_fingerprint,
};

/// 설정 키 `provider.claude.*`와 모델 고정 글 `claude/<model>`의 앞부분이다.
pub(crate) const ID: Provider = Provider::from_static("claude");

static DESCRIPTOR: Descriptor = Descriptor {
    id: ID,
    display_name: "claude",
    program: "claude",
    order: 10,
    features: &[Feature::Steer, Feature::Compact],
    instruction_doc: "CLAUDE.md",
    interface_version: INTERFACE_VERSION,
    context: ContextDefaults {
        window: 1_000_000,
        cache_write: 1.25,
    },
    // 훅은 플러그인 폴더의 `hooks/hooks.json`으로 넘기고, Saturn 소유 훅이 든 실행별 `--settings`와 섞지 않는다
    extensions: ExtensionLayout {
        skills_dir: Some("skills"),
        commands: Some(("commands", "md")),
        mcp_servers: true,
        hooks: true,
    },
};

#[derive(Debug)]
struct ClaudeAdapter;

pub(crate) fn adapter() -> Arc<dyn Adapter> {
    Arc::new(ClaudeAdapter)
}

impl Adapter for ClaudeAdapter {
    fn descriptor(&self) -> &Descriptor {
        &DESCRIPTOR
    }

    /// 사용자 `~/.claude`에 직접 설치된 항목. 읽기만 한다.
    fn direct_installs(&self, env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
        direct::read(env)
    }

    /// 질문 기능은 실행 인자로 따로 넣고, 확장은 스킬과 명령을 플러그인 폴더로, MCP 서버를 설정 파일로 만들어 실행
    /// 인자로 넘긴다. 사용자 Claude 설정은 읽지도 고치지도 않는다.
    fn translate_permission(
        &self,
        input: PermissionInput<'_>,
    ) -> Result<PermissionLaunch, ProviderError> {
        let injected =
            extensions::inject(input.saturn_home, &DESCRIPTOR.extensions, input.extensions)
                .map_err(|error| ProviderError::NotSent {
                    reason: format!("failed to prepare claude extensions: {error}"),
                })?;
        Ok(PermissionLaunch {
            questions_disabled: !input.questions,
            extra_args: injected.args,
            injection_failures: injected.failures,
            rules_fingerprint: Some(read_deny_fingerprint(input.rules)),
            ..PermissionLaunch::default()
        })
    }

    /// 읽기 `deny` 규칙은 session을 열 때 실행 설정으로 고정되므로 바뀌면 연결을 다시 시작한다.
    fn rules_fingerprint(&self, rules: &[saturn_core::permission::Rule]) -> Option<String> {
        Some(read_deny_fingerprint(rules))
    }

    /// 프로세스는 session을 열 때 띄운다.
    fn connect(
        &self,
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> BoxFuture<'_, Result<ProviderConnection, ProviderError>> {
        Box::pin(async move {
            Ok(ProviderConnection::new(
                ID,
                ClaudeClient::new(launch, supervisor),
            ))
        })
    }
}

impl AdapterConnection for ClaudeClient {
    fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        ClaudeClient::process_group(self, session)
    }

    fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        ClaudeClient::applied_settings(self, session)
    }

    fn applied_reader(&self, session: &ProviderSessionId) -> Option<AppliedReader> {
        ClaudeClient::applied_reader(self, session)
    }

    fn start_queued_turn(&mut self, agent: AgentId) -> impl Future<Output = ()> + Send {
        ClaudeClient::start_queued_turn(self, agent)
    }
}
