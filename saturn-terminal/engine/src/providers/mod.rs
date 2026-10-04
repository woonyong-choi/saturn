//! provider 어댑터 계층: 계약과 설명자, 레지스트리, 공통 실행 값, 끼워 넣기 경로 선택, 명령 목록 거르기.
//! 설계: docs/design/providers-and-sessions.md

mod adapter;
mod builtin;
mod claude;
mod codex;
mod registry;
#[cfg(test)]
pub(crate) mod test_support;
mod tool_detail;
mod worker;

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use saturn_core::providers::{ProviderCommand, SessionHandle};
use saturn_protocol::event::TurnOrigin;
use saturn_protocol::ids::{Provider, SettingsRevision};
use serde_json::Value;

use crate::secrets::Masker;

pub use adapter::ProviderConnection;
pub(crate) use adapter::{
    Adapter, AdapterConnection, AppliedReader, BoxFuture, ContextDefaults, Descriptor, Feature,
    INTERFACE_VERSION, PermissionInput,
};
pub use builtin::{HookInputError, run_pre_tool_use};
pub use registry::Registry;
pub(crate) use worker::{CallResult, Connected, ProviderHandle, ProviderMsg, Reply, spawn_connect};

/// 바로 돌아와야 하는 provider 요청(`turn/start`, `turn/steer`, interrupt)의 응답을 기다리는 최대 시간. 넘으면 그 요청만
/// 응답 없음으로 돌려주고 연결과 턴은 끊지 않는다. 응답은 요청을 받았다는 확인이지 턴 실행 시간이 아니라 평소에는 금방 온다.
/// engine은 요청 처리 루프 하나라 이 시간은 한 채팅의 정지가 다른 채팅과 멈춤 요청을 늦추는 최대 시간이기도 하다.
/// 멈춤 신호 뒤 프로세스 묶음 중지까지의 유예(10초)와 맞춘 초안 값이다.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// 시작·열기 요청(`initialize`, `thread/start`, `thread/resume`, `mcpServerStatus/list`와 목록 조회)의 응답을 기다리는
/// 최대 시간. 사용자 MCP 서버 7개 설정에서 `initialize` 0.3~0.6초, `thread/start` 0.2~0.5초, `mcpServerStatus/list`
/// 호출 하나 1.7~4.9초가 걸렸다(codex-cli 0.158.0, 호출은 서버 시작이 끝나지 않은 동안 느려진다). 첫 턴 전 MCP 준비
/// 대기가 30초이고 그 대기의 마지막 호출이 뒤따르므로 측정 최댓값의 열 배, 준비 대기의 두 배인 값으로 둔다. 초안 값.
pub(crate) const OPEN_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// engine이 입력 접수 때 고정한 설정 번호로 만든다.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub provider: Provider,
    /// 설정에 없으면 `PATH`의 기본 이름.
    pub program: PathBuf,
    pub workdir: PathBuf,
    pub settings: SettingsRevision,
    /// 있는 항목에는 Saturn 기본값을 넣지 않는다.
    pub user_config: UserProviderConfig,
    pub defaults: SaturnDefaults,
    /// 부모 환경을 `secrets::scrub`으로 거른 값.
    pub env: Vec<(OsString, OsString)>,
    /// Saturn 소유 PreToolUse 훅 설정. 훅을 받는 어댑터만 쓰고 나머지는 무시한다.
    pub hook_settings: Option<serde_json::Value>,
    /// Saturn 규칙을 번역한 provider 실행 설정.
    pub permission: PermissionLaunch,
    /// provider stderr와 오류 문구를 로그에 남기기 전에 가린다.
    pub masker: Masker,
}

/// Saturn 권한 규칙을 provider 실행 설정으로 번역한 결과. 어댑터가 번역해 채운다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionLaunch {
    /// 어댑터가 provider 프로세스 환경에 덮어쓸 변수. 비면 `LaunchSpec.env`를 그대로 쓴다.
    pub env: Vec<(OsString, OsString)>,
    /// 번역한 규칙의 지문. 규칙이 연결을 시작할 때 고정되는 어댑터만 채운다. 설정이 바뀌어 지문이 달라지면 연결을
    /// 다시 시작한다.
    pub rules_fingerprint: Option<String>,
    /// 첫 턴 전에 준비를 확인할 MCP 서버. 비면 확인하지 않는다.
    pub mcp_servers: Vec<String>,
    /// 권한 모드 `full`이라 에이전트 질문 기능을 뺀다. 기본(거짓)은 묻는다. 적용 방식은 어댑터가 정한다.
    pub questions_disabled: bool,
}

/// 값 자체는 읽지 않고 있는지만 본다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserProviderConfig {
    /// 끄기 포함.
    pub has_auto_compact: bool,
}

/// 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘기고, 사용자 설정 파일은 건드리지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaturnDefaults {
    /// `T_hard`(토큰). `ContextBudget::hard_limit` 값. `context.mode`가 `provider`면 `None`이고 안전망 값을 넣지 않는다.
    pub auto_compact_tokens: Option<u64>,
}

/// provider 설정을 바꾸는 명령도 막지 않고 이 값을 읽어 기록한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AppliedSettings {
    /// 보고하지 않으면 `None`.
    pub model: Option<String>,
    /// provider가 쓴 문자열 그대로. 보고하지 않으면 `None`.
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SteerRoute {
    Steer,
    /// 실측 전 provider라 대기로 바꾼다.
    Queue,
}

/// session마다 하나 둔다.
#[derive(Debug, Default)]
pub(crate) struct TurnOriginTracker {
    /// Saturn이 보냈고 아직 턴 시작으로 이어지지 않은 입력 수.
    pending_user_turns: u32,
}

impl TurnOriginTracker {
    /// 끼워 넣기는 부르지 않는다.
    pub(crate) fn on_user_send(&mut self) {
        self.pending_user_turns = self.pending_user_turns.saturating_add(1);
    }

    /// 보내기 전에 실패한 새 턴 입력을 되돌린다.
    pub(crate) fn cancel_user_send(&mut self) {
        self.pending_user_turns = self.pending_user_turns.saturating_sub(1);
    }

    /// 보낸 입력이 남아 있으면 `User`, 없으면 `ProviderWake`.
    pub(crate) fn on_turn_started(&mut self) -> TurnOrigin {
        if self.pending_user_turns == 0 {
            return TurnOrigin::ProviderWake;
        }
        self.pending_user_turns -= 1;
        TurnOrigin::User
    }
}

/// JSON 값 안의 모든 글자에서 router 키를 가린다.
pub(crate) fn mask_values(value: &mut Value, masker: &Masker) {
    match value {
        Value::String(text) => *text = masker.mask(text).as_str().to_owned(),
        Value::Array(items) => {
            for item in items {
                mask_values(item, masker);
            }
        }
        Value::Object(fields) => {
            for item in fields.values_mut() {
                mask_values(item, masker);
            }
        }
        _ => {}
    }
}

/// `handle.steer_verified`가 거짓(끼워 넣기 실측 #5, #27 통과 전)이면 `Queue`.
pub(crate) fn steer_route(handle: &SessionHandle) -> SteerRoute {
    if handle.steer_verified {
        SteerRoute::Steer
    } else {
        SteerRoute::Queue
    }
}

/// provider 설정을 바꾸는 명령(모델, 권한 등)은 빼지 않는다.
/// TODO(#41): 메인이 아닌 provider의 명령을 골랐을 때 처리 미정. 지금은 연결마다 제 목록만 돌려준다
pub(crate) fn filter_commands(
    all: Vec<ProviderCommand>,
    excluded: &[&str],
) -> Vec<ProviderCommand> {
    all.into_iter()
        .filter(|command| !excluded.contains(&command.name.trim_start_matches('/')))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str) -> ProviderCommand {
        ProviderCommand {
            name: name.to_owned(),
            description: String::new(),
            is_skill: false,
        }
    }

    #[test]
    fn origin_counts_sent_turns() {
        let mut tracker = TurnOriginTracker::default();

        assert_eq!(tracker.on_turn_started(), TurnOrigin::ProviderWake);
        tracker.on_user_send();
        tracker.on_user_send();
        tracker.cancel_user_send();

        assert_eq!(tracker.on_turn_started(), TurnOrigin::User);
        assert_eq!(tracker.on_turn_started(), TurnOrigin::ProviderWake);
    }

    #[test]
    fn unverified_steer_goes_to_queue() {
        let handle = |steer_verified| SessionHandle {
            provider_session: saturn_protocol::ids::ProviderSessionId("s".to_owned()),
            steer_verified,
        };

        assert_eq!(steer_route(&handle(false)), SteerRoute::Queue);
        assert_eq!(steer_route(&handle(true)), SteerRoute::Steer);
    }

    #[test]
    fn filter_drops_only_excluded_names() {
        let all = vec![
            command("compact"),
            command("resume"),
            command("model"),
            command("Resume"),
        ];

        let names: Vec<String> = filter_commands(all, &["resume"])
            .into_iter()
            .map(|command| command.name)
            .collect();

        assert_eq!(names, vec!["compact", "model", "Resume"]);
    }
}
