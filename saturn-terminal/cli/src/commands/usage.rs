//! `saturn usage`: 사용량 조회. `/usage`와 같다.
//! 설계: docs/design/tui.md

use saturn_tui::client::EngineClient;

use crate::args::UsageArgs;

/// provider가 보고하지 않은 값은 0이 아닌 빈 값으로 쓴다.
///
/// # Errors
/// 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &UsageArgs) -> anyhow::Result<()> {
    todo!("#93")
}
