use std::collections::HashMap;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::ProviderSessionId;
use serde_json::Value;

use super::ThreadState;
use super::convert::subagent_id;
use crate::providers::AppliedSettings;

/// 한꺼번에 쥐어 둘 부모를 모르는 thread 수. 넘으면 가장 오래된 thread부터 버린다.
pub(super) const HELD_THREADS: usize = 8;
/// thread 하나에서 쥐어 둘 알림 수. 넘으면 가장 오래된 알림부터 버린다.
pub(super) const HELD_PER_THREAD: usize = 64;

/// 부모 관계가 아직 확인되지 않은 thread의 알림을 한도 안에서 쥐어 둔다. codex-cli 0.158.0은 자식의 첫 알림을
/// 부모의 `spawnAgent` 완료 항목보다 먼저 보낸다. 관계가 확인되면 받은 순서대로 한 번만 꺼내 처리하고, 끝내
/// 확인되지 않는 thread의 알림은 한도에서 밀려나 버려진다.
#[derive(Debug, Default)]
pub(super) struct HeldEvents {
    /// 오래된 순서.
    order: Vec<ProviderSessionId>,
    held: HashMap<ProviderSessionId, Vec<(String, Value)>>,
}

impl HeldEvents {
    pub(super) fn hold(&mut self, thread: &ProviderSessionId, method: &str, params: &Value) {
        if !self.held.contains_key(thread) {
            if self.order.len() >= HELD_THREADS {
                let oldest = self.order.remove(0);
                self.held.remove(&oldest);
            }
            self.order.push(thread.clone());
        }
        let notes = self.held.entry(thread.clone()).or_default();
        if notes.len() >= HELD_PER_THREAD {
            notes.remove(0);
        }
        notes.push((method.to_owned(), params.clone()));
    }

    /// 쥐어 둔 알림을 받은 순서대로 꺼내고 비운다.
    pub(super) fn take(&mut self, thread: &ProviderSessionId) -> Vec<(String, Value)> {
        self.order.retain(|held| held != thread);
        self.held.remove(thread).unwrap_or_default()
    }
}

/// 부모를 모르거나 이미 있으면 빈 목록.
pub(super) fn register_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    thread: &Value,
) -> Vec<ProviderEvent> {
    let (Some(id), Some(parent)) = (thread["id"].as_str(), parent_thread_id(thread)) else {
        return Vec::new();
    };
    adopt_child(
        threads,
        &ProviderSessionId(id.to_owned()),
        &ProviderSessionId(parent.to_owned()),
        thread["model"].as_str(),
    )
}

/// 부모가 이미 등록돼 있고 자식은 처음 볼 때만 자식으로 등록한다. 부모를 모르거나 이미 있으면 빈 목록.
fn adopt_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    child: &ProviderSessionId,
    parent: &ProviderSessionId,
    model: Option<&str>,
) -> Vec<ProviderEvent> {
    if threads.contains_key(child) {
        return Vec::new();
    }
    let Some(parent_state) = threads.get(parent) else {
        return Vec::new();
    };
    let agent = parent_state.agent;
    let parent_subagent = parent_state.parent.as_ref().map(|_| subagent_id(parent));
    let applied = AppliedSettings {
        model: model.map(str::to_owned),
        permission: None,
    };
    threads.insert(
        child.clone(),
        ThreadState::new(agent, Some(parent.clone()), applied),
    );
    vec![ProviderEvent::SubagentStarted {
        agent,
        subagent: subagent_id(child),
        parent: parent_subagent,
    }]
}

/// 부모의 `spawnAgent` 완료 항목이 알린 자식을 등록한다. 항목을 낸 thread가 이미 등록돼 있고 항목의 보낸 thread도
/// 그 thread일 때만 믿는다. 처음 보는 `threadId`라는 이유만으로 자식으로 붙이지 않는다. 새로 등록한 자식의 id를
/// 시작 이벤트와 함께 돌려준다.
pub(super) fn register_spawned(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    host: &ProviderSessionId,
    item: &Value,
) -> Vec<(ProviderSessionId, ProviderEvent)> {
    let is_spawn = item["type"].as_str() == Some("collabAgentToolCall")
        && item["tool"].as_str() == Some("spawnAgent");
    let sender = item["senderThreadId"].as_str().unwrap_or(&host.0);
    if !is_spawn || sender != host.0 {
        return Vec::new();
    }
    let receivers = item["receiverThreadIds"].as_array().into_iter().flatten();
    let mut started = Vec::new();
    for receiver in receivers.filter_map(Value::as_str) {
        let child = ProviderSessionId(receiver.to_owned());
        let events = adopt_child(threads, &child, host, item["model"].as_str());
        started.extend(events.into_iter().map(|event| (child.clone(), event)));
    }
    started
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
