//! 설치 원천: 로컬 폴더는 복사하고 git 저장소 주소는 engine이 내려받는다. 네트워크는 engine만 쓴다.
//! 설계: docs/design/extensions.md#설치

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::ExtensionError;

/// 원본을 복사할 때 받는 최대 크기. 홈 폴더 같은 엉뚱한 폴더를 통째로 담는 일을 막는 초안 값이다.
const MAX_COPY_BYTES: u64 = 64 * 1024 * 1024;

/// git 저장소를 내려받는 데 기다리는 최대 시간. 초안 값이다.
const CLONE_TIMEOUT: Duration = Duration::from_secs(60);

/// git 주소로 보는 앞부분. 이 밖의 글자는 로컬 경로로 본다.
const GIT_PREFIXES: [&str; 4] = ["https://", "ssh://", "file://", "git@"];

/// 설치 원천.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// 절대 경로 폴더.
    Folder(PathBuf),
    /// git 저장소 주소.
    Git(String),
}

impl Source {
    // cost: time O(s), heap O(s), stack O(1)
    // vars: s = 글자 수
    // basis: estimate
    /// 주소 앞부분이 git이면 저장소, 아니면 절대 경로 폴더로 읽는다.
    ///
    /// # Errors
    /// 비었거나 상대 경로면 `InvalidSource`.
    pub(crate) fn parse(text: &str) -> Result<Self, ExtensionError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ExtensionError::InvalidSource {
                reason: "source is empty",
            });
        }
        if GIT_PREFIXES.iter().any(|prefix| text.starts_with(prefix)) {
            return Ok(Self::Git(text.to_owned()));
        }
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err(ExtensionError::InvalidSource {
                reason: "source should be an absolute folder path or a git address",
            });
        }
        Ok(Self::Folder(path))
    }

    /// 확장 이름으로 쓸 글자: 폴더 이름, git이면 `.git`을 뗀 저장소 이름.
    ///
    /// # Errors
    /// 이름을 얻지 못하면 `InvalidSource`.
    pub(crate) fn default_name(&self) -> Result<String, ExtensionError> {
        let last = match self {
            Self::Folder(path) => path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned),
            Self::Git(url) => url
                .trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()
                .map(|name| name.strip_suffix(".git").unwrap_or(name).to_owned()),
        };
        last.filter(|name| !name.is_empty())
            .ok_or(ExtensionError::InvalidSource {
                reason: "source has no name",
            })
    }
}

/// 이름이 폴더 이름 한 칸으로 안전한지 본다. 경로를 벗어나는 글자를 막는다.
pub(crate) fn validate_name(name: &str) -> Result<(), ExtensionError> {
    let is_safe = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if is_safe {
        Ok(())
    } else {
        Err(ExtensionError::InvalidName {
            name: name.to_owned(),
        })
    }
}

// cost: time O(f + b), heap O(d), stack O(d), io f + b
// vars: f = 파일 수, b = 바이트 수, d = 폴더 깊이
// basis: estimate
/// `from` 폴더를 새 폴더 `to`로 복사한다. `.git`과 심볼릭 링크는 건너뛴다. 링크가 폴더 밖 파일을 끌어오지 않게
/// 하기 위해서다. `to`가 이미 있으면 실패한다.
///
/// # Errors
/// 읽거나 쓰지 못하면 `Read`, 크기가 `MAX_COPY_BYTES`를 넘으면 `TooLarge`, 확장 저장소가 `from` 안에 있으면
/// `InvalidSource`.
pub(crate) fn copy_folder(from: &Path, to: &Path, store: &Path) -> Result<(), ExtensionError> {
    let from = from.canonicalize().map_err(|source| read(from, source))?;
    if !from.is_dir() {
        return Err(ExtensionError::InvalidSource {
            reason: "source is not a folder",
        });
    }
    if store
        .canonicalize()
        .is_ok_and(|store| store.starts_with(&from))
    {
        return Err(ExtensionError::InvalidSource {
            reason: "source contains the extension store",
        });
    }
    std::fs::create_dir(to).map_err(|source| read(to, source))?;
    let mut copied = 0;
    copy_into(&from, to, &mut copied)
}

/// `to`는 이미 있는 폴더다. 설치할 때 검사한 원본을 어댑터가 주입용으로 복사할 때도 쓴다.
pub(crate) fn copy_into(from: &Path, to: &Path, copied: &mut u64) -> Result<(), ExtensionError> {
    let entries = std::fs::read_dir(from).map_err(|source| read(from, source))?;
    for entry in entries {
        let entry = entry.map_err(|source| read(from, source))?;
        let kind = entry.file_type().map_err(|source| read(from, source))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() && entry.file_name() != ".git" {
            std::fs::create_dir(&target).map_err(|source| read(&target, source))?;
            copy_into(&entry.path(), &target, copied)?;
        } else if kind.is_file() {
            let size = entry.metadata().map_err(|source| read(from, source))?.len();
            *copied += size;
            if *copied > MAX_COPY_BYTES {
                return Err(ExtensionError::TooLarge {
                    limit: MAX_COPY_BYTES,
                });
            }
            std::fs::copy(entry.path(), &target).map_err(|source| read(&target, source))?;
        }
    }
    Ok(())
}

/// `program`(기본 `git`)으로 git 저장소를 `to`로 얕게 내려받고 `.git`을 지운다. `to`는 없어야 한다. 자식 프로세스에는 `PATH`, `HOME`,
/// `SSH_AUTH_SOCK`만 준다. router 키가 자식에게 새지 않게 하고, 자격 증명 질문으로 멈추지 않게 한다.
///
/// # Errors
/// 실행하지 못하거나 실패하면 `Clone`, 시간이 넘으면 `CloneTimedOut`.
pub(crate) async fn clone_repository(
    program: &Path,
    url: &str,
    to: &Path,
) -> Result<(), ExtensionError> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(["clone", "--depth", "1", "--quiet", "--", url])
        .arg(to)
        .env_clear()
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for name in ["PATH", "HOME", "SSH_AUTH_SOCK"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = tokio::time::timeout(CLONE_TIMEOUT, command.output())
        .await
        .map_err(|_| ExtensionError::CloneTimedOut)?
        .map_err(|error| ExtensionError::Clone {
            detail: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ExtensionError::Clone {
            detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let git_dir = to.join(".git");
    if git_dir.exists() {
        std::fs::remove_dir_all(&git_dir).map_err(|source| read(&git_dir, source))?;
    }
    Ok(())
}

fn read(path: &Path, source: std::io::Error) -> ExtensionError {
    ExtensionError::Read {
        path: path.display().to_string(),
        source,
    }
}
