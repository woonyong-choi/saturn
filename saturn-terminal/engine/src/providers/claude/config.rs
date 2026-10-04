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
/// `HOME`과 환경 변수는 부모 환경이 아니라 `launch.env`에서 읽고, 파일을 못 읽으면 값 없음. 초안 목록.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    let env = |name: &str| {
        launch
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let mut files = Vec::new();
    if let Some(home) = env("HOME") {
        files.push(Path::new(home).join(".claude").join("settings.json"));
    }
    let project = launch.workdir.join(".claude");
    files.push(project.join("settings.json"));
    files.push(project.join("settings.local.json"));
    let mut found = UserProviderConfig {
        has_auto_compact: AUTO_COMPACT_ENV.iter().any(|name| env(name).is_some()),
    };
    for file in files {
        let Some(settings) = read_json(&file) else {
            continue;
        };
        found.has_auto_compact |= !settings["autoCompactEnabled"].is_null();
    }
    found
}

/// `--settings`로 넘기는 값에 `permissions.ask`로 규칙 대상 도구를 나열한다. 훅 설정은 그대로 두고 합친다.
pub(crate) fn with_ask_tools(mut settings: Value) -> Value {
    settings["permissions"]["ask"] = json!(ASK_TOOLS);
    settings
}

/// `--settings`로 넘기는 값에 Bash 샌드박스를 켜고 키 저장소 경로의 읽기를 막는다. 권한 모드와 무관하게 늘 넣는다.
/// 샌드박스 밖 실행과 샌드박스 없이 시작하는 일을 막고, 작업 폴더와 더한 폴더의 쓰기는 Claude 기본 허용에 맡긴다.
/// 배열은 설정 층끼리 합쳐지므로 사용자 설정의 `denyRead`는 남는다. 사용자 설정 파일은 고치지 않는다.
pub(crate) fn with_key_sandbox(mut settings: Value, deny_read: &[PathBuf]) -> Value {
    let paths: Vec<String> = deny_read
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    settings["sandbox"] = json!({
        "enabled": true,
        "allowUnsandboxedCommands": false,
        "failIfUnavailable": true,
        "filesystem": { "denyRead": paths },
    });
    settings
}

/// 없거나 깨졌으면 `None`.
pub(super) fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
