//! 채팅 선택 창(`saturn --resume`). `/model` 선택 창처럼 방향키로 고르고 `Enter`로 연다.
//! 설계: docs/design/engine-lifecycle.md

use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::ChatListItem;

use crate::TuiError;
use crate::i18n::{self, Lang};
use crate::terminal::{self, TerminalError};
use crate::view::{MUTED, SELECTED, render_window, truncate};

/// 줄의 첫 입력 미리보기 글자 수. 초안 값.
const PREVIEW_CHARS: usize = 60;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

/// 창 테두리와 빈 줄, 키 안내가 차지하는 줄 수.
const CHROME_ROWS: usize = 4;

/// 창이 화면 가장자리와 두는 여백 칸 수(양쪽 테두리 포함).
const SIDE_MARGIN: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickerOutcome {
    Open(ChatId),
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatPicker {
    chats: Vec<ChatListItem>,
    selected: usize,
    /// 모든 폴더 목록이면 줄에 폴더를 보인다.
    show_folder: bool,
    now_ms: u64,
}

impl ChatPicker {
    pub(crate) fn new(chats: Vec<ChatListItem>, show_folder: bool, now_ms: u64) -> Self {
        Self {
            chats,
            selected: 0,
            show_folder,
            now_ms,
        }
    }

    /// `↑`, `↓`는 이동, `Enter`는 열기, `Esc`와 `Ctrl+C`는 취소.
    pub(crate) fn on_key(&mut self, key: KeyEvent) -> Option<PickerOutcome> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => {
                let last = self.chats.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
            }
            KeyCode::Enter => {
                return self
                    .chats
                    .get(self.selected)
                    .map(|chat| PickerOutcome::Open(chat.chat));
            }
            KeyCode::Esc => return Some(PickerOutcome::Cancel),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PickerOutcome::Cancel);
            }
            _ => {}
        }
        None
    }

    // cost: time O(h), heap O(h), stack O(1)
    // vars: h = 화면에 보이는 줄 수
    // basis: estimate
    /// 고른 줄이 보이도록 목록을 화면 높이만큼 잘라 그린다.
    fn render(&self, frame: &mut Frame, area: Rect, lang: Lang) {
        let visible = usize::from(area.height).saturating_sub(CHROME_ROWS).max(1);
        let first = (self.selected + 1).saturating_sub(visible);
        let width = usize::from(area.width).saturating_sub(SIDE_MARGIN);
        let mut lines: Vec<Line<'static>> = self
            .chats
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(index, chat)| {
                let text = row(lang, index + 1, chat, self.show_folder, self.now_ms);
                let style = if index == self.selected {
                    SELECTED
                } else {
                    Style::new()
                };
                Line::from(Span::styled(truncate(&text, width), style))
            })
            .collect();
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            lang.tr(i18n::CHAT_PICKER_HINT),
            MUTED,
        )));
        render_window(frame, area, lang.tr(i18n::CHAT_PICKER_TITLE), lines);
    }
}

/// `번호. ` 뒤에 `chat_summary`.
fn row(lang: Lang, number: usize, chat: &ChatListItem, show_folder: bool, now_ms: u64) -> String {
    format!("{number}. {}", summary(lang, chat, show_folder, now_ms))
}

/// `#채팅 id · 경과 · [폴더 ·] [이름 ·] 첫 입력`. 채팅 목록과 정리 미리보기가 같은 줄을 쓴다.
pub fn chat_summary(lang: Lang, chat: &ChatListItem, show_folder: bool) -> String {
    summary(lang, chat, show_folder, now_ms())
}

fn summary(lang: Lang, chat: &ChatListItem, show_folder: bool, now_ms: u64) -> String {
    let mut parts = vec![
        format!("#{}", chat.chat.0),
        age(lang, now_ms.saturating_sub(chat.last_active_ms)),
    ];
    if show_folder {
        parts.push(chat.folder.clone());
    }
    if let Some(name) = &chat.name {
        parts.push(name.clone());
    }
    parts.push(preview(lang, chat.preview.as_deref()));
    parts.join(" · ")
}

pub(crate) fn age(lang: Lang, elapsed_ms: u64) -> String {
    let (phrase, count) = if elapsed_ms >= DAY_MS {
        (i18n::CLI_AGE_DAYS, elapsed_ms / DAY_MS)
    } else if elapsed_ms >= HOUR_MS {
        (i18n::CLI_AGE_HOURS, elapsed_ms / HOUR_MS)
    } else if elapsed_ms >= MINUTE_MS {
        (i18n::CLI_AGE_MINUTES, elapsed_ms / MINUTE_MS)
    } else {
        return lang.tr(i18n::CLI_AGE_NOW).to_owned();
    };
    lang.tr(phrase).replace("{n}", &count.to_string())
}

/// 첫 줄만, `PREVIEW_CHARS`자까지.
fn preview(lang: Lang, text: Option<&str>) -> String {
    let Some(first_line) = text.and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
    else {
        return lang.tr(i18n::CLI_PICK_NO_INPUT).to_owned();
    };
    let first_line = first_line.trim();
    if first_line.chars().count() <= PREVIEW_CHARS {
        return first_line.to_owned();
    }
    let cut: String = first_line.chars().take(PREVIEW_CHARS).collect();
    format!("{cut}…")
}

pub(crate) fn now_ms() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

// cost: time O(k·h), heap O(c), stack O(1), io k
// vars: k = 누른 키 수, h = 화면 줄 수, c = 목록의 채팅 수
// basis: estimate
/// 전체 화면 선택 창을 열고 고른 채팅을 돌려준다. `Esc`면 `None`. 오류나 panic으로 끝나도 터미널을 복원한다.
///
/// # Errors
/// 터미널 설정·복원이나 그리기, 키 읽기 실패.
pub fn pick_chat(
    lang: Lang,
    chats: Vec<ChatListItem>,
    show_folder: bool,
) -> Result<Option<ChatId>, TuiError> {
    let mut picker = ChatPicker::new(chats, show_folder, now_ms());
    let mut screen = terminal::enter()?;
    let picked = loop {
        let drawn = screen
            .draw(|frame| picker.render(frame, frame.area(), lang))
            .map_err(TerminalError::Draw);
        let event = drawn
            .and_then(|_| event::read().map_err(TerminalError::Draw))
            .map_err(TuiError::from);
        match event {
            Ok(Event::Key(key)) => {
                if let Some(outcome) = picker.on_key(key) {
                    break Ok(outcome);
                }
            }
            Ok(_) => {}
            Err(error) => break Err(error),
        }
    };
    let restored = terminal::leave();
    let outcome = picked?;
    restored?;
    Ok(match outcome {
        PickerOutcome::Open(chat) => Some(chat),
        PickerOutcome::Cancel => None,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::view::buffer_lines;

    const NOW: u64 = 10 * DAY_MS;

    fn item(chat: u64, folder: &str, last_active_ms: u64, preview: Option<&str>) -> ChatListItem {
        ChatListItem {
            chat: ChatId(chat),
            folder: folder.to_owned(),
            name: None,
            last_active_ms,
            preview: preview.map(str::to_owned),
        }
    }

    fn list() -> Vec<ChatListItem> {
        vec![
            item(
                9,
                "/work/a",
                NOW - 2 * HOUR_MS,
                Some("fix login\nsecond line"),
            ),
            item(4, "/work/b", NOW - 3 * DAY_MS, None),
        ]
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn draw(picker: &ChatPicker, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| picker.render(frame, frame.area(), Lang::En))
            .unwrap();
        buffer_lines(terminal.backend().buffer()).join("\n")
    }

    #[test]
    fn arrow_keys_move_inside_the_list_and_enter_opens_the_selected_chat() {
        let mut picker = ChatPicker::new(list(), false, NOW);

        assert_eq!(picker.on_key(key(KeyCode::Up)), None);
        assert_eq!(picker.on_key(key(KeyCode::Down)), None);
        assert_eq!(picker.on_key(key(KeyCode::Down)), None);
        let opened = picker.on_key(key(KeyCode::Enter));

        assert_eq!(opened, Some(PickerOutcome::Open(ChatId(4))));
    }

    #[test]
    fn enter_on_the_first_line_opens_the_most_recent_chat() {
        let mut picker = ChatPicker::new(list(), false, NOW);

        assert_eq!(
            picker.on_key(key(KeyCode::Enter)),
            Some(PickerOutcome::Open(ChatId(9)))
        );
    }

    #[test]
    fn escape_and_ctrl_c_cancel_without_opening_a_chat() {
        for cancel in [
            key(KeyCode::Esc),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut picker = ChatPicker::new(list(), false, NOW);

            assert_eq!(picker.on_key(cancel), Some(PickerOutcome::Cancel));
        }
        let mut picker = ChatPicker::new(list(), false, NOW);
        assert_eq!(picker.on_key(key(KeyCode::Char('x'))), None);
    }

    #[test]
    fn render_lists_rows_with_the_selected_line_and_the_key_hint() {
        let picker = ChatPicker::new(list(), false, NOW);

        let text = draw(&picker, 80, 12);

        assert!(text.contains("1. #9 · 2h ago · fix login"));
        assert!(text.contains("2. #4 · 3d ago · (no input)"));
        assert!(text.contains("↑↓ move · Enter open · Esc cancel"));
    }

    #[test]
    fn render_shows_the_folder_only_for_every_folder_lists() {
        let in_folder = draw(&ChatPicker::new(list(), false, NOW), 80, 12);
        let everywhere = draw(&ChatPicker::new(list(), true, NOW), 80, 12);

        assert!(!in_folder.contains("/work/a"));
        assert!(everywhere.contains("1. #9 · 2h ago · /work/a · fix login"));
    }

    #[test]
    fn render_shows_the_chat_name_before_the_first_input() {
        let mut named = list();
        named[0].name = Some("login fix".to_owned());

        let text = draw(&ChatPicker::new(named, false, NOW), 80, 12);

        assert!(text.contains("1. #9 · 2h ago · login fix · fix login"));
        assert!(text.contains("2. #4 · 3d ago · (no input)"));
    }

    #[test]
    fn render_scrolls_to_keep_the_selected_line_visible() {
        let chats: Vec<_> = (1..=20).map(|id| item(id, "/w", NOW, Some("x"))).collect();
        let mut picker = ChatPicker::new(chats, false, NOW);
        (0..15).for_each(|_| {
            picker.on_key(key(KeyCode::Down));
        });

        let text = draw(&picker, 60, 9);

        assert!(text.contains("16. #16"));
        assert!(!text.contains("1. #1 "));
    }

    #[test]
    fn long_previews_are_cut_to_the_first_line() {
        let long = "가".repeat(PREVIEW_CHARS + 5);

        let text = preview(Lang::En, Some(&long));

        assert_eq!(text.chars().count(), PREVIEW_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
