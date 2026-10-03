//! 종료 확인 창. `on_exit`가 `ask`이고 실행 중인 작업이 있을 때 TUI를 닫기 전에 뜬다.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, render_window};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitChoice {
    Continue,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExitConfirm {
    /// 계속 처리될 작업 수.
    pub running: u32,
    pub selected: ExitChoice,
}

impl ExitConfirm {
    /// 작업을 잃지 않는 `계속`이 처음 선택이다.
    pub(crate) fn new(running: u32) -> Self {
        Self {
            running,
            selected: ExitChoice::Continue,
        }
    }

    pub(crate) fn up(&mut self) {
        self.selected = ExitChoice::Continue;
    }

    pub(crate) fn down(&mut self) {
        self.selected = ExitChoice::Stop;
    }
}

#[derive(Debug)]
pub(crate) struct ExitConfirmView<'a> {
    pub confirm: &'a ExitConfirm,
    pub lang: Lang,
}

impl ExitConfirmView<'_> {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let title = lang
            .tr(i18n::EXIT_TITLE)
            .replace("{count}", &self.confirm.running.to_string());
        let mut lines = vec![Line::from(lang.tr(i18n::EXIT_QUESTION)), Line::from("")];
        for (choice, text) in [
            (ExitChoice::Continue, i18n::EXIT_CONTINUE),
            (ExitChoice::Stop, i18n::EXIT_STOP),
        ] {
            let style = if choice == self.confirm.selected {
                SELECTED
            } else {
                Style::new()
            };
            lines.push(Line::from(Span::styled(lang.tr(text), style)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(lang.tr(i18n::EXIT_HINT), MUTED)));
        render_window(frame, area, &title, lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::buffer_lines;

    #[test]
    fn render_shows_count_and_both_choices() {
        let confirm = ExitConfirm::new(2);
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();

        terminal
            .draw(|frame| {
                ExitConfirmView {
                    confirm: &confirm,
                    lang: Lang::En,
                }
                .render(frame, frame.area());
            })
            .unwrap();

        let text = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(text.contains("Running tasks: 2"), "{text}");
        assert!(text.contains("Keep running"), "{text}");
        assert!(text.contains("Stop"), "{text}");
    }
}
