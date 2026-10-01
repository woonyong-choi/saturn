//! `saturn prune`: 기록 정리.
//! 설계: docs/design/records.md

use saturn_tui::client::EngineClient;

use crate::args::PruneArgs;

/// 삭제 제외 대상(열린 입력, 활성 session, 판단 기록 등)은 engine이 거른다.
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &PruneArgs) -> anyhow::Result<()> {
    todo!("#93")
}
