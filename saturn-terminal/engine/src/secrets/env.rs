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
    CHILD_ENV_DENYLIST
        .iter()
        .any(|denied| OsStr::new(denied) == name)
}

/// 환경 목록에서 제외 목록의 변수를 뺀 새 목록. `Supervisor`가 자식에 줄 환경을 `env_clear` 뒤 이것으로 채울 때 쓴다.
pub fn scrub(env: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(OsString, OsString)> {
    env.into_iter()
        .filter(|(name, _)| !is_denied(name))
        .collect()
}

/// 실행할 명령에 제외 목록의 변수마다 `env_remove`를 건다. 부모 환경을 물려받는 명령에 쓴다.
pub fn scrub_command(command: &mut tokio::process::Command) {
    for name in CHILD_ENV_DENYLIST {
        command.env_remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denylist_holds_judge_key_variable() {
        assert!(CHILD_ENV_DENYLIST.contains(&JUDGE_KEY_ENV));
        assert!(is_denied(OsStr::new(JUDGE_KEY_ENV)));
        assert!(!is_denied(OsStr::new("PATH")));
        assert!(!is_denied(OsStr::new(&JUDGE_KEY_ENV.to_lowercase())));
    }

    #[test]
    fn scrub_removes_every_denied_name() {
        let mut env: Vec<(OsString, OsString)> = CHILD_ENV_DENYLIST
            .iter()
            .map(|name| (OsString::from(name), OsString::from("secret-value")))
            .collect();
        env.push(("PATH".into(), "/usr/bin".into()));
        env.push(("HOME".into(), "/Users/me".into()));

        let child = scrub(env);

        assert!(child.iter().all(|(name, _)| !is_denied(name)));
        assert!(child.iter().all(|(_, value)| value != "secret-value"));
        assert_eq!(child.len(), 2);
    }

    #[test]
    fn scrub_command_removes_inherited_and_explicit_values() {
        let mut command = tokio::process::Command::new("/usr/bin/true");
        command.env(JUDGE_KEY_ENV, "secret-value").env("KEEP", "1");

        scrub_command(&mut command);

        let envs: Vec<(OsString, Option<OsString>)> = command
            .as_std()
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
            .collect();
        assert!(envs.contains(&(OsString::from(JUDGE_KEY_ENV), None)));
        assert!(envs.contains(&(OsString::from("KEEP"), Some(OsString::from("1")))));
    }
}
