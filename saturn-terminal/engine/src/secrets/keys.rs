//! judge 키 값과 네 가지 입력 방법.
//!
//! 설계: docs/design/judge-key-security.md(키 입력). 명령 인자(`--key` 등)로는 받지 않는다. `cli`에 그런 옵션을 두지 않는다.
//! 키 받는 순서는 `input_order`(초안)다.
//! TODO(#32): 환경 변수와 관리자 명령 설정 키 이름을 `SATURN_JUDGE_KEY`, `judge.key_command`로 할지, 벤더 이름을 유지할지

use std::io::{BufRead, IsTerminal};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use super::{Masker, SecretsError, scrub_command};

/// judge 키 환경 변수 이름. 자식 환경 제외 목록에도 들어간다. TODO(#32): 이름 확정
pub const JUDGE_KEY_ENV: &str = "SATURN_JUDGE_KEY";

/// judge 키 값. `Serialize`, `Display`가 없고 `Debug`는 끝 4자리만 보여 저장이나 출력으로 새지 않는다.
/// 버릴 때 메모리를 0으로 덮는다.
pub struct JudgeKey {
    value: String,
}

impl JudgeKey {
    /// 받은 문자열에서 앞뒤 공백과 끝 줄바꿈을 지우고 만든다.
    ///
    /// # Errors
    /// 남은 값이 비었으면 `Empty`.
    pub fn new(raw: String) -> Result<Self, SecretsError> {
        let trimmed = raw.trim().to_owned();
        wipe(raw);
        if trimmed.is_empty() {
            return Err(SecretsError::Empty);
        }
        Ok(Self { value: trimmed })
    }

    /// 키 원문. `judges::RemoteJudge`가 Authorization 헤더를 만들 때만 쓴다. 그 헤더는 기록하지 않는다.
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// 끝 4자리. 4자보다 짧으면 전부 `*`.
    pub fn last4(&self) -> String {
        let count = self.value.chars().count();
        if count < 4 {
            return "*".repeat(count);
        }
        self.value.chars().skip(count - 4).collect()
    }
}

impl std::fmt::Debug for JudgeKey {
    /// `JudgeKey(****abcd)` 형태로만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JudgeKey(****{})", self.last4())
    }
}

impl Drop for JudgeKey {
    /// `mem::take`로 꺼낸 바이트 버퍼를 `ptr::write_volatile`로 0으로 덮는다(컴파일러가 덮어쓰기를 지우지 못하게).
    fn drop(&mut self) {
        wipe(std::mem::take(&mut self.value));
    }
}

/// 키를 어떻게 받을지. 네 가지뿐이다. `Hidden`에 키 원문이 있어 `Debug`는 종류만 보인다.
#[derive(Clone, PartialEq, Eq)]
pub enum KeyInput {
    /// TUI 숨김 입력으로 받은 값. TUI는 받은 즉시 engine에 보내고 자기 기억과 입력 기록(`~/.saturn/history`)에 남기지 않는다.
    Hidden(String),
    /// 표준 입력 한 줄(`echo $KEY | saturn ...`). 비대화 환경용.
    Stdin,
    /// 환경 변수 `JUDGE_KEY_ENV`.
    Env,
    /// 비밀번호 관리자 명령(사용자 층 설정의 관리자 명령). 셸 없이 실행하고 stdout 첫 줄을 키로 쓴다. 받은 키는 메모리에만 둔다.
    Command {
        /// 실행 파일과 인자. 설정에서 읽은 그대로.
        argv: Vec<String>,
    },
}

impl std::fmt::Debug for KeyInput {
    /// `Hidden(..)`, `Stdin`, `Env`, `Command { argv }`로 쓴다. 숨김 입력 값은 쓰지 않는다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hidden(_) => f.write_str("Hidden(..)"),
            Self::Stdin => f.write_str("Stdin"),
            Self::Env => f.write_str("Env"),
            Self::Command { argv } => f.debug_struct("Command").field("argv", argv).finish(),
        }
    }
}

/// 키의 출처. 설정에는 이것과 끝 4자리만 남긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeySource {
    /// 숨김 입력으로 받아 키체인(또는 0600 파일)에 저장했다.
    Stored,
    /// 표준 입력으로 받아 저장했다.
    Stdin,
    /// 실행마다 환경 변수에서 읽는다. 저장하지 않는다.
    Env,
    /// 실행마다 관리자 명령으로 받는다. 메모리에만 둔다.
    Command,
}

/// 설정 파일에 남기는 키 정보. 키 자체는 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInfo {
    /// 출처.
    pub source: KeySource,
    /// 끝 4자리.
    pub last4: String,
}

/// 키를 받는다. `Hidden`은 값 그대로, `Stdin`은 한 줄, `Env`는 `JUDGE_KEY_ENV`, `Command`는 `tokio::process`로 실행한다.
/// 관리자 명령의 자식 환경도 `scrub_command`로 정리한다. 확인(`GET /v1/models`)은 호출자(`judges`)가 한다.
///
/// # Errors
/// 값이 없으면 `NotFound`, 비었으면 `Empty`, 명령 실패면 `Command`, 표준 입력 실패면 `Io`.
pub async fn acquire(input: KeyInput) -> Result<(JudgeKey, KeySource), SecretsError> {
    acquire_with_env(input, std::env::var(JUDGE_KEY_ENV).ok()).await
}

/// `acquire`의 본문. 환경 변수 값을 밖에서 받아 테스트가 프로세스 환경을 바꾸지 않게 한다.
pub(crate) async fn acquire_with_env(
    input: KeyInput,
    env_value: Option<String>,
) -> Result<(JudgeKey, KeySource), SecretsError> {
    match input {
        KeyInput::Hidden(value) => Ok((JudgeKey::new(value)?, KeySource::Stored)),
        KeyInput::Stdin => {
            let line = tokio::task::spawn_blocking(|| read_key_line(std::io::stdin().lock()))
                .await
                .map_err(std::io::Error::other)??;
            Ok((JudgeKey::new(line)?, KeySource::Stdin))
        }
        KeyInput::Env => {
            let value = env_value.ok_or(SecretsError::NotFound)?;
            Ok((JudgeKey::new(value)?, KeySource::Env))
        }
        KeyInput::Command { argv } => Ok((run_key_command(&argv).await?, KeySource::Command)),
    }
}

/// 키를 받는 순서(초안, 설계는 네 방법만 정함): 환경 변수 → 관리자 명령 설정 → 표준 입력 → 숨김 입력.
/// 표준 입력은 입력할 수 없는 환경(`interactive`가 거짓)에서만 넣는다. 숨김 입력은 TUI가 값을 보내야 하므로 목록에 넣지 않고,
/// 목록을 모두 실패한 뒤 `can_prompt`가 참일 때 호출자가 TUI에 요청한다.
pub fn input_order(key_command: Option<Vec<String>>, interactive: bool) -> Vec<KeyInput> {
    let mut order = vec![KeyInput::Env];
    if let Some(argv) = key_command {
        order.push(KeyInput::Command { argv });
    }
    if !interactive {
        order.push(KeyInput::Stdin);
    }
    order
}

/// 숨김 입력을 요청할 수 있는지. stdin과 stdout이 모두 TTY이고 TUI가 붙어 있을 때만 참. 거짓이면 호출자는 `NonInteractive`로 끝낸다.
pub fn can_prompt(tui_attached: bool) -> bool {
    tui_attached && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// 한 줄을 읽는다. 아무것도 없으면 `NotFound`.
fn read_key_line(mut reader: impl BufRead) -> Result<String, SecretsError> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(SecretsError::NotFound);
    }
    Ok(line)
}

/// 관리자 명령을 셸 없이 실행하고 stdout 첫 줄을 키로 쓴다. 자식 환경에서 제외 목록 변수를 지운다.
async fn run_key_command(argv: &[String]) -> Result<JudgeKey, SecretsError> {
    let Some((program, args)) = argv.split_first() else {
        return Err(SecretsError::Command {
            detail: "empty command".to_owned(),
        });
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    scrub_command(&mut command);
    let output = command
        .output()
        .await
        .map_err(|error| SecretsError::Command {
            detail: format!("failed to start: {}", error.kind()),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let first_line = stdout.lines().next().unwrap_or_default().to_owned();
    if !output.status.success() {
        let masker = Masker::new(vec![first_line.trim().to_owned()]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_line = stderr.lines().next().unwrap_or_default();
        let code = output
            .status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string());
        wipe(stdout);
        return Err(SecretsError::Command {
            detail: format!("exit {code}: {}", masker.mask(stderr_line).as_str()),
        });
    }
    wipe(stdout);
    JudgeKey::new(first_line)
}

/// 문자열 버퍼를 0으로 덮고 버린다.
fn wipe(text: String) {
    let mut bytes = text.into_bytes();
    for byte in bytes.iter_mut() {
        // SAFETY: `byte`는 살아 있는 `Vec`의 원소를 가리키는 유효한 가변 참조다
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn key_is_trimmed_and_never_printed_whole() {
        let key = JudgeKey::new("  sk-secret-abcd\n".to_owned()).unwrap();

        assert_eq!(key.expose(), "sk-secret-abcd");
        assert_eq!(key.last4(), "abcd");
        assert_eq!(format!("{key:?}"), "JudgeKey(****abcd)");
        assert_eq!(JudgeKey::new("abc".to_owned()).unwrap().last4(), "***");
        assert!(matches!(
            JudgeKey::new(" \n".to_owned()),
            Err(SecretsError::Empty)
        ));
    }

    #[test]
    fn hidden_input_debug_hides_value() {
        let input = KeyInput::Hidden("sk-secret".to_owned());

        assert_eq!(format!("{input:?}"), "Hidden(..)");
    }

    #[test]
    fn stdin_line_is_read_once() {
        let line = read_key_line("sk-line\nnext\n".as_bytes()).unwrap();

        assert_eq!(JudgeKey::new(line).unwrap().expose(), "sk-line");
        assert!(matches!(
            read_key_line("".as_bytes()),
            Err(SecretsError::NotFound)
        ));
    }

    #[test]
    fn input_order_is_env_command_then_stdin() {
        let command = argv(&["op", "read", "x"]);

        let piped = input_order(Some(command.clone()), false);
        let terminal = input_order(None, true);

        assert_eq!(
            piped,
            vec![
                KeyInput::Env,
                KeyInput::Command { argv: command },
                KeyInput::Stdin
            ]
        );
        assert_eq!(terminal, vec![KeyInput::Env]);
    }

    #[tokio::test]
    async fn env_and_hidden_sources() {
        let (key, source) = acquire_with_env(KeyInput::Env, Some("sk-env".to_owned()))
            .await
            .unwrap();
        assert_eq!((key.expose(), source), ("sk-env", KeySource::Env));
        let missing = acquire_with_env(KeyInput::Env, None).await.unwrap_err();
        assert!(matches!(missing, SecretsError::NotFound));
        let (_, source) = acquire_with_env(KeyInput::Hidden("sk-h".to_owned()), None)
            .await
            .unwrap();
        assert_eq!(source, KeySource::Stored);
    }

    #[tokio::test]
    async fn command_first_stdout_line_is_key() {
        let input = KeyInput::Command {
            argv: argv(&["/bin/echo", "sk-from-manager"]),
        };

        let (key, source) = acquire_with_env(input, None).await.unwrap();

        assert_eq!(
            (key.expose(), source),
            ("sk-from-manager", KeySource::Command)
        );
    }

    #[tokio::test]
    async fn failed_command_masks_key_in_stderr() {
        let script = "echo sk-leaked; echo \"bad sk-leaked\" >&2; exit 3";
        let input = KeyInput::Command {
            argv: argv(&["/bin/sh", "-c", script]),
        };

        let error = acquire_with_env(input, None).await.unwrap_err();

        let SecretsError::Command { detail } = error else {
            panic!("command failure expected");
        };
        assert_eq!(detail, "exit 3: bad [redacted]");
        let empty = run_key_command(&[]).await.unwrap_err();
        assert!(matches!(empty, SecretsError::Command { .. }));
    }

    #[tokio::test]
    async fn command_child_does_not_see_judge_key_variable() {
        let script = format!("echo \"${{{JUDGE_KEY_ENV}:-absent}}\"");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", &script])
            .env(JUDGE_KEY_ENV, "sk-parent");
        scrub_command(&mut command);

        let output = command.output().await.unwrap();

        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "absent");
    }
}
