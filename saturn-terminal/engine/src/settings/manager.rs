//! 설정 적용과 입력별 고정 번호 조회. 검사가 실패하면 이전 번호를 유지하고 경고한다.
//! 설계: docs/design/settings.md

use std::path::{Path, PathBuf};

use saturn_protocol::ids::{ChatId, SettingsRevision};

use super::layers::{default_layer, find_folder_config, fingerprint, merge, run_layer, source};
use super::{
    CONFIG_FILE, FolderTrustPrompt, Layer, LayerSource, Settings, SettingsError, TrustStatus,
    TrustStore,
};
use crate::store::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// 검사 실패면 이전 번호.
    pub revision: SettingsRevision,
    /// 검사 실패나 무시한 사용자 전용 키가 있을 때 한 줄 경고.
    pub warning: Option<String>,
}

/// engine에 하나. 작업 폴더와 실행 `-c`는 engine 시작 때 정한다.
#[derive(Debug)]
pub struct SettingsManager {
    home: PathBuf,
    workdir: PathBuf,
    run_overrides: Vec<String>,
    trust: TrustStore,
    /// 검사 실패 때 돌아갈 번호.
    current: Option<SettingsRevision>,
    /// 마지막 `apply` 때 본 설정 파일과 지문. 없던 파일은 `None`.
    seen: Option<Vec<(PathBuf, Option<String>)>>,
}

impl SettingsManager {
    /// # Errors
    /// 신뢰 기록 읽기 실패면 `Io`/`Parse`, 스냅샷 조회 실패면 `Store`.
    pub async fn new(
        home: PathBuf,
        workdir: PathBuf,
        run_overrides: Vec<String>,
        store: &Store,
    ) -> Result<Self, SettingsError> {
        let trust = TrustStore::load(&home).await?;
        let current = store.latest_settings_revision().await?;
        Ok(Self {
            home,
            workdir,
            run_overrides,
            trust,
            current,
            seen: None,
        })
    }

    /// 폴더 설정이 없으면 `None`. `Unknown`·`Changed`면 호출자가 신뢰 창을 열고 답을 `trust_folder`로 넘긴다.
    pub async fn folder_status(&self) -> Result<Option<(PathBuf, TrustStatus)>, SettingsError> {
        let Some(path) = find_folder_config(&self.workdir, &self.home).await? else {
            return Ok(None);
        };
        let content = read_file(&path)?.unwrap_or_default();
        let status = self.trust.status(&path, &content);
        Ok(Some((path, status)))
    }

    /// 신뢰 창에서 `y`를 확정했을 때 부른다.
    pub async fn trust_folder(
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
    pub async fn apply(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
    ) -> Result<Applied, SettingsError> {
        let (layers, untrusted) = self.collect_layers(store, chat).await?;
        if let Some(prompt) = untrusted {
            return Err(SettingsError::Untrusted { path: prompt.path });
        }
        self.merge_and_save(store, layers).await
    }

    /// 신뢰하지 않은 폴더 설정은 빼고 병합하고, 그 폴더 설정의 신뢰 창 내용을 함께 돌려준다.
    ///
    /// # Errors
    /// 이전 번호 없이 검사 실패면 `NoPreviousRevision`, 저장 실패면 `Store`.
    pub async fn apply_trusted(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
    ) -> Result<(Applied, Option<FolderTrustPrompt>), SettingsError> {
        let (layers, untrusted) = self.collect_layers(store, chat).await?;
        let applied = self.merge_and_save(store, layers).await?;
        Ok((applied, untrusted))
    }

    async fn collect_layers(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
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
        if let Some((path, status)) = self.folder_status().await? {
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
        self.seen = Some(seen);
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
            Err(error @ (SettingsError::Parse { .. } | SettingsError::Invalid { .. })) => {
                let Some(previous) = self.current else {
                    return Err(SettingsError::NoPreviousRevision);
                };
                return Ok(Applied {
                    revision: previous,
                    warning: Some(fallback_warning(&error, previous, &self.user_config_path())),
                });
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
        let warning = (!ignored.is_empty())
            .then(|| format!("폴더 설정의 사용자 전용 항목 무시 · {}", ignored.join(", ")));
        Ok(Applied { revision, warning })
    }

    pub(super) fn user_config_path(&self) -> PathBuf {
        self.home.join(CONFIG_FILE)
    }

    /// 입력 접수 때 이 값을 `NewInput::settings`로 고정한다.
    pub fn current(&self) -> Option<SettingsRevision> {
        self.current
    }

    /// 처리 중 설정이 바뀌어도 provider 실행과 judge 호출은 입력의 번호로 같은 값을 쓴다.
    ///
    /// # Errors
    /// 없는 번호면 `Store`.
    pub async fn at(
        &self,
        store: &Store,
        revision: SettingsRevision,
    ) -> Result<Settings, SettingsError> {
        Ok(store.settings_snapshot(revision).await?.settings)
    }

    /// 바뀌었으면 호출자가 다음 입력 접수 전에 `apply`한다. 한 번도 적용하지 않았으면 참.
    ///
    /// # Errors
    /// 파일 읽기 실패면 `Io`.
    pub async fn changed(&self) -> Result<bool, SettingsError> {
        let Some(seen) = &self.seen else {
            return Ok(true);
        };
        let mut now = Vec::new();
        let user_path = self.user_config_path();
        now.push((
            user_path.clone(),
            read_file(&user_path)?.as_deref().map(fingerprint),
        ));
        if let Some(path) = find_folder_config(&self.workdir, &self.home).await? {
            let content = read_file(&path)?;
            now.push((path, content.as_deref().map(fingerprint)));
        }
        Ok(now != *seen)
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

/// 예: `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`.
fn fallback_warning(error: &SettingsError, previous: SettingsRevision, user_path: &Path) -> String {
    let (layer, detail) = match error {
        SettingsError::Parse {
            path,
            line,
            message,
        } => (
            layer_name_of_path(path, user_path),
            format!("줄 {line}: {message}"),
        ),
        SettingsError::Invalid { key, reason, layer } => {
            (layer_name(*layer), format!("{key}: {reason}"))
        }
        other => ("설정", other.to_string()),
    };
    format!(
        "{layer} 오류 · 이전 설정 번호 {}로 계속 · {detail}",
        previous.0
    )
}

fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Default => "기본 설정",
        Layer::User => "사용자 설정",
        Layer::Folder => "폴더 설정",
        Layer::Chat => "채팅 설정",
        Layer::Run => "실행 설정",
    }
}

/// `merge`가 파일이 아닌 층에 붙이는 경로 이름과 맞춘다.
fn layer_name_of_path(path: &Path, user_path: &Path) -> &'static str {
    match path.to_str() {
        Some("-c") => layer_name(Layer::Run),
        Some("chat") => layer_name(Layer::Chat),
        Some("default") => layer_name(Layer::Default),
        _ if path == user_path => layer_name(Layer::User),
        _ => layer_name(Layer::Folder),
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
            SettingsManager::new(
                self.home.clone(),
                self.workdir.clone(),
                overrides,
                &self.store,
            )
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
        fixture.write_user("[judge.thresholds]\ninjection = 0.9\n");
        let mut manager = fixture.manager(&[]).await;

        let first = manager.apply(&fixture.store, None).await.unwrap();
        let second = manager.apply(&fixture.store, None).await.unwrap();
        let mut other = fixture.manager(&[]).await;
        let third = other.apply(&fixture.store, None).await.unwrap();

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
            "[judge]\nendpoint = \"https://x\"\n[judge.thresholds]\ninjection = 0.9\n",
        );
        let mut manager = fixture.manager(&[]).await;

        let error = manager.apply(&fixture.store, None).await.unwrap_err();
        assert!(matches!(error, SettingsError::Untrusted { .. }));

        let Some((found, TrustStatus::Unknown(prompt))) = manager.folder_status().await.unwrap()
        else {
            panic!("folder config should be unknown");
        };
        assert_eq!(found, path);
        manager
            .trust_folder(&path, &prompt.fingerprint)
            .await
            .unwrap();
        let applied = manager.apply(&fixture.store, None).await.unwrap();

        let settings = manager.at(&fixture.store, applied.revision).await.unwrap();
        assert_eq!(settings.thresholds().injection, 0.9);
        assert_eq!(settings.judge_endpoint(), "https://api.typesafe.ai");
        assert!(applied.warning.unwrap().contains("judge.endpoint"));
    }

    #[tokio::test]
    async fn apply_trusted_skips_untrusted_folder_and_returns_prompt() {
        let fixture = Fixture::new().await;
        let path = fixture.write_folder("[judge.thresholds]\ninjection = 0.9\n");
        let mut manager = fixture.manager(&[]).await;

        let (applied, prompt) = manager.apply_trusted(&fixture.store, None).await.unwrap();

        let settings = manager.at(&fixture.store, applied.revision).await.unwrap();
        assert_ne!(settings.thresholds().injection, 0.9);
        let prompt = prompt.unwrap();
        assert_eq!(prompt.path, std::fs::canonicalize(&path).unwrap());
        assert!(!manager.changed().await.unwrap());
        manager
            .trust_folder(&path, &prompt.fingerprint)
            .await
            .unwrap();
        let (trusted, none) = manager.apply_trusted(&fixture.store, None).await.unwrap();
        let settings = manager.at(&fixture.store, trusted.revision).await.unwrap();
        assert_eq!(settings.thresholds().injection, 0.9);
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn invalid_settings_keep_previous_revision() {
        let fixture = Fixture::new().await;
        let mut manager = fixture.manager(&[]).await;
        let first = manager.apply(&fixture.store, None).await.unwrap();
        fixture.write_user("\n\n\n\n\n\nbroken = = 1\n");

        let applied = manager.apply(&fixture.store, None).await.unwrap();

        assert_eq!(applied.revision, first.revision);
        let warning = applied.warning.unwrap();
        assert!(
            warning.starts_with("사용자 설정 오류 · 이전 설정 번호 1로 계속 · 줄 7: "),
            "{warning}"
        );
        assert_eq!(manager.current(), Some(first.revision));
    }

    #[tokio::test]
    async fn invalid_settings_without_previous_revision_stop_start() {
        let fixture = Fixture::new().await;
        let mut manager = fixture
            .manager(&["judge.thresholds.keep_current=0.5"])
            .await;

        let error = manager.apply(&fixture.store, None).await.unwrap_err();

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
            .set_chat_layer(chat, "judge.thresholds.injection = 0.75\n")
            .await
            .unwrap();
        let mut manager = fixture.manager(&["on_exit=\"ask\""]).await;
        let fixed = manager
            .apply(&fixture.store, Some(chat))
            .await
            .unwrap()
            .revision;
        assert!(!manager.changed().await.unwrap());

        fixture.write_user("[judge.thresholds]\ninjection = 0.95\nprogressing = 0.4\n");
        assert!(manager.changed().await.unwrap());
        let newer = manager
            .apply(&fixture.store, Some(chat))
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
