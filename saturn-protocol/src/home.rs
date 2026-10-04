//! Saturn 홈 폴더 위치. engine, TUI, cli가 같은 규칙으로 소켓과 로그를 찾는다.
//! 설계: docs/design/settings.md#이름-규칙

use std::ffi::OsString;
use std::path::PathBuf;

/// 홈 폴더를 정하는 환경 변수. 설정 파일의 키가 아니다.
pub const HOME_ENV: &str = "SATURN_HOME";

/// 사용자 홈 아래 기본 폴더 이름.
pub const DEFAULT_DIR: &str = ".saturn";

/// 홈 폴더 안의 engine 소켓 파일 이름.
pub const SOCKET_FILE: &str = "engine.sock";

/// 홈 폴더 안의 engine 로그 폴더 이름.
pub const LOG_DIR: &str = "logs";

/// `SATURN_HOME`이 있고 비어 있지 않으면 그 폴더(상대 경로는 현재 폴더 기준 절대 경로), 아니면 `<HOME>/.saturn`.
/// 둘 다 없으면 `None`.
pub fn resolve(saturn_home: Option<OsString>, user_home: Option<OsString>) -> Option<PathBuf> {
    if let Some(path) = saturn_home.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(path);
        return Some(std::path::absolute(&path).unwrap_or(path));
    }
    user_home
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(DEFAULT_DIR))
}

/// 이 프로세스의 환경 변수 `SATURN_HOME`과 `HOME`으로 정한 홈 폴더.
pub fn from_env() -> Option<PathBuf> {
    resolve(std::env::var_os(HOME_ENV), std::env::var_os("HOME"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(text: &str) -> Option<OsString> {
        Some(OsString::from(text))
    }

    #[test]
    fn saturn_home_wins_over_the_user_home() {
        assert_eq!(
            resolve(os("/data/saturn"), os("/home/me")),
            Some(PathBuf::from("/data/saturn"))
        );
    }

    #[test]
    fn default_is_dot_saturn_under_the_user_home() {
        assert_eq!(
            resolve(None, os("/home/me")),
            Some(PathBuf::from("/home/me/.saturn"))
        );
        assert_eq!(
            resolve(os(""), os("/home/me")),
            Some(PathBuf::from("/home/me/.saturn"))
        );
    }

    #[test]
    fn relative_saturn_home_becomes_absolute() {
        let path = resolve(os("rel/saturn"), None).unwrap();
        assert!(path.is_absolute());
        assert!(path.ends_with("rel/saturn"));
    }

    #[test]
    fn no_home_at_all_gives_none() {
        assert_eq!(resolve(None, None), None);
        assert_eq!(resolve(None, os("")), None);
    }
}
