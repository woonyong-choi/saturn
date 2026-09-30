//! 보류 재개 질문. 보류 작업이 있는 채팅을 다시 열 때 한 번 뜬다.
//!
//! 설계: docs/design/tui.md(영역 보류 재개 질문, 예시 멈춘 작업을 다시 연다), docs/design/engine-lifecycle.md(크래시 뒤 보류).
//! 선택지 `모두 이어서`(`Continue { task: None }`), `골라서 이어서`(목록에서 골라 작업마다 `Continue { task: Some }`),
//! `그대로 두기`(아무것도 보내지 않는다).

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{TaskId, TaskLabel};

use crate::i18n::Lang;

/// 선택지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeChoice {
    /// `모두 이어서`.
    All,
    /// `골라서 이어서`. 고르면 보류 목록 선택 단계로 넘어간다.
    Pick,
    /// `그대로 두기`.
    Leave,
}

/// 보류 재개 질문의 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    /// 아직 고르는 중.
    Pending,
    /// 채팅 보류 전부 재개.
    ContinueAll,
    /// 고른 작업만 재개. 접수 순서.
    Continue(Vec<TaskId>),
    /// 그대로 둔다.
    Leave,
}

/// 보류 재개 질문 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePrompt {
    /// 보류된 작업(이름표 순서).
    pub held: Vec<(TaskId, TaskLabel)>,
    /// 강조한 선택지.
    pub selected: ResumeChoice,
    /// `골라서 이어서` 단계면 목록 강조 위치와 고른 작업. 그 밖에는 `None`.
    pub picking: Option<(usize, Vec<TaskId>)>,
}

impl ResumePrompt {
    /// 보류 목록으로 연다. 강조는 `모두 이어서`.
    pub fn new(held: Vec<(TaskId, TaskLabel)>) -> Self {
        todo!("#92")
    }

    /// `↑` 이동.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        todo!("#92")
    }

    /// `Enter`. 선택 단계에서는 강조한 작업을 고르거나 빼고, 목록 끝의 확정 행에서 `Continue`를 돌려준다.
    pub fn confirm(&mut self) -> ResumeOutcome {
        todo!("#92")
    }
}

/// 보류 재개 질문 그리기.
#[derive(Debug)]
pub struct ResumePromptView<'a> {
    /// 창 상태.
    pub prompt: &'a ResumePrompt,
    /// 화면 언어.
    pub lang: Lang,
}

impl ResumePromptView<'_> {
    /// 보류 목록 `[A] [C]`과 선택지 세 개, 선택 단계면 작업마다 고름 표시를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
