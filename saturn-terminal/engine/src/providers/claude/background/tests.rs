//! 백그라운드 subagent 수명 시험. 이벤트 순서는 실측(`docs/experiments/claude-provider-behavior`의 `bg_none` 1회차)에서 옮겼다.

use std::time::{Duration, Instant};

use saturn_core::agents::{AgentTracker, TreeStatus};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, SubagentId};
use serde_json::{Value, json};

use super::{SETTLE, settle_due};
use crate::providers::claude::SessionState;
use crate::providers::claude::convert::convert_line;

const AGENT: AgentId = AgentId(1);
const FIXTURE: &str = include_str!("../fixtures/background-agent.jsonl");
/// 실측의 `Agent` 도구 호출 id.
const LAUNCH: &str = "toolu_01MtvFGjeAmb8bEAbAiUGQ2G";

fn subagent() -> SubagentId {
    SubagentId(LAUNCH.to_owned())
}

fn lines() -> Vec<Value> {
    FIXTURE
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn is_subtype(line: &Value, subtype: &str) -> bool {
    line["type"] == "system" && line["subtype"] == subtype
}

fn ended(events: &[ProviderEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::SubagentEnded { .. }))
        .count()
}

fn started(events: &[ProviderEvent]) -> usize {
    events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::SubagentStarted { .. }))
        .count()
}

fn feed(state: &mut SessionState, lines: &[Value]) -> Vec<ProviderEvent> {
    lines
        .iter()
        .flat_map(|line| convert_line(state, line))
        .collect()
}

/// 확정 시각이 충분히 지난 때의 정리.
fn settled(state: &mut SessionState) -> Vec<ProviderEvent> {
    settle_due(state, Instant::now() + SETTLE + Duration::from_secs(1))
}

/// 첫 `result`까지(시작 접수 직후)의 줄 수.
fn until_first_result(lines: &[Value]) -> usize {
    lines
        .iter()
        .position(|line| line["type"] == "result")
        .unwrap()
        + 1
}

#[test]
fn the_launch_result_does_not_end_the_background_subagent() {
    let lines = lines();
    let mut state = SessionState::new(AGENT);

    let events = feed(&mut state, &lines[..until_first_result(&lines)]);

    assert_eq!(started(&events), 1);
    assert_eq!(ended(&events), 0);
    assert!(state.running.contains_key(&subagent()));
    assert_eq!(ended(&settled(&mut state)), 0);
}

#[test]
fn the_subagent_stays_running_after_the_parent_result_and_ends_once_at_the_real_end() {
    let lines = lines();
    let mut state = SessionState::new(AGENT);
    let mut tracker = AgentTracker::new();
    let mut seen = Vec::new();
    let mut statuses = Vec::new();
    for line in &lines {
        for event in convert_line(&mut state, line) {
            statuses.push((
                matches!(event, ProviderEvent::TurnCompleted { .. }),
                tracker.on_event(&event),
            ));
            seen.push(event);
        }
    }

    // 모든 `result`는 자식이 남은 동안 왔으므로 트리는 유휴가 아니다
    let completed: Vec<_> = statuses.iter().filter(|(done, _)| *done).collect();
    assert_eq!(completed.len(), 3);
    assert!(
        completed
            .iter()
            .all(|(_, status)| *status == TreeStatus::AnsweredTreeRunning)
    );
    // 마지막 `task_notification`까지 읽고도 확정 시각 전이므로 아직 끝나지 않았다
    assert_eq!(ended(&seen), 0);
    assert!(!tracker.is_tree_idle(AGENT));

    let tail = settled(&mut state);
    for event in &tail {
        tracker.on_event(event);
    }

    assert_eq!(started(&seen), 1);
    assert_eq!(ended(&tail), 1);
    assert!(tracker.is_tree_idle(AGENT));
    assert!(state.running.is_empty());
    assert!(state.background.is_empty());
}

#[test]
fn the_first_completed_notice_is_not_the_end_while_its_shell_runs() {
    let lines = lines();
    let first_notice = lines
        .iter()
        .position(|line| {
            is_subtype(line, "task_notification") && line["task_id"] == "aceaa446a8a12da05"
        })
        .unwrap();
    let mut state = SessionState::new(AGENT);

    feed(&mut state, &lines[..=first_notice]);

    assert!(state.running.contains_key(&subagent()));
    assert_eq!(state.next_settle(), None);
    assert_eq!(ended(&settled(&mut state)), 0);
}

#[test]
fn a_restart_inside_the_settle_window_cancels_the_end() {
    let lines = lines();
    let shell_notice = lines
        .iter()
        .position(|line| is_subtype(line, "task_notification") && line["task_id"] == "b9geaatop")
        .unwrap();
    let mut state = SessionState::new(AGENT);

    feed(&mut state, &lines[..=shell_notice]);
    // 셸이 끝나 소유 작업이 없고 subagent는 이미 끝났다고 알렸으니 끝 확정을 기다리는 중이다
    assert!(state.next_settle().is_some());
    let restart = lines[shell_notice + 1..]
        .iter()
        .position(|line| is_subtype(line, "task_started"))
        .unwrap();
    let events = feed(
        &mut state,
        &lines[shell_notice + 1..=shell_notice + 1 + restart],
    );

    assert_eq!(started(&events), 0);
    assert_eq!(state.next_settle(), None);
    assert_eq!(ended(&settled(&mut state)), 0);
}

#[test]
fn a_restart_after_the_end_starts_the_subagent_again() {
    let lines = lines();
    let shell_notice = lines
        .iter()
        .position(|line| is_subtype(line, "task_notification") && line["task_id"] == "b9geaatop")
        .unwrap();
    let mut state = SessionState::new(AGENT);
    feed(&mut state, &lines[..=shell_notice]);
    assert_eq!(ended(&settled(&mut state)), 1);

    let restart = lines[shell_notice + 1..]
        .iter()
        .position(|line| is_subtype(line, "task_started"))
        .unwrap();
    let events = feed(
        &mut state,
        &lines[shell_notice + 1..=shell_notice + 1 + restart],
    );

    assert_eq!(started(&events), 1);
    assert!(state.running.contains_key(&subagent()));
}

#[test]
fn duplicate_and_early_notices_end_the_subagent_once() {
    let lines = lines();
    let notice = lines
        .iter()
        .rev()
        .find(|line| {
            is_subtype(line, "task_notification") && line["task_id"] == "aceaa446a8a12da05"
        })
        .unwrap()
        .clone();
    let mut state = SessionState::new(AGENT);
    // 시작 접수 결과보다 끝 알림이 먼저 오고, 알림이 두 번 온다
    let launch = lines[..4].to_vec();
    let mut reordered = vec![
        launch[0].clone(),
        launch[2].clone(),
        notice.clone(),
        notice,
        launch[3].clone(),
    ];
    reordered.push(json!({"type": "system", "subtype": "background_tasks_changed", "tasks": []}));
    let mut events = feed(&mut state, &reordered);
    events.extend(settled(&mut state));
    events.extend(feed(&mut state, &reordered[2..3]));
    events.extend(settled(&mut state));

    assert_eq!(started(&events), 1);
    assert_eq!(ended(&events), 1);
    assert!(state.running.is_empty());
}

#[test]
fn a_stopped_notice_ends_the_subagent_like_a_completed_one() {
    let lines = lines();
    let mut state = SessionState::new(AGENT);
    feed(&mut state, &lines[..until_first_result(&lines)]);

    feed(
        &mut state,
        &[json!({
            "type": "system", "subtype": "task_notification", "task_id": "aceaa446a8a12da05",
            "tool_use_id": LAUNCH, "status": "stopped", "summary": "stopped",
        })],
    );

    assert_eq!(ended(&settled(&mut state)), 1);
}

#[test]
fn an_unknown_status_is_not_an_end() {
    let lines = lines();
    let mut state = SessionState::new(AGENT);
    feed(&mut state, &lines[..until_first_result(&lines)]);

    feed(
        &mut state,
        &[json!({
            "type": "system", "subtype": "task_notification", "task_id": "aceaa446a8a12da05",
            "tool_use_id": LAUNCH, "status": "running",
        })],
    );

    assert_eq!(state.next_settle(), None);
    assert_eq!(ended(&settled(&mut state)), 0);
}

#[test]
fn a_foreground_result_still_ends_the_subagent_at_once() {
    let mut state = SessionState::new(AGENT);
    let events = feed(
        &mut state,
        &[
            json!({"type": "assistant", "parent_tool_use_id": null, "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_fg", "name": "Agent", "input": {"prompt": "look"}}]}}),
            json!({"type": "user", "parent_tool_use_id": null,
                "tool_use_result": {"status": "completed", "agentId": "a1"},
                "message": {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_fg", "content": "done"}]}}),
        ],
    );

    assert_eq!(started(&events), 1);
    assert_eq!(ended(&events), 1);
    assert!(state.running.is_empty());
}
