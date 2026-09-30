//! 전체 기록(`Ctrl+T`). 도구 셀 전체와 줄인 셀을 펼친 대화 기록.
//!
//! 설계: docs/design/tui.md(영역 전체 기록). 대화 기록과 같은 셀을 `expanded: true`로 그린다.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;
use crate::view::transcript::Transcript;

/// 전체 기록 화면 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FullTranscript {
    /// 맨 위에서 내려온 줄 수.
    pub scroll: usize,
}

impl FullTranscript {
    /// `↑` 한 줄 위로.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 한 줄 아래로. 끝을 넘지 않는 것은 그릴 때 자른다.
    pub fn down(&mut self) {
        todo!("#92")
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
    /// 화면 전체에 모든 셀을 `TranscriptCell::lines(.., expanded: true)`로 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
