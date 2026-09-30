//! judge 버전 화면(`/judge version`).
//!
//! 설계: docs/design/tui.md(영역 judge 버전 화면, 키), docs/design/judge-training.md(버전과 보정).
//! - 목록: 버전별 judge, 보정값, ECE.
//! - 상세(`Enter`): 질문별 목표 틀림 비율, 기준값, 최근 200건 틀림, 판단 수.
//! - `r` 1차 영점 복귀(`/train --reset-thresholds`), `t` 고른 버전에서 다시 학습(`/train --from`),
//!   `u` 확인 한 줄 뒤 고른 버전 사용(`saturn judge version`).
//! 목록은 `Request::ListJudgeVersions` → `Notification::JudgeVersions`, 사용은 `Request::UseJudgeVersion`.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// 상세의 최근 판단 창 크기.
pub const RECENT_WINDOW: u32 = 200;

/// 질문 하나의 상세.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionStats {
    /// 질문 id.
    pub question: String,
    /// 목표 틀림 비율.
    pub target_error: f64,
    /// 기준값.
    pub threshold: f64,
    /// 최근 200건 중 틀림 수.
    pub recent_errors: u32,
    /// 판단 수.
    pub judgments: u32,
}

/// 버전 한 행.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeVersionRow {
    /// 버전 이름.
    pub version: String,
    /// judge 이름.
    pub judge: String,
    /// 보정값(원문).
    pub calibration: String,
    /// ECE.
    pub ece: Option<f64>,
    /// 지금 쓰는 버전.
    pub active: bool,
    /// 질문별 상세.
    pub questions: Vec<QuestionStats>,
}

/// judge 버전 화면에서 고른 동작.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeVersionCommand {
    /// `r` → `Request::Train { reset_thresholds: true }`.
    ResetThresholds,
    /// `t` → 고른 버전에서 학습(`Request::Train { from: Some(버전) }`).
    TrainFrom(String),
    /// `u` 확인 뒤 고른 버전 사용.
    Use(String),
}

/// judge 버전 화면 상태.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JudgeVersionScreen {
    /// 버전 목록. 응답 전이면 빈 목록.
    pub rows: Vec<JudgeVersionRow>,
    /// 강조 행.
    pub selected: usize,
    /// `Enter` 상세를 펼쳤다.
    pub detail: bool,
    /// `u` 확인 한 줄을 띄웠다. 다시 `u`나 `Enter`면 확정, `Esc`면 취소.
    pub confirm_use: bool,
}

impl JudgeVersionScreen {
    /// `↑` 이동.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        todo!("#92")
    }

    /// 키 동작을 명령으로. `u`는 처음엔 확인 한 줄만 띄우고 `None`, 확인하면 `Use`.
    pub fn command(&mut self, action: &crate::keys::Action) -> Option<JudgeVersionCommand> {
        todo!("#92")
    }
}

/// judge 버전 화면 그리기.
#[derive(Debug)]
pub struct JudgeVersionView<'a> {
    /// 화면 상태.
    pub screen: &'a JudgeVersionScreen,
    /// 화면 언어.
    pub lang: Lang,
}

impl JudgeVersionView<'_> {
    /// 버전 목록 표(버전, judge, 보정값, ECE, 사용 중 표시), 상세면 질문별 표, 확인 중이면 확인 한 줄을 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
