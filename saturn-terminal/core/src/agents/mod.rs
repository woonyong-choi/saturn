//! provider subagent 트리 추적, 트리 유휴 판정.
//! 설계: docs/design/providers-and-sessions.md

use std::collections::HashMap;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, SubagentId};

#[derive(Debug, Default)]
struct AgentTree {
    /// 부모 턴 완료만 작업 끝 후보다.
    main_done: bool,
    /// 부모가 `None`이면 메인 바로 아래.
    running: HashMap<SubagentId, Option<SubagentId>>,
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

/// subagent 사용은 막거나 바꾸지 않고 추적만 한다.
#[derive(Debug, Default)]
pub struct AgentTracker {
    trees: HashMap<AgentId, AgentTree>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeStatus {
    Running,
    AnsweredTreeRunning,
    TreeIdle,
}

impl AgentTracker {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(1) avg, heap O(1) amortized, stack O(1)
    // basis: estimate
    /// 흐름이 끊기면 더 관찰할 수 없으므로 트리를 끝난 것으로 둔다.
    pub fn on_event(&mut self, event: &ProviderEvent) -> TreeStatus {
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
            }
            ProviderEvent::StreamLost { .. } => {
                tree.main_done = true;
                tree.running.clear();
            }
            ProviderEvent::Text { .. }
            | ProviderEvent::ToolCall { .. }
            | ProviderEvent::ToolResult { .. }
            | ProviderEvent::PermissionRequested { .. }
            | ProviderEvent::ContextSize { .. }
            | ProviderEvent::SettingsApplied { .. }
            | ProviderEvent::Usage(_) => {}
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

#[cfg(test)]
mod tests {
    use saturn_protocol::event::TurnOrigin;

    use super::*;

    const AGENT: AgentId = AgentId(1);

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

    // cost: time O(e), heap O(e), stack O(1)
    // vars: e = 이벤트 수
    // basis: estimate
    fn feed(tracker: &mut AgentTracker, events: &[ProviderEvent]) -> TreeStatus {
        events
            .iter()
            .map(|event| tracker.on_event(event))
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
}
