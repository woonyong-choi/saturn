//! 팝업: `/` 명령 목록과 값 목록, `@` 파일 목록, `$` 스킬 목록.
//!
//! 설계: docs/design/tui.md(영역 팝업, 키 팝업).
//! - 글자 입력마다 입력 토큰으로 목록을 거른다. 명령 목록은 최대 8행, 오른쪽에 출처(`Saturn` 또는 provider)를 보인다.
//! - `Enter`: 명령 목록이면 고른 명령의 전체 경로를 입력창에 기입, 값 목록이면 값 선택. `Tab`: 전체 경로까지 완성.
//! - `Esc`: 해제하고 입력 토큰이 바뀔 때까지 다시 띄우지 않는다.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// 팝업 최대 행 수.
pub const MAX_ROWS: usize = 8;

/// 팝업 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PopupKind {
    /// `/` 명령 목록(`commands::SATURN_COMMANDS`와 provider 명령).
    Command,
    /// 명령을 고른 뒤 그 명령의 값 목록(`/usage` → `chat`, `today`, `week`, `all`).
    Value,
    /// `@` 작업 폴더 파일 목록.
    File,
    /// `$` 메인 에이전트 provider의 스킬 목록. 다른 provider는 `$공급자 이름`. TODO(#46): 스킬 목록을 받는 메서드가 없다
    Skill,
}

/// 목록 한 행.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupItem {
    /// 기입할 값(명령 전체 경로, 파일 경로, 스킬 이름).
    pub value: String,
    /// 설명.
    pub description: String,
    /// 오른쪽 출처 표시(`Saturn`, `codex`, `claude`). 파일 목록은 비운다.
    pub source: String,
}

/// 떠 있는 팝업.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// 종류.
    pub kind: PopupKind,
    /// 거르는 입력 토큰(`/ju`의 `ju`).
    pub token: String,
    /// 거른 목록.
    pub items: Vec<PopupItem>,
    /// 강조 행.
    pub selected: usize,
}

impl Popup {
    /// 종류와 전체 후보로 연다. 토큰은 빈 문자열.
    pub fn open(kind: PopupKind, candidates: Vec<PopupItem>) -> Self {
        todo!("#92")
    }

    /// 토큰이 바뀌면 다시 거른다. 앞부분 일치 우선, 그다음 포함. 강조는 첫 행으로.
    pub fn filter(&mut self, token: &str, candidates: &[PopupItem]) {
        todo!("#92")
    }

    /// `↑` 위로. 첫 행에서 멈춘다.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 아래로. 끝 행에서 멈춘다.
    pub fn down(&mut self) {
        todo!("#92")
    }

    /// 강조한 행.
    pub fn selected(&self) -> Option<&PopupItem> {
        todo!("#92")
    }

    /// 그릴 행 수(최대 8).
    pub fn height(&self) -> u16 {
        todo!("#92")
    }
}

/// `Esc`로 닫은 뒤의 억제. 같은 토큰이면 다시 띄우지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PopupSuppress {
    token: Option<String>,
}

impl PopupSuppress {
    /// 이 토큰에서 닫았다고 기록한다.
    pub fn suppress(&mut self, token: &str) {
        todo!("#92")
    }

    /// 지금 토큰에서 띄워도 되는지. 토큰이 바뀌면 억제를 풀고 참.
    pub fn allows(&mut self, token: &str) -> bool {
        todo!("#92")
    }
}

/// 작업 폴더 파일 후보. `.git` 폴더와 무시 파일은 뺀다. 경로는 작업 폴더 기준 상대 경로.
pub fn file_candidates(workdir: &std::path::Path) -> Vec<PopupItem> {
    todo!("#92")
}

/// 팝업 그리기.
#[derive(Debug)]
pub struct PopupView<'a> {
    /// 그릴 팝업.
    pub popup: &'a Popup,
    /// 화면 언어.
    pub lang: Lang,
}

impl PopupView<'_> {
    /// `› 값  설명 ... 출처` 행을 최대 8행 그린다. 강조 행은 `›`, 출처는 오른쪽 정렬.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
