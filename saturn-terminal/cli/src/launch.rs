//! engine 찾기와 시작, 붙기.
//!
//! 설계: docs/design/engine-lifecycle.md(engine 시작 순서, 프로세스 배치, 중첩 saturn 거절).
//! engine이 없을 때만 `saturn-engine`을 띄운다. 사용자당 하나는 engine의 잠금이 지키므로
//! 두 `saturn`이 동시에 띄워도 하나만 남고 둘 다 같은 소켓에 붙는다.

use std::path::{Path, PathBuf};
use std::time::Duration;

use saturn_tui::client::EngineClient;

/// engine 실행 파일 이름.
const ENGINE_BINARY: &str = "saturn-engine";

/// 에이전트가 작업 중에 실행한 `saturn`이면 거절한다.
///
/// TODO(#33): 자식 Saturn을 부모와 잇는 방식이 정해지면 거절 대신 연결한다. 판별 신호도 그때 정한다
///
/// # Errors
/// 에이전트 작업 안에서 실행됐으면 오류.
pub(crate) fn ensure_not_nested() -> anyhow::Result<()> {
    todo!("#93")
}

/// 사용자 소켓에 붙는다. engine이 없으면 띄운 뒤 준비될 때까지 기다려 붙는다.
///
/// # Errors
/// engine 실행 파일이 없거나, 띄운 engine이 시작에 실패했거나(judge 확인 실패 등) 제때 소켓을 열지 않으면 오류.
pub(crate) async fn connect_or_start() -> anyhow::Result<EngineClient> {
    todo!("#93")
}

/// `saturn-engine` 실행 파일 경로. `saturn`과 같은 폴더를 먼저 보고 없으면 `PATH`에서 찾는다.
///
/// # Errors
/// 어디에도 없으면 오류.
fn engine_binary() -> anyhow::Result<PathBuf> {
    todo!("#93")
}

/// engine을 `saturn`과 분리된 프로세스로 띄운다. `saturn`이 끝나도 engine은 남는다.
///
/// judge 확인에 실패하면 engine이 숨김 입력으로 키를 묻는다. 입력할 수 없는 환경이면 engine이 안내하고 끝낸다.
/// TODO(#93): 키 입력을 engine이 받을 때 터미널을 넘기는 방식
///
/// # Errors
/// 프로세스를 띄우지 못하면 오류.
async fn spawn_engine(binary: &Path) -> anyhow::Result<()> {
    todo!("#93")
}

/// 소켓이 열릴 때까지 다시 붙어 본다.
///
/// # Errors
/// `timeout` 안에 붙지 못하거나 engine이 먼저 끝나면 오류.
async fn wait_until_ready(socket: &Path, timeout: Duration) -> anyhow::Result<EngineClient> {
    todo!("#93")
}
