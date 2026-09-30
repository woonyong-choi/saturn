//! 입력창. `›` 접두 초안과 붙여넣은 내용 요소.
//!
//! 설계: docs/design/tui.md(영역 입력창, 키 입력창, 상태 표시 `[붙여넣은 내용 1,204자]`).
//! 1,000자를 넘는 붙여넣기는 초안에 요소 하나로 접어 `[붙여넣은 내용 1,204자]`로 보이고, 제출할 때 원문으로 펼친다.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// 이 글자 수를 넘는 붙여넣기는 요소로 접는다.
pub const PASTE_COLLAPSE_CHARS: usize = 1_000;

/// 초안 조각.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 입력한 글.
    Text(String),
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
        todo!("#92")
    }

    /// 붙여넣기. 1,000자를 넘으면 `Segment::Pasted` 하나, 아니면 글로 넣는다.
    pub fn paste(&mut self, text: String) {
        todo!("#92")
    }

    /// 줄바꿈.
    pub fn newline(&mut self) {
        todo!("#92")
    }

    /// 앞 글자 지우기. 커서 바로 앞이 붙여넣은 요소면 요소 전체.
    pub fn backspace(&mut self) {
        todo!("#92")
    }

    /// 커서 왼쪽.
    pub fn left(&mut self) {
        todo!("#92")
    }

    /// 커서 오른쪽.
    pub fn right(&mut self) {
        todo!("#92")
    }

    /// `Ctrl+K` 커서부터 줄 끝까지 잘라 보관한다(이전 보관 글은 덮는다).
    pub fn kill_to_end(&mut self) {
        todo!("#92")
    }

    /// `Ctrl+Y` 보관한 글을 커서 자리에 넣는다.
    pub fn yank(&mut self) {
        todo!("#92")
    }

    /// 초안을 모두 지운다(`Ctrl+C` 초안 삭제). 지웠으면 참.
    pub fn clear(&mut self) -> bool {
        todo!("#92")
    }

    /// 초안을 바꾼다(기록 이동, `Alt+↑`로 되돌린 원문, 외부 에디터 결과). `from_history`면 `↑`/`↓` 이동을 계속 허용한다.
    pub fn set_text(&mut self, text: &str, from_history: bool) {
        todo!("#92")
    }

    /// 제출할 원문. 붙여넣은 요소는 원문으로 펼친다.
    pub fn text(&self) -> String {
        todo!("#92")
    }

    /// 제출하고 비운다. 원문을 돌려준다. 공백뿐이면 `None`이고 비우지 않는다.
    pub fn take(&mut self) -> Option<String> {
        todo!("#92")
    }

    /// 비었다(요소 포함).
    pub fn is_empty(&self) -> bool {
        todo!("#92")
    }

    /// 커서가 줄 맨 앞이다(`!` 셸 명령).
    pub fn at_line_start(&self) -> bool {
        todo!("#92")
    }

    /// 커서가 단어 맨 앞이다(`$` 스킬 목록). 줄 앞이거나 앞 글자가 공백.
    pub fn at_word_start(&self) -> bool {
        todo!("#92")
    }

    /// `↑`/`↓` 기록 이동을 받을지. 비었거나 불러온 기록을 고치지 않았을 때.
    pub fn history_browsable(&self) -> bool {
        todo!("#92")
    }

    /// 셸 명령 줄이면 `!` 뒤 명령. 첫 줄이 `!`로 시작할 때만.
    pub fn shell_command(&self) -> Option<String> {
        todo!("#92")
    }

    /// 커서가 있는 팝업 입력 토큰(`/ju`, `@src/ma`, `$rev`). 토큰 첫 글자가 `/`, `@`, `$`가 아니면 `None`.
    pub fn popup_token(&self) -> Option<String> {
        todo!("#92")
    }

    /// 팝업에서 고른 값으로 커서 토큰을 바꾼다(`/` 명령은 전체 경로와 공백 하나).
    pub fn replace_token(&mut self, value: &str) {
        todo!("#92")
    }

    /// `Ctrl+R` 검색을 시작하거나 다음 결과로 넘긴다.
    pub fn start_or_next_search(&mut self) {
        todo!("#92")
    }

    /// 검색 중이면 끝내고 참(`Ctrl+C` 검색 취소).
    pub fn cancel_search(&mut self) -> bool {
        todo!("#92")
    }

    /// 검색 상태.
    pub fn search(&self) -> Option<&HistorySearch> {
        self.search.as_ref()
    }

    /// 그릴 줄 수(초안 줄 수, 최소 1).
    pub fn height(&self) -> u16 {
        todo!("#92")
    }
}

/// 입력창 그리기.
#[derive(Debug)]
pub struct ComposerView<'a> {
    /// 그릴 입력창.
    pub composer: &'a Composer,
    /// 화면 언어.
    pub lang: Lang,
}

impl ComposerView<'_> {
    /// `› ` 접두와 초안을 그리고 커서를 둔다. 붙여넣은 요소는 `[붙여넣은 내용 1,204자]`(글자 수는 `format_count`).
    /// 검색 중이면 검색어와 찾은 기록을 보인다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
