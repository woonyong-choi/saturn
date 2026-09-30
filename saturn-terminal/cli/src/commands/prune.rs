//! `saturn prune`: 기록 정리.
//!
//! 설계: docs/design/records.md(보존과 정리, 삭제 대상 제외).

use saturn_tui::client::EngineClient;

use crate::args::PruneArgs;

/// 정리를 요청한다. `--yes`가 없으면 지울 대상만 stdout에 쓰고 지우지 않는다.
/// 열린 입력, 열린 실행, 활성 session과 판단 기록은 engine이 대상에서 뺀다.
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &PruneArgs) -> anyhow::Result<()> {
    todo!("#93")
}
