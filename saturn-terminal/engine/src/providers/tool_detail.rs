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

/// 감싸는 셸 프로그램 이름.
const WRAPPER_SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "fish"];

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

// cost: time O(c), heap O(c), stack O(1), alloc 1
// vars: c = 명령 글자 수
// basis: estimate
/// `/bin/zsh -lc '...'`처럼 셸이 명령을 감쌌으면 안쪽 명령을, 아니면 받은 글을 그대로 돌려준다.
pub(super) fn unwrap_shell(command: &str) -> String {
    let Some(words) = split_words(command) else {
        return command.to_owned();
    };
    let [program, rest @ ..] = words.as_slice() else {
        return command.to_owned();
    };
    let program = program.rsplit('/').next().unwrap_or_default();
    if !WRAPPER_SHELLS.contains(&program) {
        return command.to_owned();
    }
    let Some(flag) = rest
        .iter()
        .position(|word| word.starts_with('-') && !word.starts_with("--") && word.contains('c'))
    else {
        return command.to_owned();
    };
    rest.get(flag + 1)
        .cloned()
        .unwrap_or_else(|| command.to_owned())
}

// cost: time O(c), heap O(c), stack O(1), alloc 1
// vars: c = 명령 글자 수
// basis: estimate
/// 작은따옴표, 큰따옴표, 역슬래시만 푸는 낱말 나누기. 따옴표가 닫히지 않으면 `None`.
fn split_words(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut has_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                has_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        inner => word.push(inner),
                    }
                }
            }
            '"' => {
                has_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        inner => word.push(inner),
                    }
                }
            }
            '\\' => {
                has_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if has_word {
                    words.push(std::mem::take(&mut word));
                    has_word = false;
                }
            }
            c => {
                has_word = true;
                word.push(c);
            }
        }
    }
    if has_word {
        words.push(word);
    }
    Some(words)
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
    fn unwrap_shell_gives_the_inner_command_or_leaves_the_input() {
        let cases = [
            (
                "login shell wrapper",
                "/bin/zsh -lc 'python3 -m unittest tests.test_rules'",
                "python3 -m unittest tests.test_rules",
            ),
            ("double quoted wrapper", "bash -c \"echo hi\"", "echo hi"),
            (
                "escaped single quote",
                r"/bin/zsh -lc 'echo '\''hi'\'' there'",
                "echo 'hi' there",
            ),
            (
                "plain command",
                "python3 scripts/check.py",
                "python3 scripts/check.py",
            ),
            ("script argument", "zsh script.sh", "zsh script.sh"),
            (
                "unbalanced quote",
                "/bin/zsh -lc 'oops",
                "/bin/zsh -lc 'oops",
            ),
            ("empty", "", ""),
        ];
        for (name, command, expected) in cases {
            assert_eq!(unwrap_shell(command), expected, "{name}: {command}");
        }
    }

    #[test]
    fn line_change_counts_only_the_lines_that_differ() {
        let cases = [
            (
                "replaced lines count both sides",
                "mode=draft\nretries=1",
                "mode=harbor\nretries=tundra",
                (2, 2),
            ),
            (
                "common lines are not counted",
                "a\nb\nc",
                "a\nx\ny\nc",
                (2, 1),
            ),
            ("empty old counts only added", "", "a\nb", (2, 0)),
        ];
        for (name, old, new, expected) in cases {
            let change = line_change(old, new);
            assert_eq!((change.added, change.removed), expected, "{name}");
        }
    }
}
