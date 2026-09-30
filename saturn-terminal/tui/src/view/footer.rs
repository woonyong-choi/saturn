//! 바닥줄. 왼쪽 키 안내, 오른쪽 맥락 크기.
//!
//! 설계: docs/design/tui.md(영역 바닥줄). 상태 변경 때와 턴마다 맥락 크기를 새로 그린다.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;
use crate::state::ContextSize;

/// 맥락 크기 문구. `맥락 38K/200K`, 측정하지 못했거나 아직 모르면 `맥락 미확인`.
pub fn context_text(lang: Lang, context: Option<ContextSize>) -> String {
    todo!("#92")
}

/// 바닥줄 그리기.
#[derive(Debug)]
pub struct FooterView {
    /// 화면 언어.
    pub lang: Lang,
    /// 맥락 크기.
    pub context: Option<ContextSize>,
}

impl FooterView {
    /// 왼쪽 `/help 도움말 · Ctrl+C 멈춤`, 오른쪽 `context_text`. 폭이 모자라면 왼쪽을 먼저 자른다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
