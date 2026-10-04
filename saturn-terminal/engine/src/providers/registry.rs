//! 어댑터 레지스트리: 설명자 목록, id로 어댑터 찾기, 설치 확인, 모델 고정 글.
//! 설계: docs/design/providers-and-sessions.md#어댑터-등록

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::ModelChoice;

use super::adapter::{Adapter, ContextDefaults, Descriptor, Feature, INTERFACE_VERSION};
use super::{LaunchSpec, ProviderConnection};
use crate::processes::Supervisor;

/// 설명자가 없는 provider의 맥락 값. 기록에만 남은 id의 예산을 계산할 때만 쓴다.
const UNKNOWN_CONTEXT: ContextDefaults = ContextDefaults {
    window: 200_000,
    cache_write: 1.25,
};

/// 등록하지 못한 이유.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RegisterError {
    #[error("provider {provider} is already registered")]
    Duplicate { provider: Provider },
    #[error("provider {provider} uses interface version {version}, engine supports {supported}")]
    UnsupportedVersion {
        provider: Provider,
        version: u32,
        supported: u32,
    },
}

/// 시작할 때 어댑터를 등록해 두는 곳. 복제하면 같은 어댑터를 가리킨다.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    /// 기본 순서로 정렬돼 있다.
    adapters: Vec<Arc<dyn Adapter>>,
}

impl Registry {
    /// 어댑터 하나를 더한다. 같은 id가 이미 있거나 인터페이스 판이 지원 범위 밖이면 등록하지 않는다. 판이 어긋난
    /// 어댑터가 session 도중에 다르게 동작하는 일을 막기 위해서다.
    ///
    /// # Errors
    /// 설명자가 없는 provider의 맥락 값. 기록에만 남은 id의 예산을 계산할 때만 쓴다.
    const UNKNOWN_CONTEXT: ContextDefaults = ContextDefaults {
        window: 200_000,
        cache_write: 1.25,
    };

    /// 등록하지 못한 이유.
    pub(crate) fn register(&mut self, adapter: Arc<dyn Adapter>) -> Result<(), RegisterError> {
        let descriptor = adapter.descriptor();
        let provider = descriptor.id;
        if descriptor.interface_version != INTERFACE_VERSION {
            return Err(RegisterError::UnsupportedVersion {
                provider,
                version: descriptor.interface_version,
                supported: INTERFACE_VERSION,
            });
        }
        if self.get(provider).is_some() {
            return Err(RegisterError::Duplicate { provider });
        }
        self.adapters.push(adapter);
        self.adapters
            .sort_by_key(|adapter| adapter.descriptor().order);
        Ok(())
    }

    /// 등록을 시도하고 실패하면 로그만 남긴다. 어댑터 하나의 문제가 engine 시작을 막지 않게 하기 위해서다.
    pub(crate) fn register_or_log(&mut self, adapter: Arc<dyn Adapter>) {
        if let Err(error) = self.register(adapter) {
            tracing::warn!(%error, "provider adapter not registered");
        }
    }

    /// 기본 순서대로.
    pub(crate) fn descriptors(&self) -> impl Iterator<Item = &Descriptor> {
        self.adapters.iter().map(|adapter| adapter.descriptor())
    }

    /// 기본 순서대로.
    pub(crate) fn ids(&self) -> Vec<Provider> {
        self.descriptors().map(|descriptor| descriptor.id).collect()
    }

    pub(crate) fn get(&self, provider: Provider) -> Option<&Arc<dyn Adapter>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.descriptor().id == provider)
    }

    pub(crate) fn descriptor(&self, provider: Provider) -> Option<&Descriptor> {
        self.get(provider).map(|adapter| adapter.descriptor())
    }

    /// 사용량처럼 사용자에게 provider를 이름으로 보일 때 쓴다. 등록하지 않은 id는 id 글자를 그대로 쓴다.
    pub(crate) fn display_name(&self, provider: Provider) -> &'static str {
        self.descriptor(provider)
            .map_or(provider.as_str(), |descriptor| descriptor.display_name)
    }

    /// 등록하지 않은 id는 `UNKNOWN_CONTEXT`를 쓴다.
    pub(crate) fn context_defaults(&self, provider: Provider) -> ContextDefaults {
        self.descriptor(provider)
            .map_or(UNKNOWN_CONTEXT, |descriptor| descriptor.context)
    }

    pub(crate) fn supports(&self, provider: Provider, feature: Feature) -> bool {
        self.descriptor(provider)
            .is_some_and(|descriptor| descriptor.features.contains(&feature))
    }

    /// 패킷에 넣지 않을 provider 지시 문서 이름. 등록한 모든 어댑터의 것이다.
    pub(crate) fn instruction_docs(&self) -> Vec<String> {
        self.descriptors()
            .map(|descriptor| descriptor.instruction_doc.to_owned())
            .collect()
    }

    /// 환경의 `PATH`에서 실행 권한이 있는 어댑터 실행 파일을 찾는다. `PATH`가 없거나 등록하지 않은 id면 없는 것으로
    /// 본다.
    pub(crate) fn is_installed(&self, provider: Provider, env: &[(OsString, OsString)]) -> bool {
        let Some(descriptor) = self.descriptor(provider) else {
            return false;
        };
        let Some((_, path)) = env.iter().find(|(name, _)| name == "PATH") else {
            return false;
        };
        std::env::split_paths(path).any(|dir| {
            std::fs::metadata(dir.join(descriptor.program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    }

    /// 기본 순서에서 설치된 첫 provider.
    pub(crate) fn first_installed(
        &self,
        env: &[(OsString, OsString)],
        is_connected: impl Fn(Provider) -> bool,
    ) -> Option<Provider> {
        self.descriptors()
            .map(|descriptor| descriptor.id)
            .find(|provider| is_connected(*provider) || self.is_installed(*provider, env))
    }

    /// 입력 접수 기록에 남기는 고정 모델 글 `<provider>/<model>`. 같은 모델 이름이 두 provider에 있어도 구분된다.
    pub(crate) fn pinned_text(choice: &ModelChoice) -> String {
        format!("{}/{}", choice.provider, choice.model)
    }

    /// `pinned_text`가 만든 글을 되돌린다. 첫 `/`에서 나누고, provider 접두사가 없거나 등록하지 않은 id면 `None`.
    pub(crate) fn parse_pinned(&self, text: &str) -> Option<ModelChoice> {
        let (name, model) = text.split_once('/')?;
        let provider = Provider::from_stored(name).ok()?;
        self.get(provider)?;
        Some(ModelChoice {
            provider,
            model: model.to_owned(),
        })
    }

    /// 어댑터로 연결을 만든다.
    ///
    /// # Errors
    /// 등록하지 않은 provider면 `ConnectionLost`, 그 밖에는 어댑터가 정한다.
    pub async fn connect(
        &self,
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> Result<ProviderConnection, ProviderError> {
        let Some(adapter) = self.get(launch.provider) else {
            return Err(ProviderError::ConnectionLost);
        };
        adapter.connect(launch, supervisor).await
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::providers::test_support::{FakeAdapter, FakeProvider, fake_descriptor};

    fn adapter(id: &'static str, order: u32) -> Arc<dyn Adapter> {
        let id = Provider::from_static(id);
        let mut descriptor = fake_descriptor(id);
        descriptor.order = order;
        descriptor.program = id.as_str();
        Arc::new(FakeAdapter {
            descriptor,
            provider: FakeProvider::new(id),
        })
    }

    fn registry() -> Registry {
        let mut registry = Registry::default();
        registry.register(adapter("late", 30)).unwrap();
        registry.register(adapter("early", 10)).unwrap();
        registry.register(adapter("middle", 20)).unwrap();
        registry
    }

    #[test]
    fn descriptors_come_back_in_the_default_order() {
        let ids: Vec<&str> = registry().ids().into_iter().map(Provider::as_str).collect();

        assert_eq!(ids, ["early", "middle", "late"]);
    }

    #[test]
    fn duplicate_ids_and_other_interface_versions_are_not_registered() {
        let mut registry = registry();
        let mut other = fake_descriptor(Provider::from_static("odd"));
        other.interface_version = INTERFACE_VERSION + 1;
        let odd = Arc::new(FakeAdapter {
            descriptor: other,
            provider: FakeProvider::new(Provider::from_static("odd")),
        });

        assert!(matches!(
            registry.register(adapter("early", 1)),
            Err(RegisterError::Duplicate { .. })
        ));
        assert!(matches!(
            registry.register(odd),
            Err(RegisterError::UnsupportedVersion { .. })
        ));
        assert_eq!(registry.ids().len(), 3);
    }

    #[test]
    fn installed_means_an_executable_file_of_the_descriptor_on_the_given_path() {
        let registry = registry();
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("middle");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        let path = |dir: &std::path::Path| {
            vec![(std::ffi::OsString::from("PATH"), dir.as_os_str().to_owned())]
        };
        let middle = Provider::from_static("middle");

        let plain_file = registry.is_installed(middle, &path(dir.path()));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(!plain_file);
        assert!(registry.is_installed(middle, &path(dir.path())));
        assert!(!registry.is_installed(Provider::from_static("early"), &path(dir.path())));
        assert!(!registry.is_installed(middle, &[]));
        assert!(!registry.is_installed(Provider::from_static("absent"), &path(dir.path())));
        assert_eq!(
            registry.first_installed(&path(dir.path()), |_| false),
            Some(middle)
        );
    }

    #[test]
    fn pinned_text_round_trips_only_for_registered_ids() {
        let registry = registry();
        let choice = registry.parse_pinned("middle/a/b").unwrap();

        assert_eq!(choice.provider, Provider::from_static("middle"));
        assert_eq!(choice.model, "a/b");
        assert_eq!(Registry::pinned_text(&choice), "middle/a/b");
        assert_eq!(registry.parse_pinned("absent/x"), None);
        assert_eq!(registry.parse_pinned("no-prefix"), None);
    }

    #[test]
    fn unregistered_provider_shows_its_id_and_has_no_features() {
        let registry = registry();
        let absent = Provider::from_static("absent");

        assert_eq!(registry.display_name(absent), "absent");
        assert!(!registry.supports(absent, Feature::Steer));
        assert_eq!(registry.context_defaults(absent), UNKNOWN_CONTEXT);
    }
}
