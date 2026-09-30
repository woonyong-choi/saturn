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

/// 이름표 글자 범위. 초안 값이다(설계는 `[A]` 예시만 있음).
/// TODO(#92): 값 미정, 초안 `A`부터 `Z`까지
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
        todo!("#92")
    }

    /// 입력 이름표를 기록한다(`Notification::InputChanged`의 `label`이 있을 때). 판단 줄·대기 줄·에코에 쓴다.
    pub fn set_input(&mut self, input: InputId, label: TaskLabel) {
        todo!("#92")
    }

    /// 끝난 작업(`Done`, `Failed`)의 이름표를 지운다. 대화 기록에 이미 찍힌 이름표는 그대로 둔다.
    pub fn remove_task(&mut self, task: TaskId) {
        todo!("#92")
    }

    /// 끝 상태(`Applied`, `Rejected`, `Cancelled`)가 된 입력의 이름표를 지운다.
    pub fn remove_input(&mut self, input: InputId) {
        todo!("#92")
    }

    /// 작업 이름표.
    pub fn task(&self, task: TaskId) -> Option<TaskLabel> {
        todo!("#92")
    }

    /// 입력 이름표.
    pub fn input(&self, input: InputId) -> Option<TaskLabel> {
        todo!("#92")
    }
}

/// 살아 있는 작업인지. `Running`, `AnsweredTreeRunning`, `AwaitingPermission`만 참. `Held`는 보류 줄로 따로 센다.
pub fn is_live(state: TaskState) -> bool {
    todo!("#92")
}

/// 이름표를 보일지. 살아 있는 작업이 둘 이상이거나, 대기 줄이나 보류 줄이 하나라도 있으면 참.
pub fn visible(live_tasks: usize, queued_lines: usize, held_lines: usize) -> bool {
    todo!("#92")
}

/// 이름표 표기 `[A]`.
pub fn format(label: TaskLabel) -> String {
    todo!("#92")
}

/// 줄 앞 이름표 접두. 보이고 이름표가 있으면 `"[A] "`, 아니면 빈 문자열.
pub fn prefix(label: Option<TaskLabel>, visible: bool) -> String {
    todo!("#92")
}

/// 쓰지 않는 글자 중 가장 앞 글자. 모두 쓰였으면 `None`. engine 규칙과 같은 계산으로 화면 쪽 검증에만 쓴다.
pub fn first_free(used: &[TaskLabel]) -> Option<TaskLabel> {
    todo!("#92")
}
