//! 폴더 설정 신뢰 창. 처음 보거나 내용이 바뀐 폴더 설정 파일을 적용할지 묻는다.
//!
//! 설계: docs/design/tui.md(영역 폴더 설정 신뢰 창, 키), docs/design/settings.md(신뢰 규칙).
//! 실행 중에 만나면 다음 입력 접수 전에 띄운다.
//! TODO(#46): 신뢰 요청 알림과 답 요청이 protocol에 없다

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::i18n::Lang;

/// 선택지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustChoice {
    /// `1`, `y` 적용하고 계속.
    Apply,
    /// 두 번째 선택지. 설계 표에 키와 문구가 없다(방향키와 `Enter`로만).
    Second,
    /// `3`, `q`, `Esc`, `Ctrl+C` 종료.
    Quit,
}

/// 폴더 설정 신뢰 창 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderTrust {
    /// 폴더 설정 파일 경로.
    pub path: PathBuf,
    /// 파일 지문.
    pub fingerprint: String,
    /// 적용되는 항목.
    pub applied: Vec<String>,
    /// 무시되는 항목.
    pub ignored: Vec<String>,
    /// 이전 신뢰 뒤 바뀐 줄. 처음 보면 빈 목록.
    pub changed: Vec<String>,
    /// 강조한 선택지.
    pub selected: TrustChoice,
}

impl FolderTrust {
    /// `↑` 이전 선택지.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 다음 선택지.
    pub fn down(&mut self) {
        todo!("#92")
    }
}

/// 폴더 설정 신뢰 창 그리기.
#[derive(Debug)]
pub struct FolderTrustView<'a> {
    /// 창 상태.
    pub trust: &'a FolderTrust,
    /// 화면 언어.
    pub lang: Lang,
}

impl FolderTrustView<'_> {
    /// 경로, 지문, 적용 항목, 무시 항목, 바뀐 줄, 선택지 세 개(강조 표시)를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
