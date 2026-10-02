//! Saturn 소유 PreToolUse 훅: 에이전트의 키 저장소 조회와 Saturn 비밀 파일 접근을 막는다. 명령 이름과 차단 목록은 초안이다.
//! 설계: docs/design/router-key-security.md

use std::path::{Path, PathBuf};

use serde_json::json;

use super::storage::KEY_FILE;

/// 초안 목록.
const BLOCKED_SECURITY_SUBCOMMANDS: &[&str] = &[
    "find-generic-password",
    "find-internet-password",
    "dump-keychain",
    "export",
];

/// 건너뛰고 다음 낱말을 본다.
const COMMAND_WRAPPERS: &[&str] = &["sudo", "env", "command", "exec", "nohup", "time", "xargs"];

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

    /// 사용자 훅을 읽거나 바꾸지 않고, 사용자 설정과 합치는 일은 Claude가 한다.
    pub(crate) fn pre_tool_use_settings(&self, saturn_bin: &Path) -> serde_json::Value {
        let command = format!(
            "{} hook pre-tool-use",
            shell_quote(&saturn_bin.to_string_lossy())
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

    #[test]
    fn settings_add_one_saturn_hook_for_all_tools() {
        let (_dir, policy) = policy();

        let settings = policy.pre_tool_use_settings(Path::new("/opt/my apps/saturn"));

        let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["matcher"], "*");
        assert_eq!(
            entries[0]["hooks"][0]["command"],
            "'/opt/my apps/saturn' hook pre-tool-use"
        );
    }
}
