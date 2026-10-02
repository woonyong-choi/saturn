use std::collections::HashMap;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::ProviderSessionId;
use serde_json::Value;

use super::ThreadState;
use super::convert::subagent_id;
use crate::providers::AppliedSettings;

/// 부모를 모르거나 이미 있으면 빈 목록.
pub(super) fn register_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    thread: &Value,
) -> Vec<ProviderEvent> {
    let (Some(id), Some(parent)) = (thread["id"].as_str(), parent_thread_id(thread)) else {
        return Vec::new();
    };
    let child = ProviderSessionId(id.to_owned());
    let parent = ProviderSessionId(parent.to_owned());
    if threads.contains_key(&child) {
        return Vec::new();
    }
    let Some(parent_state) = threads.get(&parent) else {
        return Vec::new();
    };
    let agent = parent_state.agent;
    let parent_subagent = parent_state.parent.as_ref().map(|_| subagent_id(&parent));
    let applied = AppliedSettings {
        model: thread["model"].as_str().map(str::to_owned),
        permission: None,
    };
    threads.insert(
        child.clone(),
        ThreadState::new(agent, Some(parent), applied),
    );
    vec![ProviderEvent::SubagentStarted {
        agent,
        subagent: subagent_id(&child),
        parent: parent_subagent,
    }]
}

/// 턴이 진행 중이었으면 끝으로 보고, 메인 thread는 그대로 둔다.
pub(super) fn close_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    thread: &ProviderSessionId,
) -> Vec<ProviderEvent> {
    let Some(state) = threads.get(thread) else {
        return Vec::new();
    };
    if state.parent.is_none() {
        return Vec::new();
    }
    let ended = state
        .active_turn
        .is_some()
        .then(|| ProviderEvent::SubagentEnded {
            agent: state.agent,
            subagent: subagent_id(thread),
        });
    threads.remove(thread);
    ended.into_iter().collect()
}

/// `parentThreadId`, 없으면 `source.subAgent.thread_spawn.parent_thread_id`.
pub(super) fn parent_thread_id(thread: &Value) -> Option<&str> {
    thread["parentThreadId"]
        .as_str()
        .or_else(|| thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].as_str())
}

pub(super) fn remove_thread_tree(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    root: &ProviderSessionId,
) {
    let mut doomed = vec![root.clone()];
    let mut index = 0;
    while let Some(current) = doomed.get(index).cloned() {
        doomed.extend(
            threads
                .iter()
                .filter(|(_, state)| state.parent.as_ref() == Some(&current))
                .map(|(id, _)| id.clone()),
        );
        index += 1;
    }
    for id in doomed {
        threads.remove(&id);
    }
}
