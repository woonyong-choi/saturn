//! 입력 기록 `~/.saturn/history`. 기록 저장소 밖의 파일이고 TUI만 쓴다.
//!
//! 설계: docs/design/engine-lifecycle.md(파일 배치), docs/design/records.md.
//! 입력창 `↑`/`↓` 이동과 `Ctrl+R` 검색이 쓴다. judge 키 입력 창의 값은 절대 넣지 않는다.

use std::path::{Path, PathBuf};

/// 입력 기록 파일 오류.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// 파일을 읽지 못했다. 없는 파일은 오류가 아니라 빈 기록이다.
    #[error("failed to read input history: {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 파일에 덧붙이지 못했다.
    #[error("failed to append input history: {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 입력 기록과 이동 위치.
/// 파일 형식은 초안이다(설계에 없음).
/// TODO(#92): 값 미정, 초안 한 줄에 입력 하나, 줄바꿈은 `\n`, 역슬래시는 `\\`로 이스케이프, 오래된 것이 위
#[derive(Debug)]
pub struct InputHistory {
    path: PathBuf,
    entries: Vec<String>,
    cursor: Option<usize>,
}

impl InputHistory {
    /// 파일을 읽는다. 없으면 빈 기록.
    ///
    /// # Errors
    /// 파일이 있는데 읽지 못하면 `Read`.
    pub fn load(path: &Path) -> Result<Self, HistoryError> {
        todo!("#92")
    }

    /// 제출한 입력을 끝에 더하고 파일에 덧붙인다. 바로 앞 항목과 같으면 더하지 않는다. 이동 위치를 초기화한다.
    ///
    /// # Errors
    /// 파일에 쓰지 못하면 `Write`.
    pub fn push(&mut self, text: &str) -> Result<(), HistoryError> {
        todo!("#92")
    }

    /// `↑` 한 칸 이전 입력. 맨 앞이면 그대로 맨 앞.
    pub fn prev(&mut self) -> Option<&str> {
        todo!("#92")
    }

    /// `↓` 한 칸 다음 입력. 맨 끝을 지나면 `None`(빈 입력창으로).
    pub fn next(&mut self) -> Option<&str> {
        todo!("#92")
    }

    /// 이동 위치를 지운다. 입력창을 고치면 부른다.
    pub fn reset(&mut self) {
        todo!("#92")
    }

    /// `Ctrl+R` 검색. `query`를 포함하는 입력을 최근 것부터, `skip`개 건너 다음 것을 준다.
    pub fn search(&self, query: &str, skip: usize) -> Option<&str> {
        todo!("#92")
    }
}
