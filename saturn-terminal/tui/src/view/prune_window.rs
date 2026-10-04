//! 기록 정리 창(`/prune`). 지울 채팅을 보이고 `y`로 확정, `Esc`로 취소한다.
//! 설계: docs/design/tui.md

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::rpc::{ChatListItem, PruneSkipped};

use crate::i18n::{self, Lang};
use crate::view::{MUTED, SELECTED, render_window, truncate};

/// 창 테두리, 머리줄, 빈 줄, 키 안내가 차지하는 줄 수.
const CHROME_ROWS: usize = 5;

/// `PrunePreview`로 받은 지울 채팅과 남긴 채팅 수.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PruneListing {
    pub chats: Vec<ChatListItem>,
    pub kept: usize,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PruneWindow {
    /// 미리보기가 오기 전에는 `None`.
    pub listing: Option<PruneListing>,
    pub selected: usize,
    /// `y`를 보내고 결과를 기다리는 중.
    pub is_deleting: bool,
}

impl PruneWindow {
    pub(crate) fn load(&mut self, chats: Vec<ChatListItem>, skipped: &[PruneSkipped], rows: u64) {
        self.selected = 0;
        self.listing = Some(PruneListing {
            chats,
            kept: skipped.len(),
            rows,
        });
    }

    /// 지울 채팅이 있는 미리보기가 왔고 아직 확정하지 않았을 때만 참.
    pub(crate) fn can_confirm(&self) -> bool {
        !self.is_deleting
            && self
                .listing
                .as_ref()
                .is_some_and(|listing| !listing.chats.is_empty())
    }

    pub(crate) fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub(crate) fn down(&mut self) {
        let last = self
            .listing
            .as_ref()
            .map_or(0, |listing| listing.chats.len().saturating_sub(1));
        self.selected = (self.selected + 1).min(last);
    }
}

#[derive(Debug)]
pub(crate) struct PruneWindowView<'a> {
    pub window: &'a PruneWindow,
    pub lang: Lang,
}

impl PruneWindowView<'_> {
    // cost: time O(h), heap O(h), stack O(1)
    // vars: h = 화면에 보이는 줄 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let lang = self.lang;
        let mut lines: Vec<Line<'static>> = Vec::new();
        match &self.window.listing {
            None => lines.push(Line::from(Span::styled(lang.tr(i18n::LOADING), MUTED))),
            Some(listing) if listing.chats.is_empty() => {
                lines.push(Line::from(Span::styled(
                    lang.tr(i18n::CLI_PRUNE_NOTHING),
                    MUTED,
                )));
                lines.extend(self.kept_line(listing));
            }
            Some(listing) => {
                lines.push(Line::from(
                    lang.tr(i18n::CLI_PRUNE_PLAN)
                        .replace("{chats}", &listing.chats.len().to_string())
                        .replace("{rows}", &listing.rows.to_string()),
                ));
                lines.extend(self.chat_lines(listing, area));
                lines.extend(self.kept_line(listing));
            }
        }
        lines.push(Line::from(""));
        let hint = if self.window.can_confirm() {
            i18n::PRUNE_HINT
        } else {
            i18n::PRUNE_HINT_CLOSE
        };
        lines.push(Line::from(Span::styled(lang.tr(hint), MUTED)));
        render_window(frame, area, lang.tr(i18n::PRUNE_TITLE), lines);
    }

    /// 고른 줄이 보이도록 화면 높이만큼 잘라 그린다.
    fn chat_lines(&self, listing: &PruneListing, area: Rect) -> Vec<Line<'static>> {
        let visible = usize::from(area.height).saturating_sub(CHROME_ROWS).max(1);
        let first = (self.window.selected + 1).saturating_sub(visible);
        let width = usize::from(area.width).saturating_sub(8);
        listing
            .chats
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(index, chat)| {
                let style = if index == self.window.selected {
                    SELECTED
                } else {
                    Style::new()
                };
                Line::from(Span::styled(
                    truncate(&chat_line(self.lang, chat), width),
                    style,
                ))
            })
            .collect()
    }

    fn kept_line(&self, listing: &PruneListing) -> Option<Line<'static>> {
        (listing.kept > 0).then(|| {
            Line::from(Span::styled(
                self.lang
                    .tr(i18n::CLI_PRUNE_KEPT)
                    .replace("{chats}", &listing.kept.to_string()),
                MUTED,
            ))
        })
    }
}

/// `#채팅 번호 · [이름 ·] 마지막 사용 날짜 · 크기`. 크기는 그 채팅의 기록 행 수다.
fn chat_line(lang: Lang, chat: &ChatListItem) -> String {
    let mut parts = vec![format!("#{}", chat.chat.0)];
    if let Some(name) = &chat.name {
        parts.push(name.clone());
    }
    parts.push(date(chat.last_active_ms));
    if let Some(rows) = chat.rows {
        parts.push(lang.tr(i18n::PRUNE_ROWS).replace("{n}", &rows.to_string()));
    }
    parts.join(" · ")
}

/// 터미널 시간대의 `YYYY-MM-DD`.
fn date(unix_ms: u64) -> String {
    i64::try_from(unix_ms)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .map(|at| at.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use saturn_protocol::ids::ChatId;

    use super::*;
    use crate::view::buffer_lines;

    fn chat(id: u64, name: Option<&str>, rows: u64) -> ChatListItem {
        ChatListItem {
            chat: ChatId(id),
            folder: "/work".to_owned(),
            name: name.map(str::to_owned),
            last_active_ms: 1_700_000_000_000,
            preview: None,
            rows: Some(rows),
        }
    }

    fn drawn(window: &PruneWindow) -> String {
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        terminal
            .draw(|frame| {
                PruneWindowView {
                    window,
                    lang: Lang::En,
                }
                .render(frame, frame.area());
            })
            .unwrap();
        buffer_lines(terminal.backend().buffer()).join("\n")
    }

    #[test]
    fn lists_name_last_used_date_and_size_of_each_chat_to_delete() {
        let mut window = PruneWindow::default();
        window.load(
            vec![chat(3, Some("login fix"), 40), chat(5, None, 7)],
            &[PruneSkipped {
                chat: ChatId(9),
                reasons: Vec::new(),
            }],
            47,
        );

        let screen = drawn(&window);

        assert!(screen.contains("Chats to delete: 2 · Rows: 47"), "{screen}");
        assert!(screen.contains("#3 · login fix · 2023-11-1"), "{screen}");
        assert!(screen.contains("· 40 rows"), "{screen}");
        assert!(screen.contains("#5 · 2023-11-1"), "{screen}");
        assert!(screen.contains("Chats kept: 1"), "{screen}");
        assert!(screen.contains("y Delete"), "{screen}");
        assert!(window.can_confirm());
    }

    #[test]
    fn nothing_to_delete_cannot_be_confirmed_and_loading_says_so() {
        let mut window = PruneWindow::default();
        let loading = drawn(&window);
        window.load(Vec::new(), &[], 0);

        let empty = drawn(&window);

        assert!(loading.contains("Loading"), "{loading}");
        assert!(empty.contains("No chats to delete"), "{empty}");
        assert!(!empty.contains("y Delete"), "{empty}");
        assert!(!window.can_confirm());
    }
}
