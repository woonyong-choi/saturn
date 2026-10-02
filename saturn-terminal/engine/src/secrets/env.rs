//! 자식 프로세스 환경에서 router 키 변수를 지운다. engine → `Supervisor`, `Supervisor` → provider에서 두 번 지운다.
//! 설계: docs/design/router-key-security.md

use std::ffi::{OsStr, OsString};

use super::keys::ROUTER_KEY_ENV;

/// 새 비밀 변수를 만들면 여기에 더한다.
pub(crate) const CHILD_ENV_DENYLIST: &[&str] = &[ROUTER_KEY_ENV];

/// macOS 환경 변수처럼 대소문자를 구분한다.
pub(crate) fn is_denied(name: &OsStr) -> bool {
    CHILD_ENV_DENYLIST
        .iter()
        .any(|denied| OsStr::new(denied) == name)
}

/// `Supervisor`가 `env_clear` 뒤 자식 환경을 이것으로 채운다.
pub(crate) fn scrub(
    env: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    env.into_iter()
        .filter(|(name, _)| !is_denied(name))
        .collect()
}

/// 부모 환경을 물려받는 명령에 쓴다.
pub(crate) fn scrub_command(command: &mut tokio::process::Command) {
    for name in CHILD_ENV_DENYLIST {
        command.env_remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn denylist_holds_router_key_variable() {
        assert!(CHILD_ENV_DENYLIST.contains(&ROUTER_KEY_ENV));
        assert!(is_denied(OsStr::new(ROUTER_KEY_ENV)));
        assert!(!is_denied(OsStr::new("PATH")));
        assert!(!is_denied(OsStr::new(&ROUTER_KEY_ENV.to_lowercase())));
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
        command.env(ROUTER_KEY_ENV, "secret-value").env("KEEP", "1");

        scrub_command(&mut command);

        let envs: Vec<(OsString, Option<OsString>)> = command
            .as_std()
            .get_envs()
            .map(|(name, value)| (name.to_owned(), value.map(OsStr::to_owned)))
            .collect();
        assert!(envs.contains(&(OsString::from(ROUTER_KEY_ENV), None)));
        assert!(envs.contains(&(OsString::from("KEEP"), Some(OsString::from("1")))));
    }
}
