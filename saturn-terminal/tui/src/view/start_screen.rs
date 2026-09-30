//! 시작 화면. 로고, Saturn 버전, provider 버전, judge와 judge 버전, 폴더.
//!
//! 설계: docs/design/tui.md(영역 시작 화면). 실행 때 보이고, 첫 결과가 오면 대화 기록 맨 위 머리 셀로 바뀐다.
//! TODO(#46): provider 버전과 judge 정보를 받는 메서드가 protocol에 없다

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::Provider;

use crate::i18n::Lang;

/// 로고 줄.
pub const LOGO: &[&str] = &["saturn"];

/// 시작 화면 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartInfo {
    /// Saturn 버전(`CARGO_PKG_VERSION`).
    pub saturn_version: String,
    /// provider와 버전. 확인하지 못한 provider는 버전이 `None`.
    pub providers: Vec<(Provider, Option<String>)>,
    /// judge 이름. 없으면 `None`.
    pub judge: Option<String>,
    /// judge 버전.
    pub judge_version: Option<String>,
    /// 작업 폴더.
    pub folder: PathBuf,
}

impl StartInfo {
    /// 머리 셀과 plain 출력의 글 줄. 로고 다음 `Saturn 0.1.0`, provider별 한 줄, judge 한 줄, 폴더 한 줄.
    pub fn lines(&self, lang: Lang) -> Vec<String> {
        todo!("#92")
    }
}

/// 시작 화면 그리기.
#[derive(Debug)]
pub struct StartScreenView<'a> {
    /// 정보.
    pub info: &'a StartInfo,
    /// 화면 언어.
    pub lang: Lang,
}

impl StartScreenView<'_> {
    /// 대화 기록 칸 가운데에 로고와 `StartInfo::lines`를 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
