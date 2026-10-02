//! router 버전 화면(`/router version`).
//! 설계: docs/design/tui.md

use ratatui::Frame;
use ratatui::layout::Rect;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use saturn_protocol::rpc::RouterVersionInfo;

use crate::i18n::{self, Lang};
use crate::keys::Action;
use crate::view::{EMPHASIS, MUTED, SELECTED, truncate, window_block};

/// 상세의 최근 판단 건수.
pub(crate) const RECENT_WINDOW: u32 = 200;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuestionStats {
    pub question: String,
    pub target_error: f64,
    pub threshold: f64,
    pub recent_errors: u32,
    pub judgments: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RouterVersionRow {
    pub version: String,
    pub router: String,
    pub ece: Option<f64>,
    pub active: bool,
    pub questions: Vec<QuestionStats>,
}

impl RouterVersionRow {
    // cost: time O(q), heap O(q), stack O(1)
    // vars: q = 질문 수
    // basis: estimate
    /// `current`와 이름이 같으면 사용 중.
    pub(crate) fn from_info(info: RouterVersionInfo, current: &str) -> Self {
        Self {
            active: info.version == current,
            version: info.version,
            router: info.router,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RouterVersionCommand {
    ResetThresholds,
    TrainFrom(String),
    Use(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct RouterVersionScreen {
    pub rows: Vec<RouterVersionRow>,
    pub selected: usize,
    pub detail: bool,
    pub confirm_use: bool,
}

impl RouterVersionScreen {
    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.confirm_use = false;
    }

    pub(crate) fn down(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
        self.confirm_use = false;
    }

    // cost: time O(v), heap O(v), stack O(1)
    // vars: v = 버전 이름 길이
    // basis: estimate
    /// `u`는 처음엔 확인 한 줄만 띄우고 `None`을 돌려준다.
    pub(crate) fn command(&mut self, action: &Action) -> Option<RouterVersionCommand> {
        let version = self.rows.get(self.selected).map(|row| row.version.clone());
        match action {
            Action::ResetThresholds => Some(RouterVersionCommand::ResetThresholds),
            Action::TrainFrom => version.map(RouterVersionCommand::TrainFrom),
            Action::UseVersion | Action::Confirm if self.confirm_use => {
                self.confirm_use = false;
                version.map(RouterVersionCommand::Use)
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

impl RouterVersionScreen {
    /// 확인 한 줄이 떠 있었으면 닫고 `true`, 아니면 `false`(화면 종료).
    pub(crate) fn cancel(&mut self) -> bool {
        std::mem::take(&mut self.confirm_use)
    }
}

#[derive(Debug)]
pub(crate) struct RouterVersionView<'a> {
    pub screen: &'a RouterVersionScreen,
    pub lang: Lang,
}

impl RouterVersionView<'_> {
    // cost: time O(r + q), heap O(r + q), stack O(1)
    // vars: r = 버전 수, q = 상세 질문 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let block = window_block(lang.tr(i18n::ROUTER_VERSION_TITLE));
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
            Span::raw(lang.tr(i18n::ROUTER_VERSION_CONFIRM))
        } else {
            Span::styled(lang.tr(i18n::ROUTER_VERSION_HINT), MUTED)
        };
        lines.push(Line::from(hint));
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn version_text(lang: Lang, row: &RouterVersionRow) -> String {
    let ece = row
        .ece
        .map_or_else(|| "-".to_string(), |ece| format!("{ece:.3}"));
    let mut text = format!("{} · {} · ECE {ece}", row.version, row.router);
    if row.active {
        text.push_str(&format!(" · {}", lang.tr(i18n::ROUTER_VERSION_ACTIVE)));
    }
    text
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 질문 수
// basis: estimate
fn question_lines(lang: Lang, row: &RouterVersionRow) -> Vec<Line<'static>> {
    let head = format!(
        "{} · {} · {} · {} · {}",
        lang.tr(i18n::ROUTER_VERSION_QUESTION),
        lang.tr(i18n::ROUTER_VERSION_TARGET),
        lang.tr(i18n::ROUTER_VERSION_THRESHOLD),
        lang.tr(i18n::ROUTER_VERSION_RECENT),
        lang.tr(i18n::ROUTER_VERSION_JUDGMENTS)
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
    fn screen() -> RouterVersionScreen {
        let infos = [
            RouterVersionInfo {
                version: "v1".to_string(),
                router: "remote/cal-1".to_string(),
                ece: Some(0.05),
                questions: vec![("relation".to_string(), 0.1, 0.62, 7, 200)],
            },
            RouterVersionInfo {
                version: "v2".to_string(),
                router: "local/cal-2".to_string(),
                ece: None,
                questions: Vec::new(),
            },
        ];
        RouterVersionScreen {
            rows: infos
                .into_iter()
                .map(|info| RouterVersionRow::from_info(info, "v2"))
                .collect(),
            ..RouterVersionScreen::default()
        }
    }

    #[test]
    fn use_version_needs_confirmation() {
        let mut screen = screen();

        let first = screen.command(&Action::UseVersion);
        let second = screen.command(&Action::Confirm);

        assert_eq!(first, None);
        assert_eq!(second, Some(RouterVersionCommand::Use("v1".to_string())));
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
            Some(RouterVersionCommand::TrainFrom("v2".to_string()))
        );
        assert_eq!(
            screen.command(&Action::ResetThresholds),
            Some(RouterVersionCommand::ResetThresholds)
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_detail_shows_question_stats() {
        let mut screen = screen();
        screen.command(&Action::Confirm);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        let view = RouterVersionView {
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
