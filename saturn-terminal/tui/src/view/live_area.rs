//! 작업별 출력 칸. 출력이 흐르는 작업마다 최근 줄을 보인다.
//!
//! 설계: docs/design/tui.md(영역 작업별 출력 칸).
//! - 높이 상한 8줄, 화면이 작으면 상한을 줄인다(축소 규칙은 설계에 없어 `max_rows`가 정한다).
//! - 완성된 줄(줄바꿈으로 끝난 줄) 단위로 갱신한다. 줄바꿈 전 조각은 모아 두고 그리지 않는다.
//! - 출력이 시작될 때 한 번 자리를 잡고 그 뒤에는 작업끼리 순서를 바꾸지 않는다.
//! - 작업이 끝나면 전체 내용을 대화 기록으로 옮기고 칸에서 지운다.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{TaskId, TaskLabel};

use crate::i18n::Lang;

/// 작업별 출력 칸 높이 상한.
pub const MAX_ROWS: u16 = 8;

/// 작업 하나의 출력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTask {
    /// 작업 id.
    pub task: TaskId,
    /// 이름표. 보일지는 `labels::visible`.
    pub label: TaskLabel,
    /// 완성된 줄 전체. 작업 종료 때 대화 기록으로 옮긴다.
    pub lines: Vec<String>,
    /// 줄바꿈 전 조각.
    pub partial: String,
}

/// 출력 칸의 작업들. 출력이 시작된 순서.
#[derive(Debug, Default)]
pub struct LiveArea {
    tasks: VecDeque<LiveTask>,
}

impl LiveArea {
    /// 빈 칸.
    pub fn new() -> Self {
        Self::default()
    }

    /// 모델 글 조각을 더한다. 처음 보는 작업이면 끝에 자리를 잡는다. 줄바꿈이 오면 조각을 완성된 줄로 옮긴다.
    /// 완성된 줄이 새로 생겼으면 참(다시 그리기).
    pub fn push(&mut self, task: TaskId, label: TaskLabel, text: &str) -> bool {
        todo!("#92")
    }

    /// 작업이 끝났다. 남은 조각까지 줄로 만들어 돌려주고 칸에서 지운다. 없던 작업이면 빈 목록.
    pub fn finish(&mut self, task: TaskId) -> Vec<String> {
        todo!("#92")
    }

    /// 그릴 줄 수. 작업마다 최근 줄을 나눠 `max_rows(screen_height)`를 넘지 않는다.
    pub fn height(&self, screen_height: u16) -> u16 {
        todo!("#92")
    }

    /// 칸에 있는 작업.
    pub fn tasks(&self) -> impl ExactSizeIterator<Item = &LiveTask> {
        self.tasks.iter()
    }
}

/// 화면 높이에 맞춘 상한. 기본 `MAX_ROWS`, 화면 높이의 1/4이 더 작으면 그 값(최소 1줄). 1/4은 가정이다.
pub fn max_rows(screen_height: u16) -> u16 {
    todo!("#92")
}

/// 작업별 출력 칸 그리기.
#[derive(Debug)]
pub struct LiveAreaView<'a> {
    /// 그릴 칸.
    pub live: &'a LiveArea,
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
}

impl LiveAreaView<'_> {
    /// 작업마다 `[A] 최근 줄`을 칸 높이 안에서 나눠 그린다. 줄 수가 모자라면 작업마다 가장 최근 줄부터 남긴다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
