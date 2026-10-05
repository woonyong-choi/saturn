//! Claude Code PreToolUse 훅 입력(stdin JSON)을 키 보호 정책으로 판정하고 훅 출력으로 바꾼다.
//! 설계: docs/design/router-key-security.md

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::secrets::{HookPolicy, HookVerdict, ToolCall};

/// 경로 인자를 `tool_input`에서 읽는 도구와 그 필드 이름. 초안 목록.
const PATH_FIELDS: &[(&str, &str)] = &[
    ("Read", "file_path"),
    ("Edit", "file_path"),
    ("MultiEdit", "file_path"),
    ("Write", "file_path"),
    ("NotebookRead", "notebook_path"),
    ("NotebookEdit", "notebook_path"),
    ("Glob", "path"),
    ("Grep", "path"),
    ("LS", "path"),
];

const SHELL_TOOL: &str = "Bash";

/// 파일 읽기 도구. 초안 목록.
const READ_TOOLS: &[&str] = &["Read", "NotebookRead", "Glob", "Grep", "LS"];

/// 읽기 도구가 허가 요청 없이 읽을 수 있는 범위. 밖의 읽기는 훅이 `ask`로 올려 Saturn 읽기 규칙이 판정하게 한다.
/// 작업 폴더가 없으면 범위를 모르는 것이라 읽기를 올리지 않는다.
#[derive(Debug, Clone, Default)]
pub struct ReadScope {
    pub workdir: Option<PathBuf>,
    pub add_dirs: Vec<PathBuf>,
}

impl ReadScope {
    fn is_outside(&self, path: &Path) -> bool {
        let Some(workdir) = &self.workdir else {
            return false;
        };
        let path = resolve(path);
        !std::iter::once(workdir)
            .chain(&self.add_dirs)
            .any(|dir| path.starts_with(resolve(dir)))
    }
}

/// 링크를 풀고 `.`와 `..`를 걷어낸다. 없는 부분은 있는 가장 가까운 조상을 푼 뒤 나머지를 글자로 이어 붙인다.
fn resolve(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    let mut base = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    base.pop();
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => parts.push(part.to_owned()),
            other => base.push(other.as_os_str()),
        }
    }
    let mut existing = base;
    let mut rest = parts.into_iter().peekable();
    while let Some(part) = rest.peek() {
        let next = existing.join(part);
        match std::fs::canonicalize(&next) {
            Ok(resolved) => {
                existing = resolved;
                rest.next();
            }
            Err(_) => break,
        }
    }
    rest.fold(existing, |path, part| path.join(part))
}

/// 훅 입력을 읽지 못하면 도구 호출을 막아야 하므로 호출자는 종료 코드 2로 끝낸다.
#[derive(Debug, thiserror::Error)]
pub enum HookInputError {
    #[error("hook input is not valid JSON")]
    NotJson,
    #[error("hook input has no tool_name")]
    NoToolName,
}

/// 돌려주는 값은 stdout에 쓸 훅 출력이고 `None`이면 아무것도 쓰지 않고 종료 코드 0으로 끝낸다.
pub fn run_pre_tool_use(
    saturn_home: &Path,
    user_home: &Path,
    scope: &ReadScope,
    input: &str,
) -> Result<Option<String>, HookInputError> {
    let call = tool_call(input)?;
    let policy = HookPolicy::new(saturn_home, user_home);
    if let HookVerdict::Deny { reason } = policy.check(&call) {
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            },
        });
        return Ok(Some(output.to_string()));
    }
    if is_outside_read(input, scope) {
        // 사용자 설정의 읽기 허용이 Saturn 읽기 규칙을 건너뛰지 못하게 요청으로 올린다
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": "read outside the working folders is decided by Saturn read rules",
            },
        });
        return Ok(Some(output.to_string()));
    }
    Ok(None)
}

/// 읽기 도구(`Read`, `Glob`, `Grep`, `LS`, `NotebookRead`)가 작업 폴더와 더한 폴더 밖을 읽으려는지 본다.
fn is_outside_read(input: &str, scope: &ReadScope) -> bool {
    let Ok(input) = serde_json::from_str::<Value>(input) else {
        return false;
    };
    let Some(tool) = input["tool_name"].as_str() else {
        return false;
    };
    if !READ_TOOLS.contains(&tool) {
        return false;
    }
    let tool_input = &input["tool_input"];
    let cwd = input["cwd"].as_str().map(Path::new);
    let field = PATH_FIELDS
        .iter()
        .find_map(|(name, field)| (*name == tool).then_some(*field));
    let path = field.and_then(|field| tool_input[field].as_str());
    let glob = (tool == "Glob")
        .then(|| tool_input["pattern"].as_str())
        .flatten()
        .filter(|pattern| Path::new(pattern).is_absolute())
        .map(glob_root);
    path.map(|path| absolute(path, cwd))
        .into_iter()
        .chain(glob)
        .any(|path| scope.is_outside(&path))
}

/// 절대 경로 패턴에서 `*`, `?`, `[`, `{`가 나오기 전까지의 폴더.
fn glob_root(pattern: &str) -> PathBuf {
    let end = pattern.find(['*', '?', '[', '{']).unwrap_or(pattern.len());
    let fixed = &pattern[..end];
    if end == pattern.len() || fixed.ends_with('/') {
        return PathBuf::from(fixed);
    }
    Path::new(fixed)
        .parent()
        .map_or_else(|| PathBuf::from("/"), Path::to_path_buf)
}

fn tool_call(input: &str) -> Result<ToolCall, HookInputError> {
    let input: Value = serde_json::from_str(input).map_err(|_| HookInputError::NotJson)?;
    let tool = input["tool_name"]
        .as_str()
        .ok_or(HookInputError::NoToolName)?;
    let tool_input = &input["tool_input"];
    if tool == SHELL_TOOL {
        return Ok(tool_input["command"]
            .as_str()
            .map_or(ToolCall::Other, |command| {
                ToolCall::Command(command.to_owned())
            }));
    }
    let field = PATH_FIELDS
        .iter()
        .find_map(|(name, field)| (*name == tool).then_some(*field));
    let path = field.and_then(|field| tool_input[field].as_str());
    let Some(path) = path else {
        return Ok(ToolCall::Other);
    };
    let cwd = input["cwd"].as_str().map(Path::new);
    Ok(ToolCall::Path(absolute(path, cwd)))
}

/// 상대 경로는 훅 입력의 `cwd` 기준으로 절대 경로로 바꾼다. `cwd`가 없으면 상대 경로 그대로 둔다.
fn absolute(path: &str, cwd: Option<&Path>) -> PathBuf {
    match cwd {
        Some(cwd) => cwd.join(path),
        None => PathBuf::from(path),
    }
}
