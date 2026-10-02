//! 화면 그리기. 그리기 함수는 상태를 바꾸지 않는다.
//! 설계: docs/design/tui.md
//! TODO(#58): 좁은 가로 폭에서 버튼과 칸을 줄이는 방식

pub(crate) mod composer;
pub(crate) mod folder_trust;
pub(crate) mod footer;
pub(crate) mod full_transcript;
pub(crate) mod input_form;
pub(crate) mod input_request;
pub(crate) mod live_area;
pub(crate) mod model_picker;
pub(crate) mod permission;
pub(crate) mod popup;
pub(crate) mod resume_prompt;
pub(crate) mod router_key_prompt;
pub(crate) mod router_version;
pub(crate) mod start_screen;
pub(crate) mod status_board;
pub(crate) mod task_list;
pub(crate) mod train_confirm;
pub(crate) mod transcript;
pub mod usage;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

pub const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// 초안 값.
pub const WINDOW_MIN_WIDTH: usize = 40;

/// 색 없이 글자 속성만 쓴다.
pub const MUTED: Style = Style::new().add_modifier(Modifier::DIM);
pub const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
pub const EMPHASIS: Style = Style::new().add_modifier(Modifier::BOLD);

/// 높이 0인 영역은 그리지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Areas {
    pub transcript: Rect,
    pub live: Rect,
    pub status: Rect,
    pub popup: Rect,
    pub composer: Rect,
    pub footer: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Heights {
    /// 상한 적용 전.
    pub live: u16,
    /// TODO(#51): 상태판 최대 높이 규칙
    pub status: u16,
    pub popup: u16,
    pub composer: u16,
}

/// 모자라면 작업별 출력 칸, 상태판, 팝업 순으로 줄이고 입력창과 바닥줄은 줄이지 않는다(초안).
pub fn layout(area: Rect, heights: Heights) -> Areas {
    let fixed = 1 + heights.composer.max(1);
    let mut free = area.height.saturating_sub(fixed);
    let popup = heights.popup.min(free);
    free -= popup;
    let status = heights.status.min(free);
    free -= status;
    let live = heights.live.min(free);
    free -= live;
    let composer = heights.composer.max(1).min(area.height.saturating_sub(1));
    let footer = area.height.min(1);
    let mut y = area.y;
    let mut take = |height: u16| {
        let rect = Rect::new(area.x, y, area.width, height);
        y += height;
        rect
    };
    Areas {
        transcript: take(free),
        live: take(live),
        status: take(status),
        popup: take(popup),
        composer: take(composer),
        footer: take(footer),
    }
}

/// 폭과 높이는 `area` 안으로 자른다.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

pub fn spinner(tick: u64) -> char {
    SPINNER[(tick % SPINNER.len() as u64) as usize]
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = text.len()
// basis: estimate
/// 넓은 글자는 2칸으로 재고 넘치면 끝을 `…`로 바꾼다.
pub fn truncate(text: &str, width: usize) -> String {
    if Span::raw(text).width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = text.len()
// basis: estimate
/// 빈 글은 빈 줄 하나, `width`가 0이면 접지 않는다.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut rows = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > width && used > 0 {
            rows.push(String::new());
            used = 0;
        }
        if let Some(row) = rows.last_mut() {
            row.push(c);
        }
        used += w;
    }
    rows
}

pub fn char_width(c: char) -> usize {
    let mut buffer = [0; 4];
    Span::raw(&*c.encode_utf8(&mut buffer)).width()
}

pub fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

pub fn window_block(title: &str) -> Block<'static> {
    Block::bordered().title(Span::styled(format!(" {title} "), EMPHASIS))
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 창 줄 글자 수
// basis: estimate
/// 폭은 가장 긴 줄에 맞추되 `WINDOW_MIN_WIDTH`와 화면 폭 사이로 둔다.
pub fn render_window(frame: &mut Frame, area: Rect, title: &str, lines: Vec<Line<'static>>) {
    let widest = lines.iter().map(Line::width).max().unwrap_or(0);
    let widest = widest.max(text_width(title) + 2).max(WINDOW_MIN_WIDTH);
    let width = u16::try_from(widest + 2).unwrap_or(u16::MAX);
    let height = u16::try_from(lines.len() + 2).unwrap_or(u16::MAX);
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    frame.render_widget(Paragraph::new(lines).block(window_block(title)), rect);
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 테스트용. 넓은 글자 뒤에 가려진 칸은 건너뛰고 줄 끝 공백은 지운다.
#[cfg(test)]
pub(crate) fn buffer_lines(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| {
            let mut row = String::new();
            let mut skip = 0;
            for x in area.x..area.right() {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                skip = text_width(symbol).saturating_sub(1);
                row.push_str(symbol);
            }
            row.trim_end().to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(live: u16, status: u16, popup: u16, composer: u16) -> Heights {
        Heights {
            live,
            status,
            popup,
            composer,
        }
    }

    #[test]
    fn layout_stacks_areas_top_to_bottom() {
        let areas = layout(Rect::new(0, 0, 80, 30), heights(3, 2, 0, 1));

        assert_eq!(areas.transcript, Rect::new(0, 0, 80, 23));
        assert_eq!(areas.live, Rect::new(0, 23, 80, 3));
        assert_eq!(areas.status, Rect::new(0, 26, 80, 2));
        assert_eq!(areas.popup.height, 0);
        assert_eq!(areas.composer, Rect::new(0, 28, 80, 1));
        assert_eq!(areas.footer, Rect::new(0, 29, 80, 1));
    }

    #[test]
    fn layout_small_screen_shrinks_live_before_status() {
        let areas = layout(Rect::new(0, 0, 80, 6), heights(5, 3, 0, 1));

        assert_eq!(areas.transcript.height, 0);
        assert_eq!(areas.status.height, 3);
        assert_eq!(areas.live.height, 1);
        assert_eq!(areas.composer.height, 1);
        assert_eq!(areas.footer.height, 1);
    }

    #[test]
    fn centered_clamps_to_area() {
        let rect = centered(Rect::new(0, 0, 20, 10), 40, 4);

        assert_eq!(rect, Rect::new(0, 3, 20, 4));
    }

    #[test]
    fn spinner_wraps_around() {
        assert_eq!(spinner(0), '⠋');
        assert_eq!(spinner(10), '⠋');
        assert_eq!(spinner(1), '⠙');
    }

    #[test]
    fn wrap_splits_by_display_width() {
        assert_eq!(wrap("abcde", 2), vec!["ab", "cd", "e"]);
        assert_eq!(wrap("한글", 3), vec!["한", "글"]);
        assert_eq!(wrap("", 4), vec![""]);
    }

    #[test]
    fn truncate_counts_wide_chars_as_two_columns() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("한글입니다", 5), "한글…");
        assert_eq!(truncate("abc", 0), "");
    }
}
