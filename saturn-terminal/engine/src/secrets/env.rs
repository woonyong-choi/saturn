//! 자식 프로세스 환경에서 지울 변수 목록(한 곳)과 제거 함수.
//!
//! 설계: docs/design/judge-key-security.md(자식 환경의 변수 제거). 두 번 지운다:
//! engine → `Supervisor`로 넘길 때 한 번, `Supervisor` → provider로 넘길 때 다시 한 번. 목록은 여기에만 두고 테스트로 고정한다.

use std::ffi::{OsStr, OsString};

use super::keys::JUDGE_KEY_ENV;

/// 자식 환경에서 지우는 변수 이름. judge 키와 Saturn 내부 비밀 변수를 모두 여기에 둔다. 새 비밀 변수를 만들면 여기에 더한다.
/// TODO(#32): 벤더 이름 변수(예: judge 벤더의 API 키 변수)를 함께 지울지와 최종 이름
pub const CHILD_ENV_DENYLIST: &[&str] = &[JUDGE_KEY_ENV];

/// `name`이 `CHILD_ENV_DENYLIST`에 있는지. 대소문자를 구분한다(macOS 환경 변수는 구분한다).
pub fn is_denied(name: &OsStr) -> bool {
    todo!("#84")
}

/// 환경 목록에서 제외 목록의 변수를 뺀 새 목록. `Supervisor`가 자식에 줄 환경을 `env_clear` 뒤 이것으로 채울 때 쓴다.
pub fn scrub(env: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    todo!("#84")
}

/// 실행할 명령에 제외 목록의 변수마다 `env_remove`를 건다. 부모 환경을 물려받는 명령에 쓴다.
pub fn scrub_command(command: &mut tokio::process::Command) {
    todo!("#84")
}
