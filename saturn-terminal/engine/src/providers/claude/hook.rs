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
    input: &str,
) -> Result<Option<String>, HookInputError> {
    let call = tool_call(input)?;
    let policy = HookPolicy::new(saturn_home, user_home);
    let HookVerdict::Deny { reason } = policy.check(&call) else {
        return Ok(None);
    };
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        },
    });
    Ok(Some(output.to_string()))
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
