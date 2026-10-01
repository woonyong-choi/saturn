//! 폴더 설정 신뢰: 경로와 지문으로 기록하고, 처음 보거나 내용이 바뀐 폴더 설정은 한 번 묻는다.
//!
//! 설계: docs/design/settings.md(폴더 설정 신뢰). 신뢰 창(TUI)의 키: `1`·`y` 적용하고 계속, `↑`·`↓` 이동, `Enter` 확정,
//! `3`·`q`·`Esc`·`Ctrl+C` 종료. 실행 중 폴더 설정이 바뀌면 다음 입력을 접수하기 전에 창을 연다.
//! 신뢰 기록 위치와 형식(`~/.saturn/trusted.json`, 경로 → 지문 JSON 객체, 권한 0600)은 초안이다(설계에 없음).

use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::SettingsError;
use super::layers::{fingerprint, is_user_only, leaf_keys, parse_toml};

/// 신뢰 기록 파일 이름. `~/.saturn/` 아래에 둔다.
const TRUST_FILE: &str = "trusted.json";

/// 신뢰 기록 파일 권한.
const TRUST_FILE_MODE: u32 = 0o600;

/// 폴더 설정의 신뢰 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStatus {
    /// 같은 경로, 같은 지문으로 신뢰했다. 병합한다.
    Trusted,
    /// 처음 본다. 묻기 전에는 병합하지 않는다.
    Unknown(FolderTrustPrompt),
    /// 신뢰한 뒤 내용이 바뀌었다. 다시 묻기 전에는 병합하지 않는다(옛 지문 내용도 쓰지 않는다).
    Changed(FolderTrustPrompt),
}

/// 신뢰 창에 보일 내용.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderTrustPrompt {
    /// 폴더 설정 파일 경로.
    pub path: PathBuf,
    /// 지금 내용의 지문(hex). 해시 SHA-256은 초안이다(설계는 지문만 정함).
    pub fingerprint: String,
    /// 적용되는 점 경로 키.
    pub applied: Vec<String>,
    /// 무시되는 점 경로 키(`USER_ONLY`).
    pub ignored: Vec<String>,
    /// 바뀐 줄(`Changed`일 때만). 신뢰할 때 내용을 모르므로 줄 번호와 새 줄만 보인다.
    /// 옛 내용을 두지 않아 비교할 수 없으니 빈 줄과 주석을 뺀 모든 줄을 보인다(초안).
    pub changed_lines: Vec<(usize, String)>,
}

/// 신뢰 기록. 경로마다 마지막으로 신뢰한 지문 하나.
#[derive(Debug, Default)]
pub struct TrustStore {
    path: PathBuf,
    entries: Vec<(PathBuf, String)>,
}

impl TrustStore {
    /// 신뢰 기록 파일을 읽는다. 없으면 빈 기록.
    ///
    /// # Errors
    /// 읽기 실패면 `Io`, 형식이 깨졌으면 `Parse`.
    pub async fn load(saturn_home: &Path) -> Result<Self, SettingsError> {
        let path = saturn_home.join(TRUST_FILE);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    entries: Vec::new(),
                });
            }
            Err(source) => return Err(SettingsError::Io { path, source }),
        };
        let map: BTreeMap<PathBuf, String> =
            serde_json::from_str(&content).map_err(|error| SettingsError::Parse {
                path: path.clone(),
                line: error.line(),
                message: error.to_string(),
            })?;
        Ok(Self {
            path,
            entries: map.into_iter().collect(),
        })
    }

    /// 폴더 설정 `path`의 지금 내용 `content`로 상태를 판정한다. 경로는 정규화(심볼릭 링크 해제)해 비교한다.
    pub fn status(&self, path: &Path, content: &str) -> TrustStatus {
        let path = canonical(path);
        let current = fingerprint(content);
        let known = self
            .entries
            .iter()
            .find(|(trusted, _)| *trusted == path)
            .map(|(_, fingerprint)| fingerprint);
        match known {
            Some(trusted) if *trusted == current => TrustStatus::Trusted,
            Some(_) => {
                let mut prompt = prompt(path, current, content);
                prompt.changed_lines = content
                    .lines()
                    .enumerate()
                    .filter(|(_, line)| {
                        let line = line.trim();
                        !line.is_empty() && !line.starts_with('#')
                    })
                    .map(|(index, line)| (index + 1, line.to_owned()))
                    .collect();
                TrustStatus::Changed(prompt)
            }
            None => TrustStatus::Unknown(prompt(path, current, content)),
        }
    }

    /// 사용자가 `y`로 확정했을 때 경로와 지문을 기록하고 파일에 쓴다. 같은 경로의 옛 지문은 바꾼다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Io`.
    pub async fn trust(&mut self, path: &Path, fingerprint: &str) -> Result<(), SettingsError> {
        let path = canonical(path);
        match self
            .entries
            .iter_mut()
            .find(|(trusted, _)| *trusted == path)
        {
            Some(entry) => entry.1 = fingerprint.to_owned(),
            None => self.entries.push((path, fingerprint.to_owned())),
        }
        let map: BTreeMap<&PathBuf, &String> = self
            .entries
            .iter()
            .map(|(path, fingerprint)| (path, fingerprint))
            .collect();
        let body = serde_json::to_string_pretty(&map).expect("trust map should serialize");
        write_private(&self.path, body.as_bytes()).map_err(|source| SettingsError::Io {
            path: self.path.clone(),
            source,
        })
    }
}

/// 신뢰 창 내용. 적용되는 키와 무시되는 키를 나눈다. 문법 오류면 둘 다 비운다(병합에서 오류로 보인다).
fn prompt(path: PathBuf, fingerprint: String, content: &str) -> FolderTrustPrompt {
    let mut keys = Vec::new();
    if let Ok(values) = parse_toml(content, &path) {
        leaf_keys(&values, "", &mut keys);
    }
    let (ignored, applied) = keys.into_iter().partition(|key| is_user_only(key));
    FolderTrustPrompt {
        path,
        fingerprint,
        applied,
        ignored,
        changed_lines: Vec::new(),
    }
}

/// 같은 폴더의 임시 파일에 0600으로 쓴 뒤 이름을 바꿔 한 번에 갈아 끼운다.
pub(crate) fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = partial_path(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(TRUST_FILE_MODE)
        .open(&partial)?;
    file.write_all(body)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

/// 갈아 끼우기 전 쓰는 파일 `<이름>.partial`.
pub(crate) fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    path.with_file_name(name)
}

/// 심볼릭 링크를 푼 경로. 없는 경로면 그대로.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[tokio::test]
    async fn unknown_then_trusted_then_changed() {
        let home = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("config.toml");
        std::fs::write(&path, "x").unwrap();
        let content = "[judge]\nendpoint = \"https://x\"\n[judge.thresholds]\ninjection = 0.9\n";
        let mut store = TrustStore::load(home.path()).await.unwrap();

        let TrustStatus::Unknown(prompt) = store.status(&path, content) else {
            panic!("first sight should be unknown");
        };
        assert_eq!(prompt.applied, vec!["judge.thresholds.injection"]);
        assert_eq!(prompt.ignored, vec!["judge.endpoint"]);
        store.trust(&path, &prompt.fingerprint).await.unwrap();

        let reloaded = TrustStore::load(home.path()).await.unwrap();
        assert_eq!(reloaded.status(&path, content), TrustStatus::Trusted);
        let TrustStatus::Changed(prompt) = reloaded.status(&path, "# new\nx = 1\n") else {
            panic!("new content should need trust again");
        };
        assert_eq!(prompt.changed_lines, vec![(2, "x = 1".to_owned())]);
        let mode = std::fs::metadata(home.path().join(TRUST_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, TRUST_FILE_MODE);
    }

    #[tokio::test]
    async fn broken_trust_file_is_parse_error() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(TRUST_FILE), "{ not json").unwrap();

        let error = TrustStore::load(home.path()).await.unwrap_err();

        assert!(matches!(error, SettingsError::Parse { .. }));
    }
}
