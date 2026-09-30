//! 입력창 `!` 셸 명령. 줄 앞 `!`로 시작한 입력을 작업 폴더에서 바로 실행한다.
//!
//! 설계: docs/design/tui.md(키 `!`). 결과는 대화 기록에 보이고 메인 에이전트의 다음 입력에 첨부한다.
//! judge를 거치지 않고 engine에도 보내지 않는다.

use std::path::Path;

/// 셸 명령 실행 오류. 대화 기록에 한 줄로 보인다.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    /// 셸을 띄우지 못했다.
    #[error("failed to spawn shell")]
    Spawn(#[source] std::io::Error),
}

/// 셸 명령 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    /// `!` 뒤 명령 원문.
    pub command: String,
    /// 종료 코드. 신호로 끝났으면 `None`.
    pub status: Option<i32>,
    /// stdout과 stderr를 도착 순서로 합친 글.
    pub output: String,
}

impl ShellOutput {
    /// 다음 입력에 붙일 첨부 글. 명령, 종료 코드, 출력을 담는다.
    pub fn to_attachment(&self) -> String {
        todo!("#92")
    }
}

/// 셸로 `workdir`에서 실행하고 끝날 때까지 기다린다. 표준 입력은 닫는다. 셸 선택은 초안이다(설계에 없음).
/// TODO(#92): 값 미정, 초안 `$SHELL -c`, 없으면 `sh -c`
/// 전체 화면이면 호출 전에 화면을 복원하지 않는다(출력을 잡아 대화 기록에 넣는다).
///
/// # Errors
/// 셸을 띄우지 못하면 `Spawn`.
pub async fn run(command: &str, workdir: &Path) -> Result<ShellOutput, ShellError> {
    todo!("#92")
}
