//! 백그라운드 subagent의 수명. `Agent` 도구 결과는 시작 접수(`async_launched`)일 수 있어 끝으로 보지 않고,
//! `system`의 `task_*`와 `background_tasks_changed`로 실제 끝을 확인한다.
//! 설계: docs/design/providers-and-sessions.md

use std::time::{Duration, Instant};

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::SubagentId;
use serde_json::Value;

use super::SessionState;

/// subagent 작업이 끝난 뒤 이 시간 안에 같은 subagent의 새 작업이나 소유 작업이 시작하지 않으면 끝으로 낸다.
/// 소유한 셸이 끝나면 약 10밀리초 안에 같은 `tool_use_id`가 다시 `task_started`로 시작한다(실측). 초안 값.
pub(super) const SETTLE: Duration = Duration::from_secs(1);

/// 끝으로 보는 `task_notification`의 `status`. 실측한 값은 `completed`와 `stopped`다.
const ENDED_STATUSES: &[&str] = &["completed", "failed", "stopped", "killed"];

/// 백그라운드로 시작한 subagent 하나.
#[derive(Debug, Default)]
pub(super) struct Background {
    /// 끝을 알리는 `task_notification`을 받았다. 새 `task_started`가 오면 거짓으로 돌아간다.
    notified: bool,
    /// 이 시각이 지나도 그대로이면 끝으로 낸다.
    settle_at: Option<Instant>,
}

/// 도구 결과가 시작 접수(`async_launched`)인가.
pub(super) fn is_async_launch(line: &Value) -> bool {
    line["tool_use_result"]["status"] == "async_launched"
}

impl SessionState {
    /// 도구 결과가 시작 접수라면 subagent를 백그라운드로 올리고 끝내지 않는다.
    pub(super) fn mark_background(&mut self, subagent: &SubagentId) {
        self.background.entry(subagent.clone()).or_default();
    }

    /// 흐름이 끊기거나 오류로 끝나 더 관찰할 수 없을 때.
    pub(super) fn clear_subagents(&mut self) {
        self.running.clear();
        self.background.clear();
        self.owned_tasks.clear();
    }

    /// 가장 가까운 끝 확정 시각. 없으면 기다릴 것이 없다.
    pub(super) fn next_settle(&self) -> Option<Instant> {
        self.background
            .values()
            .filter_map(|background| background.settle_at)
            .min()
    }
}

/// `task_started`, `task_notification`, `background_tasks_changed`만 읽고 그 밖의 `system` 줄은 버린다.
pub(super) fn on_system(
    state: &mut SessionState,
    line: &Value,
    now: Instant,
) -> Vec<ProviderEvent> {
    match line["subtype"].as_str() {
        Some("task_started") => on_task_started(state, line),
        Some("task_notification") => {
            on_task_notification(state, line);
            Vec::new()
        }
        Some("background_tasks_changed") => {
            let live: Vec<&str> = line["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|task| task["task_id"].as_str())
                .collect();
            state.owned_tasks.retain(|id| live.contains(&id.as_str()));
            Vec::new()
        }
        _ => return Vec::new(),
    }
    .into_iter()
    .chain(settle(state, now))
    .collect()
}

fn on_task_started(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let backgrounded = line["is_backgrounded"] == true;
    let Some(task_id) = line["task_id"].as_str() else {
        return Vec::new();
    };
    match line["task_type"].as_str() {
        Some("local_agent") => {
            let Some(id) = line["tool_use_id"].as_str() else {
                return Vec::new();
            };
            let subagent = SubagentId(id.to_owned());
            if backgrounded {
                state.mark_background(&subagent);
            }
            // 끝난 subagent가 소유한 작업이 끝나 같은 `tool_use_id`로 다시 시작하는 경우
            let restarted = state.running.insert(subagent.clone(), None).is_none();
            if let Some(background) = state.background.get_mut(&subagent) {
                *background = Background::default();
            }
            if restarted {
                return vec![ProviderEvent::SubagentStarted {
                    agent: state.agent,
                    subagent,
                    parent: None,
                }];
            }
        }
        Some(_) if line["owned_by_subagent"] == true && backgrounded => {
            state.owned_tasks.insert(task_id.to_owned());
        }
        _ => {}
    }
    Vec::new()
}

fn on_task_notification(state: &mut SessionState, line: &Value) {
    if let Some(task_id) = line["task_id"].as_str() {
        state.owned_tasks.remove(task_id);
    }
    let ended = line["status"]
        .as_str()
        .is_some_and(|status| ENDED_STATUSES.contains(&status));
    let Some(id) = line["tool_use_id"].as_str() else {
        return;
    };
    let subagent = SubagentId(id.to_owned());
    if !ended || !state.running.contains_key(&subagent) {
        return;
    }
    if let Some(background) = state.background.get_mut(&subagent) {
        background.notified = true;
    }
}

/// 끝을 확정할 수 있는 subagent의 시각을 정하고, 지난 것은 끝으로 낸다.
/// 소유한 백그라운드 작업이 남아 있으면 그 subagent들은 아직 일이 남은 것이다.
fn settle(state: &mut SessionState, now: Instant) -> Vec<ProviderEvent> {
    let work_left = !state.owned_tasks.is_empty();
    for background in state.background.values_mut() {
        background.settle_at = match (background.notified && !work_left, background.settle_at) {
            (true, None) => Some(now + SETTLE),
            (true, at) => at,
            (false, _) => None,
        };
    }
    settle_due(state, now)
}

/// 확정 시각이 지난 subagent를 끝으로 낸다. 호출자는 `next_settle` 시각에 부른다.
pub(super) fn settle_due(state: &mut SessionState, now: Instant) -> Vec<ProviderEvent> {
    let mut due: Vec<SubagentId> = state
        .background
        .iter()
        .filter(|(_, background)| background.settle_at.is_some_and(|at| at <= now))
        .map(|(subagent, _)| subagent.clone())
        .collect();
    due.sort_by(|left, right| left.0.cmp(&right.0));
    due.into_iter()
        .map(|subagent| {
            state.background.remove(&subagent);
            state.running.remove(&subagent);
            ProviderEvent::SubagentEnded {
                agent: state.agent,
                subagent,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
