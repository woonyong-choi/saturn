//! 학습 확인 창. 실행 조건(채점 후보 200건 이상)을 채운 `/train`에서 뜬다.
//!
//! 설계: docs/design/tui.md(영역 학습 확인 창, 키), docs/design/judge-training.md.
//! 조건을 못 채우면 창 대신 대화 기록에 `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`.
//! 값은 `Notification::TrainPreview`로 받고 답은 `Request::ConfirmTrain`으로 보낸다.

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::i18n::{self, Lang};
use crate::view::{SELECTED, render_window};

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
        self.selected = TrainChoice::Run;
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        self.selected = TrainChoice::Cancel;
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
    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 조정 대상 글자 수
    // basis: estimate
    /// 채점 후보 수, 채점 모델, 예상 토큰(천 단위 쉼표), 기준값 조정 대상, 모델 추가 학습 여부, 선택지를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let confirm = self.confirm;
        let yes_no = |value: bool| lang.tr(if value { i18n::YES } else { i18n::NO });
        let targets = if confirm.threshold_targets.is_empty() {
            "-".to_string()
        } else {
            confirm.threshold_targets.join(", ")
        };
        let mut lines = vec![
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_CANDIDATES),
                i18n::format_count(u64::from(confirm.candidates))
            )),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_GRADER),
                confirm.grading_model
            )),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_TOKENS),
                i18n::format_count(confirm.estimated_tokens)
            )),
            Line::from(format!("{}: {targets}", lang.tr(i18n::TRAIN_TARGETS))),
            Line::from(format!(
                "{}: {}",
                lang.tr(i18n::TRAIN_RETRAIN),
                yes_no(confirm.fine_tune)
            )),
            Line::from(""),
        ];
        for (choice, text) in [
            (TrainChoice::Run, i18n::PERMISSION_ALLOW),
            (TrainChoice::Cancel, i18n::CANCEL),
        ] {
            let style = if choice == confirm.selected {
                SELECTED
            } else {
                Style::new()
            };
            lines.push(Line::from(Span::styled(lang.tr(text), style)));
        }
        render_window(frame, area, lang.tr(i18n::TRAIN_TITLE), lines);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn confirm() -> TrainConfirm {
        TrainConfirm {
            candidates: 240,
            grading_model: "grader-1".to_string(),
            estimated_tokens: 1_200_000,
            threshold_targets: vec!["relation".to_string()],
            fine_tune: false,
            reset_thresholds: false,
            selected: TrainChoice::Run,
        }
    }

    #[test]
    fn up_down_switch_choice() {
        let mut confirm = confirm();

        confirm.down();
        let down = confirm.selected;
        confirm.up();

        assert_eq!(down, TrainChoice::Cancel);
        assert_eq!(confirm.selected, TrainChoice::Run);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_values_with_separators() {
        let confirm = confirm();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let view = TrainConfirmView {
            confirm: &confirm,
            lang: Lang::En,
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
        assert!(content.contains("candidates: 240"));
        assert!(content.contains("estimated tokens: 1,200,000"));
        assert!(content.contains("retrain model: no"));
    }
}
