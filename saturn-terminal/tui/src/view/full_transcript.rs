//! 전체 기록(`Ctrl+T`). 도구 셀 전체와 줄인 셀을 펼친 대화 기록.
//!
//! 설계: docs/design/tui.md(영역 전체 기록). 대화 기록과 같은 셀을 `expanded: true`로 그린다.

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::i18n::Lang;
use crate::view::transcript::{Transcript, styled_rows};

/// 전체 기록 화면 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FullTranscript {
    /// 맨 위에서 내려온 줄 수.
    pub scroll: usize,
}

impl FullTranscript {
    /// `↑` 한 줄 위로.
    pub fn up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    /// `↓` 한 줄 아래로. 끝을 넘지 않는 것은 그릴 때 자른다.
    pub fn down(&mut self) {
        self.scroll += 1;
    }
}

/// 전체 기록 그리기.
#[derive(Debug)]
pub struct FullTranscriptView<'a> {
    /// 화면 상태.
    pub state: &'a FullTranscript,
    /// 대화 기록.
    pub transcript: &'a Transcript,
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
}

impl FullTranscriptView<'_> {
    // cost: time O(c), heap O(c), stack O(1)
    // vars: c = 대화 기록 글자 수
    // basis: estimate
    /// 화면 전체에 모든 셀을 `TranscriptCell::lines(.., expanded: true)`로 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
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
