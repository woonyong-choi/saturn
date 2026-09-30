//! 하위 명령 없는 `saturn`: 채팅에 붙어 대화 화면을 연다.
//!
//! 설계: docs/design/tui.md(전체 화면과 plain 출력), docs/design/settings.md(`-c` 실행 층).

use saturn_tui::client::EngineClient;

use crate::args::ConfigOverride;

/// 화면 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenMode {
    /// 전체 화면 TUI.
    FullScreen,
    /// 파이프와 CI용 plain 출력. 같은 명령은 전체 화면과 같은 결과를 낸다.
    Plain,
}

/// 채팅에 붙어 화면 방식에 맞게 대화를 연다. 끝나면 채팅에서 떨어진다.
///
/// # Errors
/// `-c` 값을 engine이 받지 않았거나 연결이 끊기면 오류.
pub(crate) async fn run(
    client: &mut EngineClient,
    config: &[ConfigOverride],
) -> anyhow::Result<()> {
    todo!("#93")
}

/// 표준 입출력이 터미널이면 전체 화면, 아니면 plain.
///
/// TODO(#57): plain을 켜는 조건과 우선순위, 설정 키
fn detect_mode() -> ScreenMode {
    todo!("#93")
}

/// 실행 층 설정을 engine에 넘긴다.
///
/// `Request::Attach`의 `overrides`로 넘긴다.
///
/// # Errors
/// engine에 보내지 못하면 오류.
async fn send_run_layer(
    client: &mut EngineClient,
    config: &[ConfigOverride],
) -> anyhow::Result<()> {
    todo!("#93")
}

/// 전체 화면 TUI를 실행한다. TUI 진입은 `saturn-tui`가 맡는다.
///
/// # Errors
/// 터미널 설정이나 연결에 실패하면 오류.
async fn run_full_screen(client: &mut EngineClient) -> anyhow::Result<()> {
    todo!("#93")
}

/// plain 모드: 표준 입력 줄을 입력으로 보내고 알림을 줄 단위로 stdout에 쓴다.
///
/// # Errors
/// 입출력이나 연결에 실패하면 오류.
async fn run_plain(client: &mut EngineClient) -> anyhow::Result<()> {
    todo!("#93")
}
