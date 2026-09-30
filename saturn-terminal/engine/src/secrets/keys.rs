//! judge 키 값과 네 가지 입력 방법.
//!
//! 설계: docs/design/judge-key-security.md(키 입력). 명령 인자(`--key` 등)로는 받지 않는다. `cli`에 그런 옵션을 두지 않는다.
//! TODO(#32): 환경 변수와 관리자 명령 설정 키 이름을 `SATURN_JUDGE_KEY`, `judge.key_command`로 할지, 벤더 이름을 유지할지

use serde::{Deserialize, Serialize};

use super::SecretsError;

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
        todo!("#84")
    }

    /// 키 원문. `judges::RemoteJudge`가 Authorization 헤더를 만들 때만 쓴다. 그 헤더는 기록하지 않는다.
    pub fn expose(&self) -> &str {
        todo!("#84")
    }

    /// 끝 4자리. 4자보다 짧으면 전부 `*`.
    pub fn last4(&self) -> String {
        todo!("#84")
    }
}

impl std::fmt::Debug for JudgeKey {
    /// `JudgeKey(****abcd)` 형태로만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("#84")
    }
}

impl Drop for JudgeKey {
    /// `mem::take`로 꺼낸 바이트 버퍼를 `ptr::write_volatile`로 0으로 덮는다(컴파일러가 덮어쓰기를 지우지 못하게).
    fn drop(&mut self) {
        todo!("#84")
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
        todo!("#84")
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
    todo!("#84")
}

/// 숨김 입력을 요청할 수 있는지. stdin과 stdout이 모두 TTY이고 TUI가 붙어 있을 때만 참. 거짓이면 호출자는 `NonInteractive`로 끝낸다.
pub fn can_prompt(tui_attached: bool) -> bool {
    todo!("#84")
}
