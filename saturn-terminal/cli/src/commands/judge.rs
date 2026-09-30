//! `saturn judge`: judge 버전 관리.
//!
//! 설계: docs/design/judge-training.md(judge 버전 화면의 `u`와 같은 동작).

use saturn_tui::client::EngineClient;

use crate::args::JudgeVersionArgs;

/// 확인 한 줄을 받은 뒤 고른 judge 버전을 현재 버전으로 쓴다.
///
/// 전환은 `Request::UseJudgeVersion`으로 보낸다.
/// TODO(#93): 입력할 수 없는 환경(파이프, CI)에서 확인 한 줄을 받는 방식
///
/// # Errors
/// 버전이 없거나 사용자가 확인하지 않았거나 연결이 끊기면 오류.
pub(crate) async fn use_version(
    client: &mut EngineClient,
    args: &JudgeVersionArgs,
) -> anyhow::Result<()> {
    todo!("#93")
}
