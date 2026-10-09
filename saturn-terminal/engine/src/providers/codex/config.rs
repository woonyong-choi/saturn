use super::AUTO_COMPACT_KEY;
use crate::providers::{LaunchSpec, UserProviderConfig};
use std::ffi::OsString;
use std::path::PathBuf;

pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !user.has_auto_compact
        && let Some(tokens) = launch.defaults.auto_compact_tokens
    {
        args.push("-c".to_owned());
        args.push(format!("{AUTO_COMPACT_KEY}={tokens}"));
    }
    args
}

/// 권한 번역이 정한 환경 변수(전용 `CODEX_HOME`)를 부모 환경 값 위에 덮어쓴다.
pub(super) fn with_env_overrides(mut launch: LaunchSpec) -> LaunchSpec {
    for (name, value) in launch.permission.env.clone() {
        launch.env.retain(|(existing, _)| *existing != name);
        launch.env.push((name, value));
    }
    launch
}

/// 사용자 Codex 폴더(읽기용). 환경 `CODEX_HOME`이 비어 있지 않으면 그 폴더, 아니면 `HOME` 아래 `.codex`다. 환경은 부모
/// 환경이 아니라 provider가 받는 값이다. 상대 경로는 정하지 못하므로(채팅마다 provider 작업 폴더가 다르다) `None`이고
/// 연결을 준비하기 전에 `relative_home`으로 거절한다. 변수와 `HOME`이 모두 없어도 `None`. Saturn 전용 `CODEX_HOME`
/// (주입용)은 `permission.env`에만 있고 이 함수가 보는 환경에는 들어오지 않는다.
pub(super) fn user_folder(env: &[(OsString, OsString)]) -> Option<PathBuf> {
    if let Some(custom) = env_value(env, "CODEX_HOME") {
        let dir = PathBuf::from(custom);
        return dir.is_absolute().then_some(dir);
    }
    Some(PathBuf::from(env_value(env, "HOME")?).join(".codex"))
}

/// `CODEX_HOME`이 상대 경로이면 그 값. 이 경우 `user_folder`가 폴더를 정하지 못한다.
pub(super) fn relative_home(env: &[(OsString, OsString)]) -> Option<PathBuf> {
    env_value(env, "CODEX_HOME")
        .map(PathBuf::from)
        .filter(|dir| !dir.is_absolute())
}

fn env_value<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

/// 폴더는 부모 환경이 아니라 `launch.env`에서 정하고, 파일을 못 읽으면 값 없음으로 본다.
/// 작업 공간에 TOML 파서가 없어 키 존재만 줄 단위로 본다. 초안 키 목록.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    let Some(home) = user_folder(&launch.env) else {
        return UserProviderConfig::default();
    };
    let Ok(content) = std::fs::read_to_string(home.join("config.toml")) else {
        return UserProviderConfig::default();
    };
    scan_user_config(&content)
}

pub(super) fn scan_user_config(content: &str) -> UserProviderConfig {
    let mut profile = None;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if let Some(("profile", value)) = key_value(line) {
            profile = Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned());
        }
    }
    let selected = profile.map(|name| format!("profiles.{name}"));
    let mut section = String::new();
    let mut found = UserProviderConfig::default();
    for line in content.lines() {
        let line = line.trim();
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = header.trim().trim_matches('"').to_owned();
            continue;
        }
        let in_scope = section.is_empty() || selected.as_deref() == Some(section.as_str());
        let Some((key, _)) = key_value(line) else {
            continue;
        };
        if !in_scope {
            continue;
        }
        if key == AUTO_COMPACT_KEY {
            found.has_auto_compact = true;
        }
    }
    found
}

/// 주석과 빈 줄은 `None`.
pub(super) fn key_value(line: &str) -> Option<(&str, &str)> {
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim().trim_matches('"'), value.trim()))
}
