//! 학습 확인 창. 실행 조건(채점 후보 200건 이상)을 채운 `/train`에서 뜬다.
//!
//! 설계: docs/design/tui.md(영역 학습 확인 창, 키), docs/design/judge-training.md.
//! 조건을 못 채우면 창 대신 대화 기록에 `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`.
//! TODO(#46): 학습 확인에 필요한 값을 받는 알림이 protocol에 없다

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// `/train` 실행에 필요한 채점 후보 수.
pub const MIN_CANDIDATES: u32 = 200;

/// 선택지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainChoice {
    /// 실행.
    Run,
    /// 취소.
    Cancel,
}

/// 학습 확인 창 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainConfirm {
    /// 채점 후보 수.
    pub candidates: u32,
    /// 채점 모델.
    pub grading_model: String,
    /// 예상 토큰.
    pub estimated_tokens: u64,
    /// 기준값 조정 대상 질문.
    pub threshold_targets: Vec<String>,
    /// 모델 추가 학습 여부.
    pub fine_tune: bool,
    /// `--reset-thresholds`로 열었다.
    pub reset_thresholds: bool,
    /// 강조한 선택지.
    pub selected: TrainChoice,
}

impl TrainConfirm {
    /// `↑` 이동.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        todo!("#92")
    }
}

/// 학습 확인 창 그리기.
#[derive(Debug)]
pub struct TrainConfirmView<'a> {
    /// 창 상태.
    pub confirm: &'a TrainConfirm,
    /// 화면 언어.
    pub lang: Lang,
}

impl TrainConfirmView<'_> {
    /// 채점 후보 수, 채점 모델, 예상 토큰(천 단위 쉼표), 기준값 조정 대상, 모델 추가 학습 여부, 선택지를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
