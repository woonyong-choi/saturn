//! 입력창 `!` 셸 명령. router와 engine을 거치지 않고 작업 폴더에서 바로 실행한다.
//! 설계: docs/design/tui.md

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::ROUTER_KEY_ENV;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ShellError {
    #[error("failed to spawn shell")]
    Spawn(#[source] std::io::Error),
    #[error("failed to read shell output")]
    Output(#[source] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShellOutput {
    pub command: String,
    /// 신호로 끝났으면 `None`.
    pub status: Option<i32>,
    /// stdout과 stderr를 도착 순서로 합친 글.
    pub output: String,
}

impl ShellOutput {
    pub(crate) fn to_attachment(&self) -> String {
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
/// 끝날 때까지 기다리고 표준 입력은 닫는다.
///
/// # Errors
/// 셸을 띄우지 못하면 `Spawn`.
pub(crate) async fn run(command: &str, workdir: &Path) -> Result<ShellOutput, ShellError> {
    let command = command.to_string();
    let workdir = workdir.to_path_buf();
    tokio::task::spawn_blocking(move || run_blocking(&command, &workdir))
        .await
        .map_err(|join| ShellError::Spawn(std::io::Error::other(join)))?
}

// cost: time O(1), heap O(1), stack O(1), io 1
// basis: estimate
/// `$SHELL`이 비어 있으면 `sh`(초안).
fn shell_program() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "sh".to_string())
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 셸은 router 키 환경 변수 없이 작업 폴더에서 띄운다.
fn shell_command(program: &str, command: &str, workdir: &Path) -> Command {
    let mut shell = Command::new(program);
    shell
        .arg("-c")
        .arg(command)
        .env_remove(ROUTER_KEY_ENV)
        .current_dir(workdir);
    shell
}

// cost: time O(n), heap O(n), stack O(1), io 1
// vars: n = 출력 바이트 수
// basis: estimate
/// stdout과 stderr를 한 파이프로 받아 도착 순서를 지킨다.
fn run_blocking(command: &str, workdir: &Path) -> Result<ShellOutput, ShellError> {
    let (mut reader, writer) = std::io::pipe().map_err(ShellError::Spawn)?;
    let mut child = {
        let stderr = writer.try_clone().map_err(ShellError::Spawn)?;
        // `Command`가 파이프 쓰는 쪽을 쥐고 있으므로 블록 끝에서 버려야 읽기가 끝난다.
        shell_command(&shell_program(), command, workdir)
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
    fn the_shell_is_started_in_the_workdir_without_the_router_key() {
        let command = shell_command("sh", "ls", Path::new("/work"));

        assert_eq!(command.get_program(), "sh");
        assert_eq!(command.get_current_dir(), Some(Path::new("/work")));
        let envs: Vec<_> = command.get_envs().collect();
        assert_eq!(envs, [(std::ffi::OsStr::new(ROUTER_KEY_ENV), None)]);
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
