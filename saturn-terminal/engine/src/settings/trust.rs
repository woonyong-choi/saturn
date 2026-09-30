//! 폴더 설정 신뢰: 경로와 지문으로 기록하고, 처음 보거나 내용이 바뀐 폴더 설정은 한 번 묻는다.
//!
//! 설계: docs/design/settings.md(폴더 설정 신뢰). 신뢰 창(TUI)의 키: `1`·`y` 적용하고 계속, `↑`·`↓` 이동, `Enter` 확정,
//! `3`·`q`·`Esc`·`Ctrl+C` 종료. 실행 중 폴더 설정이 바뀌면 다음 입력을 접수하기 전에 창을 연다.
//! TODO(#83): 신뢰 기록 위치(설계에 없음). 초안은 `~/.saturn/trusted.json`(경로 → 지문), 권한 0600

use std::path::{Path, PathBuf};

use super::SettingsError;

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
    /// 지금 내용의 지문(SHA-256 hex).
    pub fingerprint: String,
    /// 적용되는 점 경로 키.
    pub applied: Vec<String>,
    /// 무시되는 점 경로 키(`USER_ONLY`).
    pub ignored: Vec<String>,
    /// 바뀐 줄(`Changed`일 때만). 신뢰할 때 내용을 모르므로 줄 번호와 새 줄만 보인다.
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
        todo!("#83")
    }

    /// 폴더 설정 `path`의 지금 내용 `content`로 상태를 판정한다. 경로는 정규화(심볼릭 링크 해제)해 비교한다.
    pub fn status(&self, path: &Path, content: &str) -> TrustStatus {
        todo!("#83")
    }

    /// 사용자가 `y`로 확정했을 때 경로와 지문을 기록하고 파일에 쓴다. 같은 경로의 옛 지문은 바꾼다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Io`.
    pub async fn trust(&mut self, path: &Path, fingerprint: &str) -> Result<(), SettingsError> {
        todo!("#83")
    }
}
