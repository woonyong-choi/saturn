//! 입력 기록 `~/.saturn/history`. 기록 저장소 밖의 파일이고 TUI만 쓴다.
//! 설계: docs/design/records.md

use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    #[error("failed to read input history: {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to append input history: {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// router 키 입력 창의 값은 절대 넣지 않는다.
#[derive(Debug)]
pub(crate) struct InputHistory {
    path: PathBuf,
    entries: Vec<String>,
    cursor: Option<usize>,
}

impl InputHistory {
    // cost: time O(f), heap O(f), stack O(1), io 1
    // vars: f = 기록 파일 바이트 수
    // basis: estimate
    /// 파일이 없으면 빈 기록.
    ///
    /// # Errors
    /// 파일이 있는데 읽지 못하면 `Read`.
    pub(crate) fn load(path: &Path) -> Result<Self, HistoryError> {
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
    /// 바로 앞 항목과 같으면 더하지 않는다.
    ///
    /// # Errors
    /// 파일에 쓰지 못하면 `Write`.
    pub(crate) fn push(&mut self, text: &str) -> Result<(), HistoryError> {
        self.cursor = None;
        if self.entries.last().map(String::as_str) == Some(text) {
            return Ok(());
        }
        self.append(text).map_err(|source| HistoryError::Write {
            path: self.path.clone(),
            source,
        })?;
        self.entries.push(text.to_string());
        Ok(())
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 맨 앞이면 맨 앞에 머문다.
    pub(crate) fn older(&mut self) -> Option<&str> {
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
    /// 맨 끝을 지나면 `None`.
    pub(crate) fn newer(&mut self) -> Option<&str> {
        let index = self.cursor? + 1;
        if index >= self.entries.len() {
            self.cursor = None;
            return None;
        }
        self.cursor = Some(index);
        self.entries.get(index).map(String::as_str)
    }

    pub(crate) fn reset(&mut self) {
        self.cursor = None;
    }

    // cost: time O(e·q), heap O(1), stack O(1)
    // vars: e = 기록 글자 수 합, q = query.len()
    // basis: estimate
    /// `query`를 포함하는 입력을 최근 것부터 `skip`개 건너 고른다.
    pub(crate) fn search(&self, query: &str, skip: usize) -> Option<&str> {
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
    /// 테스트용. `push`는 파일 쓰기가 실패할 수 있어 쓰지 않는다.
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
    /// 폴더가 없으면 만든다.
    fn append(&self, text: &str) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&self.path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        writeln!(file, "{}", escape(text))
    }
}

// cost: time O(n), heap O(n), stack O(1), alloc 1
// vars: n = text.len()
// basis: estimate
/// 한 줄에 입력 하나인 초안 형식으로, 역슬래시는 `\\`, 줄바꿈은 `\n`으로 바꾼다.
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
/// 알 수 없는 이스케이프는 그대로 둔다.
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

    #[test]
    fn failed_append_does_not_add_history_entry() {
        let mut history = InputHistory {
            path: PathBuf::from("/dev/null/history"),
            entries: Vec::new(),
            cursor: None,
        };

        assert!(history.push("unsaved").is_err());
        assert_eq!(history.older(), None);
    }

    #[test]
    fn history_file_is_private() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".saturn-history-test-{}", std::process::id()));
        let mut history = InputHistory::load(&path).unwrap();

        history.push("sample").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        std::fs::remove_file(&path).unwrap();

        assert_eq!(mode, 0o600);
    }
}
