//! 명령으로 설정 파일 고치기: 읽은 버전을 확인한 뒤 쓰고, 주석과 순서를 보존한다.
//!
//! 설계: docs/design/settings.md(설정 파일 편집). 판단기 키 자체는 쓰지 않는다. 키는 `KeyInfo`(출처와 끝 4자리)만 쓴다.
//! 편집은 `toml_edit::DocumentMut`로 해 주석과 서식을 그대로 둔다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::layers::{fingerprint, parse_key, set_path};
use super::manager::read_file;
use super::trust::partial_path;
use super::{CONFIG_FILE, SettingsError, SettingsManager};
use crate::secrets::KeyInfo;

/// 사용자 층 키 정보의 점 경로 키(초안, TODO(#49)).
const KEY_INFO_KEY: &str = "judge.key.info";

/// 읽은 순간의 파일 버전. 쓰기 직전에 다시 재서 다르면 쓰지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    /// 파일 경로.
    pub path: PathBuf,
    /// 내용 지문(hex). 파일이 없었으면 `None`. 해시 SHA-256은 초안이다(설계는 지문만 정함).
    pub fingerprint: Option<String>,
    /// 수정 시각. 지문 비교 전에 빠른 확인용.
    pub modified: Option<SystemTime>,
}

impl SettingsManager {
    /// 파일을 읽고 버전을 함께 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Io`(없는 파일은 빈 내용과 `fingerprint: None`).
    pub async fn read_for_edit(&self, path: &Path) -> Result<(String, FileVersion), SettingsError> {
        read_versioned(path)
    }

    /// 점 경로 키 하나를 `value`(TOML 값 문법)로 바꾼다. 흐름: 읽기 → `toml_edit`로 그 키만 고치기 → 쓰기 직전 버전 비교
    /// → 같으면 임시 파일에 쓰고 이름 바꾸기. 주석, 빈 줄, 키 순서는 그대로 둔다. 사용자 파일과 폴더 파일만 고친다.
    ///
    /// # Errors
    /// 읽은 뒤 파일이 바뀌었으면 `Conflict`, 값이 TOML이 아니면 `Parse`, 쓰기 실패면 `Io`.
    pub async fn set_value(
        &self,
        path: &Path,
        key: &str,
        value: &str,
        read: &FileVersion,
    ) -> Result<(), SettingsError> {
        self.ensure_settings_file(path)?;
        let parse_error = |message: String| SettingsError::Parse {
            path: path.to_path_buf(),
            line: 1,
            message,
        };
        let keys = parse_key(key).map_err(parse_error)?;
        let value: toml_edit::Value = value
            .trim()
            .parse()
            .map_err(|error: toml_edit::TomlError| parse_error(error.message().to_owned()))?;
        let (content, now) = read_versioned(path)?;
        if now.fingerprint != read.fingerprint {
            return Err(SettingsError::Conflict {
                path: path.to_path_buf(),
            });
        }
        let mut doc: toml_edit::DocumentMut = content
            .parse()
            .map_err(|error: toml_edit::TomlError| parse_error(error.message().to_owned()))?;
        // 키가 없고 주석만 있는 파일은 주석이 문서 끝 꼬리로 읽힌다. 새 키 뒤로 밀리지 않게 앞에 다시 붙인다
        let mut leading = String::new();
        if doc.as_table().is_empty() {
            leading = doc.trailing().as_str().unwrap_or_default().to_owned();
            doc.set_trailing("");
        }
        set_path(doc.as_table_mut(), &keys, value);
        // 고친 내용을 만드는 사이에 바뀌었을 수 있으니 쓰기 직전에 한 번 더 잰다
        let (_, latest) = read_versioned(path)?;
        if latest.fingerprint != read.fingerprint {
            return Err(SettingsError::Conflict {
                path: path.to_path_buf(),
            });
        }
        replace_file(path, format!("{leading}{doc}").as_bytes()).map_err(|source| {
            SettingsError::Io {
                path: path.to_path_buf(),
                source,
            }
        })
    }

    /// 사용자 파일(`~/.saturn/config.toml`)이나 폴더 파일(`<폴더>/.saturn/config.toml`)인지 확인한다.
    ///
    /// # Errors
    /// 아니면 `Io`(`InvalidInput`).
    fn ensure_settings_file(&self, path: &Path) -> Result<(), SettingsError> {
        let is_user = path == self.user_config_path();
        let is_folder = path.file_name().is_some_and(|name| name == CONFIG_FILE)
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == ".saturn");
        if is_user || is_folder {
            return Ok(());
        }
        Err(SettingsError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a saturn settings file",
            ),
        })
    }

    /// 사용자 층에 판단기 키 정보(출처와 끝 4자리)를 쓴다. 키 원문은 받지 않는다. `set_value`와 같은 버전 확인을 거친다.
    ///
    /// # Errors
    /// `set_value`와 같다.
    pub async fn record_key_info(&self, info: &KeyInfo) -> Result<(), SettingsError> {
        let path = self.user_config_path();
        let source = serde_json::to_value(info.source)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .expect("key source should serialize to its variant name");
        let mut table = toml_edit::InlineTable::new();
        table.insert("source", source.into());
        table.insert("last4", info.last4.as_str().into());
        let (_, read) = self.read_for_edit(&path).await?;
        self.set_value(&path, KEY_INFO_KEY, &table.to_string(), &read)
            .await
    }
}

/// 파일 내용과 버전. 없는 파일은 빈 내용과 `fingerprint: None`.
fn read_versioned(path: &Path) -> Result<(String, FileVersion), SettingsError> {
    let content = read_file(path)?;
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok();
    let version = FileVersion {
        path: path.to_path_buf(),
        fingerprint: content.as_deref().map(fingerprint),
        modified,
    };
    Ok((content.unwrap_or_default(), version))
}

/// 같은 폴더의 임시 파일에 쓴 뒤 이름을 바꿔 갈아 끼운다. 원래 파일 권한을 그대로 둔다.
fn replace_file(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = partial_path(path);
    let mut file = std::fs::File::create(&partial)?;
    if let Ok(meta) = std::fs::metadata(path) {
        file.set_permissions(meta.permissions())?;
    }
    file.write_all(body)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::KeySource;
    use crate::store::Store;

    async fn manager(home: &Path) -> (Store, SettingsManager) {
        let (store, _) = Store::open(home).await.unwrap();
        let manager =
            SettingsManager::new(home.to_path_buf(), home.to_path_buf(), Vec::new(), &store)
                .await
                .unwrap();
        (store, manager)
    }

    #[tokio::test]
    async fn edit_keeps_comments_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(dir.path()).await;
        let path = dir.path().join(CONFIG_FILE);
        let original = "# 내 설정\non_exit = \"background\" # 닫아도 계속\n\n[judge.thresholds]\n# 조금 엄하게\ninjection = 0.7\n";
        std::fs::write(&path, original).unwrap();

        let (_, read) = manager.read_for_edit(&path).await.unwrap();
        manager
            .set_value(&path, "judge.thresholds.injection", "0.9", &read)
            .await
            .unwrap();

        let edited = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            edited,
            original.replace("injection = 0.7", "injection = 0.9")
        );
    }

    #[tokio::test]
    async fn edit_after_outside_change_is_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(dir.path()).await;
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "on_exit = \"background\"\n").unwrap();
        let (_, read) = manager.read_for_edit(&path).await.unwrap();
        std::fs::write(&path, "on_exit = \"ask\"\n").unwrap();

        let error = manager
            .set_value(&path, "on_exit", "\"stop\"", &read)
            .await
            .unwrap_err();

        assert!(matches!(error, SettingsError::Conflict { .. }));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "on_exit = \"ask\"\n"
        );
    }

    #[tokio::test]
    async fn edit_rejects_other_files_and_bad_values() {
        let dir = tempfile::tempdir().unwrap();
        let (_store, manager) = manager(dir.path()).await;
        let other = dir.path().join("other.toml");
        let (_, read) = manager.read_for_edit(&other).await.unwrap();
        assert!(read.fingerprint.is_none());

        let wrong_file = manager.set_value(&other, "on_exit", "\"ask\"", &read).await;
        let path = dir.path().join(CONFIG_FILE);
        let (_, read) = manager.read_for_edit(&path).await.unwrap();
        let bad_value = manager.set_value(&path, "on_exit", "not toml", &read).await;

        assert!(matches!(wrong_file, Err(SettingsError::Io { .. })));
        assert!(matches!(bad_value, Err(SettingsError::Parse { .. })));
    }

    #[tokio::test]
    async fn key_info_is_written_without_key() {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut manager) = manager(dir.path()).await;
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "# 사용자 설정\n").unwrap();
        let info = KeyInfo {
            source: KeySource::Stored,
            last4: "c0de".to_owned(),
        };

        manager.record_key_info(&info).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# 사용자 설정\n"), "{text}");
        let applied = manager.apply(&store, None).await.unwrap();
        let settings = manager.at(&store, applied.revision).await.unwrap();
        assert_eq!(settings.key_info(), Some(info));
    }
}
