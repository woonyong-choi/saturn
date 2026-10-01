//! judge 키 입력 창. 시작 때 judge 확인이 실패하면 원인과 가린 키 입력칸을 보인다.
//!
//! 설계: docs/design/tui.md(영역 judge 키 입력 창, 키), docs/design/judge-key-security.md(받은 키의 처리).
//! 키 글은 화면, 로그, 오류, 디버그 출력, 입력 기록 어디에도 원문으로 남기지 않는다. engine에 보낸 뒤 바로 지운다.
//! 키는 `Request::SubmitJudgeKey`로 engine에 넘긴다.

use std::fmt;

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, render_window};

/// 가린 입력. `Debug`는 글자 수만 보인다(`MaskedInput(len=40)`).
#[derive(Clone, Default)]
pub struct MaskedInput(String);

impl MaskedInput {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 글자 하나 더한다.
    pub fn push(&mut self, c: char) {
        self.0.push(c);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 끝 글자 지운다.
    pub fn pop(&mut self) {
        self.0.pop();
    }

    /// 글자 수. 화면에는 이 수만큼 `•`를 그린다.
    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 비었다.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 원문을 꺼내고 비운다. engine에 보낼 때 한 번만.
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

/// judge 키 입력 창 상태. `Enter` 키 확인, `Esc` 종료.
#[derive(Debug, Clone, Default)]
pub struct JudgeKeyPrompt {
    /// judge 확인 실패 원인 한 줄(engine이 키를 가린 문구).
    pub cause: String,
    /// 가린 키 입력칸.
    pub input: MaskedInput,
}

/// judge 키 입력 창 그리기.
#[derive(Debug)]
pub struct JudgeKeyPromptView<'a> {
    /// 창 상태.
    pub prompt: &'a JudgeKeyPrompt,
    /// 화면 언어.
    pub lang: Lang,
}

impl JudgeKeyPromptView<'_> {
    /// 가운데 창에 원인과 `•` 입력칸, `Enter 확인 · Esc 종료` 안내를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let lines = vec![
            Line::from(self.prompt.cause.clone()),
            Line::from(""),
            Line::from(format!("› {}", "•".repeat(self.prompt.input.len()))),
            Line::from(""),
            Line::from(Span::styled(lang.tr(i18n::JUDGE_KEY_HINT), MUTED)),
        ];
        render_window(frame, area, lang.tr(i18n::JUDGE_KEY_TITLE), lines);
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
        let prompt = JudgeKeyPrompt {
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
        let prompt = JudgeKeyPrompt {
            cause: "judge check failed".to_string(),
            input: masked("secret"),
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).unwrap();
        let view = JudgeKeyPromptView {
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
        assert!(content.contains("judge check failed"));
    }
}
