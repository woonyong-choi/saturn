//! `saturn usage`: 사용량 조회. `/usage`와 같다.
//!
//! 설계: docs/design/tui.md(사용량 화면).

use saturn_tui::client::EngineClient;

use crate::args::UsageArgs;

/// 범위의 사용량을 요청해 stdout에 쓴다. provider가 보고하지 않은 값은 0이 아닌 빈 값으로 보인다.
///
/// # Errors
/// 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &UsageArgs) -> anyhow::Result<()> {
    todo!("#93")
}
