//! router 키 입력 창. 시작 때 router 확인이 실패하면 원인과 가린 키 입력칸을 보인다.
//! 설계: docs/design/router-key-security.md

use std::fmt;

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, render_window};

/// 키 원문은 화면, 로그, 입력 기록 어디에도 남기지 않으므로 `Debug`도 글자 수만 보인다.
#[derive(Clone, Default)]
pub struct MaskedInput(String);

impl MaskedInput {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn push(&mut self, c: char) {
        self.0.push(c);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn pop(&mut self) {
        self.0.pop();
    }

    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// engine에 보낼 때 한 번만 부른다.
    pub fn take(&mut self) -> String {
        std::mem::take(&mut self.0)
    }
}

impl fmt::Debug for MaskedInput {
    /// 원문 대신 글자 수만 쓴다.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MaskedInput(len={})", self.len())
    }
}

#[derive(Debug, Clone, Default)]
pub struct RouterKeyPrompt {
    /// engine이 키를 가린 문구.
    pub cause: String,
    pub input: MaskedInput,
}

#[derive(Debug)]
pub struct RouterKeyPromptView<'a> {
    pub prompt: &'a RouterKeyPrompt,
    pub lang: Lang,
}

impl RouterKeyPromptView<'_> {
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let lines = vec![
            Line::from(self.prompt.cause.clone()),
            Line::from(""),
            Line::from(format!("› {}", "•".repeat(self.prompt.input.len()))),
            Line::from(""),
            Line::from(Span::styled(lang.tr(i18n::ROUTER_KEY_HINT), MUTED)),
        ];
        render_window(frame, area, lang.tr(i18n::ROUTER_KEY_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn masked(text: &str) -> MaskedInput {
        let mut input = MaskedInput::default();
        text.chars().for_each(|c| input.push(c));
        input
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn debug_hides_key_text() {
        let prompt = RouterKeyPrompt {
            cause: "invalid key".to_string(),
            input: masked("sk-secret"),
        };

        let debug = format!("{prompt:?}");

        assert!(debug.contains("MaskedInput(len=9)"));
        assert!(!debug.contains("sk-secret"));
    }

    #[test]
    fn take_returns_text_once() {
        let mut input = masked("ab");
        input.pop();
        input.push('c');

        assert_eq!(input.take(), "ac");
        assert!(input.is_empty());
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_masks_key_with_dots() {
        let prompt = RouterKeyPrompt {
            cause: "router check failed".to_string(),
            input: masked("secret"),
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        let view = RouterKeyPromptView {
            prompt: &prompt,
            lang: Lang::Ko,
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
        assert!(content.contains("••••••"));
        assert!(!content.contains("secret"));
        assert!(content.contains("router check failed"));
    }
}
