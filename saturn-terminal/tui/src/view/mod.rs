//! 화면 그리기. 영역마다 파일 하나, 창마다 파일 하나.
//!
//! 설계: docs/design/tui.md(배치, 영역). 위에서 아래로 대화 기록, 작업별 출력 칸, 상태판, 팝업, 입력창, 바닥줄을 쌓는다.
//! 창(허가 요청, 작업 목록 등)은 이 배치 위에 덮어 그린다. 그리기 함수는 상태를 바꾸지 않는다(`&self`).
//! TODO(#58): 좁은 가로 폭에서 폭 구간별로 버튼과 칸을 줄일지, 줄 끝부터 말줄임할지, 버튼 대신 명령 안내를 보일지

pub mod composer;
pub mod folder_trust;
pub mod footer;
pub mod full_transcript;
pub mod judge_key_prompt;
pub mod judge_version;
pub mod live_area;
pub mod permission;
pub mod popup;
pub mod resume_prompt;
pub mod start_screen;
pub mod status_board;
pub mod task_list;
pub mod train_confirm;
pub mod transcript;
pub mod usage;

use ratatui::layout::Rect;

/// 실행 줄·판단 줄·학습 줄의 스피너 글자. 틱마다 한 칸 넘긴다.
pub const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// 영역별 칸. 높이 0인 영역은 그리지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Areas {
    /// 대화 기록. 남는 높이를 모두 쓴다.
    pub transcript: Rect,
    /// 작업별 출력 칸. `live_area::max_rows`까지.
    pub live: Rect,
    /// 상태판. 줄 수만큼.
    pub status: Rect,
    /// 팝업. 열렸을 때만, 최대 8행.
    pub popup: Rect,
    /// 입력창. 초안 줄 수만큼.
    pub composer: Rect,
    /// 바닥줄 한 줄.
    pub footer: Rect,
}

/// 영역별로 원하는 높이. `layout`이 화면 높이에 맞춰 줄인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Heights {
    /// 작업별 출력 칸 줄 수(상한 적용 전).
    pub live: u16,
    /// 상태판 줄 수. TODO(#51): 상태판 최대 높이를 화면 높이 비율로 둘지, 고정 줄 수로 둘지, 상한을 두지 않을지
    pub status: u16,
    /// 팝업 행 수(최대 8).
    pub popup: u16,
    /// 입력창 줄 수.
    pub composer: u16,
}

/// 화면을 영역으로 나눈다. 바닥줄 1줄, 입력창, 팝업, 상태판, 작업별 출력 칸을 아래부터 잡고 남는 높이를 대화 기록에 준다.
/// 모자라면 작업별 출력 칸 → 상태판 순으로 줄이고, 입력창과 바닥줄은 줄이지 않는다.
pub fn layout(area: Rect, heights: Heights) -> Areas {
    todo!("#92")
}

/// 가운데 창 칸. 폭·높이를 `area` 안으로 자른다.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    todo!("#92")
}

/// 틱 번호의 스피너 글자.
pub fn spinner(tick: u64) -> char {
    todo!("#92")
}

/// 표시 폭 `width` 칸까지 자르고 넘치면 끝을 `…`로. 한글 등 넓은 글자는 2칸(ratatui `text::Span::width`로 잰다).
pub fn truncate(text: &str, width: usize) -> String {
    todo!("#92")
}
