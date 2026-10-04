use std::time::Duration;

use saturn_core::providers::ProviderCommand;
use saturn_core::sessions::context::DEFAULT_CACHE_TTL;
use saturn_protocol::event::{
    Activity, LineChange, LineRange, PermissionCall, PermissionTool, ProviderEvent, ToolCategory,
    ToolDetail, UsageReport, UsageScope,
};
use saturn_protocol::ids::SubagentId;
use saturn_protocol::rpc::PermissionAnswer;
use serde_json::{Value, json};

use super::{
    DENY_MESSAGE, EDIT_TOOLS, READ_TOOLS, SHELL_TOOL, SUBAGENT_TOOLS, SUBSCRIPTION_CACHE_TTL,
    SUBSCRIPTION_KEY_SOURCE, SessionState,
};
use crate::providers::AppliedSettings;
use crate::providers::claude_input::{self, ASK_TOOL};
use crate::providers::tool_detail::{classify_command, line_change};

/// subagent 등록과 `turn_active`, 적용값, 명령 목록 갱신도 여기서 한다. 버릴 줄이면 빈 목록.
pub(super) fn convert_line(
    state: &mut SessionState,
    line: &serde_json::Value,
) -> Vec<ProviderEvent> {
    let agent = state.agent;
    match line["type"].as_str() {
        Some("system") if line["subtype"] == "init" => apply_init(state, line),
        Some("assistant") => convert_assistant(state, line),
        Some("user") => convert_tool_results(state, line),
        Some("result") => convert_result(state, line),
        Some("control_request") if line["request"]["subtype"] == "can_use_tool" => {
            let request = &line["request"];
            let tool = request["tool_name"].as_str().unwrap_or_default();
            let target = request["input"]["command"]
                .as_str()
                .or_else(|| request["input"]["file_path"].as_str());
            let summary =
                target.map_or_else(|| tool.to_owned(), |target| format!("{tool}: {target}"));
            let request_id = line["request_id"].as_str().unwrap_or_default().to_owned();
            if tool == ASK_TOOL
                && let Some(asked) = claude_input::request(&request["input"])
            {
                state
                    .inputs
                    .insert(request_id.clone(), request["input"].clone());
                return vec![ProviderEvent::InputRequested {
                    agent,
                    request_id,
                    request: asked,
                }];
            }
            state
                .permissions
                .insert(request_id.clone(), request["input"].clone());
            vec![ProviderEvent::PermissionRequested {
                agent,
                request_id,
                summary,
                reason: request["decision_reason"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                call: permission_call(tool, &request["input"]),
            }]
        }
        _ => Vec::new(),
    }
}

/// 성공한 결과만 `TurnCompleted`로 올린다. 오류 결과는 완료 신호 없이 끊긴 흐름과 같게 `StreamLost`로 올려 작업을
/// 결과 확인 필요로 두고 자동으로 다시 보내지 않는다. 턴이 이미 돌았을 수 있어 보내지 않음이 확정이 아니기 때문이다.
/// 맥락 초과(`terminal_reason`이 `prompt_too_long`)도 같다. `ContextExceeded`는 보내지 않음이 확정일 때만 쓴다.
/// 멈춤 요청 뒤의 결과는 오류 모양이어도 요청한 완료로 본다.
fn convert_result(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    let stop_requested = std::mem::take(&mut state.stop_requested);
    state.turn_active = false;
    let origin = state.origin.on_turn_started();
    let usage = &line["usage"];
    let mut events = vec![
        ProviderEvent::Usage(UsageReport {
            agent,
            subagent: None,
            model: state.applied.model.clone(),
            scope: UsageScope::MainTurn,
            input: usage["input_tokens"].as_u64(),
            cache_read: usage["cache_read_input_tokens"].as_u64(),
            cache_write: usage["cache_creation_input_tokens"].as_u64(),
            output: usage["output_tokens"].as_u64(),
            reasoning: None,
        }),
        ProviderEvent::ContextSize {
            agent,
            tokens: state.context_tokens,
        },
    ];
    if stop_requested || !is_error_result(line) {
        events.push(ProviderEvent::TurnCompleted { agent, origin });
        return events;
    }
    tracing::warn!(
        subtype = line["subtype"].as_str().unwrap_or_default(),
        terminal_reason = line["terminal_reason"].as_str().unwrap_or_default(),
        "claude turn ended with an error result"
    );
    state.running.clear();
    events.push(ProviderEvent::StreamLost { agent });
    events
}

/// `is_error`가 참이거나 `subtype`이 `success`가 아니면 오류 결과다. `subtype`이 없으면 `is_error`만 본다.
fn is_error_result(line: &Value) -> bool {
    line["is_error"] == true
        || line["subtype"]
            .as_str()
            .is_some_and(|kind| kind != "success")
}

/// 규칙 대상 도구의 요청만 규칙이 읽는 호출로 바꾼다. 그 밖의 도구는 `None`이라 사용자에게 묻는다.
pub(super) fn permission_call(tool: &str, input: &Value) -> Option<PermissionCall> {
    let text = |key: &str| input[key].as_str().map(str::to_owned);
    if tool == SHELL_TOOL {
        return Some(shell_call(text("command").unwrap_or_default()));
    }
    if EDIT_TOOLS.contains(&tool) {
        let path = text("file_path").or_else(|| text("notebook_path"));
        return Some(PermissionCall {
            tool: PermissionTool::Edit,
            target: String::new(),
            paths: path.into_iter().collect(),
        });
    }
    if READ_TOOLS.contains(&tool) {
        let path = text("file_path").or_else(|| text("path"));
        return Some(PermissionCall {
            tool: PermissionTool::Read,
            target: String::new(),
            paths: path.into_iter().collect(),
        });
    }
    if SUBAGENT_TOOLS.contains(&tool) {
        return Some(PermissionCall {
            tool: PermissionTool::Subagent,
            target: text("subagent_type").unwrap_or_default(),
            paths: Vec::new(),
        });
    }
    tool.starts_with("mcp__").then(|| PermissionCall {
        tool: PermissionTool::Mcp,
        target: tool.to_owned(),
        paths: Vec::new(),
    })
}

pub(super) fn shell_call(command: String) -> PermissionCall {
    PermissionCall {
        tool: PermissionTool::Shell,
        target: command,
        paths: Vec::new(),
    }
}

/// 허용은 요청 `input`을 그대로 돌려주고, 거부는 모델에 전달되는 고정 문구를 붙인다.
/// `AllowAlways`는 `AllowOnce`와 같게 보낸다. 규칙으로 읽은 호출의 항상 허용은 engine이 저장해 판정하고, 읽지 못한
/// 요청은 Claude 세션 규칙으로 보내는 값(`updatedPermissions`)을 실측하지 않아 되풀이해 묻는다.
/// TODO(#56): 거부와 함께 남기는 말은 정해지기 전에는 보내지 않는다
pub(super) fn permission_response(answer: &PermissionAnswer, input: &Value) -> Value {
    match answer {
        PermissionAnswer::AllowOnce | PermissionAnswer::AllowAlways => {
            json!({ "behavior": "allow", "updatedInput": input })
        }
        PermissionAnswer::Deny { .. } => json!({ "behavior": "deny", "message": DENY_MESSAGE }),
    }
}

/// `apiKeySource`가 `none`이면 구독이라 1시간, 다른 값이면 5분이다. 값이 없으면 판단할 수 없어 `None`.
pub(super) fn cache_ttl(line: &Value) -> Option<Duration> {
    let source = line["apiKeySource"].as_str()?;
    Some(if source == SUBSCRIPTION_KEY_SOURCE {
        SUBSCRIPTION_CACHE_TTL
    } else {
        DEFAULT_CACHE_TTL
    })
}

/// 캐시 유지 시간을 판단할 수 있으면 `CacheWindow`를 알리고, 두 번째부터 모델이나 권한 방식이 바뀌었으면 `SettingsApplied`도 알린다.
pub(super) fn apply_init(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let cache_window = cache_ttl(line).map(|ttl| ProviderEvent::CacheWindow {
        agent: state.agent,
        ttl_secs: ttl.as_secs(),
    });
    let applied = AppliedSettings {
        model: line["model"].as_str().map(str::to_owned),
        permission: line["permissionMode"].as_str().map(str::to_owned),
    };
    let skills: Vec<&str> = line["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| skill.as_str().or_else(|| skill["name"].as_str()))
        .collect();
    state.commands = line["slash_commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| ProviderCommand {
            name: name.trim_start_matches('/').to_owned(),
            description: String::new(),
            is_skill: skills.contains(&name),
        })
        .collect();
    let changed = state.initialized && applied != state.applied;
    state.initialized = true;
    state.applied = applied;
    if !changed {
        return cache_window.into_iter().collect();
    }
    let values = [
        ("model", state.applied.model.clone()),
        ("permission_mode", state.applied.permission.clone()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
    .collect();
    cache_window
        .into_iter()
        .chain([ProviderEvent::SettingsApplied {
            agent: state.agent,
            values,
        }])
        .collect()
}

/// Task/Agent 호출은 subagent 시작으로 등록한다.
pub(super) fn convert_assistant(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    let parent = parent_subagent(line);
    if parent.is_none() {
        let usage = &line["message"]["usage"];
        let parts = [
            "input_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .map(|key| usage[key].as_u64());
        if parts.iter().any(Option::is_some) {
            state.context_tokens = Some(parts.iter().flatten().sum());
        }
    }
    let mut events = Vec::new();
    for item in line["message"]["content"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("text") => {
                if let Some(text) = item["text"].as_str() {
                    events.push(ProviderEvent::Text {
                        agent,
                        subagent: parent.clone(),
                        text: text.to_owned(),
                    });
                }
            }
            Some("tool_use") => {
                let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) else {
                    continue;
                };
                if SUBAGENT_TOOLS.contains(&name) {
                    let subagent = SubagentId(id.to_owned());
                    state.running.insert(subagent.clone(), parent.clone());
                    events.push(ProviderEvent::SubagentStarted {
                        agent,
                        subagent,
                        parent: parent.clone(),
                    });
                } else {
                    if name == SHELL_TOOL {
                        state.shell_calls.insert(id.to_owned());
                    }
                    events.push(ProviderEvent::ToolCall {
                        agent,
                        subagent: parent.clone(),
                        call_id: id.to_owned(),
                        activity: activity_of(name, &item["input"]),
                        detail: detail_of(name, &item["input"]),
                    });
                }
            }
            _ => {}
        }
    }
    events
}

/// subagent 호출의 결과는 subagent 끝이다.
pub(super) fn convert_tool_results(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    let parent = parent_subagent(line);
    let mut events = Vec::new();
    for item in line["message"]["content"].as_array().into_iter().flatten() {
        if item["type"] != "tool_result" {
            continue;
        }
        let Some(id) = item["tool_use_id"].as_str() else {
            continue;
        };
        let subagent = SubagentId(id.to_owned());
        if state.running.remove(&subagent).is_some() {
            events.push(ProviderEvent::SubagentEnded { agent, subagent });
            continue;
        }
        let output = tool_result_text(&item["content"]);
        let exit_code = if state.shell_calls.remove(id) {
            shell_exit_code(item["is_error"].as_bool().unwrap_or(false), &output)
        } else {
            None
        };
        events.push(ProviderEvent::ToolResult {
            agent,
            subagent: parent.clone(),
            call_id: id.to_owned(),
            output,
            exit_code,
        });
    }
    events
}

pub(super) fn parent_subagent(line: &Value) -> Option<SubagentId> {
    line["parent_tool_use_id"]
        .as_str()
        .map(|id| SubagentId(id.to_owned()))
}

pub(super) fn activity_of(name: &str, input: &Value) -> Activity {
    if READ_TOOLS.contains(&name) {
        return Activity::ReadingFile;
    }
    if EDIT_TOOLS.contains(&name) {
        return Activity::EditingFile;
    }
    let command = match name {
        SHELL_TOOL => input["command"].as_str().unwrap_or_default().to_owned(),
        other => other.to_owned(),
    };
    Activity::RunningCommand { command }
}

// cost: time O(i), heap O(i), stack O(1), alloc 1
// vars: i = 입력 글자 수
// basis: estimate
pub(super) fn detail_of(name: &str, input: &Value) -> ToolDetail {
    let path_of = |key: &str| input[key].as_str().map(str::to_owned).into_iter().collect();
    if name == SHELL_TOOL {
        let command = input["command"].as_str().unwrap_or_default();
        return ToolDetail {
            category: classify_command(command),
            ..ToolDetail::default()
        };
    }
    if name == "Read" {
        return ToolDetail {
            category: ToolCategory::FileRead,
            paths: path_of("file_path"),
            read_lines: read_range(input),
            changed: None,
        };
    }
    if READ_TOOLS.contains(&name) {
        return ToolDetail {
            category: ToolCategory::FileRead,
            paths: path_of("path"),
            ..ToolDetail::default()
        };
    }
    if EDIT_TOOLS.contains(&name) {
        let key = if name == "NotebookEdit" {
            "notebook_path"
        } else {
            "file_path"
        };
        return ToolDetail {
            category: ToolCategory::FileEdit,
            paths: path_of(key),
            read_lines: None,
            changed: edit_change(name, input),
        };
    }
    ToolDetail::default()
}

/// 시작 줄과 줄 수가 모두 있을 때만 범위를 낸다. 한쪽만 있으면 전체 범위를 알 수 없다.
pub(super) fn read_range(input: &Value) -> Option<LineRange> {
    let first = u32::try_from(input["offset"].as_u64()?).ok()?;
    let limit = u32::try_from(input["limit"].as_u64()?).ok()?;
    let last = first.checked_add(limit)?.checked_sub(1)?;
    (first >= 1 && first <= last).then_some(LineRange { first, last })
}

// cost: time O(i), heap O(i), stack O(1), alloc 2
// vars: i = 입력 글자 수
// basis: estimate
/// `Edit`와 `MultiEdit`만 센다. `Write`는 덮어쓴 줄을 입력으로 알 수 없어 `None`이다.
pub(super) fn edit_change(name: &str, input: &Value) -> Option<LineChange> {
    let one = |edit: &Value| -> Option<LineChange> {
        Some(line_change(
            edit["old_string"].as_str()?,
            edit["new_string"].as_str()?,
        ))
    };
    let edits: Vec<Option<LineChange>> = match name {
        "Edit" => vec![one(input)],
        "MultiEdit" => input["edits"].as_array()?.iter().map(one).collect(),
        _ => return None,
    };
    edits.into_iter().try_fold(
        LineChange {
            added: 0,
            removed: 0,
        },
        |total, change| {
            let change = change?;
            Some(LineChange {
                added: total.added.saturating_add(change.added),
                removed: total.removed.saturating_add(change.removed),
            })
        },
    )
}

/// 실패한 `Bash` 결과는 `Exit code N`으로 시작하고, 성공은 0이다. 중단처럼 코드가 없으면 `None`.
pub(super) fn shell_exit_code(is_error: bool, output: &str) -> Option<i32> {
    if !is_error {
        return Some(0);
    }
    output
        .strip_prefix("Exit code ")?
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .next()?
        .parse()
        .ok()
}

/// 문자열이거나 `text` 항목 배열이다.
pub(super) fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
