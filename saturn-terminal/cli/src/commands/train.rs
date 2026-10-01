//! `saturn train`: judge 학습. `/train`과 같다.
//! 설계: docs/design/judge-training.md

use saturn_tui::client::EngineClient;

use crate::args::TrainArgs;

/// TODO(#47): 거절과 승격 실패의 종료 코드
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &TrainArgs) -> anyhow::Result<()> {
    todo!("#93")
}
