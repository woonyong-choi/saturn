//! 보류 재개 질문. 보류 작업이 있는 채팅을 다시 열 때 한 번 뜬다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{TaskId, TaskLabel};

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::view::transcript::held_labels;
use crate::view::{SELECTED, choice_text, render_window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeChoice {
    All,
    Pick,
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResumeOutcome {
    Pending,
    ContinueAll,
    /// 접수 순서.
    Continue(Vec<TaskId>),
    Leave,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumePrompt {
    /// 이름표 순서.
    pub held: Vec<(TaskId, TaskLabel)>,
    pub selected: ResumeChoice,
    /// `골라서 이어서` 단계의 강조 위치와 고른 작업.
    pub picking: Option<(usize, Vec<TaskId>)>,
}

impl ResumePrompt {
    pub(crate) fn new(held: Vec<(TaskId, TaskLabel)>) -> Self {
        Self {
            held,
            selected: ResumeChoice::All,
            picking: None,
        }
    }

    pub(crate) fn up(&mut self) {
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

    pub(crate) fn down(&mut self) {
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
    /// 선택 단계에서는 목록 끝의 확정 행에서만 `Continue`를 돌려준다.
    pub(crate) fn confirm(&mut self) -> ResumeOutcome {
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

#[derive(Debug)]
pub(crate) struct ResumePromptView<'a> {
    pub prompt: &'a ResumePrompt,
    pub lang: Lang,
}

impl ResumePromptView<'_> {
    // cost: time O(h²), heap O(h), stack O(1)
    // vars: h = 보류 작업 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
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
        lines.extend(
            rows.into_iter()
                .enumerate()
                .map(|(index, (text, selected))| {
                    let style = if selected { SELECTED } else { Style::new() };
                    Line::from(Span::styled(choice_text(index, selected, text), style))
                }),
        );
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
