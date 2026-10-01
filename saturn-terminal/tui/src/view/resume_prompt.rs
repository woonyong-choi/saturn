//! 보류 재개 질문. 보류 작업이 있는 채팅을 다시 열 때 한 번 뜬다.
//!
//! 설계: docs/design/tui.md(영역 보류 재개 질문, 예시 멈춘 작업을 다시 연다), docs/design/engine-lifecycle.md(크래시 뒤 보류).
//! 선택지 `모두 이어서`(`Continue { task: None }`), `골라서 이어서`(목록에서 골라 작업마다 `Continue { task: Some }`),
//! `그대로 두기`(아무것도 보내지 않는다).

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{TaskId, TaskLabel};

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::view::transcript::held_labels;
use crate::view::{SELECTED, render_window};

/// 선택지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeChoice {
    /// `모두 이어서`.
    All,
    /// `골라서 이어서`. 고르면 보류 목록 선택 단계로 넘어간다.
    Pick,
    /// `그대로 두기`.
    Leave,
}

/// 보류 재개 질문의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// 아직 고르는 중.
    Pending,
    /// 채팅 보류 전부 재개.
    ContinueAll,
    /// 고른 작업만 재개. 접수 순서.
    Continue(Vec<TaskId>),
    /// 그대로 둔다.
    Leave,
}

/// 보류 재개 질문 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePrompt {
    /// 보류된 작업(이름표 순서).
    pub held: Vec<(TaskId, TaskLabel)>,
    /// 강조한 선택지.
    pub selected: ResumeChoice,
    /// `골라서 이어서` 단계면 목록 강조 위치와 고른 작업. 그 밖에는 `None`.
    pub picking: Option<(usize, Vec<TaskId>)>,
}

impl ResumePrompt {
    /// 보류 목록으로 연다. 강조는 `모두 이어서`.
    pub fn new(held: Vec<(TaskId, TaskLabel)>) -> Self {
        Self {
            held,
            selected: ResumeChoice::All,
            picking: None,
        }
    }

    /// `↑` 이동.
    pub fn up(&mut self) {
        match &mut self.picking {
            Some((row, _)) => *row = row.saturating_sub(1),
            None => {
                self.selected = match self.selected {
                    ResumeChoice::All | ResumeChoice::Pick => ResumeChoice::All,
                    ResumeChoice::Leave => ResumeChoice::Pick,
                };
            }
        }
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        let last = self.held.len();
        match &mut self.picking {
            Some((row, _)) => *row = (*row + 1).min(last),
            None => {
                self.selected = match self.selected {
                    ResumeChoice::All => ResumeChoice::Pick,
                    ResumeChoice::Pick | ResumeChoice::Leave => ResumeChoice::Leave,
                };
            }
        }
    }

    // cost: time O(h²), heap O(h), stack O(1)
    // vars: h = 보류 작업 수
    // basis: estimate
    /// `Enter`. 선택 단계에서는 강조한 작업을 고르거나 빼고, 목록 끝의 확정 행에서 `Continue`를 돌려준다.
    pub fn confirm(&mut self) -> ResumeOutcome {
        let Some((row, chosen)) = &mut self.picking else {
            return match self.selected {
                ResumeChoice::All => ResumeOutcome::ContinueAll,
                ResumeChoice::Pick => {
                    self.picking = Some((0, Vec::new()));
                    ResumeOutcome::Pending
                }
                ResumeChoice::Leave => ResumeOutcome::Leave,
            };
        };
        match self.held.get(*row) {
            Some((task, _)) => {
                match chosen.iter().position(|c| c == task) {
                    Some(index) => {
                        chosen.remove(index);
                    }
                    None => chosen.push(*task),
                }
                ResumeOutcome::Pending
            }
            None => {
                let ordered = self
                    .held
                    .iter()
                    .map(|(task, _)| *task)
                    .filter(|task| chosen.contains(task))
                    .collect();
                ResumeOutcome::Continue(ordered)
            }
        }
    }
}

/// 보류 재개 질문 그리기.
#[derive(Debug)]
pub struct ResumePromptView<'a> {
    /// 창 상태.
    pub prompt: &'a ResumePrompt,
    /// 화면 언어.
    pub lang: Lang,
}

impl ResumePromptView<'_> {
    // cost: time O(h²), heap O(h), stack O(1)
    // vars: h = 보류 작업 수
    // basis: estimate
    /// 보류 목록 `[A] [C]`과 선택지 세 개, 선택 단계면 작업마다 고름 표시를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let prompt = self.prompt;
        let labels: Vec<TaskLabel> = prompt.held.iter().map(|(_, label)| *label).collect();
        let mut lines = vec![Line::from(held_labels(&labels)), Line::from("")];
        let rows: Vec<(String, bool)> = match &prompt.picking {
            None => [
                (ResumeChoice::All, i18n::RESUME_ALL),
                (ResumeChoice::Pick, i18n::RESUME_PICK),
                (ResumeChoice::Leave, i18n::RESUME_LEAVE),
            ]
            .into_iter()
            .map(|(choice, text)| (lang.tr(text).to_string(), choice == prompt.selected))
            .collect(),
            Some((row, chosen)) => {
                let mut rows: Vec<(String, bool)> = prompt
                    .held
                    .iter()
                    .enumerate()
                    .map(|(i, (task, label))| {
                        let mark = if chosen.contains(task) { "[x]" } else { "[ ]" };
                        (format!("{mark} {}", labels::format(*label)), i == *row)
                    })
                    .collect();
                rows.push((
                    lang.tr(i18n::RESUME_CONFIRM).to_string(),
                    *row == prompt.held.len(),
                ));
                rows
            }
        };
        lines.extend(rows.into_iter().map(|(text, selected)| {
            let style = if selected { SELECTED } else { Style::new() };
            Line::from(Span::styled(text, style))
        }));
        render_window(frame, area, lang.tr(i18n::RESUME_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt() -> ResumePrompt {
        ResumePrompt::new(vec![
            (TaskId(1), TaskLabel('A')),
            (TaskId(3), TaskLabel('C')),
        ])
    }

    #[test]
    fn confirm_all_continues_everything() {
        assert_eq!(prompt().confirm(), ResumeOutcome::ContinueAll);
    }

    #[test]
    fn confirm_leave_after_moving_down_twice() {
        let mut prompt = prompt();

        prompt.down();
        prompt.down();
        prompt.down();

        assert_eq!(prompt.confirm(), ResumeOutcome::Leave);
    }

    #[test]
    fn pick_toggles_tasks_and_confirms_in_order() {
        let mut prompt = prompt();
        prompt.down();
        assert_eq!(prompt.confirm(), ResumeOutcome::Pending);

        prompt.down();
        prompt.confirm();
        prompt.up();
        prompt.confirm();
        prompt.confirm();
        prompt.down();
        prompt.down();

        assert_eq!(prompt.confirm(), ResumeOutcome::Continue(vec![TaskId(3)]));
    }
}
