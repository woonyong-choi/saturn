//! judge 버전 화면(`/judge version`).
//!
//! 설계: docs/design/tui.md(영역 judge 버전 화면, 키), docs/design/judge-training.md(버전과 보정).
//! - 목록: 버전별 judge, 보정값, ECE.
//! - 상세(`Enter`): 질문별 목표 틀림 비율, 기준값, 최근 200건 틀림, 판단 수.
//! - `r` 1차 영점 복귀(`/train --reset-thresholds`), `t` 고른 버전에서 다시 학습(`/train --from`),
//!   `u` 확인 한 줄 뒤 고른 버전 사용(`saturn judge version`).
//!
//! 목록은 `Request::ListJudgeVersions` → `Notification::JudgeVersions`, 사용은 `Request::UseJudgeVersion`.

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use saturn_protocol::rpc::JudgeVersionInfo;

use crate::i18n::{self, Lang};
use crate::keys::Action;
use crate::view::{EMPHASIS, MUTED, SELECTED, truncate, window_block};

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
    /// judge와 보정값(원문, `JudgeVersionInfo::judge`).
    pub judge: String,
    /// ECE.
    pub ece: Option<f64>,
    /// 지금 쓰는 버전.
    pub active: bool,
    /// 질문별 상세.
    pub questions: Vec<QuestionStats>,
}

impl JudgeVersionRow {
    // cost: time O(q), heap O(q), stack O(1)
    // vars: q = 질문 수
    // basis: estimate
    /// protocol 행에서 만든다. `current`와 이름이 같으면 사용 중.
    pub fn from_info(info: JudgeVersionInfo, current: &str) -> Self {
        Self {
            active: info.version == current,
            version: info.version,
            judge: info.judge,
            ece: info.ece,
            questions: info
                .questions
                .into_iter()
                .map(
                    |(question, target_error, threshold, recent_errors, judgments)| QuestionStats {
                        question,
                        target_error,
                        threshold,
                        recent_errors,
                        judgments,
                    },
                )
                .collect(),
        }
    }
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
        self.selected = self.selected.saturating_sub(1);
        self.confirm_use = false;
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
        self.confirm_use = false;
    }

    // cost: time O(v), heap O(v), stack O(1)
    // vars: v = 버전 이름 길이
    // basis: estimate
    /// 키 동작을 명령으로. `u`는 처음엔 확인 한 줄만 띄우고 `None`, 다시 `u`나 `Enter`면 `Use`.
    /// 확인 중이 아닌 `Enter`는 상세를 펼치거나 접는다.
    pub fn command(&mut self, action: &Action) -> Option<JudgeVersionCommand> {
        let version = self.rows.get(self.selected).map(|row| row.version.clone());
        match action {
            Action::ResetThresholds => Some(JudgeVersionCommand::ResetThresholds),
            Action::TrainFrom => version.map(JudgeVersionCommand::TrainFrom),
            Action::UseVersion | Action::Confirm if self.confirm_use => {
                self.confirm_use = false;
                version.map(JudgeVersionCommand::Use)
            }
            Action::UseVersion => {
                self.confirm_use = version.is_some();
                None
            }
            Action::Confirm => {
                self.detail = !self.detail;
                None
            }
            _ => None,
        }
    }
}

impl JudgeVersionScreen {
    /// `Esc`: 확인 한 줄이 떠 있으면 닫고 참, 아니면 거짓(화면 종료).
    pub fn cancel(&mut self) -> bool {
        std::mem::take(&mut self.confirm_use)
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
    // cost: time O(r + q), heap O(r + q), stack O(1)
    // vars: r = 버전 수, q = 상세 질문 수
    // basis: estimate
    /// 버전 목록 표(버전, judge, 보정값, ECE, 사용 중 표시), 상세면 질문별 표, 확인 중이면 확인 한 줄을 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let block = window_block(lang.tr(i18n::JUDGE_VERSION_TITLE));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let width = usize::from(inner.width);
        let mut lines: Vec<Line> = Vec::new();
        if self.screen.rows.is_empty() {
            lines.push(Line::from(Span::styled(lang.tr(i18n::LOADING), MUTED)));
        }
        for (index, row) in self.screen.rows.iter().enumerate() {
            let style = if index == self.screen.selected {
                SELECTED
            } else {
                Style::new()
            };
            lines.push(Line::from(Span::styled(
                truncate(&version_text(lang, row), width),
                style,
            )));
        }
        if let Some(row) = self.screen.rows.get(self.screen.selected)
            && self.screen.detail
        {
            lines.push(Line::from(""));
            lines.extend(question_lines(lang, row));
        }
        lines.push(Line::from(""));
        let hint = if self.screen.confirm_use {
            Span::raw(lang.tr(i18n::JUDGE_VERSION_CONFIRM))
        } else {
            Span::styled(lang.tr(i18n::JUDGE_VERSION_HINT), MUTED)
        };
        lines.push(Line::from(hint));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

/// 버전 행 `v3 · remote/cal-2 · ECE 0.031 · 사용 중`.
fn version_text(lang: Lang, row: &JudgeVersionRow) -> String {
    let ece = row
        .ece
        .map_or_else(|| "-".to_string(), |ece| format!("{ece:.3}"));
    let mut text = format!("{} · {} · ECE {ece}", row.version, row.judge);
    if row.active {
        text.push_str(&format!(" · {}", lang.tr(i18n::JUDGE_VERSION_ACTIVE)));
    }
    text
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 질문 수
// basis: estimate
/// 질문별 상세 표.
fn question_lines(lang: Lang, row: &JudgeVersionRow) -> Vec<Line<'static>> {
    let head = format!(
        "{} · {} · {} · {} · {}",
        lang.tr(i18n::JUDGE_VERSION_QUESTION),
        lang.tr(i18n::JUDGE_VERSION_TARGET),
        lang.tr(i18n::JUDGE_VERSION_THRESHOLD),
        lang.tr(i18n::JUDGE_VERSION_RECENT),
        lang.tr(i18n::JUDGE_VERSION_JUDGMENTS)
    );
    let mut lines = vec![Line::from(Span::styled(head, EMPHASIS))];
    lines.extend(row.questions.iter().map(|q| {
        Line::from(format!(
            "{} · {:.3} · {:.3} · {} / {RECENT_WINDOW} · {}",
            q.question, q.target_error, q.threshold, q.recent_errors, q.judgments
        ))
    }));
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn screen() -> JudgeVersionScreen {
        let infos = [
            JudgeVersionInfo {
                version: "v1".to_string(),
                judge: "remote/cal-1".to_string(),
                ece: Some(0.05),
                questions: vec![("relation".to_string(), 0.1, 0.62, 7, 200)],
            },
            JudgeVersionInfo {
                version: "v2".to_string(),
                judge: "local/cal-2".to_string(),
                ece: None,
                questions: Vec::new(),
            },
        ];
        JudgeVersionScreen {
            rows: infos
                .into_iter()
                .map(|info| JudgeVersionRow::from_info(info, "v2"))
                .collect(),
            ..JudgeVersionScreen::default()
        }
    }

    #[test]
    fn use_version_needs_confirmation() {
        let mut screen = screen();

        let first = screen.command(&Action::UseVersion);
        let second = screen.command(&Action::Confirm);

        assert_eq!(first, None);
        assert_eq!(second, Some(JudgeVersionCommand::Use("v1".to_string())));
    }

    #[test]
    fn cancel_drops_confirmation_only_once() {
        let mut screen = screen();
        screen.command(&Action::UseVersion);

        assert!(screen.cancel());
        assert!(!screen.cancel());
    }

    #[test]
    fn train_from_uses_selected_version() {
        let mut screen = screen();
        screen.down();

        assert_eq!(
            screen.command(&Action::TrainFrom),
            Some(JudgeVersionCommand::TrainFrom("v2".to_string()))
        );
        assert_eq!(
            screen.command(&Action::ResetThresholds),
            Some(JudgeVersionCommand::ResetThresholds)
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_detail_shows_question_stats() {
        let mut screen = screen();
        screen.command(&Action::Confirm);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let view = JudgeVersionView {
            screen: &screen,
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
        assert!(content.contains("v2 · local/cal-2 · ECE - · in use"));
        assert!(content.contains("relation · 0.100 · 0.620 · 7 / 200 · 200"));
    }
}
