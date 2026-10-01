//! provider와 무관한 도구 값 계산: 명령의 도구 종류, 줄 수 변화.
//! 설계: docs/design/context-selection.md#도구-결과-메모

use saturn_protocol::event::{LineChange, ToolCategory};

/// 명령 앞뒤 따옴표를 뗀 낱말이 이 짝과 이어서 나오면 테스트 실행이다.
const TEST_COMMANDS: &[&[&str]] = &[
    &["unittest"],
    &["pytest"],
    &["py.test"],
    &["jest"],
    &["vitest"],
    &["cargo", "test"],
    &["cargo", "nextest"],
    &["go", "test"],
    &["npm", "test"],
    &["npm", "run", "test"],
    &["yarn", "test"],
    &["pnpm", "test"],
    &["mvn", "test"],
    &["gradle", "test"],
    &["make", "test"],
];

// cost: time O(c·t), heap O(c), stack O(1), alloc 1
// vars: c = 명령 글자 수, t = TEST_COMMANDS 항목 수
// basis: estimate
/// 테스트 실행 명령이면 `TestRun`, 아니면 `Shell`.
pub(super) fn classify_command(command: &str) -> ToolCategory {
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|'))
        .map(|word| word.trim_matches(['\'', '"']))
        .filter(|word| !word.is_empty())
        .collect();
    let is_test = TEST_COMMANDS.iter().any(|pattern| {
        words
            .windows(pattern.len())
            .any(|window| window == *pattern)
    });
    if is_test {
        ToolCategory::TestRun
    } else {
        ToolCategory::Shell
    }
}

// cost: time O(o + n), heap O(o + n), stack O(1), alloc 2
// vars: o = old 줄 수, n = new 줄 수
// basis: estimate
/// 앞뒤에서 같은 줄을 뺀 나머지를 지운 줄과 더한 줄로 센다.
pub(super) fn line_change(old: &str, new: &str) -> LineChange {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    LineChange {
        added: count(new.len() - prefix - suffix),
        removed: count(old.len() - prefix - suffix),
    }
}

/// 줄 수는 `u32`를 넘지 않는다고 본다.
fn count(lines: usize) -> u32 {
    u32::try_from(lines).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_command_test_runners_are_test_runs() {
        for command in [
            "python3 -m unittest tests.test_rules",
            "/bin/zsh -lc 'python3 -m unittest tests.test_rules'",
            "cd app && cargo test --workspace",
            "npm run test",
            "pytest -q",
        ] {
            assert_eq!(
                classify_command(command),
                ToolCategory::TestRun,
                "{command}"
            );
        }
    }

    #[test]
    fn classify_command_other_commands_are_shell() {
        for command in ["python3 scripts/check.py", "cat tests/test_rules.py", "ls"] {
            assert_eq!(classify_command(command), ToolCategory::Shell, "{command}");
        }
    }

    #[test]
    fn line_change_replaced_lines_count_both_sides() {
        let change = line_change("mode=draft\nretries=1", "mode=harbor\nretries=tundra");

        assert_eq!((change.added, change.removed), (2, 2));
    }

    #[test]
    fn line_change_common_lines_are_not_counted() {
        let change = line_change("a\nb\nc", "a\nx\ny\nc");

        assert_eq!((change.added, change.removed), (2, 1));
    }

    #[test]
    fn line_change_empty_old_counts_only_added() {
        let change = line_change("", "a\nb");

        assert_eq!((change.added, change.removed), (2, 0));
    }
}
