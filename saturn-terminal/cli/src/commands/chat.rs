//! 하위 명령 없는 `saturn`: 대화 화면을 연다.
//! 설계: docs/design/tui.md

use saturn_tui::client::EngineClient;

use crate::args::ConfigOverride;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenMode {
    FullScreen,
    /// 파이프와 CI용. 같은 명령은 전체 화면과 같은 결과를 낸다.
    Plain,
}

/// # Errors
/// `-c` 값을 engine이 받지 않았거나 연결이 끊기면 오류.
pub(crate) async fn run(
    client: &mut EngineClient,
    config: &[ConfigOverride],
) -> anyhow::Result<()> {
    todo!("#93")
}

/// TODO(#57): plain을 켜는 조건과 우선순위, 설정 키
fn detect_mode() -> ScreenMode {
    todo!("#93")
}

async fn send_run_layer(
    client: &mut EngineClient,
    config: &[ConfigOverride],
) -> anyhow::Result<()> {
    todo!("#93")
}

async fn run_full_screen(client: &mut EngineClient) -> anyhow::Result<()> {
    todo!("#93")
}

async fn run_plain(client: &mut EngineClient) -> anyhow::Result<()> {
    todo!("#93")
}
