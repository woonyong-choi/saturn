//! 멈춤 확인 창. 끼워 넣기를 받지 않은 충돌 입력을 멈추고 실행할지 묻는다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::InputId;

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, choice_text, render_window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopChoice {
    Wait,
    StopAndRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StopConfirm {
    pub input: InputId,
    pub text: String,
    pub selected: StopChoice,
}

impl StopConfirm {
    /// 작업을 멈추지 않는 `대기`가 처음 선택이다.
    pub(crate) fn new(input: InputId, text: String) -> Self {
        Self {
            input,
            text,
            selected: StopChoice::Wait,
        }
    }

    pub(crate) fn up(&mut self) {
        self.selected = StopChoice::Wait;
    }

    pub(crate) fn down(&mut self) {
        self.selected = StopChoice::StopAndRun;
    }
}

#[derive(Debug)]
pub(crate) struct StopConfirmView<'a> {
    pub confirm: &'a StopConfirm,
    pub lang: Lang,
}

impl StopConfirmView<'_> {
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 입력 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let mut lines = vec![Line::from(self.confirm.text.clone()), Line::from("")];
        for (index, (choice, text)) in [
            (StopChoice::Wait, i18n::QUEUED),
            (StopChoice::StopAndRun, i18n::STOP_CONFIRM_RUN),
        ]
        .into_iter()
        .enumerate()
        {
            let selected = choice == self.confirm.selected;
            let style = if selected { SELECTED } else { Style::new() };
            lines.push(Line::from(Span::styled(
                choice_text(index, selected, lang.tr(text)),
                style,
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            lang.tr(i18n::STOP_CONFIRM_HINT),
            MUTED,
        )));
        render_window(frame, area, lang.tr(i18n::STOP_CONFIRM_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::buffer_lines;

    #[test]
    fn render_shows_the_input_and_both_choices() {
        let confirm = StopConfirm::new(InputId(1), "stop and use pytest instead".to_string());
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();

        terminal
            .draw(|frame| {
                StopConfirmView {
                    confirm: &confirm,
                    lang: Lang::En,
                }
                .render(frame, frame.area());
            })
            .unwrap();

        let text = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(text.contains("Stop now and run the new input?"), "{text}");
        assert!(text.contains("stop and use pytest instead"), "{text}");
        assert!(text.contains("Queued"), "{text}");
        assert!(text.contains("Stop and run"), "{text}");
    }
}
