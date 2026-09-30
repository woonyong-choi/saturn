//! judge 키 입력 창. 시작 때 judge 확인이 실패하면 원인과 가린 키 입력칸을 보인다.
//!
//! 설계: docs/design/tui.md(영역 judge 키 입력 창, 키), docs/design/judge-key-security.md(받은 키의 처리).
//! 키 글은 화면, 로그, 오류, 디버그 출력, 입력 기록 어디에도 원문으로 남기지 않는다. engine에 보낸 뒤 바로 지운다.
//! 키는 `Request::SubmitJudgeKey`로 engine에 넘긴다.

use std::fmt;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// 가린 입력. `Debug`는 글자 수만 보인다(`MaskedInput(len=40)`).
#[derive(Clone, Default)]
pub struct MaskedInput(String);

impl MaskedInput {
    /// 글자 하나 더한다.
    pub fn push(&mut self, c: char) {
        todo!("#92")
    }

    /// 끝 글자 지운다.
    pub fn pop(&mut self) {
        todo!("#92")
    }

    /// 글자 수. 화면에는 이 수만큼 `•`를 그린다.
    pub fn len(&self) -> usize {
        todo!("#92")
    }

    /// 비었다.
    pub fn is_empty(&self) -> bool {
        todo!("#92")
    }

    /// 원문을 꺼내고 비운다. engine에 보낼 때 한 번만.
    pub fn take(&mut self) -> String {
        todo!("#92")
    }
}

impl fmt::Debug for MaskedInput {
    /// 원문 대신 글자 수만 쓴다.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("#92")
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
        todo!("#92")
    }
}
