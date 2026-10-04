//! 작업 이름표 규칙. 글자는 engine이 정하고 TUI는 보일지와 표기만 정한다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::TaskLabel;
use saturn_protocol::state::TaskState;

/// 초안 값.
pub(crate) const LABEL_RANGE: std::ops::RangeInclusive<char> = 'A'..='Z';

/// `Held`는 보류 줄로 따로 센다.
pub(crate) fn is_live(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Running
            | TaskState::AnsweredTreeRunning
            | TaskState::AwaitingPermission
            | TaskState::AwaitingInput
    )
}

pub(crate) fn visible(live_tasks: usize, queued_lines: usize, held_lines: usize) -> bool {
    live_tasks > 1 || queued_lines > 0 || held_lines > 0
}

pub(crate) fn format(label: TaskLabel) -> String {
    format!("[{}]", label.0)
}

pub(crate) fn prefix(label: Option<TaskLabel>, visible: bool) -> String {
    match label {
        Some(label) if visible => format!("{} ", format(label)),
        _ => String::new(),
    }
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
    fn is_live_held_and_finished_are_not_live() {
        assert!(is_live(TaskState::AwaitingPermission));
        assert!(is_live(TaskState::AwaitingInput));
        assert!(!is_live(TaskState::Held));
        assert!(!is_live(TaskState::Done));
    }
}
