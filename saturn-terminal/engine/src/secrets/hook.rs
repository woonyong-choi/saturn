//! Saturn 소유 PreToolUse 훅: 에이전트의 키 저장소 조회와 Saturn 비밀 파일 접근을 막는다. 명령 이름과 차단 목록은 초안이다.
//! 설계: docs/design/router-key-security.md

use std::path::{Path, PathBuf};

use serde_json::json;

use self::shell_words::{Segment, tokenize};
use super::storage::KEY_FILE;

mod shell_words;

/// 초안 목록.
const BLOCKED_SECURITY_SUBCOMMANDS: &[&str] = &[
    "find-generic-password",
    "find-internet-password",
    "dump-keychain",
    "export",
];

/// 건너뛰고 다음 낱말을 본다.
const COMMAND_WRAPPERS: &[&str] = &[
    "sudo",
    "doas",
    "env",
    "command",
    "builtin",
    "exec",
    "nohup",
    "time",
    "xargs",
    "timeout",
    "nice",
    "setsid",
    "arch",
    "caffeinate",
    "{",
    "!",
    "if",
    "then",
    "elif",
    "else",
    "while",
    "until",
    "do",
];

/// `-c` 인자와 표준 입력을 같은 판정기로 다시 본다.
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "ash", "csh", "tcsh",
];

/// 이름이 이 낱말로 시작하는 실행 파일(`python3.12`, `nodejs`)도 포함한다.
const INTERPRETER_PREFIXES: &[&str] = &["python", "perl", "ruby", "node", "php", "lua"];

const INTERPRETERS: &[&str] = &["osascript", "deno", "bun", "swift", "awk", "gawk", "mawk"];

/// 셸 인자 안의 셸 인자를 따라가는 깊이. 넘으면 막는다.
const MAX_NESTING: usize = 4;

const COMMAND_SEPARATORS: &[&str] = &["&&", "||", ";", "|", "$(", "`", "(", ")", "\n", "&"];

/// 에이전트에 그대로 보이므로 경로와 키 값을 넣지 않는다.
const DENY_REASON: &str = "blocked by saturn: router key storage is not available to agents";

/// provider 고유 형식은 `providers/claude`가 이것으로 바꿔 넘긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolCall {
    Command(String),
    /// 절대 경로로 바꾼 값.
    Path(PathBuf),
    /// 판정하지 않고 허용한다.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookVerdict {
    /// 사용자의 다른 훅이 이어서 판정한다.
    Allow,
    Deny {
        /// 에이전트에 그대로 보이는 한 줄이라 키 경로나 키 값을 넣지 않는다.
        reason: String,
    },
}

/// engine 시작 때 한 번 만든다.
#[derive(Debug, Clone)]
pub(crate) struct HookPolicy {
    /// 훅 명령에 `--home`으로 넘겨 훅 프로세스가 같은 키 파일을 막게 한다.
    saturn_home: PathBuf,
    /// 셸 연결로 나뉜 각 부분을 따로 본다.
    blocked_commands: Vec<String>,
    /// 심볼릭 링크를 푼 뒤 비교한다.
    blocked_paths: Vec<PathBuf>,
    /// 명령 속 `~`, `$HOME`을 풀 때 쓴다.
    user_home: PathBuf,
}

impl HookPolicy {
    pub(crate) fn new(saturn_home: &Path, user_home: &Path) -> Self {
        let blocked_commands = BLOCKED_SECURITY_SUBCOMMANDS
            .iter()
            .map(|sub| format!("security {sub}"))
            .collect();
        let blocked_paths = [
            saturn_home.join(KEY_FILE),
            user_home.join("Library").join("Keychains"),
            PathBuf::from("/Library/Keychains"),
        ]
        .into_iter()
        .flat_map(|path| [resolve(&path), path])
        .collect();
        Self {
            saturn_home: saturn_home.to_path_buf(),
            blocked_commands,
            blocked_paths,
            user_home: user_home.to_path_buf(),
        }
    }

    pub(crate) fn check(&self, call: &ToolCall) -> HookVerdict {
        let blocked = match call {
            ToolCall::Command(command) => self.command_blocked(command),
            ToolCall::Path(path) => self.path_blocked(path),
            ToolCall::Other => false,
        };
        if blocked {
            HookVerdict::Deny {
                reason: DENY_REASON.to_owned(),
            }
        } else {
            HookVerdict::Allow
        }
    }

    /// 키 저장소 경로(링크를 푼 경로 포함, 중복 없음). provider 명령 샌드박스의 읽기 금지 목록에 쓴다.
    pub(crate) fn key_store_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        for path in &self.blocked_paths {
            // 링크를 푼 경로는 끝에 `/`가 붙을 수 있어 구성 요소로 다시 이어 없앤다
            let path: PathBuf = path.components().collect();
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        paths
    }

    /// 사용자 훅을 읽거나 바꾸지 않고, 사용자 설정과 합치는 일은 Claude가 한다.
    pub(crate) fn pre_tool_use_settings(&self, saturn_bin: &Path) -> serde_json::Value {
        let command = format!(
            "{} hook pre-tool-use --home {}",
            shell_quote(&saturn_bin.to_string_lossy()),
            shell_quote(&self.saturn_home.to_string_lossy())
        );
        json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "*",
                    "hooks": [{ "type": "command", "command": command }],
                }],
            },
        })
    }

    fn command_blocked(&self, command: &str) -> bool {
        self.split_command_blocked(command) || self.command_blocked_at(command, 0)
    }

    /// 따옴표를 무시하고 연결 기호로 먼저 쪼개 보는 넓은 검사. 따옴표 안의 `$(`와 백틱도 잡는다.
    fn split_command_blocked(&self, command: &str) -> bool {
        let mut parts = vec![command.to_owned()];
        for separator in COMMAND_SEPARATORS {
            parts = parts
                .iter()
                .flat_map(|part| part.split(separator).map(str::to_owned).collect::<Vec<_>>())
                .collect();
        }
        parts.iter().any(|part| {
            let words: Vec<String> = part
                .split_whitespace()
                .map(|word| word.trim_matches(|c| c == '\'' || c == '"').to_owned())
                .collect();
            self.runs_blocked_command(&words)
                || words
                    .iter()
                    .any(|word| self.path_blocked(&self.expand_home(word)))
        })
    }

    fn runs_blocked_command(&self, words: &[String]) -> bool {
        let mut rest = words
            .iter()
            .skip_while(|word| word.contains('=') || COMMAND_WRAPPERS.contains(&word.as_str()));
        let Some(program) = rest.next() else {
            return false;
        };
        let program = Path::new(program)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sub = rest.find(|word| !word.starts_with('-'));
        let Some(sub) = sub else {
            return false;
        };
        let line = format!("{program} {sub}");
        self.blocked_commands.contains(&line)
    }

    /// 따옴표와 이스케이프를 풀어 낱말로 나눈 뒤 셸 `-c`, `eval`, 표준 입력 셸, 인터프리터 문자열까지 본다.
    /// 해석할 수 없거나 깊이 상한을 넘으면 막는다.
    fn command_blocked_at(&self, command: &str, depth: usize) -> bool {
        if depth > MAX_NESTING {
            return true;
        }
        let Some(segments) = tokenize(command) else {
            return true;
        };
        segments.iter().enumerate().any(|(index, segment)| {
            let previous = index.checked_sub(1).map(|prev| &segments[prev]);
            self.segment_blocked(segment, previous, depth)
        })
    }

    fn segment_blocked(&self, segment: &Segment, previous: Option<&Segment>, depth: usize) -> bool {
        let words = &segment.words;
        if words
            .iter()
            .any(|word| self.path_blocked(&self.expand_home(word)))
        {
            return true;
        }
        // 큰따옴표 안의 `$(...)`와 백틱은 낱말 안에 남으므로 다시 푼다.
        if words.iter().any(|word| {
            (word.contains("$(") || word.contains('`')) && self.command_blocked_at(word, depth + 1)
        }) {
            return true;
        }
        command_starts(words).into_iter().any(|start| {
            let program = base_name(&words[start]);
            let args = &words[start + 1..];
            if program == "security" {
                return self.security_blocked(args);
            }
            if program == "eval" {
                return self.command_blocked_at(&args.join(" "), depth + 1);
            }
            if SHELLS.contains(&program.as_str()) {
                return self.shell_blocked(segment, previous, args, depth);
            }
            if is_interpreter(&program) {
                return args
                    .iter()
                    .chain(segment.heredoc.iter())
                    .any(|text| self.text_reaches_key_store(text));
            }
            false
        })
    }

    fn security_blocked(&self, args: &[String]) -> bool {
        if args.iter().any(|arg| arg == "-i" || arg == "--interactive") {
            return true;
        }
        args.iter()
            .find(|arg| !arg.starts_with('-'))
            .is_some_and(|sub| BLOCKED_SECURITY_SUBCOMMANDS.contains(&sub.as_str()))
    }

    fn shell_blocked(
        &self,
        segment: &Segment,
        previous: Option<&Segment>,
        args: &[String],
        depth: usize,
    ) -> bool {
        let command_flag = args.iter().position(|arg| {
            arg == "--command"
                || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('c'))
        });
        if let Some(flag) = command_flag {
            return args[flag + 1..]
                .iter()
                .find(|arg| !arg.starts_with('-'))
                .is_some_and(|script| self.command_blocked_at(script, depth + 1));
        }
        if let Some(here) = args.iter().position(|arg| arg == "<<<") {
            return args
                .get(here + 1)
                .is_some_and(|script| self.command_blocked_at(script, depth + 1));
        }
        if let Some(body) = &segment.heredoc {
            return self.command_blocked_at(body, depth + 1);
        }
        let reads_stdin = args.iter().all(|arg| arg.starts_with('-'));
        if reads_stdin
            && segment.piped
            && let Some(previous) = previous
        {
            let mut fed = previous
                .words
                .iter()
                .skip(1)
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(body) = &previous.heredoc {
                fed.push('\n');
                fed.push_str(body);
            }
            return self.command_blocked_at(&fed, depth + 1);
        }
        false
    }

    /// 인터프리터 문자열은 해석하지 않고 키 저장소를 가리키는 표현이 있는지만 본다.
    fn text_reaches_key_store(&self, text: &str) -> bool {
        (text.contains("security")
            && BLOCKED_SECURITY_SUBCOMMANDS
                .iter()
                .any(|sub| text.contains(sub)))
            || text.contains("Library/Keychains")
            || text.contains(KEY_FILE)
            || self
                .blocked_paths
                .iter()
                .any(|path| text.contains(path.to_string_lossy().as_ref()))
    }

    /// 있는 경로면 심볼릭 링크를 푼 뒤 본다.
    fn path_blocked(&self, path: &Path) -> bool {
        if !path.is_absolute() {
            return false;
        }
        let resolved = resolve(path);
        self.blocked_paths
            .iter()
            .any(|blocked| resolved.starts_with(blocked) || path.starts_with(blocked))
    }

    fn expand_home(&self, word: &str) -> PathBuf {
        for prefix in ["~", "$HOME", "${HOME}"] {
            if let Some(rest) = word.strip_prefix(prefix)
                && (rest.is_empty() || rest.starts_with('/'))
            {
                return self.user_home.join(rest.trim_start_matches('/'));
            }
        }
        PathBuf::from(word)
    }
}

/// 훅 명령에 읽기 범위(작업 폴더와 더한 폴더)를 인자로 붙인다. 훅이 범위 밖 읽기를 `ask`로 올릴 때 쓴다.
/// 훅 명령이 없는 설정은 그대로 둔다.
pub fn with_read_scope(
    mut settings: serde_json::Value,
    workdir: &Path,
    add_dirs: &[PathBuf],
) -> serde_json::Value {
    let mut suffix = format!(" --workdir {}", shell_quote(&workdir.to_string_lossy()));
    for dir in add_dirs {
        suffix.push_str(&format!(
            " --add-dir {}",
            shell_quote(&dir.to_string_lossy())
        ));
    }
    let Some(groups) = settings["hooks"]["PreToolUse"].as_array_mut() else {
        return settings;
    };
    for group in groups {
        let Some(hooks) = group["hooks"].as_array_mut() else {
            continue;
        };
        for hook in hooks {
            if let Some(command) = hook["command"].as_str() {
                hook["command"] = json!(format!("{command}{suffix}"));
            }
        }
    }
    settings
}

/// 실행 파일이 `hook pre-tool-use`로 받는 훅 명령을 넣은 Claude 실행별 설정.
pub fn pre_tool_use_hook_settings(
    saturn_home: &Path,
    user_home: &Path,
    saturn_bin: &Path,
) -> serde_json::Value {
    HookPolicy::new(saturn_home, user_home).pre_tool_use_settings(saturn_bin)
}

/// 낱말 목록에서 명령이 시작할 수 있는 자리. 감싸는 명령 뒤에는 옵션 인자를 알 수 없어 뒤의 낱말을 모두 후보로 본다.
fn command_starts(words: &[String]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut wrapped = false;
    for (index, word) in words.iter().enumerate() {
        if word.contains('=') || (wrapped && word.starts_with('-')) {
            continue;
        }
        if COMMAND_WRAPPERS.contains(&base_name(word).as_str()) {
            wrapped = true;
            continue;
        }
        starts.push(index);
        if !wrapped {
            break;
        }
    }
    starts
}

fn base_name(program: &str) -> String {
    Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_interpreter(program: &str) -> bool {
    INTERPRETERS.contains(&program)
        || INTERPRETER_PREFIXES
            .iter()
            .any(|prefix| program.starts_with(prefix))
}

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// 없는 부분은 있는 가장 가까운 조상을 푼 뒤 나머지를 그대로 붙인다.
fn resolve(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(real) = ancestor.canonicalize() {
            let rest = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
            return real.join(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> (tempfile::TempDir, HookPolicy) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let saturn = home.join(".saturn");
        std::fs::create_dir_all(&saturn).unwrap();
        std::fs::create_dir_all(home.join("Library").join("Keychains")).unwrap();
        let policy = HookPolicy::new(&saturn, &home);
        (dir, policy)
    }

    fn denied(policy: &HookPolicy, call: ToolCall) -> bool {
        matches!(policy.check(&call), HookVerdict::Deny { .. })
    }

    #[test]
    fn keychain_lookup_commands_are_denied() {
        let (_dir, policy) = policy();
        let commands = [
            "security find-generic-password -s saturn -w",
            "ls && /usr/bin/security dump-keychain",
            "FOO=1 sudo security -q find-internet-password -a x",
            "echo $(security export -k login.keychain)",
            "cat ~/Library/Keychains/login.keychain-db",
            "cp \"$HOME/.saturn/router.key\" /tmp/x",
        ];

        for command in commands {
            assert!(
                denied(&policy, ToolCall::Command(command.to_owned())),
                "{command}"
            );
        }
    }

    #[test]
    fn ordinary_commands_are_allowed() {
        let (_dir, policy) = policy();
        let commands = [
            "cargo test -p saturn-engine",
            "security list-keychains",
            "grep -rn security src",
            "cat ~/.saturn/config.toml",
        ];

        for command in commands {
            assert!(
                !denied(&policy, ToolCall::Command(command.to_owned())),
                "{command}"
            );
        }
        assert!(!denied(&policy, ToolCall::Other));
    }

    #[test]
    fn blocked_paths_are_denied_after_resolving_links() {
        let (dir, policy) = policy();
        let home = dir.path().join("home");
        let key_file = home.join(".saturn").join(KEY_FILE);
        std::fs::write(&key_file, "sk").unwrap();
        let link = dir.path().join("innocent.txt");
        std::os::unix::fs::symlink(&key_file, &link).unwrap();

        assert!(denied(&policy, ToolCall::Path(key_file)));
        assert!(denied(&policy, ToolCall::Path(link)));
        assert!(denied(
            &policy,
            ToolCall::Path(home.join("Library/Keychains/login.keychain-db"))
        ));
        assert!(!denied(
            &policy,
            ToolCall::Path(home.join(".saturn").join("config.toml"))
        ));
    }

    #[test]
    fn deny_reason_has_no_paths() {
        let (_dir, policy) = policy();

        let verdict = policy.check(&ToolCall::Command(
            "security find-generic-password".to_owned(),
        ));

        let HookVerdict::Deny { reason } = verdict else {
            panic!("should deny");
        };
        assert!(!reason.contains('/'));
    }

    fn assert_all_denied(commands: &[&str]) {
        let (_dir, policy) = policy();
        for command in commands {
            assert!(
                denied(&policy, ToolCall::Command((*command).to_owned())),
                "{command}"
            );
        }
    }

    fn assert_all_allowed(commands: &[&str]) {
        let (_dir, policy) = policy();
        for command in commands {
            assert!(
                !denied(&policy, ToolCall::Command((*command).to_owned())),
                "{command}"
            );
        }
    }

    #[test]
    fn shell_wrapped_lookups_are_denied() {
        assert_all_denied(&[
            r#"sh -c "/usr/bin/security find-generic-password -s x -w""#,
            "bash -c 'security find-generic-password -s x -w'",
            r#"zsh -c "security find-internet-password -s x""#,
            r#"/bin/sh -c "security dump-keychain""#,
            r#"dash -c "security export""#,
            "ksh -c 'security find-generic-password -s x'",
            "fish -c 'security find-generic-password -s x'",
            r#"env sh -c "security find-generic-password -s x -w""#,
            "env -i /bin/bash -lc 'security export'",
            r#"sudo -u root sh -c "security find-generic-password -s x""#,
            "xargs sh -c 'security find-generic-password -s x'",
            "bash -o pipefail -c 'security find-generic-password -s x'",
        ]);
    }

    #[test]
    fn quoting_and_escapes_inside_shell_strings_do_not_hide_lookups() {
        assert_all_denied(&[
            r#"sh -c 'sec"ur"ity find-generic-password -s x -w'"#,
            r#"sh -c "s\ecurity find-generic-password -s x""#,
            r#"sh -c "'security' 'find-generic-password' -s x""#,
            r#"sh -c "sec''urity find-generic-password -s x""#,
            r#"sh -c "echo $(security find-generic-password -s x)""#,
            r#"sh -c 'echo "$(sec""urity find-generic-password -s x)"'"#,
            r#"sh -c 'cat "$HOME"/.saturn/router.key'"#,
        ]);
    }

    #[test]
    fn separators_inside_shell_strings_are_split() {
        assert_all_denied(&[
            r#"sh -c "true; security find-generic-password -s x""#,
            r#"sh -c "true && security find-generic-password -s x""#,
            "sh -c 'true || (security find-generic-password -s x)'",
            "sh -c 'ls | security find-generic-password -s x'",
            "sh -c 'ls\nsecurity find-generic-password -s x'",
        ]);
    }

    #[test]
    fn eval_strings_are_judged_again() {
        assert_all_denied(&[
            r#"eval "security find-generic-password -s x -w""#,
            "eval security find-generic-password -s x -w",
            "bash -c \"eval 'security find-generic-password -s x'\"",
            "true && eval 'security export'",
        ]);
    }

    #[test]
    fn nested_shells_are_judged_down_to_the_limit() {
        let mut command = "security find-generic-password -s x".to_owned();
        for _ in 0..3 {
            command = format!("sh -c '{}'", command.replace('\'', r"'\''"));
        }
        assert_all_denied(&[&command]);
    }

    #[test]
    fn nesting_beyond_the_limit_is_denied() {
        let mut command = "echo hello".to_owned();
        for _ in 0..12 {
            command = format!("sh -c '{}'", command.replace('\'', r"'\''"));
        }
        assert_all_denied(&[&command]);
    }

    #[test]
    fn unparseable_commands_are_denied() {
        assert_all_denied(&[
            r#"sh -c "security find-generic-password"#,
            "sh -c 'echo \"unterminated'",
            "echo 'unterminated",
            "cat <<",
        ]);
    }

    #[test]
    fn interpreter_one_liners_naming_key_stores_are_denied() {
        assert_all_denied(&[
            "python3 -c \"import subprocess; subprocess.run(['security','find-generic-password','-s','x','-w'])\"",
            "python -c 'import os; os.system(\"security dump-keychain\")'",
            r#"perl -e 'system("security find-generic-password -s x -w")'"#,
            "ruby -e 'puts `security find-generic-password -s x -w`'",
            "node -e \"require('child_process').execSync('security find-generic-password -s x')\"",
            r#"osascript -e 'do shell script "security find-generic-password -s x -w"'"#,
            "python3 -c \"print(open('/Users/someone/.saturn/router.key').read())\"",
            "node -e \"require('fs').readFileSync(require('os').homedir() + '/Library/Keychains/login.keychain-db')\"",
            "/usr/bin/env python3 -c 'import os; os.system(\"security export\")'",
            "sh -c \"python3 -c 'import os; os.system(\\\"security export\\\")'\"",
        ]);
    }

    #[test]
    fn shells_fed_by_pipe_here_string_or_here_document_are_judged() {
        assert_all_denied(&[
            r#"echo "security find-generic-password -s x -w" | sh"#,
            r#"printf 'security export' | bash"#,
            r#"sh <<< "security find-generic-password -s x -w""#,
            "sh <<EOF\nsecurity find-generic-password -s x -w\nEOF",
            "cat <<'EOF' | bash\nsecurity find-generic-password -s x\nEOF",
            "python3 <<EOF\nimport os\nos.system('security dump-keychain')\nEOF",
        ]);
    }

    #[test]
    fn interactive_security_is_denied() {
        assert_all_denied(&[
            "security -i",
            r#"echo "find-generic-password -s x -w" | /usr/bin/security -i"#,
        ]);
    }

    #[test]
    fn wrapped_commands_outside_the_list_are_allowed() {
        assert_all_allowed(&[
            r#"sh -c "ls""#,
            "bash -c 'echo hello && cargo test'",
            r#"sh -c "security list-keychains""#,
            "zsh -lc 'grep -rn security src'",
            r#"eval "echo hi""#,
            "env FOO=1 sh -c 'cargo test'",
            "sh script.sh",
            "bash -c 'echo \"security notes\"'",
            "python3 -c \"print('security')\"",
            r#"node -e "console.log(1)""#,
            "perl -e 'print 1'",
            "ruby -e 'puts 1'",
            "osascript -e 'display dialog \"hi\"'",
            "ls | sh -c 'wc -l'",
            "echo hello | bash",
        ]);
    }

    #[test]
    fn quotes_comments_and_here_documents_in_ordinary_commands_are_allowed() {
        assert_all_allowed(&[
            r#"git commit -m "don't leak""#,
            "cat <<'EOF'\ndon't stop\nEOF",
            "echo done # don't worry",
            "cat > notes.txt <<EOF\nit's fine\nEOF\nls",
        ]);
    }

    #[test]
    fn settings_add_one_saturn_hook_for_all_tools() {
        let (dir, policy) = policy();

        let settings = policy.pre_tool_use_settings(Path::new("/opt/my apps/saturn"));

        let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["matcher"], "*");
        assert_eq!(
            entries[0]["hooks"][0]["command"],
            format!(
                "'/opt/my apps/saturn' hook pre-tool-use --home '{}'",
                dir.path().join("home/.saturn").display()
            )
        );
    }
}
