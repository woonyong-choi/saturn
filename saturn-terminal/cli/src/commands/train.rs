//! `saturn train`: judge 학습. `/train`과 같다.
//!
//! 설계: docs/design/judge-training.md(`/train` 실행, 승격 게이트).

use saturn_tui::client::EngineClient;

use crate::args::TrainArgs;

/// 학습을 요청하고 결과(승격 여부)를 stdout에 쓴다. 채점 안 된 판단이 200건 미만이면 engine이 거절한다.
///
/// TODO(#47): 거절과 승격 실패의 종료 코드
///
/// # Errors
/// engine이 거절했거나 연결이 끊기면 오류.
pub(crate) async fn run(client: &mut EngineClient, args: &TrainArgs) -> anyhow::Result<()> {
    todo!("#93")
}
