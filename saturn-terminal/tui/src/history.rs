//! 입력 기록 `~/.saturn/history`. 기록 저장소 밖의 파일이고 TUI만 쓴다.
//!
//! 설계: docs/design/engine-lifecycle.md(파일 배치), docs/design/records.md.
//! 입력창 `↑`/`↓` 이동과 `Ctrl+R` 검색이 쓴다. judge 키 입력 창의 값은 절대 넣지 않는다.

use std::io::Write;
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
/// 파일 형식은 초안이다(설계에 없음, docs/design/tui.md 초안 값): 한 줄에 입력 하나, 줄바꿈은 `\n`,
/// 역슬래시는 `\\`로 이스케이프, 오래된 것이 위.
#[derive(Debug)]
pub struct InputHistory {
    path: PathBuf,
    entries: Vec<String>,
    cursor: Option<usize>,
}

impl InputHistory {
    // cost: time O(f), heap O(f), stack O(1), io 1
    // vars: f = 기록 파일 바이트 수
    // basis: estimate
    /// 파일을 읽는다. 없으면 빈 기록.
    ///
    /// # Errors
    /// 파일이 있는데 읽지 못하면 `Read`.
    pub fn load(path: &Path) -> Result<Self, HistoryError> {
        let entries = match std::fs::read_to_string(path) {
            Ok(content) => content.lines().map(unescape).collect(),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(source) => {
                return Err(HistoryError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        Ok(Self {
            path: path.to_path_buf(),
            entries,
            cursor: None,
        })
    }

    // cost: time O(n), heap O(n), stack O(1), io 1
    // vars: n = text.len()
    // basis: estimate
    /// 제출한 입력을 끝에 더하고 파일에 덧붙인다. 바로 앞 항목과 같으면 더하지 않는다. 이동 위치를 초기화한다.
    ///
    /// # Errors
    /// 파일에 쓰지 못하면 `Write`.
    pub fn push(&mut self, text: &str) -> Result<(), HistoryError> {
        self.cursor = None;
        if self.entries.last().map(String::as_str) == Some(text) {
            return Ok(());
        }
        self.entries.push(text.to_string());
        self.append(text).map_err(|source| HistoryError::Write {
            path: self.path.clone(),
            source,
        })
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `↑` 한 칸 이전 입력. 맨 앞이면 그대로 맨 앞.
    pub fn older(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        let index = match self.cursor {
            None => self.entries.len() - 1,
            Some(index) => index.saturating_sub(1),
        };
        self.cursor = Some(index);
        self.entries.get(index).map(String::as_str)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `↓` 한 칸 다음 입력. 맨 끝을 지나면 `None`(빈 입력창으로).
    pub fn newer(&mut self) -> Option<&str> {
        let index = self.cursor? + 1;
        if index >= self.entries.len() {
            self.cursor = None;
            return None;
        }
        self.cursor = Some(index);
        self.entries.get(index).map(String::as_str)
    }

    /// 이동 위치를 지운다. 입력창을 고치면 부른다.
    pub fn reset(&mut self) {
        self.cursor = None;
    }

    // cost: time O(e·q), heap O(1), stack O(1)
    // vars: e = 기록 글자 수 합, q = query.len()
    // basis: estimate
    /// `Ctrl+R` 검색. `query`를 포함하는 입력을 최근 것부터, `skip`개 건너 다음 것을 준다.
    pub fn search(&self, query: &str, skip: usize) -> Option<&str> {
        if query.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .filter(|entry| entry.contains(query))
            .nth(skip)
            .map(String::as_str)
    }

    // cost: time O(e), heap O(e), stack O(1)
    // vars: e = 기록 글자 수 합
    // basis: estimate
    /// 테스트용: 파일 없이 기록을 채운다. `push`는 쓰지 않는 경로에 덧붙이려다 실패할 수 있다.
    #[cfg(test)]
    pub(crate) fn with_entries(entries: &[&str]) -> Self {
        Self {
            path: PathBuf::from("/nonexistent/saturn-history"),
            entries: entries.iter().map(|e| e.to_string()).collect(),
            cursor: None,
        }
    }

    // cost: time O(n), heap O(n), stack O(1), io 3
    // vars: n = text.len()
    // basis: estimate
    /// 한 줄을 파일 끝에 덧붙인다. 폴더가 없으면 만든다.
    fn append(&self, text: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", escape(text))
    }
}

// cost: time O(n), heap O(n), stack O(1), alloc 1
// vars: n = text.len()
// basis: estimate
/// 파일 한 줄로 바꾼다. 역슬래시는 `\\`, 줄바꿈은 `\n`.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

// cost: time O(n), heap O(n), stack O(1), alloc 1
// vars: n = line.len()
// basis: estimate
/// `escape`의 반대. 알 수 없는 이스케이프는 그대로 둔다.
fn unescape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn history(entries: &[&str]) -> InputHistory {
        InputHistory {
            path: PathBuf::from("unused"),
            entries: entries.iter().map(|e| e.to_string()).collect(),
            cursor: None,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn escape_round_trips_newlines_and_backslashes() {
        let text = "첫 줄\n둘째 \\n 줄";

        assert_eq!(unescape(&escape(text)), text);
        assert!(!escape(text).contains('\n'));
    }

    #[test]
    fn prev_walks_back_and_stops_at_oldest() {
        let mut history = history(&["a", "b"]);

        assert_eq!(history.older(), Some("b"));
        assert_eq!(history.older(), Some("a"));
        assert_eq!(history.older(), Some("a"));
    }

    #[test]
    fn next_past_newest_returns_none() {
        let mut history = history(&["a", "b"]);
        history.older();
        history.older();

        assert_eq!(history.newer(), Some("b"));
        assert_eq!(history.newer(), None);
    }

    #[test]
    fn prev_empty_returns_none() {
        assert_eq!(history(&[]).older(), None);
    }

    #[test]
    fn search_returns_recent_matches_with_skip() {
        let history = history(&["build app", "test", "build lib"]);

        assert_eq!(history.search("build", 0), Some("build lib"));
        assert_eq!(history.search("build", 1), Some("build app"));
        assert_eq!(history.search("build", 2), None);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn load_missing_file_returns_empty_history() {
        let path = std::env::temp_dir().join("saturn-tui-history-missing-file");

        let mut history = InputHistory::load(&path).unwrap();

        assert_eq!(history.older(), None);
    }
}
