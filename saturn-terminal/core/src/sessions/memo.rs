//! 도구 결과 축약본의 한 줄 메모.
//! 설계: docs/design/context-selection.md#도구-결과-메모

/// engine의 `providers`가 provider 도구 이름을 이 종류로 바꾼다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolKind {
    Shell {
        command: String,
        /// 신호로 끝나 코드가 없으면 `None`.
        exit_code: Option<i32>,
    },
    TestRun {
        passed: u32,
        failed: u32,
        failed_names: Vec<String>,
    },
    FileRead {
        path: String,
        /// 1부터 세는 닫힌 범위. 전체를 읽었으면 `None`.
        lines: Option<(u32, u32)>,
    },
    FileEdit {
        path: String,
        added: u32,
        removed: u32,
    },
    WebFetch {
        url: String,
        status: Option<u16>,
    },
    Other,
}

/// 결과를 기록하기 전에 끊긴 도구 실행의 결과 자리에 넣는 오류 글. 일부 실행됐을 수 있다.
pub const INTERRUPTED_RESULT: &str =
    "Interrupted before a result was recorded · It may have partially run";

const SEPARATOR: &str = " · ";
const ERROR_WORDS: [&str; 3] = ["error", "failed", "panic"];

// cost: time O(r), heap O(r), stack O(1)
// vars: r = 결과 글자 수
// basis: estimate
/// 같은 종류와 결과에서 늘 같은 메모를 만들고, `Other`는 빈 메모다.
pub fn tool_memo(kind: &ToolKind, result: &str) -> String {
    match kind {
        ToolKind::Shell { command, exit_code } => shell_memo(command, *exit_code, result),
        ToolKind::TestRun {
            passed,
            failed,
            failed_names,
        } => {
            let mut memo = format!("passed {passed}, failed {failed}");
            if !failed_names.is_empty() {
                memo.push_str(": ");
                memo.push_str(&failed_names.join(", "));
            }
            memo
        }
        ToolKind::FileRead { path, lines } => match lines {
            Some((first, last)) => format!("{path}{SEPARATOR}lines {first}-{last}"),
            None => format!("{path}{SEPARATOR}all lines"),
        },
        ToolKind::FileEdit {
            path,
            added,
            removed,
        } => format!("{path}{SEPARATOR}+{added} -{removed}"),
        ToolKind::WebFetch { url, status } => match status {
            Some(code) => format!("{url}{SEPARATOR}{code}"),
            None => format!("{url}{SEPARATOR}no status"),
        },
        ToolKind::Other => String::new(),
    }
}

// cost: time O(r), heap O(r), stack O(1)
// vars: r = 결과 글자 수
// basis: estimate
fn shell_memo(command: &str, exit_code: Option<i32>, result: &str) -> String {
    let command = command.lines().next().unwrap_or_default();
    let exit = exit_code.map_or_else(|| "no exit code".to_string(), |code| format!("exit {code}"));
    let mut memo = format!(
        "{command}{SEPARATOR}{exit}{SEPARATOR}{} lines",
        result.lines().count()
    );
    if let Some(line) = result.lines().find(|line| has_error_word(line)) {
        memo.push_str(SEPARATOR);
        memo.push_str(line.trim());
    }
    memo
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 줄 글자 수
// basis: estimate
fn has_error_word(line: &str) -> bool {
    let lower = line.to_lowercase();
    ERROR_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(exit_code: Option<i32>) -> ToolKind {
        ToolKind::Shell {
            command: "cargo build".into(),
            exit_code,
        }
    }

    #[test]
    fn tool_memo_summarizes_each_tool_kind() {
        // (사례, 도구 종류, 결과 원문, 예상 메모)
        let cases = [
            (
                "shell lists command, exit, lines and first error",
                shell(Some(101)),
                "Compiling a\nerror[E0425]: cannot find value\nerror: aborting",
                "cargo build · exit 101 · 3 lines · error[E0425]: cannot find value",
            ),
            (
                "shell without error line or code",
                shell(None),
                "ok",
                "cargo build · no exit code · 1 lines",
            ),
            (
                "test run lists counts and failed names",
                ToolKind::TestRun {
                    passed: 10,
                    failed: 2,
                    failed_names: vec!["login_ok".into(), "logout_ok".into()],
                },
                "",
                "passed 10, failed 2: login_ok, logout_ok",
            ),
            (
                "file read",
                ToolKind::FileRead {
                    path: "src/a.rs".into(),
                    lines: Some((1, 40)),
                },
                "",
                "src/a.rs · lines 1-40",
            ),
            (
                "file edit",
                ToolKind::FileEdit {
                    path: "src/a.rs".into(),
                    added: 3,
                    removed: 1,
                },
                "",
                "src/a.rs · +3 -1",
            ),
            (
                "web fetch",
                ToolKind::WebFetch {
                    url: "https://example.com".into(),
                    status: Some(200),
                },
                "",
                "https://example.com · 200",
            ),
            ("other is empty", ToolKind::Other, "anything", ""),
        ];

        for (name, kind, result, expected) in cases {
            assert_eq!(tool_memo(&kind, result), expected, "{name}");
        }
    }

    #[test]
    fn tool_memo_same_result_gives_same_memo() {
        let result = "line\npanic at src/main.rs\nmore";

        assert_eq!(
            tool_memo(&shell(Some(1)), result),
            tool_memo(&shell(Some(1)), result)
        );
    }
}
