//! engine 찾기와 시작, 붙기. 동시 시작은 engine 잠금이 하나로 줄인다.
//! 설계: docs/design/engine-lifecycle.md

use std::path::{Path, PathBuf};
use std::time::Duration;

use saturn_tui::client::EngineClient;

const ENGINE_BINARY: &str = "saturn-engine";

/// TODO(#33): 자식 Saturn을 부모와 잇는 방식과 판별 신호가 정해지면 거절 대신 연결한다
///
/// # Errors
/// 에이전트 작업 안에서 실행됐으면 오류.
pub(crate) fn ensure_not_nested() -> anyhow::Result<()> {
    todo!("#93")
}

/// # Errors
/// engine 실행 파일이 없거나, engine이 시작에 실패했거나 제때 소켓을 열지 않으면 오류.
pub(crate) async fn connect_or_start() -> anyhow::Result<EngineClient> {
    todo!("#93")
}

/// `saturn`과 같은 폴더를 먼저 보고 없으면 `PATH`에서 찾는다.
fn engine_binary() -> anyhow::Result<PathBuf> {
    todo!("#93")
}

/// `saturn`이 끝나도 남도록 분리된 프로세스로 띄운다. TODO(#93): judge 키 입력 때 터미널을 넘기는 방식
async fn spawn_engine(binary: &Path) -> anyhow::Result<()> {
    todo!("#93")
}

/// # Errors
/// `timeout` 안에 붙지 못하거나 engine이 먼저 끝나면 오류.
async fn wait_until_ready(socket: &Path, timeout: Duration) -> anyhow::Result<EngineClient> {
    todo!("#93")
}
