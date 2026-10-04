use std::collections::HashMap;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::ProviderSessionId;
use serde_json::Value;

use super::convert::subagent_id;
use super::{PendingApproval, ThreadState};
use crate::providers::AppliedSettings;

/// 한꺼번에 쥐어 둘 부모를 모르는 thread 수. 넘으면 가장 오래된 thread부터 버린다.
pub(super) const HELD_THREADS: usize = 8;
/// thread 하나에서 쥐어 둘 알림 수. 넘으면 가장 오래된 알림부터 버린다.
pub(super) const HELD_PER_THREAD: usize = 64;

/// 쥐어 둔 항목 하나. 알림과 서버 요청을 받은 순서대로 한 줄에 둔다.
#[derive(Debug, Clone)]
pub(super) enum Held {
    Notice {
        method: String,
        params: Value,
    },
    /// 답을 기다리는 서버 요청. `id`는 응답에 그대로 돌려줘야 한다.
    Request {
        method: String,
        id: Value,
        params: Value,
    },
}

/// 부모 관계가 아직 확인되지 않은 thread의 알림과 서버 요청을 한도 안에서 쥐어 둔다. codex-cli 0.158.0은 자식의 첫
/// 알림을 부모의 `spawnAgent` 완료 항목보다 먼저 보낸다. 관계가 확인되면 받은 순서대로 한 번만 꺼내 처리하고, 끝내
/// 확인되지 않는 thread의 항목은 한도에서 밀려난다. 밀려난 서버 요청은 응답 없이 두면 provider가 멈추므로
/// `dropped`에 모아 호출자가 거절 응답을 보낸다.
#[derive(Debug, Default)]
pub(super) struct HeldEvents {
    /// 오래된 순서.
    order: Vec<ProviderSessionId>,
    held: HashMap<ProviderSessionId, Vec<Held>>,
    /// 다시 처리하다 올린 승인과 입력 요청. 호출자가 `approvals`에 넣는다.
    released: Vec<(String, PendingApproval)>,
    /// 한도에서 밀려난 서버 요청.
    dropped: Vec<Held>,
}

impl HeldEvents {
    pub(super) fn hold(&mut self, thread: &ProviderSessionId, method: &str, params: &Value) {
        self.push(
            thread,
            Held::Notice {
                method: method.to_owned(),
                params: params.clone(),
            },
        );
    }

    pub(super) fn hold_request(
        &mut self,
        thread: &ProviderSessionId,
        method: &str,
        id: &Value,
        params: &Value,
    ) {
        self.push(
            thread,
            Held::Request {
                method: method.to_owned(),
                id: id.clone(),
                params: params.clone(),
            },
        );
    }

    fn push(&mut self, thread: &ProviderSessionId, item: Held) {
        if !self.held.contains_key(thread) {
            if self.order.len() >= HELD_THREADS {
                let oldest = self.order.remove(0);
                self.drop_all(&oldest);
            }
            self.order.push(thread.clone());
        }
        let notes = self.held.entry(thread.clone()).or_default();
        if notes.len() >= HELD_PER_THREAD {
            let evicted = notes.remove(0);
            if matches!(evicted, Held::Request { .. }) {
                self.dropped.push(evicted);
            }
        }
        notes.push(item);
    }

    fn drop_all(&mut self, thread: &ProviderSessionId) {
        let evicted = self.held.remove(thread).unwrap_or_default();
        self.dropped.extend(
            evicted
                .into_iter()
                .filter(|item| matches!(item, Held::Request { .. })),
        );
    }

    /// 쥐어 둔 항목을 받은 순서대로 꺼내고 비운다.
    pub(super) fn take(&mut self, thread: &ProviderSessionId) -> Vec<Held> {
        self.order.retain(|held| held != thread);
        self.held.remove(thread).unwrap_or_default()
    }

    pub(super) fn release(&mut self, request_id: String, pending: PendingApproval) {
        self.released.push((request_id, pending));
    }

    /// 다시 처리하다 올라온 요청과 한도에서 밀려난 요청을 꺼낸다.
    pub(super) fn drain_requests(&mut self) -> (Vec<(String, PendingApproval)>, Vec<Held>) {
        (
            std::mem::take(&mut self.released),
            std::mem::take(&mut self.dropped),
        )
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
    let mut state = ThreadState::new(agent, Some(parent.clone()), applied);
    state.subagent_open = true;
    threads.insert(child.clone(), state);
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

/// 시작을 알렸고 종료는 아직 알리지 않은 자식이면 끝으로 보고, 메인 thread는 그대로 둔다. 턴이 있었는지와 무관하게
/// 시작과 종료를 짝으로 알려야 하위 트리가 유휴가 된다.
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
    let ended = state.subagent_open.then(|| ProviderEvent::SubagentEnded {
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
