//! 입력창. `›` 접두 초안과 붙여넣은 내용 요소.
//!
//! 설계: docs/design/tui.md(영역 입력창, 키 입력창, 상태 표시 `[붙여넣은 내용 1,204자]`).
//! 1,000자를 넘는 붙여넣기는 초안에 요소 하나로 접어 `[붙여넣은 내용 1,204자]`로 보이고, 제출할 때 원문으로 펼친다.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::i18n::{self, Lang};
use crate::view::{MUTED, text_width, truncate};

/// 이 글자 수를 넘는 붙여넣기는 요소로 접는다.
pub const PASTE_COLLAPSE_CHARS: usize = 1_000;

/// 초안 조각. 커서는 조각 단위로 움직인다(붙여넣은 요소는 한 칸).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 입력한 글자 하나.
    Char(char),
    /// 1,000자를 넘는 붙여넣은 내용. 화면에는 `[붙여넣은 내용 N자]` 한 덩어리.
    Pasted(String),
}

/// `Ctrl+R` 입력 기록 검색 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistorySearch {
    /// 검색어.
    pub query: String,
    /// 같은 검색어로 건너뛴 수. `Ctrl+R`을 다시 누르면 하나 늘린다.
    pub skip: usize,
}

/// 입력창 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composer {
    segments: Vec<Segment>,
    cursor: usize,
    kill_buffer: String,
    search: Option<HistorySearch>,
    from_history: bool,
}

impl Composer {
    /// 빈 입력창.
    pub fn new() -> Self {
        Self::default()
    }

    /// 커서 자리에 글자 하나.
    pub fn insert(&mut self, c: char) {
        self.insert_segment(Segment::Char(c));
    }

    /// 붙여넣기. 1,000자를 넘으면 `Segment::Pasted` 하나, 아니면 글로 넣는다.
    pub fn paste(&mut self, text: String) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.chars().count() > PASTE_COLLAPSE_CHARS {
            self.insert_segment(Segment::Pasted(text));
        } else {
            text.chars()
                .for_each(|c| self.insert_segment(Segment::Char(c)));
        }
    }

    /// 줄바꿈.
    pub fn newline(&mut self) {
        self.insert('\n');
    }

    /// 앞 글자 지우기. 커서 바로 앞이 붙여넣은 요소면 요소 전체.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        self.segments.remove(self.cursor);
        self.from_history = false;
    }

    /// 커서 왼쪽.
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// 커서 오른쪽.
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.segments.len());
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 조각 수
    // basis: estimate
    /// `Ctrl+K` 커서부터 줄 끝까지 잘라 보관한다(이전 보관 글은 덮는다).
    pub fn kill_to_end(&mut self) {
        let end = self.segments[self.cursor..]
            .iter()
            .position(|s| *s == Segment::Char('\n'))
            .map_or(self.segments.len(), |offset| self.cursor + offset);
        let end = if end == self.cursor && end < self.segments.len() {
            end + 1
        } else {
            end
        };
        let killed: Vec<Segment> = self.segments.drain(self.cursor..end).collect();
        self.kill_buffer = segments_text(&killed);
        self.from_history = false;
    }

    /// `Ctrl+Y` 보관한 글을 커서 자리에 넣는다.
    pub fn yank(&mut self) {
        let text = self.kill_buffer.clone();
        text.chars()
            .for_each(|c| self.insert_segment(Segment::Char(c)));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 초안을 모두 지운다(`Ctrl+C` 초안 삭제). 지웠으면 참.
    pub fn clear(&mut self) -> bool {
        if self.is_empty() && self.search.is_none() {
            return false;
        }
        self.segments.clear();
        self.cursor = 0;
        self.search = None;
        self.from_history = false;
        true
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = text.len()
    // basis: estimate
    /// 초안을 바꾼다(기록 이동, `Alt+↑`로 되돌린 원문, 외부 에디터 결과). `from_history`면 `↑`/`↓` 이동을 계속 허용한다.
    pub fn set_text(&mut self, text: &str, from_history: bool) {
        self.segments = text.chars().map(Segment::Char).collect();
        self.cursor = self.segments.len();
        self.search = None;
        self.from_history = from_history;
    }

    /// 제출할 원문. 붙여넣은 요소는 원문으로 펼친다.
    pub fn text(&self) -> String {
        segments_text(&self.segments)
    }

    /// 제출하고 비운다. 원문을 돌려준다. 공백뿐이면 `None`이고 비우지 않는다.
    pub fn take(&mut self) -> Option<String> {
        let text = self.text();
        if text.trim().is_empty() {
            return None;
        }
        self.segments.clear();
        self.cursor = 0;
        self.search = None;
        self.from_history = false;
        Some(text)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 비었다(요소 포함).
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// 커서가 줄 맨 앞이다(`!` 셸 명령).
    pub fn at_line_start(&self) -> bool {
        self.cursor == 0 || self.segments[self.cursor - 1] == Segment::Char('\n')
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 커서가 단어 맨 앞이다(`$` 스킬 목록). 줄 앞이거나 앞 글자가 공백.
    pub fn at_word_start(&self) -> bool {
        match self.cursor.checked_sub(1).map(|i| &self.segments[i]) {
            None => true,
            Some(Segment::Char(c)) => c.is_whitespace(),
            Some(Segment::Pasted(_)) => false,
        }
    }

    /// `↑`/`↓` 기록 이동을 받을지. 비었거나 불러온 기록을 고치지 않았을 때.
    pub fn history_browsable(&self) -> bool {
        self.is_empty() || self.from_history
    }

    /// 셸 명령 줄이면 `!` 뒤 명령. 첫 줄이 `!`로 시작할 때만.
    pub fn shell_command(&self) -> Option<String> {
        let text = self.text();
        let first = text.lines().next()?;
        let command = first.strip_prefix('!')?.trim();
        if command.is_empty() {
            return None;
        }
        Some(text.strip_prefix('!')?.trim().to_string())
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 커서 앞 조각 수
    // basis: estimate
    /// 커서가 있는 팝업 입력 토큰(`/ju`, `@src/ma`, `$rev`). 토큰 첫 글자가 `/`, `@`, `$`가 아니면 `None`.
    pub fn popup_token(&self) -> Option<String> {
        let start = self.token_start();
        let token: String = self.segments[start..self.cursor]
            .iter()
            .filter_map(|s| match s {
                Segment::Char(c) => Some(*c),
                Segment::Pasted(_) => None,
            })
            .collect();
        match token.chars().next() {
            Some('/' | '@' | '$') => Some(token),
            _ => None,
        }
    }

    /// 팝업에서 고른 값으로 커서 토큰을 바꾼다(`/` 명령은 전체 경로와 공백 하나).
    pub fn replace_token(&mut self, value: &str) {
        let start = self.token_start();
        self.segments.drain(start..self.cursor);
        self.cursor = start;
        value
            .chars()
            .for_each(|c| self.insert_segment(Segment::Char(c)));
    }

    /// `Ctrl+R` 검색을 시작하거나 다음 결과로 넘긴다.
    pub fn start_or_next_search(&mut self) {
        match &mut self.search {
            Some(search) => search.skip += 1,
            None => self.search = Some(HistorySearch::default()),
        }
    }

    /// 검색 중이면 끝내고 참(`Ctrl+C` 검색 취소).
    pub fn cancel_search(&mut self) -> bool {
        self.search.take().is_some()
    }

    /// 검색 상태.
    pub fn search(&self) -> Option<&HistorySearch> {
        self.search.as_ref()
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 조각 수
    // basis: estimate
    /// 그릴 줄 수(초안 줄 수, 최소 1).
    pub fn height(&self) -> u16 {
        let newlines = self
            .segments
            .iter()
            .filter(|s| **s == Segment::Char('\n'))
            .count();
        u16::try_from(newlines + 1).unwrap_or(u16::MAX)
    }

    /// `↑` 커서를 윗줄 같은 칸으로. 첫 줄이면 그대로.
    pub fn cursor_up(&mut self) {
        let (row, col) = self.cursor_row_col();
        if row > 0 {
            self.cursor = self.index_at(row - 1, col);
        }
    }

    /// `↓` 커서를 아랫줄 같은 칸으로. 끝 줄이면 그대로.
    pub fn cursor_down(&mut self) {
        let (row, col) = self.cursor_row_col();
        if row + 1 < usize::from(self.height()) {
            self.cursor = self.index_at(row + 1, col);
        }
    }

    /// 검색어에 글자 하나를 더한다. 건너뛴 수는 처음부터.
    pub fn push_search_char(&mut self, c: char) {
        if let Some(search) = &mut self.search {
            search.query.push(c);
            search.skip = 0;
        }
    }

    /// 검색어 끝 글자를 지운다.
    pub fn pop_search_char(&mut self) {
        if let Some(search) = &mut self.search {
            search.query.pop();
            search.skip = 0;
        }
    }

    /// 검색을 끝내고 찾은 기록을 초안으로 둔다. 못 찾았으면 초안을 그대로 둔다.
    pub fn accept_search(&mut self, found: Option<&str>) {
        self.search = None;
        if let Some(found) = found {
            self.set_text(found, true);
        }
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 조각 수
    // basis: estimate
    /// 화면 줄. 붙여넣은 요소는 `[붙여넣은 내용 1,204자]`. 커서가 있는 줄과 칸(표시 폭)도 돌려준다.
    pub fn display(&self, lang: Lang) -> (Vec<String>, (usize, usize)) {
        let mut lines = vec![String::new()];
        let mut cursor = (0, 0);
        for (index, segment) in self.segments.iter().enumerate() {
            if index == self.cursor {
                cursor = current_position(&lines);
            }
            match segment {
                Segment::Char('\n') => lines.push(String::new()),
                Segment::Char(c) => push_last(&mut lines, &c.to_string()),
                Segment::Pasted(text) => push_last(&mut lines, &pasted_label(lang, text)),
            }
        }
        if self.cursor >= self.segments.len() {
            cursor = current_position(&lines);
        }
        (lines, cursor)
    }

    /// 조각 하나를 커서 자리에 넣는다. 고친 초안은 기록 그대로가 아니다.
    fn insert_segment(&mut self, segment: Segment) {
        self.segments.insert(self.cursor, segment);
        self.cursor += 1;
        self.from_history = false;
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 커서 앞 조각 수
    // basis: estimate
    /// 커서가 있는 낱말의 시작. 공백, 줄바꿈, 붙여넣은 요소 바로 뒤에서 멈춘다.
    fn token_start(&self) -> usize {
        self.segments[..self.cursor]
            .iter()
            .rposition(|s| match s {
                Segment::Char(c) => c.is_whitespace(),
                Segment::Pasted(_) => true,
            })
            .map_or(0, |i| i + 1)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 커서 앞 조각 수
    // basis: estimate
    /// 커서의 줄과 칸(조각 단위).
    fn cursor_row_col(&self) -> (usize, usize) {
        let before = &self.segments[..self.cursor];
        let row = before.iter().filter(|s| **s == Segment::Char('\n')).count();
        let line_start = before
            .iter()
            .rposition(|s| *s == Segment::Char('\n'))
            .map_or(0, |i| i + 1);
        (row, self.cursor - line_start)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 조각 수
    // basis: estimate
    /// 줄 `row`의 칸 `col` 조각 위치. 줄이 짧으면 줄 끝.
    fn index_at(&self, row: usize, col: usize) -> usize {
        let mut start = 0;
        for _ in 0..row {
            match self.segments[start..]
                .iter()
                .position(|s| *s == Segment::Char('\n'))
            {
                Some(offset) => start += offset + 1,
                None => return self.segments.len(),
            }
        }
        let end = self.segments[start..]
            .iter()
            .position(|s| *s == Segment::Char('\n'))
            .map_or(self.segments.len(), |offset| start + offset);
        (start + col).min(end)
    }
}

/// 입력창 그리기.
#[derive(Debug)]
pub struct ComposerView<'a> {
    /// 그릴 입력창.
    pub composer: &'a Composer,
    /// 화면 언어.
    pub lang: Lang,
    /// 검색 중이면 찾은 기록.
    pub search_result: Option<&'a str>,
}

impl ComposerView<'_> {
    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 조각 수와 붙여넣은 글자 수
    // basis: estimate
    /// `› ` 접두와 초안을 그리고 커서를 둔다. 붙여넣은 요소는 `[붙여넣은 내용 1,204자]`(글자 수는 `format_count`).
    /// 검색 중이면 검색어와 찾은 기록을 보인다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        if let Some(search) = self.composer.search() {
            let found = self.search_result.unwrap_or_default();
            let head = format!("{}: {}", self.lang.tr(i18n::TASKS_SEARCH), search.query);
            let line = Line::from(vec![
                Span::raw(format!("› {head}")),
                Span::styled(format!("  {found}"), MUTED),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            let x = area.x + (2 + text_width(&head)) as u16;
            frame.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), area.y));
            return;
        }
        let (lines, (row, col)) = self.composer.display(self.lang);
        let width = usize::from(area.width).saturating_sub(2);
        let first_row = (row + 1).saturating_sub(usize::from(area.height));
        let rows: Vec<Line> = lines
            .iter()
            .enumerate()
            .skip(first_row)
            .map(|(i, line)| {
                let head = if i == 0 { "› " } else { "  " };
                Line::from(format!("{head}{}", truncate(line, width)))
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), area);
        let x = area.x + 2 + col.min(width) as u16;
        let y = area.y + (row - first_row) as u16;
        frame.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), y));
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 원문 길이
// basis: estimate
/// 조각을 원문으로 잇는다. 붙여넣은 요소는 원문으로 펼친다.
fn segments_text(segments: &[Segment]) -> String {
    let mut text = String::new();
    for segment in segments {
        match segment {
            Segment::Char(c) => text.push(*c),
            Segment::Pasted(pasted) => text.push_str(pasted),
        }
    }
    text
}

/// 붙여넣은 요소 표시 `[붙여넣은 내용 1,204자]`.
pub fn pasted_label(lang: Lang, text: &str) -> String {
    let count = i18n::format_count(text.chars().count() as u64);
    match lang {
        Lang::Ko => format!("[{} {count}{}]", i18n::PASTED, i18n::CHARS_SUFFIX),
        Lang::En => format!(
            "[{} {count} {}]",
            lang.tr(i18n::PASTED),
            lang.tr(i18n::CHARS_SUFFIX)
        ),
    }
}

/// 마지막 줄에 글을 붙인다.
fn push_last(lines: &mut [String], text: &str) {
    if let Some(last) = lines.last_mut() {
        last.push_str(text);
    }
}

/// 마지막 줄 끝의 줄과 칸(표시 폭).
fn current_position(lines: &[String]) -> (usize, usize) {
    let row = lines.len().saturating_sub(1);
    (row, lines.last().map_or(0, |line| text_width(line)))
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn typed(text: &str) -> Composer {
        let mut composer = Composer::new();
        text.chars().for_each(|c| composer.insert(c));
        composer
    }

    #[test]
    fn paste_over_limit_collapses_to_one_segment() {
        let mut composer = Composer::new();
        let long = "x".repeat(1_204);

        composer.paste(long.clone());

        assert_eq!(
            composer.display(Lang::Ko).0,
            vec!["[붙여넣은 내용 1,204자]"]
        );
        assert_eq!(composer.text(), long);
        composer.backspace();
        assert!(composer.is_empty());
    }

    #[test]
    fn paste_under_limit_inserts_text() {
        let mut composer = Composer::new();

        composer.paste("abc".to_string());

        assert_eq!(composer.text(), "abc");
        assert_eq!(composer.display(Lang::Ko).0, vec!["abc"]);
    }

    #[test]
    fn kill_to_end_then_yank_restores_text() {
        let mut composer = typed("hello world");
        (0..5).for_each(|_| composer.left());

        composer.kill_to_end();
        let after_kill = composer.text();
        composer.yank();

        assert_eq!(after_kill, "hello ");
        assert_eq!(composer.text(), "hello world");
    }

    #[test]
    fn take_blank_returns_none_and_keeps_draft() {
        let mut composer = typed("  ");

        assert_eq!(composer.take(), None);
        assert_eq!(composer.text(), "  ");
    }

    #[test]
    fn take_returns_text_and_clears() {
        let mut composer = typed("hi");

        assert_eq!(composer.take().as_deref(), Some("hi"));
        assert!(composer.is_empty());
    }

    #[test]
    fn at_line_and_word_start_follow_cursor() {
        let mut composer = Composer::new();
        assert!(composer.at_line_start());

        composer.insert('a');
        let after_char = (composer.at_line_start(), composer.at_word_start());
        composer.insert(' ');

        assert_eq!(after_char, (false, false));
        assert!(composer.at_word_start());
        composer.newline();
        assert!(composer.at_line_start());
    }

    #[test]
    fn history_browsable_until_edit() {
        let mut composer = Composer::new();
        composer.set_text("old", true);
        let loaded = composer.history_browsable();

        composer.insert('!');

        assert!(loaded);
        assert!(!composer.history_browsable());
    }

    #[test]
    fn shell_command_requires_leading_bang() {
        assert_eq!(typed("!ls -la").shell_command().as_deref(), Some("ls -la"));
        assert_eq!(typed("ls !x").shell_command(), None);
        assert_eq!(typed("!").shell_command(), None);
    }

    #[test]
    fn popup_token_and_replace_token() {
        let mut composer = typed("look at @src/ma");

        let token = composer.popup_token();
        composer.replace_token("@src/main.rs ");

        assert_eq!(token.as_deref(), Some("@src/ma"));
        assert_eq!(composer.text(), "look at @src/main.rs ");
        assert_eq!(typed("plain").popup_token(), None);
    }

    #[test]
    fn search_cycles_and_cancels() {
        let mut composer = Composer::new();

        composer.start_or_next_search();
        composer.push_search_char('b');
        composer.start_or_next_search();

        assert_eq!(
            composer.search(),
            Some(&HistorySearch {
                query: "b".to_string(),
                skip: 1
            })
        );
        assert!(composer.cancel_search());
        assert!(!composer.cancel_search());
    }

    #[test]
    fn cursor_up_down_keeps_column() {
        let mut composer = typed("abcd\nxy");

        composer.cursor_up();
        composer.insert('!');

        assert_eq!(composer.text(), "ab!cd\nxy");
    }

    #[test]
    fn height_counts_lines() {
        assert_eq!(typed("a\nb\nc").height(), 3);
        assert_eq!(Composer::new().height(), 1);
    }

    #[test]
    fn render_draws_prompt_prefix() {
        let composer = typed("안녕");
        let mut terminal = Terminal::new(TestBackend::new(20, 1)).unwrap();
        let view = ComposerView {
            composer: &composer,
            lang: Lang::Ko,
            search_result: None,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "›");
        assert_eq!(buffer[(2, 0)].symbol(), "안");
    }
}
