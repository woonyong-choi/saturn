//! 규칙 패턴 일치, 셸 명령 나누기, 경로 정리. 파일을 읽지 않는 글자 계산만 한다.
//! 설계: docs/design/permissions.md#권한-규칙

use std::path::{Component, Path, PathBuf};

/// 패턴에서 `*`는 아무 글자열, `\`는 다음 글자를 그대로 쓴다.
const WILDCARD: char = '*';

const ESCAPE: char = '\\';

// cost: time O(p·t), heap O(t), stack O(1), alloc 2
// vars: p = pattern 글자 수, t = text 글자 수
// basis: estimate
/// `pattern`이 `text` 전체와 일치하는지 본다.
pub(super) fn matches(pattern: &str, text: &str) -> bool {
    let pattern: Vec<Token> = tokens(pattern);
    let text: Vec<char> = text.chars().collect();
    if match_tokens(&pattern, &text) {
        return true;
    }
    let ends_with_args = matches!(
        pattern.as_slice(),
        [.., Token::Literal(' '), Token::Wildcard]
    );
    ends_with_args && match_tokens(&pattern[..pattern.len() - 2], &text)
}

// cost: time O(n), heap O(n), stack O(1), alloc 1
// vars: n = text 글자 수
// basis: estimate
/// `*`와 `\`를 문자 그대로 일치시키는 패턴으로 바꾼다.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if c == WILDCARD || c == ESCAPE {
            escaped.push(ESCAPE);
        }
        escaped.push(c);
    }
    escaped
}

// cost: time O(p), heap O(p), stack O(1), alloc 2
// vars: p = pattern 글자 수
// basis: estimate
/// 앞쪽 글자열이 정해진 부분. 첫 `*` 앞까지다.
pub fn literal_prefix(pattern: &str) -> String {
    tokens(pattern)
        .into_iter()
        .map_while(|token| match token {
            Token::Literal(c) => Some(c),
            Token::Wildcard => None,
        })
        .collect()
}

// cost: time O(p), heap O(p), stack O(1), alloc 2
// vars: p = pattern 글자 수
// basis: estimate
/// 경로 glob으로 옮긴 패턴. `*`는 `/`도 포함하므로 `**`가 된다. glob 문자(`*?[]{}`)나 `\`가 글자로 든 패턴은 같은 뜻으로
/// 옮길 수 없어 `None`이다.
pub(super) fn glob_form(pattern: &str) -> Option<String> {
    let mut glob = String::with_capacity(pattern.len());
    for token in tokens(pattern) {
        match token {
            Token::Wildcard if glob.ends_with("**") => {}
            Token::Wildcard => glob.push_str("**"),
            Token::Literal('*' | '?' | '[' | ']' | '{' | '}' | '\\') => return None,
            Token::Literal(c) => glob.push(c),
        }
    }
    Some(glob)
}

// cost: time O(p), heap O(p), stack O(1), alloc 1
// vars: p = pattern 글자 수
// basis: estimate
/// 패턴에 `*`가 하나도 없으면 참.
pub fn is_literal(pattern: &str) -> bool {
    !tokens(pattern).contains(&Token::Wildcard)
}

// cost: time O(p), heap O(p), stack O(1), alloc 3
// vars: p = pattern 글자 수
// basis: estimate
/// 패턴이 일치할 수 있는 글자열 중 `prefix`로 시작하는 것이 있는지 본다.
pub fn may_match_prefix(pattern: &str, prefix: &str) -> bool {
    let fixed = literal_prefix(pattern);
    if is_literal(pattern) {
        return fixed.starts_with(prefix);
    }
    fixed.starts_with(prefix) || prefix.starts_with(&fixed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Literal(char),
    Wildcard,
}

// cost: time O(p), heap O(p), stack O(1), alloc 1
// vars: p = pattern 글자 수
// basis: estimate
fn tokens(pattern: &str) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            ESCAPE => tokens.push(Token::Literal(chars.next().unwrap_or(ESCAPE))),
            WILDCARD => tokens.push(Token::Wildcard),
            other => tokens.push(Token::Literal(other)),
        }
    }
    tokens
}

// cost: time O(p·t), heap O(t), stack O(1), alloc 2
// vars: p = pattern.len(), t = text.len()
// basis: estimate
fn match_tokens(pattern: &[Token], text: &[char]) -> bool {
    let mut reachable = vec![false; text.len() + 1];
    reachable[0] = true;
    for token in pattern {
        let mut next = vec![false; text.len() + 1];
        match token {
            Token::Wildcard => {
                let mut open = false;
                for (index, slot) in next.iter_mut().enumerate() {
                    open |= reachable[index];
                    *slot = open;
                }
            }
            Token::Literal(expected) => {
                for (index, c) in text.iter().enumerate() {
                    next[index + 1] = reachable[index] && c == expected;
                }
            }
        }
        reachable = next;
    }
    reachable[text.len()]
}

/// 나눈 명령 조각과, 안쪽 명령을 정확히 읽지 못했는지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ShellParts {
    pub(super) parts: Vec<String>,
    /// 명령 치환이나 닫히지 않은 따옴표가 있어 조각이 실제 실행과 다를 수 있다.
    pub(super) is_opaque: bool,
    /// 따옴표 밖에 구분자, 리디렉션, 명령 치환이 없는 단순 명령 하나인지.
    pub(super) is_plain: bool,
}

// cost: time O(c), heap O(c), stack O(1), alloc c
// vars: c = command 글자 수
// basis: estimate
/// `&&`, `||`, `;`, `|`, `&`, 줄바꿈, 괄호, 역따옴표 밖의 따옴표 안이 아닌 자리에서 나눈다.
/// `<`, `>`는 나누지 않지만 `is_plain`을 거짓으로 만든다.
pub(super) fn split_shell(command: &str) -> ShellParts {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut is_opaque = false;
    let mut is_plain = true;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ESCAPE && quote != Some('\'') {
            current.push(c);
            current.extend(chars.next());
            continue;
        }
        if let Some(open) = quote {
            if c == open {
                quote = None;
            }
            is_opaque |= opens_substitution(c, chars.peek().copied());
            current.push(c);
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '`' | '(' | ')' | ';' | '|' | '&' | '\n' => {
                is_plain = false;
                is_opaque |= c == '`' || opens_substitution(c, chars.peek().copied());
                is_opaque |= current.ends_with('$') && c == '(';
                push_part(&mut parts, &mut current);
            }
            other => {
                is_plain &= other != '<' && other != '>';
                current.push(other);
            }
        }
    }
    push_part(&mut parts, &mut current);
    ShellParts {
        parts,
        is_opaque: is_opaque || quote.is_some(),
        is_plain: is_plain && quote.is_none(),
    }
}

/// 따옴표 밖에 구분자, 리디렉션, 명령 치환이 없는 단순 명령 하나인지. 파이프, `&&`, `||`, `;`가 앞 명령의 실패를 숨기는지
/// 판단하지 않고 모두 단순하지 않은 것으로 본다.
pub fn is_plain_shell(command: &str) -> bool {
    let split = split_shell(command);
    split.is_plain && !split.is_opaque && split.parts.len() == 1
}

fn opens_substitution(c: char, next: Option<char>) -> bool {
    (c == '$' && next == Some('(')) || c == '`'
}

fn push_part(parts: &mut Vec<String>, current: &mut String) {
    let part = current.trim();
    if !part.is_empty() {
        parts.push(part.to_owned());
    }
    current.clear();
}

// cost: time O(n), heap O(n), stack O(1), alloc 1
// vars: n = 경로 구성 요소 수
// basis: estimate
/// `.`과 `..`를 글자만으로 정리한다. 상대 경로는 `base` 아래로 본다. 링크는 따라가지 않는다.
pub(super) fn normalize(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_star_spans_any_text_including_separators() {
        assert!(matches("rm *", "rm -rf /tmp/a b"));
        assert!(matches("*.lock", "/work/Cargo.lock"));
        assert!(matches("a*c*e", "abcde"));
        assert!(!matches("a*c", "abd"));
        assert!(!matches("git status", "git status -s"));
    }

    #[test]
    fn matches_trailing_args_wildcard_also_matches_bare_command() {
        assert!(matches("git status *", "git status"));
        assert!(matches("git status *", "git status -s"));
        assert!(!matches("git status *", "git statusx"));
        assert!(!matches("git *", "gitk"));
    }

    #[test]
    fn matches_escaped_star_is_literal() {
        let pattern = escape("rm *.tmp");

        assert!(matches(&pattern, "rm *.tmp"));
        assert!(!matches(&pattern, "rm a.tmp"));
        assert!(matches(&escape(r"a\b"), r"a\b"));
    }

    #[test]
    fn prefix_helpers_read_the_fixed_part() {
        assert_eq!(literal_prefix("mcp__s__read_*"), "mcp__s__read_");
        assert!(is_literal("mcp__s__echo"));
        assert!(!is_literal("mcp__*"));
        assert!(may_match_prefix("mcp__*", "mcp__s__"));
        assert!(may_match_prefix("mcp__s__echo", "mcp__s__"));
        assert!(!may_match_prefix("mcp__t__echo", "mcp__s__"));
        assert!(!may_match_prefix("mcp__t__*", "mcp__s__"));
    }

    #[test]
    fn split_shell_cuts_on_every_separator_outside_quotes() {
        let split = split_shell("git status && rm -rf x; ls | wc -l\nmake &");

        assert_eq!(
            split.parts,
            vec!["git status", "rm -rf x", "ls", "wc -l", "make"]
        );
        assert!(!split.is_opaque);
    }

    #[test]
    fn split_shell_keeps_quoted_separators() {
        let split = split_shell(r#"echo "a; b" 'c && d' e\;f"#);

        assert_eq!(split.parts, vec![r#"echo "a; b" 'c && d' e\;f"#]);
        assert!(!split.is_opaque);
    }

    #[test]
    fn split_shell_marks_substitution_and_open_quote_as_opaque() {
        assert!(split_shell("echo $(rm x)").is_opaque);
        assert!(split_shell("echo `rm x`").is_opaque);
        assert!(split_shell(r#"echo "$(rm x)""#).is_opaque);
        assert!(split_shell("echo 'open").is_opaque);
        assert_eq!(split_shell("echo $(rm x)").parts, vec!["echo $", "rm x"]);
    }

    #[test]
    fn split_shell_plain_only_without_syntax_outside_quotes() {
        assert!(split_shell(r#"ls -la "a|b>c""#).is_plain);
        assert!(!split_shell("ls > out").is_plain);
        assert!(!split_shell("cat < in").is_plain);
        assert!(!split_shell("ls;").is_plain);
        assert!(!split_shell("ls &&").is_plain);
        assert!(!split_shell("echo $(x)").is_plain);
    }

    #[test]
    fn split_shell_empty_command_has_no_parts() {
        assert!(split_shell("  ").parts.is_empty());
    }

    #[test]
    fn normalize_resolves_dots_without_touching_the_disk() {
        let base = Path::new("/work");

        assert_eq!(
            normalize(base, Path::new("src/../a/./b.rs")),
            PathBuf::from("/work/a/b.rs")
        );
        assert_eq!(
            normalize(base, Path::new("/work/../etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
    }
}
