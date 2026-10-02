//! provider subagent 트리 추적, 트리 유휴 판정, 사용량 턴 값 계산.
//! 설계: docs/design/providers-and-sessions.md

use std::collections::HashMap;

use saturn_protocol::event::{ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{AgentId, SessionId, SubagentId};

#[derive(Debug, Default)]
struct AgentTree {
    /// 부모 턴 완료만 작업 끝 후보다.
    main_done: bool,
    /// 부모가 `None`이면 메인 바로 아래.
    running: HashMap<SubagentId, Option<SubagentId>>,
    /// 누적 사용량 보고 사이에 빠진 턴을 찾는 데 쓴다.
    turns_completed: u64,
}

impl AgentTree {
    fn status(&self) -> TreeStatus {
        match (self.main_done, self.running.is_empty()) {
            (false, _) => TreeStatus::Running,
            (true, false) => TreeStatus::AnsweredTreeRunning,
            (true, true) => TreeStatus::TreeIdle,
        }
    }

    // cost: time O(d), heap O(1), stack O(1)
    // vars: d = 트리 깊이
    // basis: estimate
    /// 메인 바로 아래가 1이고, 고리가 있어도 살아 있는 수에서 멈춘다.
    fn depth(&self, subagent: &SubagentId) -> usize {
        let mut depth = 1;
        let mut current = self.running.get(subagent).cloned().flatten();
        while let Some(parent) = current {
            if depth > self.running.len() {
                break;
            }
            depth += 1;
            current = self.running.get(&parent).cloned().flatten();
        }
        depth
    }
}

#[derive(Debug, Clone)]
struct UsageRow {
    session: SessionId,
    report: UsageReport,
    /// 보고를 받은 시점 값.
    turns_completed: u64,
}

/// subagent 사용은 막거나 바꾸지 않고 추적만 한다.
#[derive(Debug, Default)]
pub struct AgentTracker {
    trees: HashMap<AgentId, AgentTree>,
    usage: Vec<UsageRow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeStatus {
    Running,
    AnsweredTreeRunning,
    TreeIdle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnUsage {
    /// 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론 순서이고, 보고되지 않으면 `None`이다.
    pub tokens: [Option<u64>; 5],
    /// 중간 보고가 빠지면 0으로 채우지 않고 여러 턴에 걸쳤다고 표시한다.
    pub spans_turns: bool,
}

impl AgentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(1) avg, heap O(1) amortized, stack O(1)
    // basis: estimate
    /// 흐름이 끊기면 더 관찰할 수 없으므로 트리를 끝난 것으로 둔다.
    pub fn on_event(&mut self, session: SessionId, event: &ProviderEvent) -> TreeStatus {
        let agent = agent_of(event);
        let tree = self.trees.entry(agent).or_default();
        match event {
            // 부모 턴이 끝난 뒤 메인 출력이 오면 입력 없는 provider-wake도 새 턴으로 본다.
            ProviderEvent::Text { subagent: None, .. }
            | ProviderEvent::ToolCall { subagent: None, .. } => tree.main_done = false,
            ProviderEvent::SubagentStarted {
                subagent, parent, ..
            } => {
                tree.running.insert(subagent.clone(), parent.clone());
            }
            ProviderEvent::SubagentEnded { subagent, .. } => {
                tree.running.remove(subagent);
            }
            ProviderEvent::TurnCompleted { .. } => {
                tree.main_done = true;
                tree.turns_completed += 1;
            }
            ProviderEvent::StreamLost { .. } => {
                tree.main_done = true;
                tree.running.clear();
            }
            ProviderEvent::Usage(report) => {
                let turns_completed = tree.turns_completed;
                let status = tree.status();
                self.usage.push(UsageRow {
                    session,
                    report: report.clone(),
                    turns_completed,
                });
                return status;
            }
            ProviderEvent::Text { .. }
            | ProviderEvent::ToolCall { .. }
            | ProviderEvent::ToolResult { .. }
            | ProviderEvent::PermissionRequested { .. }
            | ProviderEvent::ContextSize { .. }
            | ProviderEvent::SettingsApplied { .. } => {}
        }
        tree.status()
    }

    // cost: time O(1) avg, heap O(1), stack O(1)
    // basis: estimate
    /// 이벤트를 받은 적 없는 에이전트는 `None`이다.
    pub fn status(&self, agent: AgentId) -> Option<TreeStatus> {
        self.trees.get(&agent).map(AgentTree::status)
    }

    /// 이벤트를 받은 적 없는 에이전트는 추적 중인 실행이 없으므로 참이다.
    pub fn is_tree_idle(&self, agent: AgentId) -> bool {
        self.trees
            .get(&agent)
            .is_none_or(|tree| tree.status() == TreeStatus::TreeIdle)
    }

    // cost: time O(s·d + s log s), heap O(s), stack O(1), alloc 2
    // vars: s = 살아 있는 subagent 수, d = 트리 깊이
    // basis: estimate
    /// 깊은 subagent부터(깊이가 같으면 id 순서) 보내고 마지막이 메인(`None`)이다.
    pub fn interrupt_order(&self, agent: AgentId) -> Vec<Option<SubagentId>> {
        let Some(tree) = self.trees.get(&agent) else {
            return vec![None];
        };
        let mut subagents: Vec<(usize, &SubagentId)> = tree
            .running
            .keys()
            .map(|subagent| (tree.depth(subagent), subagent))
            .collect();
        subagents
            .sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.0.cmp(&right.1.0)));
        subagents
            .into_iter()
            .map(|(_, subagent)| Some(subagent.clone()))
            .chain(std::iter::once(None))
            .collect()
    }

    // cost: time O(u), heap O(1), stack O(1)
    // vars: u = 사용량 보고 행 수
    // basis: estimate
    /// `index`는 session 보고 중 0부터 센 순서이고, 누적 보고는 같은 계열의 직전 값을 뺀다.
    pub fn turn_usage(&self, session: SessionId, index: usize) -> TurnUsage {
        let rows: Vec<&UsageRow> = self
            .usage
            .iter()
            .filter(|row| row.session == session)
            .collect();
        let Some(current) = rows.get(index) else {
            return TurnUsage {
                tokens: [None; 5],
                spans_turns: false,
            };
        };
        let raw = tokens_of(&current.report);
        if current.report.scope != UsageScope::ThreadCumulative {
            return TurnUsage {
                tokens: raw,
                spans_turns: false,
            };
        }
        let previous: Vec<&UsageRow> = rows[..index]
            .iter()
            .rev()
            .filter(|row| is_same_cumulative(row, current))
            .copied()
            .collect();
        subtract_cumulative(current, &previous)
    }
}

fn agent_of(event: &ProviderEvent) -> AgentId {
    match event {
        ProviderEvent::Text { agent, .. }
        | ProviderEvent::ToolCall { agent, .. }
        | ProviderEvent::ToolResult { agent, .. }
        | ProviderEvent::SubagentStarted { agent, .. }
        | ProviderEvent::SubagentEnded { agent, .. }
        | ProviderEvent::PermissionRequested { agent, .. }
        | ProviderEvent::TurnCompleted { agent, .. }
        | ProviderEvent::ContextSize { agent, .. }
        | ProviderEvent::StreamLost { agent }
        | ProviderEvent::SettingsApplied { agent, .. } => *agent,
        ProviderEvent::Usage(report) => report.agent,
    }
}

/// 순서는 `TurnUsage::tokens`와 같다.
fn tokens_of(report: &UsageReport) -> [Option<u64>; 5] {
    [
        report.input,
        report.cache_read,
        report.cache_write,
        report.output,
        report.reasoning,
    ]
}

fn is_same_cumulative(row: &UsageRow, current: &UsageRow) -> bool {
    row.report.scope == UsageScope::ThreadCumulative
        && row.report.agent == current.report.agent
        && row.report.subagent == current.report.subagent
}

// cost: time O(p), heap O(1), stack O(1)
// vars: p = 같은 계열의 앞 누적 보고 수
// basis: estimate
/// `previous`는 가까운 것부터이고, 직전 값을 확정할 수 없으면 `spans_turns`를 켠다.
fn subtract_cumulative(current: &UsageRow, previous: &[&UsageRow]) -> TurnUsage {
    let raw = tokens_of(&current.report);
    let Some(last) = previous.first() else {
        return TurnUsage {
            tokens: raw,
            spans_turns: false,
        };
    };
    let mut spans_turns = current.turns_completed.saturating_sub(last.turns_completed) > 1;
    let mut tokens = [None; 5];
    for (slot, value) in raw.iter().enumerate() {
        let Some(value) = value else {
            continue;
        };
        let earlier = previous
            .iter()
            .enumerate()
            .find_map(|(distance, row)| tokens_of(&row.report)[slot].map(|seen| (distance, seen)));
        let Some((distance, seen)) = earlier else {
            tokens[slot] = Some(*value);
            spans_turns = true;
            continue;
        };
        if distance > 0 {
            spans_turns = true;
        }
        match value.checked_sub(seen) {
            Some(delta) => tokens[slot] = Some(delta),
            None => spans_turns = true,
        }
    }
    TurnUsage {
        tokens,
        spans_turns,
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::event::TurnOrigin;

    use super::*;

    const AGENT: AgentId = AgentId(1);
    const SESSION: SessionId = SessionId(10);

    fn sub(id: &str) -> SubagentId {
        SubagentId(id.to_string())
    }

    // cost: time O(l), heap O(l), stack O(1), alloc 2
    // vars: l = id 글자 수
    // basis: estimate
    fn started(id: &str, parent: Option<&str>) -> ProviderEvent {
        ProviderEvent::SubagentStarted {
            agent: AGENT,
            subagent: sub(id),
            parent: parent.map(sub),
        }
    }

    fn ended(id: &str) -> ProviderEvent {
        ProviderEvent::SubagentEnded {
            agent: AGENT,
            subagent: sub(id),
        }
    }

    fn completed() -> ProviderEvent {
        ProviderEvent::TurnCompleted {
            agent: AGENT,
            origin: TurnOrigin::User,
        }
    }

    // cost: time O(l), heap O(l), stack O(1), alloc 2
    // vars: l = id 글자 수
    // basis: estimate
    fn text(subagent: Option<&str>) -> ProviderEvent {
        ProviderEvent::Text {
            agent: AGENT,
            subagent: subagent.map(sub),
            text: "working".into(),
        }
    }

    fn report(scope: UsageScope, input: Option<u64>, output: Option<u64>) -> UsageReport {
        UsageReport {
            agent: AGENT,
            subagent: None,
            model: None,
            scope,
            input,
            cache_read: None,
            cache_write: None,
            output,
            reasoning: None,
        }
    }

    // cost: time O(e), heap O(e), stack O(1)
    // vars: e = 이벤트 수
    // basis: estimate
    fn feed(tracker: &mut AgentTracker, events: &[ProviderEvent]) -> TreeStatus {
        events
            .iter()
            .map(|event| tracker.on_event(SESSION, event))
            .last()
            .expect("events should not be empty")
    }

    #[test]
    fn on_event_running_until_turn_completed() {
        let mut tracker = AgentTracker::new();

        let status = feed(&mut tracker, &[text(None)]);

        assert_eq!(status, TreeStatus::Running);
        assert!(!tracker.is_tree_idle(AGENT));
    }

    #[test]
    fn on_event_answer_with_subagent_left_is_answered_tree_running() {
        let mut tracker = AgentTracker::new();

        let status = feed(&mut tracker, &[started("a", None), completed()]);

        assert_eq!(status, TreeStatus::AnsweredTreeRunning);
        assert!(!tracker.is_tree_idle(AGENT));
    }

    #[test]
    fn status_is_none_until_the_first_event_and_follows_the_tree() {
        let mut tracker = AgentTracker::new();
        assert_eq!(tracker.status(AGENT), None);

        feed(&mut tracker, &[started("a", None), completed()]);

        assert_eq!(tracker.status(AGENT), Some(TreeStatus::AnsweredTreeRunning));
    }

    #[test]
    fn on_event_all_subagents_ended_is_tree_idle() {
        let mut tracker = AgentTracker::new();

        let status = feed(
            &mut tracker,
            &[
                started("a", None),
                started("b", Some("a")),
                completed(),
                ended("b"),
                ended("a"),
            ],
        );

        assert_eq!(status, TreeStatus::TreeIdle);
        assert!(tracker.is_tree_idle(AGENT));
    }

    #[test]
    fn on_event_subagent_end_alone_is_not_task_end() {
        let mut tracker = AgentTracker::new();

        let status = feed(&mut tracker, &[started("a", None), ended("a")]);

        assert_eq!(status, TreeStatus::Running);
    }

    #[test]
    fn on_event_main_text_after_completion_starts_new_turn() {
        let mut tracker = AgentTracker::new();

        let status = feed(&mut tracker, &[completed(), text(None)]);

        assert_eq!(status, TreeStatus::Running);
    }

    #[test]
    fn on_event_subagent_text_after_completion_keeps_main_done() {
        let mut tracker = AgentTracker::new();

        let status = feed(
            &mut tracker,
            &[started("a", None), completed(), text(Some("a"))],
        );

        assert_eq!(status, TreeStatus::AnsweredTreeRunning);
    }

    #[test]
    fn on_event_stream_lost_ends_tree() {
        let mut tracker = AgentTracker::new();

        let status = feed(
            &mut tracker,
            &[
                started("a", None),
                ProviderEvent::StreamLost { agent: AGENT },
            ],
        );

        assert_eq!(status, TreeStatus::TreeIdle);
    }

    #[test]
    fn is_tree_idle_unknown_agent_is_true() {
        let tracker = AgentTracker::new();

        assert!(tracker.is_tree_idle(AgentId(99)));
    }

    #[test]
    fn interrupt_order_deepest_first_and_main_last() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                started("a", None),
                started("b", Some("a")),
                started("c", Some("b")),
                started("d", None),
            ],
        );

        let order = tracker.interrupt_order(AGENT);

        assert_eq!(
            order,
            vec![
                Some(sub("c")),
                Some(sub("b")),
                Some(sub("a")),
                Some(sub("d")),
                None
            ]
        );
    }

    #[test]
    fn interrupt_order_unknown_agent_is_main_only() {
        let tracker = AgentTracker::new();

        assert_eq!(tracker.interrupt_order(AGENT), vec![None]);
    }

    #[test]
    fn turn_usage_main_turn_is_reported_value() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[ProviderEvent::Usage(report(
                UsageScope::MainTurn,
                Some(100),
                None,
            ))],
        );

        let usage = tracker.turn_usage(SESSION, 0);

        assert_eq!(usage.tokens, [Some(100), None, None, None, None]);
        assert!(!usage.spans_turns);
    }

    #[test]
    fn turn_usage_cumulative_subtracts_previous() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(100), Some(10))),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(250), Some(30))),
            ],
        );

        let first = tracker.turn_usage(SESSION, 0);
        let second = tracker.turn_usage(SESSION, 1);

        assert_eq!(first.tokens, [Some(100), None, None, Some(10), None]);
        assert_eq!(second.tokens, [Some(150), None, None, Some(20), None]);
        assert!(!second.spans_turns);
    }

    #[test]
    fn turn_usage_new_session_starts_over() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[ProviderEvent::Usage(report(
                UsageScope::ThreadCumulative,
                Some(100),
                None,
            ))],
        );
        tracker.on_event(
            SessionId(11),
            &ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(40), None)),
        );

        let usage = tracker.turn_usage(SessionId(11), 0);

        assert_eq!(usage.tokens[0], Some(40));
        assert!(!usage.spans_turns);
    }

    #[test]
    fn turn_usage_missing_middle_report_spans_turns() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(100), None)),
                completed(),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(300), None)),
            ],
        );

        let usage = tracker.turn_usage(SESSION, 1);

        assert_eq!(usage.tokens[0], Some(200));
        assert!(usage.spans_turns);
    }

    #[test]
    fn turn_usage_decreasing_cumulative_is_none_and_spans() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(300), None)),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(100), None)),
            ],
        );

        let usage = tracker.turn_usage(SESSION, 1);

        assert_eq!(usage.tokens[0], None);
        assert!(usage.spans_turns);
    }

    #[test]
    fn turn_usage_unreported_field_stays_none() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(100), Some(5))),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(150), None)),
            ],
        );

        let usage = tracker.turn_usage(SESSION, 1);

        assert_eq!(usage.tokens, [Some(50), None, None, None, None]);
    }

    #[test]
    fn turn_usage_field_skipped_in_previous_report_spans_turns() {
        let mut tracker = AgentTracker::new();
        feed(
            &mut tracker,
            &[
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(100), Some(5))),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(150), None)),
                completed(),
                ProviderEvent::Usage(report(UsageScope::ThreadCumulative, Some(200), Some(25))),
            ],
        );

        let usage = tracker.turn_usage(SESSION, 2);

        assert_eq!(usage.tokens[0], Some(50));
        assert_eq!(usage.tokens[3], Some(20));
        assert!(usage.spans_turns);
    }

    #[test]
    fn turn_usage_out_of_range_is_all_none() {
        let tracker = AgentTracker::new();

        let usage = tracker.turn_usage(SESSION, 0);

        assert_eq!(usage.tokens, [None; 5]);
        assert!(!usage.spans_turns);
    }
}
