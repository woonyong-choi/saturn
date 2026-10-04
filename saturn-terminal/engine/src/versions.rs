//! provider CLI 버전 감지: engine을 시작할 때 설치된 CLI의 버전을 읽어 마지막으로 확인한 버전과 비교한다.
//! 설계: docs/design/providers-and-sessions.md#직접-연결과-acp-어댑터

use std::ffi::OsString;

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{Alert, ProviderInfo};

use crate::Engine;

/// 버전이 바뀐 provider 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VersionChange {
    pub(crate) provider: Provider,
    pub(crate) from: String,
    pub(crate) to: String,
}

impl VersionChange {
    pub(crate) fn alert(&self) -> Alert {
        Alert::ProviderUpdated {
            provider: self.provider,
            from: self.from.clone(),
            to: self.to.clone(),
        }
    }
}

impl Engine {
    /// 등록한 어댑터마다 설치된 CLI의 버전을 읽어 기록한다. 마지막으로 확인한 버전이 있고 다르면 첫 TUI에 알릴 변경으로
    /// 둔다. 처음 확인하는 provider는 알리지 않고 기록만 한다. 읽지 못한 provider는 기록을 그대로 둔다. 읽기와 기록의
    /// 실패는 시작을 막지 않고 로그만 남긴다.
    pub(crate) async fn detect_provider_versions(&mut self, env: &[(OsString, OsString)]) {
        let reads: Vec<_> = self
            .registry
            .ids()
            .into_iter()
            .filter_map(|provider| {
                let adapter = self.registry.get(provider)?.clone();
                let env = env.to_vec();
                Some((
                    provider,
                    tokio::spawn(async move { adapter.read_version(&env).await }),
                ))
            })
            .collect();
        for (provider, read) in reads {
            if let Some(version) = read.await.ok().flatten() {
                self.flow.cli_versions.insert(provider, version.clone());
                self.compare_version(provider, version).await;
            }
        }
    }

    /// 마지막으로 확인한 버전과 비교해 다르면 기록하고, 이전 값이 있었으면 알릴 변경으로 둔다.
    async fn compare_version(&mut self, provider: Provider, version: String) {
        let previous = match self.store.provider_cli_version(provider).await {
            Ok(previous) => previous,
            Err(error) => {
                self.warn_failure(
                    "failed to read the last checked version",
                    Err::<(), _>(error),
                );
                return;
            }
        };
        if previous.as_deref() == Some(version.as_str()) {
            return;
        }
        let recorded = self
            .store
            .record_provider_cli_version(provider, &version)
            .await;
        self.warn_failure("failed to record the checked version", recorded);
        if let Some(from) = previous {
            self.notices.provider_updates.push(VersionChange {
                provider,
                from,
                to: version,
            });
        }
    }

    /// 붙을 때 알리는 provider 목록. 버전은 시작 때 읽은 값이고 읽지 못했으면 빈 글자다.
    pub(crate) fn provider_infos(&self) -> Vec<ProviderInfo> {
        self.registry
            .descriptors()
            .map(|descriptor| ProviderInfo {
                provider: descriptor.id,
                display_name: descriptor.display_name.to_owned(),
                version: self
                    .flow
                    .cli_versions
                    .get(&descriptor.id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect()
    }
}
