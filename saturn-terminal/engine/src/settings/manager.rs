//! 설정 적용과 입력별 고정 번호 조회. 검사가 실패하면 이전 번호를 유지하고 경고한다.
//! 설계: docs/design/settings.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use saturn_protocol::ids::{ChatId, SettingsRevision};
use saturn_protocol::rpc::{SettingsFault, SettingsLayer, SettingsWarning};

use super::layers::{default_layer, find_folder_config, fingerprint, merge, run_layer, source};
use super::permission;
use super::{
    CONFIG_FILE, FolderTrustPrompt, Layer, LayerSource, Settings, SettingsError, TrustStatus,
    TrustStore,
};
use crate::store::Store;

/// 설정 파일 경로마다 내용의 지문. 없는 파일은 `None`.
pub(crate) type FileFingerprints = Vec<(PathBuf, Option<String>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    /// 검사 실패면 이전 번호.
    pub revision: SettingsRevision,
    /// 검사 실패나 무시한 사용자 전용 키가 있을 때의 경고.
    pub warning: Option<SettingsWarning>,
    /// 병합한 `tui.keymap`. 이전 번호로 계속하면 `None`.
    pub keymap: Option<String>,
}

/// engine에 하나. 실행 `-c`는 engine 시작 때 정하고, 작업 폴더는 채팅마다 호출 때 받는다.
#[derive(Debug)]
pub(crate) struct SettingsManager {
    home: PathBuf,
    run_overrides: Vec<String>,
    trust: TrustStore,
    /// 검사 실패 때 돌아갈 번호.
    current: Option<SettingsRevision>,
    /// 작업 폴더마다 마지막 `apply` 때 본 설정 파일과 지문. 없던 파일은 `None`.
    seen: HashMap<PathBuf, FileFingerprints>,
}

impl SettingsManager {
    /// # Errors
    /// 신뢰 기록 읽기 실패면 `Io`/`Parse`, 스냅샷 조회 실패면 `Store`.
    pub(crate) async fn new(
        home: PathBuf,
        run_overrides: Vec<String>,
        store: &Store,
    ) -> Result<Self, SettingsError> {
        let trust = TrustStore::load(&home).await?;
        let current = store.latest_settings_revision().await?;
        Ok(Self {
            home,
            run_overrides,
            trust,
            current,
            seen: HashMap::new(),
        })
    }

    /// 폴더 설정이 없으면 `None`. `Unknown`·`Changed`면 호출자가 신뢰 창을 열고 답을 `trust_folder`로 넘긴다.
    pub(crate) async fn folder_status(
        &self,
        workdir: &Path,
    ) -> Result<Option<(PathBuf, TrustStatus)>, SettingsError> {
        let Some(path) = find_folder_config(workdir, &self.home).await? else {
            return Ok(None);
        };
        let content = read_file(&path)?.unwrap_or_default();
        let mut status = self.trust.status(&path, &content);
        let user = read_file(&self.user_config_path())?;
        if permission::is_folder_mode_ignored(user.as_deref(), &content)
            && let TrustStatus::Unknown(prompt) | TrustStatus::Changed(prompt) = &mut status
        {
            prompt.ignore(permission::MODE_KEY);
        }
        Ok(Some((path, status)))
    }

    /// 신뢰 창에서 `y`를 확정했을 때 부른다.
    pub(crate) async fn trust_folder(
        &mut self,
        path: &Path,
        fingerprint: &str,
    ) -> Result<(), SettingsError> {
        self.trust.trust(path, fingerprint).await
    }

    /// 검사가 실패하면 `current`의 번호와 경고를 돌려준다. 보조 에이전트도 부모의 `chat`을 넘긴다.
    ///
    /// # Errors
    /// 폴더 설정 미신뢰면 `Untrusted`, 이전 번호 없이 검사 실패면 `NoPreviousRevision`, 저장 실패면 `Store`.
    pub(crate) async fn apply(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
        workdir: &Path,
    ) -> Result<Applied, SettingsError> {
        let (layers, untrusted) = self.collect_layers(store, chat, Some(workdir)).await?;
        if let Some(prompt) = untrusted {
            return Err(SettingsError::Untrusted { path: prompt.path });
        }
        self.merge_and_save(store, layers).await
    }

    /// 신뢰하지 않은 폴더 설정은 빼고 병합하고, 그 폴더 설정의 신뢰 창 내용을 함께 돌려준다.
    ///
    /// # Errors
    /// 이전 번호 없이 검사 실패면 `NoPreviousRevision`, 저장 실패면 `Store`.
    pub(crate) async fn apply_trusted(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
        workdir: &Path,
    ) -> Result<(Applied, Option<FolderTrustPrompt>), SettingsError> {
        let (layers, untrusted) = self.collect_layers(store, chat, Some(workdir)).await?;
        let applied = self.merge_and_save(store, layers).await?;
        Ok((applied, untrusted))
    }

    /// 폴더 층과 채팅 층 없이 병합한다. 채팅이 붙기 전인 engine 시작 때 쓴다.
    ///
    /// # Errors
    /// 이전 번호 없이 검사 실패면 `NoPreviousRevision`, 저장 실패면 `Store`.
    pub(crate) async fn apply_user(&mut self, store: &Store) -> Result<Applied, SettingsError> {
        let (layers, _) = self.collect_layers(store, None, None).await?;
        self.merge_and_save(store, layers).await
    }

    async fn collect_layers(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
        workdir: Option<&Path>,
    ) -> Result<(Vec<(LayerSource, String)>, Option<FolderTrustPrompt>), SettingsError> {
        let user_path = self.user_config_path();
        let mut layers = vec![(
            source(Layer::Default, None, default_layer()),
            default_layer().to_owned(),
        )];
        let mut seen = Vec::new();
        let mut untrusted = None;
        let user = read_file(&user_path)?;
        seen.push((user_path.clone(), user.as_deref().map(fingerprint)));
        if let Some(content) = user {
            layers.push((source(Layer::User, Some(user_path), &content), content));
        }
        if let Some(workdir) = workdir
            && let Some((path, status)) = self.folder_status(workdir).await?
        {
            let content = read_file(&path)?.unwrap_or_default();
            seen.push((path.clone(), Some(fingerprint(&content))));
            match status {
                TrustStatus::Trusted => {
                    layers.push((source(Layer::Folder, Some(path), &content), content));
                }
                TrustStatus::Unknown(prompt) | TrustStatus::Changed(prompt) => {
                    untrusted = Some(prompt);
                }
            }
        }
        if let Some(chat) = chat
            && let Some(content) = store.chat_layer(chat).await?
        {
            layers.push((source(Layer::Chat, None, &content), content));
        }
        if let Some(workdir) = workdir {
            self.seen.insert(workdir.to_path_buf(), seen);
        }
        Ok((layers, untrusted))
    }

    async fn merge_and_save(
        &mut self,
        store: &Store,
        mut layers: Vec<(LayerSource, String)>,
    ) -> Result<Applied, SettingsError> {
        let merged = match run_layer(&self.run_overrides) {
            Ok(content) => {
                layers.push((source(Layer::Run, None, &content), content));
                merge(layers)
            }
            Err(error) => Err(error),
        };
        let snapshot = match merged {
            Ok(snapshot) => snapshot,
            Err(SettingsError::Parse {
                path,
                line,
                message,
            }) => {
                let layer = layer_of_path(&path, &self.user_config_path());
                let line = u32::try_from(line).unwrap_or(u32::MAX);
                return self.fall_back(layer, SettingsFault::Parse { line, message });
            }
            Err(SettingsError::Invalid { key, reason, layer }) => {
                return self.fall_back(layer.into(), SettingsFault::Invalid { key, reason });
            }
            Err(error) => return Err(error),
        };
        let revision = store.save_settings_snapshot(&snapshot).await?;
        store.mark_settings_applied(revision).await?;
        self.current = Some(revision);
        let ignored: Vec<&str> = snapshot
            .layers
            .iter()
            .flat_map(|layer| layer.ignored.iter().map(String::as_str))
            .collect();
        let warning = (!ignored.is_empty()).then(|| SettingsWarning::IgnoredFolderKeys {
            keys: ignored.iter().map(|key| (*key).to_owned()).collect(),
        });
        let keymap = Some(snapshot.settings.keymap().to_owned());
        Ok(Applied {
            revision,
            warning,
            keymap,
        })
    }

    /// 이전 번호로 계속한다.
    ///
    /// # Errors
    /// 이전 번호가 없으면 `NoPreviousRevision`.
    fn fall_back(
        &self,
        layer: SettingsLayer,
        fault: SettingsFault,
    ) -> Result<Applied, SettingsError> {
        let previous = self.current.ok_or(SettingsError::NoPreviousRevision)?;
        Ok(Applied {
            revision: previous,
            warning: Some(SettingsWarning::Fallback { layer, fault }),
            keymap: None,
        })
    }

    pub(super) fn user_config_path(&self) -> PathBuf {
        self.home.join(CONFIG_FILE)
    }

    /// 입력 접수 때 이 값을 `NewInput::settings`로 고정한다.
    pub(crate) fn current(&self) -> Option<SettingsRevision> {
        self.current
    }

    /// 처리 중 설정이 바뀌어도 provider 실행과 router 호출은 입력의 번호로 같은 값을 쓴다.
    ///
    /// # Errors
    /// 없는 번호면 `Store`.
    pub(crate) async fn at(
        &self,
        store: &Store,
        revision: SettingsRevision,
    ) -> Result<Settings, SettingsError> {
        Ok(store.settings_snapshot(revision).await?.settings)
    }

    /// 바뀌었으면 호출자가 다음 입력 접수 전에 `apply`한다. 이 작업 폴더로 한 번도 적용하지 않았으면 참.
    ///
    /// # Errors
    /// 파일 읽기 실패면 `Io`.
    pub(crate) async fn changed(&self, workdir: &Path) -> Result<bool, SettingsError> {
        Ok(self.observe(workdir).await?.is_some())
    }

    /// 마지막 `apply` 뒤 설정 파일이 바뀌었으면 지금 파일의 지문을 돌려준다. 이 작업 폴더로 한 번도 적용하지 않았으면
    /// 바뀐 것으로 본다. 파일 감시가 같은 지문을 두 번 연달아 본 뒤에만 적용하는 데 쓴다.
    ///
    /// # Errors
    /// 파일 읽기 실패면 `Io`.
    pub(crate) async fn observe(
        &self,
        workdir: &Path,
    ) -> Result<Option<FileFingerprints>, SettingsError> {
        let mut now = Vec::new();
        let user_path = self.user_config_path();
        now.push((
            user_path.clone(),
            read_file(&user_path)?.as_deref().map(fingerprint),
        ));
        if let Some(path) = find_folder_config(workdir, &self.home).await? {
            let content = read_file(&path)?;
            now.push((path, content.as_deref().map(fingerprint)));
        }
        let unchanged = self.seen.get(workdir).is_some_and(|seen| *seen == now);
        Ok((!unchanged).then_some(now))
    }
}

/// 없으면 `None`.
///
/// # Errors
/// 없음 밖의 읽기 실패면 `Io`.
pub(crate) fn read_file(path: &Path) -> Result<Option<String>, SettingsError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SettingsError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

impl From<Layer> for SettingsLayer {
    fn from(layer: Layer) -> Self {
        match layer {
            Layer::Default => Self::Default,
            Layer::User => Self::User,
            Layer::Folder => Self::Folder,
            Layer::Chat => Self::Chat,
            Layer::Run => Self::Run,
        }
    }
}

/// `merge`가 파일이 아닌 층에 붙이는 경로 이름과 맞춘다.
fn layer_of_path(path: &Path, user_path: &Path) -> SettingsLayer {
    match path.to_str() {
        Some("-c") => SettingsLayer::Run,
        Some("chat") => SettingsLayer::Chat,
        Some("default") => SettingsLayer::Default,
        _ if path == user_path => SettingsLayer::User,
        _ => SettingsLayer::Folder,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    struct Fixture {
        _root: tempfile::TempDir,
        home: PathBuf,
        workdir: PathBuf,
        store: Store,
    }

    impl Fixture {
        async fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().join("home");
            let workdir = root.path().join("work");
            std::fs::create_dir_all(workdir.join(".git")).unwrap();
            let (store, _) = Store::open(&home).await.unwrap();
            Self {
                _root: root,
                home,
                workdir,
                store,
            }
        }

        async fn manager(&self, overrides: &[&str]) -> SettingsManager {
            let overrides = overrides.iter().map(|item| (*item).to_owned()).collect();
            SettingsManager::new(self.home.clone(), overrides, &self.store)
                .await
                .unwrap()
        }

        fn write_user(&self, content: &str) {
            std::fs::write(self.home.join(CONFIG_FILE), content).unwrap();
        }

        fn write_folder(&self, content: &str) -> PathBuf {
            let path = self.workdir.join(".saturn").join(CONFIG_FILE);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, content).unwrap();
            path
        }

        async fn snapshot_count(&self) -> u64 {
            let mut count = 0;
            while self
                .store
                .settings_snapshot(SettingsRevision(count + 1))
                .await
                .is_ok()
            {
                count += 1;
            }
            count
        }
    }

    #[tokio::test]
    async fn same_content_reuses_revision() {
        let fixture = Fixture::new().await;
        fixture.write_user("[router.thresholds]\ninjection = 0.9\n");
        let mut manager = fixture.manager(&[]).await;

        let first = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        let second = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        let mut other = fixture.manager(&[]).await;
        let third = other
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        assert_eq!(first.revision, SettingsRevision(1));
        assert_eq!(second.revision, first.revision);
        assert_eq!(third.revision, first.revision);
        assert_eq!(fixture.snapshot_count().await, 1);
        assert_eq!(
            fixture.store.latest_settings_revision().await.unwrap(),
            Some(first.revision)
        );
    }

    #[tokio::test]
    async fn untrusted_folder_is_not_merged_until_trusted() {
        let fixture = Fixture::new().await;
        let path = fixture.write_folder(
            "[router]\nendpoint = \"https://x\"\n[router.thresholds]\ninjection = 0.9\n",
        );
        let mut manager = fixture.manager(&[]).await;

        let error = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap_err();
        assert!(matches!(error, SettingsError::Untrusted { .. }));

        let Some((found, TrustStatus::Unknown(prompt))) =
            manager.folder_status(&fixture.workdir).await.unwrap()
        else {
            panic!("folder config should be unknown");
        };
        assert_eq!(found, path);
        manager
            .trust_folder(&path, &prompt.fingerprint)
            .await
            .unwrap();
        let applied = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        let settings = manager.at(&fixture.store, applied.revision).await.unwrap();
        assert_eq!(settings.thresholds().injection, 0.9);
        assert_eq!(settings.router_endpoint(), "https://api.typesafe.ai");
        let Some(SettingsWarning::IgnoredFolderKeys { keys }) = applied.warning else {
            panic!("ignored keys should warn");
        };
        assert!(keys.iter().any(|key| key.contains("router.endpoint")));
    }

    #[tokio::test]
    async fn apply_trusted_skips_untrusted_folder_and_returns_prompt() {
        let fixture = Fixture::new().await;
        let path = fixture.write_folder("[router.thresholds]\ninjection = 0.9\n");
        let mut manager = fixture.manager(&[]).await;

        let (applied, prompt) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        let settings = manager.at(&fixture.store, applied.revision).await.unwrap();
        assert_ne!(settings.thresholds().injection, 0.9);
        let prompt = prompt.unwrap();
        assert_eq!(prompt.path, std::fs::canonicalize(&path).unwrap());
        assert!(!manager.changed(&fixture.workdir).await.unwrap());
        manager
            .trust_folder(&path, &prompt.fingerprint)
            .await
            .unwrap();
        let (trusted, none) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        let settings = manager.at(&fixture.store, trusted.revision).await.unwrap();
        assert_eq!(settings.thresholds().injection, 0.9);
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn folder_permission_mode_that_does_not_lower_is_listed_as_ignored_in_the_trust_prompt() {
        let fixture = Fixture::new().await;
        fixture.write_user("permission.mode = \"edit\"\n");
        fixture.write_folder("permission.mode = \"full\"\npermission.shell = \"allow\"\n");
        let mut manager = fixture.manager(&[]).await;

        let (_, prompt) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        let prompt = prompt.unwrap();
        assert_eq!(prompt.ignored, vec!["permission.mode"]);
        assert_eq!(prompt.applied, vec!["permission.shell"]);
    }

    #[tokio::test]
    async fn folder_permission_mode_that_lowers_is_applied_and_trusted_rules_merge_after_user_rules()
     {
        let fixture = Fixture::new().await;
        fixture.write_user("[permission.shell]\n\"ls\" = \"allow\"\n");
        let path = fixture
            .write_folder("permission.mode = \"ask\"\n[permission.shell]\n\"ls\" = \"ask\"\n");
        let mut manager = fixture.manager(&[]).await;
        let (_, prompt) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        let prompt = prompt.unwrap();
        assert!(prompt.ignored.is_empty());
        manager
            .trust_folder(&path, &prompt.fingerprint)
            .await
            .unwrap();

        let (applied, _) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        let permission = manager
            .at(&fixture.store, applied.revision)
            .await
            .unwrap()
            .permission();
        assert_eq!(permission.mode, saturn_core::permission::Mode::Ask);
        let verdicts: Vec<_> = permission.rules.iter().map(|rule| rule.verdict).collect();
        assert_eq!(
            verdicts,
            vec![
                saturn_core::permission::Verdict::Allow,
                saturn_core::permission::Verdict::Ask
            ]
        );
    }

    #[tokio::test]
    async fn folder_layer_follows_the_workdir_given_per_call() {
        let fixture = Fixture::new().await;
        let path = fixture.write_folder("[router.thresholds]\ninjection = 0.9\n");
        let other = fixture._root.path().join("other");
        std::fs::create_dir_all(other.join(".git")).unwrap();
        let mut manager = fixture.manager(&[]).await;
        let (_, prompt) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        manager
            .trust_folder(&path, &prompt.unwrap().fingerprint)
            .await
            .unwrap();

        let (here, _) = manager
            .apply_trusted(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        let (there, none) = manager
            .apply_trusted(&fixture.store, None, &other)
            .await
            .unwrap();
        let user_only = manager.apply_user(&fixture.store).await.unwrap();

        let injection = |revision| {
            let store = &fixture.store;
            let manager = &manager;
            async move {
                manager
                    .at(store, revision)
                    .await
                    .unwrap()
                    .thresholds()
                    .injection
            }
        };
        assert_eq!(injection(here.revision).await, 0.9);
        assert_ne!(injection(there.revision).await, 0.9);
        assert_eq!(there.revision, user_only.revision);
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn invalid_settings_keep_previous_revision() {
        let fixture = Fixture::new().await;
        let mut manager = fixture.manager(&[]).await;
        let first = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();
        fixture.write_user("\n\n\n\n\n\nbroken = = 1\n");

        let applied = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap();

        assert_eq!(applied.revision, first.revision);
        let Some(SettingsWarning::Fallback {
            layer,
            fault: SettingsFault::Parse { line, .. },
        }) = applied.warning
        else {
            panic!("parse failure should fall back");
        };
        assert_eq!((layer, line), (SettingsLayer::User, 7));
        assert_eq!(manager.current(), Some(first.revision));
    }

    #[tokio::test]
    async fn invalid_settings_without_previous_revision_stop_start() {
        let fixture = Fixture::new().await;
        let mut manager = fixture
            .manager(&["router.thresholds.keep_current=0.5"])
            .await;

        let error = manager
            .apply(&fixture.store, None, &fixture.workdir)
            .await
            .unwrap_err();

        assert!(matches!(error, SettingsError::NoPreviousRevision));
    }

    #[tokio::test]
    async fn fixed_revision_keeps_old_values_after_change() {
        let fixture = Fixture::new().await;
        let chat = fixture
            .store
            .create_chat(fixture.workdir.clone())
            .await
            .unwrap();
        fixture
            .store
            .set_chat_layer(chat, "router.thresholds.injection = 0.75\n")
            .await
            .unwrap();
        let mut manager = fixture.manager(&["on_exit=\"ask\""]).await;
        let fixed = manager
            .apply(&fixture.store, Some(chat), &fixture.workdir)
            .await
            .unwrap()
            .revision;
        assert!(!manager.changed(&fixture.workdir).await.unwrap());

        fixture.write_user("[router.thresholds]\ninjection = 0.95\nprogressing = 0.4\n");
        assert!(manager.changed(&fixture.workdir).await.unwrap());
        let newer = manager
            .apply(&fixture.store, Some(chat), &fixture.workdir)
            .await
            .unwrap()
            .revision;

        assert_ne!(newer, fixed);
        let old = manager.at(&fixture.store, fixed).await.unwrap();
        let new = manager.at(&fixture.store, newer).await.unwrap();
        assert_eq!(old.thresholds().injection, 0.75);
        assert_eq!(old.thresholds().progressing, 0.2);
        assert_eq!(new.thresholds().injection, 0.75);
        assert_eq!(new.thresholds().progressing, 0.4);
        assert_eq!(new.on_exit(), saturn_protocol::state::OnExit::Ask);
    }
}
