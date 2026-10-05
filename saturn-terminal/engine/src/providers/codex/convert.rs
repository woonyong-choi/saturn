use std::collections::HashMap;

use saturn_protocol::event::{
    Activity, LineChange, ProviderEvent, ToolCategory, ToolDetail, UsageReport, UsageScope,
};
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use serde_json::{Value, json};

use super::input::{self, InputKind};
use super::permission::{call_of, file_change_paths, runs_outside_sandbox};
use super::threads::{Held, HeldEvents, close_child, register_child, register_spawned};
use super::{
    APPROVAL_METHODS, ELICITATION_METHOD, PERMISSIONS_METHOD, PendingApproval, PendingInput,
    ThreadState, USER_INPUT_METHOD, value_text,
};
use crate::providers::tool_detail::{classify_command, unwrap_shell};

/// 한 턴의 두 메시지 사이에 넣는 구분(빈 줄 하나).
const MESSAGE_SEPARATOR: &str = "\n\n";

/// 권한은 Saturn 규칙이 정하므로 권한 인자는 넣지 않는다. 승인 정책과 샌드박스는 `thread/start`가 정한다.
/// 숨긴 모델이거나 모델 이름이 없으면 `None`.
pub(super) fn model_info(entry: &Value) -> Option<ModelInfo> {
    if entry["hidden"].as_bool() == Some(true) {
        return None;
    }
    let model = entry["model"].as_str().or_else(|| entry["id"].as_str())?;
    Some(ModelInfo {
        choice: ModelChoice {
            provider: super::adapter::ID,
            model: model.to_owned(),
        },
        name: entry["displayName"].as_str().unwrap_or(model).to_owned(),
    })
}

/// 변환하는 알림 이름. 부모 관계를 모르는 thread의 알림 가운데 이 알림만 쥐어 둔다. `thread/closed`도 쥐어 두었다가
/// 자식을 등록한 직후 처리해, 등록 전에 닫힌 자식이 끝나지 않은 채 남지 않게 한다.
const CONVERTED_METHODS: [&str; 8] = [
    "thread/closed",
    "turn/started",
    "turn/completed",
    "item/agentMessage/delta",
    "item/started",
    "item/completed",
    "thread/tokenUsage/updated",
    "thread/settings/updated",
];

/// 자식 thread 등록과 `active_turn` 갱신도 여기서 한다. 자식은 `thread/started`의 부모 thread나 부모의 `spawnAgent`
/// 완료 항목으로 등록하고, 등록 전에 온 자식의 알림은 `held`에 쥐어 두었다가 등록 직후 받은 순서대로 처리한다.
/// 버릴 알림이면 빈 목록.
pub(super) fn convert_notification(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    held: &mut HeldEvents,
    method: &str,
    params: &serde_json::Value,
) -> Vec<ProviderEvent> {
    if method == "thread/started" {
        let started = register_child(threads, &params["thread"]);
        let Some(id) = params["thread"]["id"]
            .as_str()
            .filter(|_| !started.is_empty())
        else {
            return started;
        };
        let mut events = started;
        events.extend(replay_held(
            threads,
            held,
            &ProviderSessionId(id.to_owned()),
        ));
        return events;
    }
    let Some(thread_id) = params["threadId"].as_str() else {
        return Vec::new();
    };
    let thread = ProviderSessionId(thread_id.to_owned());
    if method == "thread/closed" && threads.contains_key(&thread) {
        return close_child(threads, &thread);
    }
    if !threads.contains_key(&thread) {
        if CONVERTED_METHODS.contains(&method) {
            held.hold(&thread, method, params);
        }
        return Vec::new();
    }
    let mut events = convert_known(threads, method, params, &thread);
    if method == "item/completed" {
        for (child, started) in register_spawned(threads, &thread, &params["item"]) {
            events.push(started);
            events.extend(replay_held(threads, held, &child));
        }
    }
    events
}

/// 등록한 자식이 부모 관계 확인 전에 낸 알림을 받은 순서대로 처리한다. 자식이 낸 `spawnAgent` 완료 항목도
/// 같은 경로를 지나 손자를 등록한다.
fn replay_held(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    held: &mut HeldEvents,
    child: &ProviderSessionId,
) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    for item in held.take(child) {
        match item {
            Held::Notice { method, params } => {
                events.extend(convert_notification(threads, held, &method, &params));
            }
            Held::Request { method, id, params } => {
                if let Some((request_id, pending, event)) =
                    build_request(threads, &method, &id, &params)
                {
                    held.release(request_id, pending);
                    events.push(event);
                }
            }
        }
    }
    events
}

fn convert_known(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    params: &serde_json::Value,
    thread: &ProviderSessionId,
) -> Vec<ProviderEvent> {
    let Some(state) = threads.get_mut(thread) else {
        return Vec::new();
    };
    let agent = state.agent;
    let subagent = state.parent.as_ref().map(|_| subagent_id(thread));
    match method {
        "turn/started" => on_turn_started(state, params),
        "turn/completed" => on_turn_completed(state, params, agent, subagent),
        "item/agentMessage/delta" => on_message_delta(state, params, agent, subagent),
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

/// 메시지 항목(`itemId`)이 앞 조각과 달라지면 새 메시지이므로 빈 줄을 앞에 붙여 두 메시지가 이어 붙지 않게 한다.
/// `itemId`가 없는 조각은 그대로 둔다.
fn on_message_delta(
    state: &mut ThreadState,
    params: &Value,
    agent: AgentId,
    subagent: Option<SubagentId>,
) -> Vec<ProviderEvent> {
    let Some(delta) = params["delta"].as_str() else {
        return Vec::new();
    };
    let mut text = String::new();
    if let Some(item) = params["itemId"].as_str() {
        if state
            .message_item
            .as_deref()
            .is_some_and(|last| last != item)
        {
            text.push_str(MESSAGE_SEPARATOR);
        }
        state.message_item = Some(item.to_owned());
    }
    text.push_str(delta);
    vec![ProviderEvent::Text {
        agent,
        subagent,
        text,
    }]
}

fn on_turn_started(state: &mut ThreadState, params: &Value) -> Vec<ProviderEvent> {
    state.active_turn = params["turn"]["id"].as_str().map(str::to_owned);
    state.message_item = None;
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
        state.subagent_open = false;
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
    held: &mut HeldEvents,
    method: &str,
    id: &Value,
    params: &Value,
) -> Vec<ProviderEvent> {
    if !is_request_method(method, params) {
        return Vec::new();
    }
    if let Some(thread) = request_thread(params)
        && !threads.contains_key(&thread)
    {
        // 자식이 부모의 `spawnAgent` 완료 항목보다 먼저 물은 요청이다. 응답 없이 버리면 provider가 멈추므로 쥐어 둔다
        held.hold_request(&thread, method, id, params);
        return Vec::new();
    }
    build_request(threads, method, id, params)
        .map(|(request_id, pending, event)| {
            approvals.insert(request_id, pending);
            event
        })
        .into_iter()
        .collect()
}

/// 이 연결이 사용자에게 올리는 서버 요청(승인, 입력 요청)인지.
fn is_request_method(method: &str, params: &Value) -> bool {
    input_kind(method, params).is_some() || APPROVAL_METHODS.iter().any(|(name, _)| *name == method)
}

/// 요청이 속한 thread. `threadId`, 없으면 `conversationId`.
fn request_thread(params: &Value) -> Option<ProviderSessionId> {
    params["threadId"]
        .as_str()
        .or_else(|| params["conversationId"].as_str())
        .map(|id| ProviderSessionId(id.to_owned()))
}

/// 등록된 thread의 요청을 올릴 이벤트와 답을 기다릴 기록으로 바꾼다. 읽을 수 없는 요청이면 `None`.
pub(super) fn build_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    id: &Value,
    params: &Value,
) -> Option<(String, PendingApproval, ProviderEvent)> {
    if let Some(kind) = input_kind(method, params) {
        return convert_input_request(threads, kind, id, params);
    }
    let (_, summary) = APPROVAL_METHODS.iter().find(|(name, _)| *name == method)?;
    let state = thread_of_request(threads, params)?;
    let outside = if params["command"].is_string() && runs_outside_sandbox(params) {
        " outside sandbox"
    } else {
        ""
    };
    let summary = params["command"]
        .as_str()
        .map(|command| format!("{summary}{outside}: {command}"))
        .or_else(|| {
            (method == ELICITATION_METHOD)
                .then(|| params["message"].as_str().map(str::to_owned))
                .flatten()
        })
        .unwrap_or_else(|| (*summary).to_owned());
    let request_id = value_text(id).unwrap_or_default();
    let pending = PendingApproval {
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
    };
    let event = ProviderEvent::PermissionRequested {
        agent: state.agent,
        request_id: request_id.clone(),
        summary,
        reason: params["reason"].as_str().unwrap_or_default().to_owned(),
        call: call_of(method, params, &state.file_changes),
    };
    Some((request_id, pending, event))
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
    kind: InputKind,
    id: &Value,
    params: &Value,
) -> Option<(String, PendingApproval, ProviderEvent)> {
    let state = thread_of_request(threads, params)?;
    let request = match kind {
        InputKind::Elicitation => input::elicitation_request(params)?,
        InputKind::UserInput => input::user_input_request(params),
    };
    let request_id = value_text(id).unwrap_or_default();
    let pending = PendingApproval {
        id: id.clone(),
        method: request_method(kind).to_owned(),
        available: None,
        permissions: Value::Null,
        input: Some(PendingInput {
            kind,
            request: request.clone(),
        }),
    };
    let event = ProviderEvent::InputRequested {
        agent: state.agent,
        request_id: request_id.clone(),
        request,
    };
    Some((request_id, pending, event))
}

/// 한도에서 밀려나 처리하지 못한 요청에 보낼 거절 응답. 승인은 거절 결정으로, 입력 요청은 오류 응답으로 답해
/// provider가 답을 기다리며 멈추지 않게 한다. 쥐어 둔 요청이 아니면 `None`.
pub(super) fn rejection(item: &Held) -> Option<Value> {
    let Held::Request { method, id, params } = item else {
        return None;
    };
    if input_kind(method, params).is_some() {
        return Some(json!({
            "id": id,
            "error": { "code": -32000, "message": "the thread of this request is not known" },
        }));
    }
    let pending = PendingApproval {
        id: id.clone(),
        method: method.clone(),
        available: params["availableDecisions"].as_array().map(|decisions| {
            decisions
                .iter()
                .filter_map(|decision| decision.as_str().map(str::to_owned))
                .collect()
        }),
        permissions: params["permissions"].clone(),
        input: None,
    };
    Some(json!({
        "id": id,
        "result": approval_result(&pending, &PermissionAnswer::Deny { note: None }),
    }))
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
