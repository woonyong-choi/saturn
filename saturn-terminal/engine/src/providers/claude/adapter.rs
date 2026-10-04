//! Claude Code 어댑터: 설명자와 연결 만들기. 권한 규칙은 실행 인자와 훅 설정으로 `ClaudeClient`가 번역한다.
//! 설계: docs/design/providers-and-sessions.md#provider-계층과-어댑터

use std::sync::Arc;

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};

use super::ClaudeClient;
use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::{
    Adapter, AdapterConnection, AppliedReader, AppliedSettings, BoxFuture, ContextDefaults,
    Descriptor, Feature, INTERFACE_VERSION, LaunchSpec, ProviderConnection,
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
