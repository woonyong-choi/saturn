//! 입력창 `!` 셸 명령. 줄 앞 `!`로 시작한 입력을 작업 폴더에서 바로 실행한다.
//!
//! 설계: docs/design/tui.md(키 `!`). 결과는 대화 기록에 보이고 메인 에이전트의 다음 입력에 첨부한다.
//! judge를 거치지 않고 engine에도 보내지 않는다.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

/// 셸 명령 실행 오류. 대화 기록에 한 줄로 보인다.
#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    /// 셸을 띄우지 못했다.
    #[error("failed to spawn shell")]
    Spawn(#[source] std::io::Error),
    /// 셸 출력을 읽지 못했거나 끝을 기다리지 못했다.
    #[error("failed to read shell output")]
    Output(#[source] std::io::Error),
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
        let status = match self.status {
            Some(code) => code.to_string(),
            None => "signal".to_string(),
        };
        format!(
            "$ {}\n{}\n[exit {}]",
            self.command,
            self.output.trim_end(),
            status
        )
    }
}

// cost: time O(n), heap O(n), stack O(1), io 1
// vars: n = 출력 바이트 수
// basis: estimate
/// 셸로 `workdir`에서 실행하고 끝날 때까지 기다린다. 표준 입력은 닫는다.
/// 셸 선택 `$SHELL -c`, 없으면 `sh -c`는 초안이다(설계에 없음, docs/design/tui.md 초안 값).
/// 전체 화면이면 호출 전에 화면을 복원하지 않는다(출력을 잡아 대화 기록에 넣는다).
///
/// # Errors
/// 셸을 띄우지 못하면 `Spawn`.
pub async fn run(command: &str, workdir: &Path) -> Result<ShellOutput, ShellError> {
    let command = command.to_string();
    let workdir = workdir.to_path_buf();
    tokio::task::spawn_blocking(move || run_blocking(&command, &workdir))
        .await
        .map_err(|join| ShellError::Spawn(std::io::Error::other(join)))?
}

// cost: time O(1), heap O(1), stack O(1), io 1
// basis: estimate
/// 실행할 셸. `$SHELL`이 비어 있지 않으면 그것, 아니면 `sh`.
fn shell_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

// cost: time O(n), heap O(n), stack O(1), io 1
// vars: n = 출력 바이트 수
// basis: estimate
/// `run`의 막히는 본체. stdout과 stderr를 한 파이프로 받아 도착 순서를 지킨다.
fn run_blocking(command: &str, workdir: &Path) -> Result<ShellOutput, ShellError> {
    let (mut reader, writer) = std::io::pipe().map_err(ShellError::Spawn)?;
    let mut child = {
        let stderr = writer.try_clone().map_err(ShellError::Spawn)?;
        // `Command`가 파이프 쓰는 쪽을 쥐고 있으므로 블록 끝에서 버려야 읽기가 끝난다.
        Command::new(shell_program())
            .arg("-c")
            .arg(command)
            .current_dir(workdir)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(stderr)
            .spawn()
            .map_err(ShellError::Spawn)?
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(ShellError::Output)?;
    let status = child.wait().map_err(ShellError::Output)?;
    Ok(ShellOutput {
        command: command.to_string(),
        status: status.code(),
        output: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[tokio::test]
    async fn run_merges_stdout_and_stderr_with_status() {
        let workdir = std::env::current_dir().unwrap();

        let output = run("echo out; echo err 1>&2; exit 3", &workdir)
            .await
            .unwrap();

        assert_eq!(output.status, Some(3));
        assert!(output.output.contains("out"));
        assert!(output.output.contains("err"));
    }

    #[test]
    fn to_attachment_includes_command_output_and_status() {
        let output = ShellOutput {
            command: "ls".to_string(),
            status: Some(0),
            output: "a\nb\n".to_string(),
        };

        assert_eq!(output.to_attachment(), "$ ls\na\nb\n[exit 0]");
    }
}
