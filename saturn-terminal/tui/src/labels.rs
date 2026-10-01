//! 작업 이름표 규칙. 글자는 engine이 정해 알림에 싣고, TUI는 보일지와 어떻게 쓸지만 정한다.
//!
//! 설계: docs/design/tui.md(이름표).
//! - 살아 있는 작업이 둘 이상이거나 대기 줄이나 보류 줄이 있을 때만 보인다. 작업이 하나이고 대기와 보류가 없으면 숨긴다.
//! - 끝난 작업의 글자는 비어 있는 글자 중 가장 앞 글자로 다시 쓴다(engine이 계산, `first_free`는 같은 규칙).
//! - 끼워 넣은 입력의 에코에는 그 입력이 합쳐진 작업의 이름표를 붙인다(`Notification::InputChanged::label`).
//! - provider subagent는 이름표 없이 부모 작업 줄 아래에 흐리게 접어 보인다.

use std::collections::BTreeMap;

use saturn_protocol::ids::{InputId, TaskId, TaskLabel};
use saturn_protocol::state::TaskState;

/// 이름표 글자 범위 `A`부터 `Z`까지. 초안 값이다(설계는 `[A]` 예시만 있음, docs/design/tui.md 초안 값).
pub const LABEL_RANGE: std::ops::RangeInclusive<char> = 'A'..='Z';

/// engine이 알린 이름표를 작업과 입력별로 기억한다.
#[derive(Debug, Default)]
pub struct LabelBook {
    tasks: BTreeMap<TaskId, TaskLabel>,
    inputs: BTreeMap<InputId, TaskLabel>,
}

impl LabelBook {
    /// 빈 이름표 모음.
    pub fn new() -> Self {
        Self::default()
    }

    /// 작업 이름표를 기록한다(`Notification::TaskChanged`). 같은 작업의 글자가 바뀌면 새 글자로 덮는다.
    pub fn set_task(&mut self, task: TaskId, label: TaskLabel) {
        self.tasks.insert(task, label);
    }

    /// 입력 이름표를 기록한다(`Notification::InputChanged`의 `label`이 있을 때). 판단 줄·대기 줄·에코에 쓴다.
    pub fn set_input(&mut self, input: InputId, label: TaskLabel) {
        self.inputs.insert(input, label);
    }

    /// 끝난 작업(`Done`, `Failed`)의 이름표를 지운다. 대화 기록에 이미 찍힌 이름표는 그대로 둔다.
    pub fn remove_task(&mut self, task: TaskId) {
        self.tasks.remove(&task);
    }

    /// 끝 상태(`Applied`, `Rejected`, `Cancelled`)가 된 입력의 이름표를 지운다.
    pub fn remove_input(&mut self, input: InputId) {
        self.inputs.remove(&input);
    }

    /// 작업 이름표.
    pub fn task(&self, task: TaskId) -> Option<TaskLabel> {
        self.tasks.get(&task).copied()
    }

    /// 입력 이름표.
    pub fn input(&self, input: InputId) -> Option<TaskLabel> {
        self.inputs.get(&input).copied()
    }
}

/// 살아 있는 작업인지. `Running`, `AnsweredTreeRunning`, `AwaitingPermission`만 참. `Held`는 보류 줄로 따로 센다.
pub fn is_live(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Running | TaskState::AnsweredTreeRunning | TaskState::AwaitingPermission
    )
}

/// 이름표를 보일지. 살아 있는 작업이 둘 이상이거나, 대기 줄이나 보류 줄이 하나라도 있으면 참.
pub fn visible(live_tasks: usize, queued_lines: usize, held_lines: usize) -> bool {
    live_tasks > 1 || queued_lines > 0 || held_lines > 0
}

/// 이름표 표기 `[A]`.
pub fn format(label: TaskLabel) -> String {
    format!("[{}]", label.0)
}

/// 줄 앞 이름표 접두. 보이고 이름표가 있으면 `"[A] "`, 아니면 빈 문자열.
pub fn prefix(label: Option<TaskLabel>, visible: bool) -> String {
    match label {
        Some(label) if visible => format!("{} ", format(label)),
        _ => String::new(),
    }
}

// cost: time O(r·u), heap O(1), stack O(1)
// vars: r = LABEL_RANGE 글자 수, u = used.len()
// basis: estimate
/// 쓰지 않는 글자 중 가장 앞 글자. 모두 쓰였으면 `None`. engine 규칙과 같은 계산으로 화면 쪽 검증에만 쓴다.
pub fn first_free(used: &[TaskLabel]) -> Option<TaskLabel> {
    LABEL_RANGE
        .map(TaskLabel)
        .find(|candidate| !used.contains(candidate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_single_task_without_queue_or_hold_hides_labels() {
        assert!(!visible(1, 0, 0));
        assert!(!visible(0, 0, 0));
    }

    #[test]
    fn visible_two_tasks_or_queue_or_hold_shows_labels() {
        assert!(visible(2, 0, 0));
        assert!(visible(1, 1, 0));
        assert!(visible(0, 0, 1));
    }

    #[test]
    fn prefix_hidden_or_missing_label_returns_empty() {
        assert_eq!(prefix(Some(TaskLabel('A')), true), "[A] ");
        assert_eq!(prefix(Some(TaskLabel('A')), false), "");
        assert_eq!(prefix(None, true), "");
    }

    #[test]
    fn first_free_returns_earliest_unused_letter() {
        let used = [TaskLabel('A'), TaskLabel('C')];

        assert_eq!(first_free(&used), Some(TaskLabel('B')));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn first_free_all_used_returns_none() {
        let used: Vec<TaskLabel> = LABEL_RANGE.map(TaskLabel).collect();

        assert_eq!(first_free(&used), None);
    }

    #[test]
    fn is_live_held_and_finished_are_not_live() {
        assert!(is_live(TaskState::AwaitingPermission));
        assert!(!is_live(TaskState::Held));
        assert!(!is_live(TaskState::Done));
    }

    #[test]
    fn label_book_remove_task_forgets_label() {
        let mut book = LabelBook::new();
        book.set_task(TaskId(1), TaskLabel('A'));

        book.remove_task(TaskId(1));

        assert_eq!(book.task(TaskId(1)), None);
    }
}
