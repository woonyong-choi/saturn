use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    ASK_TOOLS, ASK_USER_QUESTION_TOOL, AUTO_COMPACT_ENV, AUTO_COMPACT_FLAG, AUTO_COMPACT_MAX,
    AUTO_COMPACT_MIN, DISALLOWED_TOOLS_FLAG, PERMISSION_PROMPT_ARGS,
};
use crate::providers::{LaunchSpec, UserProviderConfig};

/// 권한은 Saturn 규칙이 정하므로 `--permission-mode` 기본값은 넣지 않고, 승인 요청은 항상 호스트로 받는다.
pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    let mut args: Vec<String> = PERMISSION_PROMPT_ARGS
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect();
    if launch.permission.questions_disabled {
        args.push(DISALLOWED_TOOLS_FLAG.to_owned());
        args.push(ASK_USER_QUESTION_TOOL.to_owned());
    }
    if !user.has_auto_compact
        && let Some(tokens) = launch.defaults.auto_compact_tokens
    {
        let tokens = tokens.clamp(AUTO_COMPACT_MIN, AUTO_COMPACT_MAX);
        args.push(AUTO_COMPACT_FLAG.to_owned());
        args.push(tokens.to_string());
    }
    args.extend(launch.permission.extra_args.iter().cloned());
    args
}

/// 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있어도 자동 압축 값이 있는 것으로 본다.
/// 사용자 폴더와 환경 변수는 부모 환경이 아니라 `launch.env`에서 읽고, 파일을 못 읽으면 값 없음. 초안 목록.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    let env = |name: &str| {
        launch
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let mut found = UserProviderConfig {
        has_auto_compact: AUTO_COMPACT_ENV.iter().any(|name| env(name).is_some()),
    };
    for file in settings_files(launch) {
        let Some(settings) = read_json(&file) else {
            continue;
        };
        found.has_auto_compact |= !settings["autoCompactEnabled"].is_null();
    }
    found
}

/// Claude가 사용자 설정을 읽는 폴더와 MCP 서버가 든 상태 파일. 설정 탐색, 확장 탐색, 제외 명령 검사가 모두 이 값을 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UserFolder {
    /// `settings.json`, `skills/`, `commands/`, `plugins/`가 든 폴더.
    pub(super) dir: PathBuf,
    /// 사용자 범위 MCP 서버가 든 파일. `CLAUDE_CONFIG_DIR`이 있으면 그 폴더 안, 없으면 홈 바로 아래다.
    pub(super) state_file: PathBuf,
}

/// 환경 `CLAUDE_CONFIG_DIR`이 비어 있지 않으면 그 폴더, 아니면 `HOME` 아래 `.claude`. 환경은 부모 환경이 아니라 provider가
/// 받는 값(`launch.env`)이다. 빈 값은 없는 것으로 본다. 심볼릭 링크는 Claude처럼 따라 읽고, 폴더가 없거나 읽을 수 없으면
/// 그 층의 파일이 없는 것으로 본다. 상대 경로는 정하지 못하므로(채팅마다 provider 작업 폴더가 다르다) `None`이고,
/// session을 열기 전에 `relative_config_dir`로 거절한다. 변수와 `HOME`이 모두 없어도 `None`.
pub(super) fn user_folder(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> Option<UserFolder> {
    if let Some(custom) = env_value(env, "CLAUDE_CONFIG_DIR") {
        let dir = PathBuf::from(custom);
        if !dir.is_absolute() {
            return None;
        }
        let state_file = dir.join(".claude.json");
        return Some(UserFolder { dir, state_file });
    }
    let home = Path::new(env_value(env, "HOME")?);
    Some(UserFolder {
        dir: home.join(".claude"),
        state_file: home.join(".claude.json"),
    })
}

/// `CLAUDE_CONFIG_DIR`이 상대 경로이면 그 값. 이 경우 `user_folder`가 폴더를 정하지 못한다.
pub(crate) fn relative_config_dir(
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Option<PathBuf> {
    env_value(env, "CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.is_absolute())
}

fn env_value<'a>(
    env: &'a [(std::ffi::OsString, std::ffi::OsString)],
    name: &str,
) -> Option<&'a std::ffi::OsString> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
}

/// 사용자 Claude 설정 층 파일: 사용자, 프로젝트, 프로젝트 로컬.
fn settings_files(launch: &LaunchSpec) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(user) = user_folder(&launch.env) {
        files.push(user.dir.join("settings.json"));
    }
    let project = launch.workdir.join(".claude");
    files.push(project.join("settings.json"));
    files.push(project.join("settings.local.json"));
    files
}

/// `sandbox.excludedCommands`가 비어 있지 않은 사용자 설정 파일. 제외된 명령은 샌드박스 밖에서 돌아 키 저장소 읽기 금지가
/// 닿지 않는다. 배열은 설정 층끼리 합쳐져 실행별 설정으로 비울 수 없으므로 session을 열기 전에 찾아 거절하는 데 쓴다.
/// 어느 층의 값이든 세고(프로젝트 층이 적용되지 않는 것으로 관측됐어도 보장하지 않는다), 목록의 값은 읽지 않고 파일만 돌려준다.
pub(crate) fn sandbox_exclusions(launch: &LaunchSpec) -> Vec<PathBuf> {
    settings_files(launch)
        .into_iter()
        .filter(|file| {
            read_json(file).is_some_and(|settings| {
                settings["sandbox"]["excludedCommands"]
                    .as_array()
                    .is_some_and(|commands| !commands.is_empty())
            })
        })
        .collect()
}

/// `--settings`로 넘기는 값에 `permissions.ask`로 규칙 대상 도구를 나열한다. 훅 설정은 그대로 두고 합친다.
pub(crate) fn with_ask_tools(mut settings: Value) -> Value {
    settings["permissions"]["ask"] = json!(ASK_TOOLS);
    settings
}

/// `--settings`로 넘기는 값에 읽기 `deny` 규칙을 번역해 넣는다. `permissions.deny`의 `Read(...)` 규칙은 읽기 도구(Read, Glob,
/// Grep, LS)의 호출을 요청 전에 막고, 같은 glob을 Bash 샌드박스의 `filesystem.denyRead`에 더해 `cat`, 파이프, 스크립트가
/// 만드는 하위 프로세스의 읽기도 막는다. 폴더 안과 밖, 링크 모두 같은 규칙을 받는다. 배열은 설정 층끼리 합쳐지므로
/// 사용자 설정의 `deny`는 남고, 사용자 설정 파일은 고치지 않는다. 번역할 규칙이 없으면 아무것도 바꾸지 않는다.
pub(crate) fn with_read_deny(mut settings: Value, globs: &[String]) -> Value {
    if globs.is_empty() {
        return settings;
    }
    let rules: Vec<String> = globs.iter().map(|glob| format!("Read(/{glob})")).collect();
    settings["permissions"]["deny"] = json!(rules);
    settings
}

/// `--settings`로 넘기는 값에 Bash 샌드박스를 켜고 키 저장소 경로의 읽기를 막는다. 권한 모드와 무관하게 늘 넣는다.
/// 샌드박스를 켜면 Claude Code가 Bash를 허가 요청 없이 자동 허용하므로(`autoAllowBashIfSandboxed` 기본값) 끄고,
/// 모든 Bash 호출이 `permissions.ask`로 호스트에 와 Saturn 규칙이 판정하게 한다.
/// 샌드박스 밖 실행과 샌드박스 없이 시작하는 일을 막고, 작업 폴더와 더한 폴더의 쓰기는 Claude 기본 허용에 맡긴다.
/// 배열은 설정 층끼리 합쳐지므로 사용자 설정의 `denyRead`는 남는다. 사용자 설정 파일은 고치지 않는다.
/// 샌드박스는 Unix 소켓 접속도 막으므로 `engine_socket`이 있으면 그 소켓의 표기 경로와 실제 경로만 허용한다. 에이전트 작업 안의 `saturn`이
/// 출입증으로 engine에 붙는 길(`saturn evidence`, 하위 접속)이고, 다른 소켓은 계속 막는다.
pub(crate) fn with_key_sandbox(
    mut settings: Value,
    deny_read: &[PathBuf],
    engine_socket: Option<&Path>,
) -> Value {
    let paths: Vec<String> = deny_read
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    settings["sandbox"] = json!({
        "enabled": true,
        "allowUnsandboxedCommands": false,
        "autoAllowBashIfSandboxed": false,
        "failIfUnavailable": true,
        "filesystem": { "denyRead": paths },
    });
    if let Some(socket) = engine_socket {
        let mut sockets = vec![socket.to_string_lossy().into_owned()];
        if let Ok(canonical) = socket.canonicalize() {
            let canonical = canonical.to_string_lossy().into_owned();
            if canonical != sockets[0] {
                sockets.push(canonical);
            }
        }
        settings["sandbox"]["network"] = json!({ "allowUnixSockets": sockets });
    }
    settings
}

/// 없거나 깨졌으면 `None`.
pub(super) fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
