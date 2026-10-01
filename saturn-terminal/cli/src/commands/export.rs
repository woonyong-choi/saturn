//! `saturn export`: 판단 기록 JSONL 내보내기.
//! 설계: docs/design/records.md

use saturn_tui::client::EngineClient;

use crate::args::ExportArgs;

/// 경로는 절대 경로로 바꿔 보낸다. engine과 작업 폴더가 다를 수 있기 때문이다.
///
/// # Errors
/// 경로를 만들 수 없거나 engine이 쓰지 못하면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &ExportArgs) -> anyhow::Result<()> {
    todo!("#93")
}
