use std::collections::HashMap;

use saturn_protocol::event::{
    Activity, LineChange, ProviderEvent, ToolCategory, ToolDetail, UsageReport, UsageScope,
};
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use serde_json::{Value, json};

use super::threads::{close_child, register_child};
use super::{
    APPROVAL_METHODS, ELICITATION_METHOD, PERMISSIONS_METHOD, PendingApproval, PendingInput,
    ThreadState, USER_INPUT_METHOD, value_text,
};
use crate::providers::codex_input::{self, InputKind};
use crate::providers::codex_permission::{call_of, file_change_paths};
use crate::providers::tool_detail::{classify_command, unwrap_shell};

/// 권한은 Saturn 규칙이 정하므로 권한 인자는 넣지 않는다. 승인 정책과 샌드박스는 `thread/start`가 정한다.
/// 숨긴 모델이거나 모델 이름이 없으면 `None`.
pub(super) fn model_info(entry: &Value) -> Option<ModelInfo> {
    if entry["hidden"].as_bool() == Some(true) {
        return None;
    }
    let model = entry["model"].as_str().or_else(|| entry["id"].as_str())?;
    Some(ModelInfo {
        choice: ModelChoice {
            provider: crate::providers::CODEX,
            model: model.to_owned(),
        },
        name: entry["displayName"].as_str().unwrap_or(model).to_owned(),
    })
}

/// 자식 thread 등록과 `active_turn` 갱신도 여기서 한다. 버릴 알림이면 빈 목록.
pub(super) fn convert_notification(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    params: &serde_json::Value,
) -> Vec<ProviderEvent> {
    if method == "thread/started" {
        return register_child(threads, &params["thread"]);
    }
    let Some(thread_id) = params["threadId"].as_str() else {
        return Vec::new();
    };
    let thread = ProviderSessionId(thread_id.to_owned());
    if method == "thread/closed" {
        return close_child(threads, &thread);
    }
    let Some(state) = threads.get_mut(&thread) else {
        return Vec::new();
    };
    let agent = state.agent;
    let subagent = state.parent.as_ref().map(|_| subagent_id(&thread));
    match method {
        "turn/started" => on_turn_started(state, params),
        "turn/completed" => on_turn_completed(state, params, agent, subagent),
        "item/agentMessage/delta" => params["delta"]
            .as_str()
            .map(|text| ProviderEvent::Text {
                agent,
                subagent,
                text: text.to_owned(),
            })
            .into_iter()
            .collect(),
        "item/started" => on_item_started(state, &params["item"], agent, subagent),
        "item/completed" => tool_output(&params["item"])
            .zip(params["item"]["id"].as_str())
            .map(|(output, id)| ProviderEvent::ToolResult {
                agent,
                subagent,
                call_id: id.to_owned(),
                output,
                exit_code: exit_code_of(&params["item"]),
            })
            .into_iter()
            .collect(),
        "thread/tokenUsage/updated" => {
            on_token_usage(state, &params["tokenUsage"], agent, subagent)
        }
        "thread/settings/updated" => on_settings_updated(state, &params["threadSettings"], agent),
        _ => Vec::new(),
    }
}

fn on_turn_started(state: &mut ThreadState, params: &Value) -> Vec<ProviderEvent> {
    state.active_turn = params["turn"]["id"].as_str().map(str::to_owned);
    if state.parent.is_none() {
        state.turn_origin = Some(state.origin.on_turn_started());
    }
    Vec::new()
}

fn on_turn_completed(
    state: &mut ThreadState,
    params: &Value,
    agent: AgentId,
    subagent: Option<SubagentId>,
) -> Vec<ProviderEvent> {
    state.active_turn = None;
    state.last_completed_turn = params["turn"]["id"].as_str().map(str::to_owned);
    if let Some(subagent) = subagent {
        return vec![ProviderEvent::SubagentEnded { agent, subagent }];
    }
    let origin = state
        .turn_origin
        .take()
        .unwrap_or_else(|| state.origin.on_turn_started());
    vec![
        ProviderEvent::ContextSize {
            agent,
            tokens: state.context_tokens,
        },
        ProviderEvent::TurnCompleted { agent, origin },
    ]
}

fn on_item_started(
    state: &mut ThreadState,
    item: &Value,
    agent: AgentId,
    subagent: Option<SubagentId>,
) -> Vec<ProviderEvent> {
    if let (Some("fileChange"), Some(id)) = (item["type"].as_str(), item["id"].as_str()) {
        state
            .file_changes
            .insert(id.to_owned(), file_change_paths(item));
    }
    activity_of(item)
        .zip(item["id"].as_str())
        .map(|(activity, id)| ProviderEvent::ToolCall {
            agent,
            subagent,
            call_id: id.to_owned(),
            activity,
            detail: detail_of(item),
        })
        .into_iter()
        .collect()
}

fn on_token_usage(
    state: &mut ThreadState,
    usage: &Value,
    agent: AgentId,
    subagent: Option<SubagentId>,
) -> Vec<ProviderEvent> {
    state.context_tokens = usage["last"]["totalTokens"].as_u64();
    vec![ProviderEvent::Usage(cumulative_usage(
        agent,
        subagent,
        state.applied.model.clone(),
        &usage["total"],
    ))]
}

fn on_settings_updated(
    state: &mut ThreadState,
    settings: &Value,
    agent: AgentId,
) -> Vec<ProviderEvent> {
    state.applied.model = settings["model"].as_str().map(str::to_owned);
    state.applied.permission = value_text(&settings["approvalPolicy"]);
    let values = [
        ("model", state.applied.model.clone()),
        ("approval_policy", state.applied.permission.clone()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
    .collect();
    vec![ProviderEvent::SettingsApplied { agent, values }]
}

/// 승인 요청은 `PermissionRequested`로, 입력 요청은 `InputRequested`로 올리고 나머지는 버린다. 올린 요청은
/// 답할 수 있게 `approvals`에 둔다.
pub(super) fn convert_server_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    approvals: &mut HashMap<String, PendingApproval>,
    method: &str,
    id: &Value,
    params: &Value,
) -> Vec<ProviderEvent> {
    if let Some(kind) = input_kind(method, params) {
        return convert_input_request(threads, approvals, kind, id, params)
            .into_iter()
            .collect();
    }
    let Some((_, summary)) = APPROVAL_METHODS.iter().find(|(name, _)| *name == method) else {
        return Vec::new();
    };
    let Some(state) = thread_of_request(threads, params) else {
        return Vec::new();
    };
    let summary = params["command"]
        .as_str()
        .map(|command| format!("{summary}: {command}"))
        .or_else(|| {
            (method == ELICITATION_METHOD)
                .then(|| params["message"].as_str().map(str::to_owned))
                .flatten()
        })
        .unwrap_or_else(|| (*summary).to_owned());
    let request_id = value_text(id).unwrap_or_default();
    approvals.insert(
        request_id.clone(),
        PendingApproval {
            id: id.clone(),
            method: method.to_owned(),
            available: params["availableDecisions"].as_array().map(|decisions| {
                decisions
                    .iter()
                    .filter_map(|decision| decision.as_str().map(str::to_owned))
                    .collect()
            }),
            permissions: params["permissions"].clone(),
            input: None,
        },
    );
    vec![ProviderEvent::PermissionRequested {
        agent: state.agent,
        request_id,
        summary,
        reason: params["reason"].as_str().unwrap_or_default().to_owned(),
        call: call_of(method, params, &state.file_changes),
    }]
}

/// 승인이 아닌 elicitation과 에이전트 질문.
pub(super) fn input_kind(method: &str, params: &Value) -> Option<InputKind> {
    match method {
        USER_INPUT_METHOD => Some(InputKind::UserInput),
        ELICITATION_METHOD if !params["_meta"]["codex_approval_kind"].is_string() => {
            Some(InputKind::Elicitation)
        }
        _ => None,
    }
}

pub(super) fn thread_of_request<'a>(
    threads: &'a HashMap<ProviderSessionId, ThreadState>,
    params: &Value,
) -> Option<&'a ThreadState> {
    let thread = params["threadId"]
        .as_str()
        .or_else(|| params["conversationId"].as_str())
        .map(|id| ProviderSessionId(id.to_owned()))?;
    threads.get(&thread)
}

/// 읽을 수 없는 요청(모르는 `mode`, 모르는 thread)은 올리지 않는다.
pub(super) fn convert_input_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    approvals: &mut HashMap<String, PendingApproval>,
    kind: InputKind,
    id: &Value,
    params: &Value,
) -> Option<ProviderEvent> {
    let state = thread_of_request(threads, params)?;
    let request = match kind {
        InputKind::Elicitation => codex_input::elicitation_request(params)?,
        InputKind::UserInput => codex_input::user_input_request(params),
    };
    let request_id = value_text(id).unwrap_or_default();
    approvals.insert(
        request_id.clone(),
        PendingApproval {
            id: id.clone(),
            method: request_method(kind).to_owned(),
            available: None,
            permissions: Value::Null,
            input: Some(PendingInput {
                kind,
                request: request.clone(),
            }),
        },
    );
    Some(ProviderEvent::InputRequested {
        agent: state.agent,
        request_id,
        request,
    })
}

pub(super) fn request_method(kind: InputKind) -> &'static str {
    match kind {
        InputKind::Elicitation => ELICITATION_METHOD,
        InputKind::UserInput => USER_INPUT_METHOD,
    }
}

/// 답을 요청 종류별 결정 값으로 바꾼다. `AllowAlways`는 Codex 세션 동안 허용(`acceptForSession`,
/// `approved_for_session`, 권한 `scope: session`)으로 보내고, 요청이 그 결정을 허용 목록에서 뺐거나 보낼 값이
/// 없는(MCP elicitation) 요청이면 `AllowOnce`와 같게 보낸다.
/// `decline`은 `availableDecisions`에 없어도 받아들여지는 것을 실측했다.
/// engine은 규칙으로 읽은 호출의 `항상 허용`을 직접 저장하고 `AllowOnce`로 보내므로, 이 값은 읽을 수 없는
/// 요청(권한 요청 등)에만 쓰인다.
/// TODO(#56): 거부와 함께 남기는 말은 정해지기 전에는 보내지 않는다
pub(super) fn approval_result(pending: &PendingApproval, answer: &PermissionAnswer) -> Value {
    match pending.method.as_str() {
        ELICITATION_METHOD => elicitation_result(answer),
        PERMISSIONS_METHOD => permissions_result(pending, answer),
        "execCommandApproval" | "applyPatchApproval" => legacy_decision_result(answer),
        _ => decision_result(pending, answer),
    }
}

fn elicitation_result(answer: &PermissionAnswer) -> Value {
    if matches!(answer, PermissionAnswer::AllowAlways) {
        tracing::debug!("codex MCP approval has no always value, sent as allow once");
    }
    if matches!(answer, PermissionAnswer::Deny { .. }) {
        json!({ "action": "decline" })
    } else {
        json!({ "action": "accept", "content": {} })
    }
}

fn permissions_result(pending: &PendingApproval, answer: &PermissionAnswer) -> Value {
    let scope = if matches!(answer, PermissionAnswer::AllowAlways) {
        "session"
    } else {
        "turn"
    };
    let permissions = if matches!(answer, PermissionAnswer::Deny { .. }) {
        json!({})
    } else {
        pending.permissions.clone()
    };
    json!({ "permissions": permissions, "scope": scope })
}

fn legacy_decision_result(answer: &PermissionAnswer) -> Value {
    let decision = match answer {
        PermissionAnswer::Deny { .. } => "denied",
        PermissionAnswer::AllowAlways => "approved_for_session",
        _ => "approved",
    };
    json!({ "decision": decision })
}

fn decision_result(pending: &PendingApproval, answer: &PermissionAnswer) -> Value {
    let for_session_listed = pending.available.as_ref().is_none_or(|available| {
        available
            .iter()
            .any(|decision| decision == "acceptForSession")
    });
    let decision = match answer {
        PermissionAnswer::Deny { .. } => "decline",
        PermissionAnswer::AllowAlways if for_session_listed => "acceptForSession",
        _ => "accept",
    };
    json!({ "decision": decision })
}

/// 도구 항목이 아니면 `None`.
pub(super) fn activity_of(item: &Value) -> Option<Activity> {
    match item["type"].as_str()? {
        "commandExecution" => {
            let actions = item["commandActions"].as_array();
            let only_reads = actions.is_some_and(|actions| {
                !actions.is_empty()
                    && actions.iter().all(|action| {
                        matches!(
                            action["type"].as_str(),
                            Some("read" | "listFiles" | "search")
                        )
                    })
            });
            if only_reads {
                return Some(Activity::ReadingFile);
            }
            Some(Activity::RunningCommand {
                command: unwrap_shell(item["command"].as_str().unwrap_or_default()),
            })
        }
        "fileChange" => Some(Activity::EditingFile),
        "reasoning" => Some(Activity::Thinking),
        "contextCompaction" => Some(Activity::Compacting),
        "mcpToolCall" => Some(Activity::RunningCommand {
            command: format!(
                "{}/{}",
                item["server"].as_str().unwrap_or_default(),
                item["tool"].as_str().unwrap_or_default()
            ),
        }),
        "dynamicToolCall" | "webSearch" => Some(Activity::RunningCommand {
            command: item["tool"]
                .as_str()
                .or_else(|| item["query"].as_str())
                .unwrap_or_default()
                .to_owned(),
        }),
        _ => None,
    }
}

/// 도구 항목이 아니면 `None`.
pub(super) fn tool_output(item: &Value) -> Option<String> {
    activity_of(item)?;
    let output = match item["type"].as_str()? {
        "commandExecution" => item["aggregatedOutput"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mcpToolCall" => match &item["result"] {
            Value::Null => item["error"].to_string(),
            result => result.to_string(),
        },
        "fileChange" => patch_text(item),
        _ => String::new(),
    };
    Some(output)
}

// cost: time O(a + p), heap O(a + p), stack O(1), alloc 1
// vars: a = commandActions 수, p = 패치 글자 수
// basis: estimate
/// `commandActions`의 경로와 `fileChange`의 경로, 바뀐 줄 수를 옮긴다. 다른 항목은 기본값이다.
pub(super) fn detail_of(item: &Value) -> ToolDetail {
    match item["type"].as_str() {
        Some("commandExecution") => {
            let actions = item["commandActions"].as_array();
            let paths = actions
                .into_iter()
                .flatten()
                .filter_map(|action| action["path"].as_str().map(str::to_owned))
                .collect();
            let command = unwrap_shell(item["command"].as_str().unwrap_or_default());
            let category = if matches!(activity_of(item), Some(Activity::ReadingFile)) {
                ToolCategory::FileRead
            } else {
                classify_command(&command)
            };
            ToolDetail {
                category,
                paths,
                ..ToolDetail::default()
            }
        }
        Some("fileChange") => {
            let changes = item["changes"].as_array().map_or(&[][..], Vec::as_slice);
            let total = changes.iter().map(change_lines).fold(
                LineChange {
                    added: 0,
                    removed: 0,
                },
                |total, change| LineChange {
                    added: total.added.saturating_add(change.added),
                    removed: total.removed.saturating_add(change.removed),
                },
            );
            ToolDetail {
                category: ToolCategory::FileEdit,
                paths: changes
                    .iter()
                    .filter_map(|change| change["path"].as_str().map(str::to_owned))
                    .collect(),
                read_lines: None,
                changed: Some(total),
            }
        }
        Some("reasoning") => ToolDetail {
            category: ToolCategory::Reasoning,
            ..ToolDetail::default()
        },
        _ => ToolDetail::default(),
    }
}

// cost: time O(d), heap O(1), stack O(1)
// vars: d = diff 글자 수
// basis: estimate
/// 새 파일은 글 전체를 더한 줄로, 지운 파일은 지운 줄로 세고, 고친 파일은 diff의 `+`와 `-` 줄을 센다.
pub(super) fn change_lines(change: &Value) -> LineChange {
    let diff = change["diff"].as_str().unwrap_or_default();
    let lines = || u32::try_from(diff.lines().count()).unwrap_or(u32::MAX);
    match change["kind"]["type"].as_str() {
        Some("add") => LineChange {
            added: lines(),
            removed: 0,
        },
        Some("delete") => LineChange {
            added: 0,
            removed: lines(),
        },
        _ => {
            let count = |sign: &str, header: &str| {
                let count = diff
                    .lines()
                    .filter(|line| line.starts_with(sign) && !line.starts_with(header))
                    .count();
                u32::try_from(count).unwrap_or(u32::MAX)
            };
            LineChange {
                added: count("+", "+++"),
                removed: count("-", "---"),
            }
        }
    }
}

/// 파일마다 경로 줄과 diff를 이어 붙인다.
pub(super) fn patch_text(item: &Value) -> String {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|change| {
            format!(
                "{}\n{}",
                change["path"].as_str().unwrap_or_default(),
                change["diff"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 명령이 코드로 끝났을 때만 값이 있다.
pub(super) fn exit_code_of(item: &Value) -> Option<i32> {
    if item["type"] != "commandExecution" {
        return None;
    }
    i32::try_from(item["exitCode"].as_i64()?).ok()
}

/// 새 입력은 캐시 포함 입력에서 캐시 읽기를 뺀 값이다.
pub(super) fn cumulative_usage(
    agent: AgentId,
    subagent: Option<SubagentId>,
    model: Option<String>,
    total: &Value,
) -> UsageReport {
    let input = total["inputTokens"].as_u64();
    let cache_read = total["cachedInputTokens"].as_u64();
    UsageReport {
        agent,
        subagent,
        model,
        scope: UsageScope::ThreadCumulative,
        input: input.map(|input| input.saturating_sub(cache_read.unwrap_or(0))),
        cache_read,
        cache_write: total["cacheWriteInputTokens"].as_u64(),
        output: total["outputTokens"].as_u64(),
        reasoning: total["reasoningOutputTokens"].as_u64(),
    }
}

/// 자식 `thread_id`를 그대로 쓴다.
pub(super) fn subagent_id(thread: &ProviderSessionId) -> SubagentId {
    SubagentId(thread.0.clone())
}
