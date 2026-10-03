//! 전체 기록(`Ctrl+T`). 도구 셀 전체와 줄인 셀을 펼친 대화 기록.
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::i18n::Lang;
use crate::view::transcript::{Transcript, styled_rows};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FullTranscript {
    /// 맨 위에서 내려온 줄 수.
    pub scroll: usize,
}

impl FullTranscript {
    pub(crate) fn up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// 끝을 넘는 값은 그릴 때 자른다.
    pub(crate) fn down(&mut self) {
        self.scroll += 1;
    }
}

#[derive(Debug)]
pub(crate) struct FullTranscriptView<'a> {
    pub state: &'a FullTranscript,
    pub transcript: &'a Transcript,
    pub lang: Lang,
    pub labels_visible: bool,
}

impl FullTranscriptView<'_> {
    // cost: time O(c), heap O(c), stack O(1)
    // vars: c = 대화 기록 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let rows = styled_rows(
            self.transcript.cells(),
            self.lang,
            self.labels_visible,
            true,
            area.width,
        );
        let height = usize::from(area.height);
        let start = self.state.scroll.min(rows.len().saturating_sub(height));
        let lines: Vec<Line> = rows
            .into_iter()
            .skip(start)
            .take(height)
            .map(|(text, style)| Line::from(Span::styled(text, style)))
            .collect();
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines), area);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use saturn_protocol::event::Activity;

    use super::*;
    use crate::view::transcript::TranscriptCell;

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_expands_tool_output() {
        let mut transcript = Transcript::new();
        transcript.push(TranscriptCell::Tool {
            label: None,
            call_id: "c".to_string(),
            activity: Activity::ReadingFile,
            output: "contents".to_string(),
            is_interrupted: false,
        });
        let state = FullTranscript::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).unwrap();
        let view = FullTranscriptView {
            state: &state,
            transcript: &transcript,
            lang: Lang::Ko,
            labels_visible: false,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        let row: String = (0..20).map(|x| buffer[(x, 1)].symbol()).collect();
        assert_eq!(row.trim_end(), "  contents");
    }

    #[test]
    fn up_stops_at_top() {
        let mut state = FullTranscript::default();

        state.down();
        state.up();
        state.up();

        assert_eq!(state.scroll, 0);
    }
}
