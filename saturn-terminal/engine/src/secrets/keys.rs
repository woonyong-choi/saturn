//! router 키 값과 세 가지 입력 방법. 명령 인자와 표준 입력으로는 받지 않는다.
//! 설계: docs/design/router-key-security.md

use std::process::Stdio;

use serde::{Deserialize, Serialize};

use super::{SecretsError, scrub_command};

pub(crate) const ROUTER_KEY_ENV: &str = "SATURN_KEY";

/// 저장이나 출력으로 새지 않게 `Serialize`, `Display`가 없고, 버릴 때 메모리를 0으로 덮는다.
pub(crate) struct RouterKey {
    value: String,
}

impl RouterKey {
    /// 앞뒤 공백과 끝 줄바꿈을 지우고 만든다.
    ///
    /// # Errors
    /// 남은 값이 비었으면 `Empty`.
    pub(crate) fn new(raw: String) -> Result<Self, SecretsError> {
        let trimmed = raw.trim().to_owned();
        wipe(raw);
        if trimmed.is_empty() {
            return Err(SecretsError::Empty);
        }
        Ok(Self { value: trimmed })
    }

    /// Authorization 헤더를 만들 때만 쓰고, 그 헤더는 기록하지 않는다.
    pub(crate) fn expose(&self) -> &str {
        &self.value
    }

    /// 4자보다 짧으면 전부 `*`.
    pub(crate) fn last4(&self) -> String {
        let count = self.value.chars().count();
        if count < 4 {
            return "*".repeat(count);
        }
        self.value.chars().skip(count - 4).collect()
    }
}

impl std::fmt::Debug for RouterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RouterKey(****{})", self.last4())
    }
}

impl Drop for RouterKey {
    /// 컴파일러가 덮어쓰기를 지우지 못하게 `write_volatile`을 쓴다.
    fn drop(&mut self) {
        wipe(std::mem::take(&mut self.value));
    }
}

/// `Hidden`에 키 원문이 있어 `Debug`는 종류만 보인다.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum KeyInput {
    /// TUI는 받은 즉시 engine에 보내고 자기 기억과 입력 기록에 남기지 않는다.
    Hidden(String),
    Env,
    /// 셸 없이 실행하고 받은 키는 메모리에만 둔다.
    Command {
        /// 설정에서 읽은 그대로.
        argv: Vec<String>,
    },
}

impl std::fmt::Debug for KeyInput {
    /// 숨김 입력 값은 쓰지 않는다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hidden(_) => f.write_str("Hidden(..)"),
            Self::Env => f.write_str("Env"),
            Self::Command { .. } => f.write_str("Command(..)"),
        }
    }
}

/// 설정에는 이것과 끝 4자리만 남긴다. 쓰기는 소문자, 읽기는 옛 대문자 시작 값도 받는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KeySource {
    /// 키체인(또는 0600 파일)에 저장했다.
    #[serde(alias = "Stored")]
    Stored,
    /// 저장하지 않는다.
    #[serde(alias = "Env")]
    Env,
    /// 메모리에만 둔다.
    #[serde(alias = "Command")]
    Command,
}

/// 키 자체는 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeyInfo {
    pub source: KeySource,
    pub last4: String,
}

/// 관리자 명령의 자식 환경에서도 키 변수를 지운다. 키 확인은 호출자가 한다.
///
/// # Errors
/// 값이 없으면 `NotFound`, 비었으면 `Empty`, 명령 실패면 `Command`.
pub(crate) async fn acquire(input: KeyInput) -> Result<(RouterKey, KeySource), SecretsError> {
    acquire_with_env(input, std::env::var(ROUTER_KEY_ENV).ok()).await
}

/// 테스트가 프로세스 환경을 바꾸지 않게 환경 변수 값을 밖에서 받는다.
pub(crate) async fn acquire_with_env(
    input: KeyInput,
    env_value: Option<String>,
) -> Result<(RouterKey, KeySource), SecretsError> {
    match input {
        KeyInput::Hidden(value) => Ok((RouterKey::new(value)?, KeySource::Stored)),
        KeyInput::Env => {
            let value = env_value.ok_or(SecretsError::NotFound)?;
            Ok((RouterKey::new(value)?, KeySource::Env))
        }
        KeyInput::Command { argv } => Ok((run_key_command(&argv).await?, KeySource::Command)),
    }
}

/// 숨김 입력은 TUI가 값을 보내야 해서 넣지 않는다. 초안 순서.
pub(crate) fn input_order(key_command: Option<Vec<String>>) -> Vec<KeyInput> {
    let mut order = vec![KeyInput::Env];
    if let Some(argv) = key_command {
        order.push(KeyInput::Command { argv });
    }
    order
}

/// 셸 없이 실행하고 자식 환경에서 키 변수를 지운다.
async fn run_key_command(argv: &[String]) -> Result<RouterKey, SecretsError> {
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
        .stderr(Stdio::null())
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
        let code = output
            .status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string());
        wipe(stdout);
        return Err(SecretsError::Command {
            detail: format!("exit {code}"),
        });
    }
    wipe(stdout);
    RouterKey::new(first_line)
}

fn wipe(text: String) {
    let mut bytes = text.into_bytes();
    for byte in bytes.iter_mut() {
        // SAFETY: `byte`는 살아 있는 `Vec`의 원소를 가리키는 유효한 가변 참조다
        #[expect(unsafe_code, reason = "컴파일러가 지우지 못하게 하는 volatile 쓰기")]
        unsafe {
            std::ptr::write_volatile(byte, 0)
        };
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
        let key = RouterKey::new("  sk-secret-abcd\n".to_owned()).unwrap();

        assert_eq!(key.expose(), "sk-secret-abcd");
        assert_eq!(key.last4(), "abcd");
        assert_eq!(format!("{key:?}"), "RouterKey(****abcd)");
        assert_eq!(RouterKey::new("abc".to_owned()).unwrap().last4(), "***");
        assert!(matches!(
            RouterKey::new(" \n".to_owned()),
            Err(SecretsError::Empty)
        ));
    }

    #[test]
    fn hidden_input_debug_hides_value() {
        let input = KeyInput::Hidden("sk-secret".to_owned());

        assert_eq!(format!("{input:?}"), "Hidden(..)");
        let command = KeyInput::Command {
            argv: vec!["echo".to_owned(), "sk-secret".to_owned()],
        };
        assert_eq!(format!("{command:?}"), "Command(..)");
    }

    #[test]
    fn input_order_is_env_then_command_and_never_stdin() {
        let command = argv(&["op", "read", "x"]);

        let with_command = input_order(Some(command.clone()));
        let without_command = input_order(None);

        assert_eq!(
            with_command,
            vec![KeyInput::Env, KeyInput::Command { argv: command }]
        );
        assert_eq!(without_command, vec![KeyInput::Env]);
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
            argv: argv(&["/usr/bin/printf", "sk-from-manager\\nsecond-line\\n"]),
        };

        let (key, source) = acquire_with_env(input, None).await.unwrap();

        assert_eq!(
            (key.expose(), source),
            ("sk-from-manager", KeySource::Command)
        );
    }

    #[tokio::test]
    async fn failed_command_does_not_expose_stderr() {
        let script = "echo \"bad sk-leaked\" >&2; exit 3";
        let input = KeyInput::Command {
            argv: argv(&["/bin/sh", "-c", script]),
        };

        let error = acquire_with_env(input, None).await.unwrap_err();

        let SecretsError::Command { detail } = error else {
            panic!("command failure expected");
        };
        assert_eq!(detail, "exit 3");
        let empty = run_key_command(&[]).await.unwrap_err();
        assert!(matches!(empty, SecretsError::Command { .. }));
    }

    #[tokio::test]
    async fn command_child_does_not_see_router_key_variable() {
        let script = format!("echo \"${{{ROUTER_KEY_ENV}:-absent}}\"");
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", &script])
            .env(ROUTER_KEY_ENV, "sk-parent");
        scrub_command(&mut command);

        let output = command.output().await.unwrap();

        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "absent");
    }
}
