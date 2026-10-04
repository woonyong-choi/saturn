//! 학습 확인 창. 실행 조건(채점 후보 200건 이상)을 채운 `/train`에서 뜬다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{SELECTED, choice_text, render_window};

#[expect(dead_code, reason = "#91 학습 실행 구현 전")]
pub(crate) const MIN_CANDIDATES: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrainChoice {
    Run,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrainConfirm {
    pub candidates: u32,
    pub grading_model: String,
    pub estimated_tokens: u64,
    pub threshold_targets: Vec<String>,
    pub fine_tune: bool,
    pub reset_thresholds: bool,
    pub selected: TrainChoice,
}

impl TrainConfirm {
    pub(crate) fn up(&mut self) {
        self.selected = TrainChoice::Run;
    }

    pub(crate) fn down(&mut self) {
        self.selected = TrainChoice::Cancel;
    }
}

#[derive(Debug)]
pub(crate) struct TrainConfirmView<'a> {
    pub confirm: &'a TrainConfirm,
    pub lang: Lang,
}

impl TrainConfirmView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 조정 대상 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let confirm = self.confirm;
        let yes_no = |value: bool| lang.tr(if value { i18n::YES } else { i18n::NO });
        let targets = if confirm.threshold_targets.is_empty() {
            "-".to_string()
        } else {
            confirm.threshold_targets.join(", ")
        };
        let mut lines = vec![
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_CANDIDATES),
                i18n::format_count(u64::from(confirm.candidates))
            )),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_GRADER),
                confirm.grading_model
            )),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_TOKENS),
                i18n::format_count(confirm.estimated_tokens)
            )),
            Line::from(format!("{}: {targets}", lang.tr(i18n::TRAIN_TARGETS))),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_RETRAIN),
                yes_no(confirm.fine_tune)
            )),
            Line::from(""),
        ];
        for (index, (choice, text)) in [
            (TrainChoice::Run, i18n::TRAIN_RUN),
            (TrainChoice::Cancel, i18n::CANCEL),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = choice == confirm.selected;
            let style = if selected { SELECTED } else { Style::new() };
            lines.push(Line::from(Span::styled(
                choice_text(index, selected, lang.tr(text)),
                style,
            )));
        }
        render_window(frame, area, lang.tr(i18n::TRAIN_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn confirm() -> TrainConfirm {
        TrainConfirm {
            candidates: 240,
            grading_model: "grader-1".to_string(),
            estimated_tokens: 1_200_000,
            threshold_targets: vec!["relation".to_string()],
            fine_tune: false,
            reset_thresholds: false,
            selected: TrainChoice::Run,
        }
    }

    #[test]
    fn up_down_switch_choice() {
        let mut confirm = confirm();

        confirm.down();
        let down = confirm.selected;
        confirm.up();

        assert_eq!(down, TrainChoice::Cancel);
        assert_eq!(confirm.selected, TrainChoice::Run);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_values_with_separators() {
        let confirm = confirm();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let view = TrainConfirmView {
            confirm: &confirm,
            lang: Lang::En,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("Candidates: 240"));
        assert!(content.contains("Estimated tokens: 1,200,000"));
        assert!(content.contains("Retrain model: No"));
    }
}
