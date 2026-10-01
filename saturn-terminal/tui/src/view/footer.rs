//! 바닥줄. 왼쪽 키 안내, 오른쪽 맥락 크기.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::i18n::{self, Lang};
use crate::state::ContextSize;
use crate::view::{MUTED, text_width, truncate};

pub fn context_text(lang: Lang, context: Option<ContextSize>) -> String {
    match context {
        Some(ContextSize {
            tokens: Some(tokens),
            threshold,
        }) => format!(
            "{} {}/{}",
            lang.tr(i18n::CONTEXT),
            i18n::format_kilo(tokens),
            i18n::format_kilo(threshold)
        ),
        _ => lang.tr(i18n::CONTEXT_UNKNOWN).to_string(),
    }
}

#[derive(Debug)]
pub struct FooterView {
    pub lang: Lang,
    pub context: Option<ContextSize>,
}

impl FooterView {
    /// 폭이 모자라면 왼쪽을 먼저 자른다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let right = context_text(self.lang, self.context);
        let right_width = text_width(&right);
        let available = usize::from(area.width);
        let left_width = available.saturating_sub(right_width + 1);
        let left = truncate(self.lang.tr(i18n::FOOTER_HINT), left_width);
        let gap = available.saturating_sub(text_width(&left) + right_width);
        let line = Line::from(vec![
            Span::styled(left, MUTED),
            Span::raw(" ".repeat(gap)),
            Span::raw(right),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn context_text_formats_size_or_unknown() {
        let size = ContextSize {
            tokens: Some(38_400),
            threshold: 200_000,
        };
        let unknown = ContextSize {
            tokens: None,
            threshold: 200_000,
        };

        assert_eq!(context_text(Lang::Ko, Some(size)), "맥락 38K/200K");
        assert_eq!(context_text(Lang::Ko, Some(unknown)), "맥락 미확인");
        assert_eq!(context_text(Lang::En, None), "context unknown");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_puts_hint_left_and_context_right() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        let view = FooterView {
            lang: Lang::En,
            context: Some(ContextSize {
                tokens: Some(38_000),
                threshold: 200_000,
            }),
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (0..60).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.starts_with("/help help · Ctrl+C stop"));
        assert!(row.ends_with("context 38K/200K"));
    }
}
