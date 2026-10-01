//! provider 연결 구현: 종류별 생성, 공통 실행 준비, 끼워 넣기 경로 선택, 명령 목록 거르기.
//!
//! 설계: docs/design/providers-and-sessions.md(provider 연결, provider 실행과 기본값 인자, provider 명령과 스킬 전달, 이벤트 수신과 변환).
//! provider 고유 이름(메서드, 필드, 실행 인자)은 `codex.rs`, `claude.rs` 안에서만 쓴다. 이 파일은 Saturn 용어만 쓴다.
//!
//! 실행 흐름(설계 문서 provider 실행과 기본값 인자 1~6단계):
//! 1. engine이 입력 접수 때 고정한 설정 번호로 `LaunchSpec`을 만든다.
//! 2. engine이 부모 환경을 `secrets::scrub`으로 걸러 `LaunchSpec::env`에 넣는다(첫 번째 제거). `Supervisor::spawn`이 실행 때 다시 거른다.
//! 3. 각 provider 파일의 `default_args`가 `UserProviderConfig`에 값이 없는 항목에만 Saturn 기본값 인자를 더한다.
//! 4. Claude는 `LaunchSpec::hook_settings`(`secrets::HookPolicy::pre_tool_use_settings` 값)를 실행별 설정으로 넘긴다.
//! 5. `Supervisor::spawn`이 실행하고 감시한다.
//! 6. 연결이 명령 목록을 모으고 `filter_commands`로 거른다.
//!
//! 이벤트 공통 규칙(두 provider 모두):
//! - provider 이벤트는 `saturn_protocol::event::ProviderEvent`로만 내보낸다. 고유 필드는 버린다.
//! - Saturn이 입력을 보내지 않았는데 시작된 턴은 `TurnOrigin::ProviderWake`, 보낸 입력으로 시작된 턴은 `TurnOrigin::User`(`TurnOriginTracker`).
//! - 완료 신호 없이 흐름이 끝남, 읽는 중 프로세스 종료, 끝 이벤트 없이 끊긴 subagent는 `ProviderEvent::StreamLost`를 한 번 내보낸다.
//! - 이벤트를 읽는 백그라운드 작업이 `tokio::sync::mpsc`로 넘기고 `next_event`는 그 채널에서 받는다.
//! - provider stderr와 오류 문구는 `LaunchSpec::masker`로 가린 뒤에만 `tracing`으로 남긴다.
//!
//! TODO(#34): 상시 연결을 열 수 없을 때 한 번 실행 경로(`exec`, `-p`)로 대체할지 미정. 정해지기 전에는 대체하지 않고 `ConnectionLost`
//! TODO(#38): 추론과 도구 실행이 오래 걸릴 때 타임아웃을 둘지와 값 미정. 정해지기 전에는 기다리기만 한다

mod claude;
mod codex;

use std::ffi::OsString;
use std::path::PathBuf;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{ProviderEvent, TurnOrigin};
use saturn_protocol::ids::{Provider, ProviderSessionId, SettingsRevision};

use crate::processes::{ProcessGroupId, Supervisor};
use crate::secrets::Masker;

pub use claude::ClaudeClient;
pub use codex::CodexClient;

/// 끼워 넣기를 대기로 바꿀 때 화면 문구.
pub const STEER_PENDING_NOTICE: &str = "바로 반영: 준비 중";

/// provider 실행 준비값. engine이 입력 접수 때 고정한 설정 번호로 만든다.
#[derive(Debug, Clone)]
pub struct LaunchSpec {
    /// provider 종류.
    pub provider: Provider,
    /// 실행 파일. 설정에 없으면 `PATH`의 기본 이름.
    pub program: PathBuf,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// 입력 접수 때 고정한 설정 번호.
    pub settings: SettingsRevision,
    /// 사용자 provider 설정에 값이 있는 항목. 있는 항목에는 Saturn 기본값을 넣지 않는다.
    pub user_config: UserProviderConfig,
    /// Saturn 기본값.
    pub defaults: SaturnDefaults,
    /// 자식 환경. 부모 환경을 `secrets::scrub`으로 거른 값. `ProcessSpec::env`로 그대로 넘긴다.
    pub env: Vec<(OsString, OsString)>,
    /// Saturn 소유 PreToolUse 훅을 담은 실행별 설정. `secrets::HookPolicy::pre_tool_use_settings` 값.
    /// Claude만 `--settings`로 넘기고 Codex는 무시한다. 사용자의 기존 훅은 감싸거나 지우지 않는다.
    pub hook_settings: Option<serde_json::Value>,
    /// provider stderr와 오류 문구를 로그에 남기기 전 가림.
    pub masker: Masker,
}

/// 사용자 provider 설정에 값이 있는지. 값 자체는 읽지 않고 있는지만 본다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserProviderConfig {
    /// 권한(승인, 샌드박스) 값이 있다.
    pub has_permission: bool,
    /// 자동 압축 값이 있다(끄기 포함).
    pub has_auto_compact: bool,
}

/// Saturn 기본값. 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. 사용자 설정 파일은 건드리지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaturnDefaults {
    /// 권한 기본값. 항상 수정 허용.
    pub allow_edits: bool,
    /// 자동 압축 안전망 `T_hard`(토큰). `saturn_core::sessions::context::ContextBudget::hard_limit` 값.
    pub auto_compact_tokens: u64,
}

/// provider가 실제 적용한 설정. provider 설정을 바꾸는 명령도 막지 않고, 이 값을 읽어 기록한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedSettings {
    /// 적용된 모델. 보고하지 않으면 `None`.
    pub model: Option<String>,
    /// 적용된 권한 방식(provider가 쓴 문자열 그대로). 보고하지 않으면 `None`.
    pub permission: Option<String>,
}

/// 끼워 넣기 경로.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerRoute {
    /// provider 끼워 넣기로 보낸다.
    Steer,
    /// 실측 전 provider라 대기로 바꾼다. 화면에 `STEER_PENDING_NOTICE`를 보인다.
    Queue,
}

/// 종류별 provider 연결. `ProviderClient`는 `impl Future`를 돌려 dyn으로 못 쓰므로 enum으로 나눈다.
#[derive(Debug)]
pub enum ProviderConnection {
    /// Codex app-server 연결.
    Codex(CodexClient),
    /// Claude Code stream-json 연결.
    Claude(ClaudeClient),
}

impl ProviderConnection {
    /// `launch.provider`에 맞는 연결을 만든다. Codex는 app-server를 바로 띄우고, Claude는 session을 열 때 띄운다.
    ///
    /// # Errors
    /// 실행 실패나 초기화 응답 없음은 `ConnectionLost`.
    pub async fn connect(
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> Result<Self, ProviderError> {
        match launch.provider {
            Provider::Codex => Ok(Self::Codex(CodexClient::start(launch, supervisor).await?)),
            Provider::Claude => Ok(Self::Claude(ClaudeClient::new(launch, supervisor))),
        }
    }

    /// provider 종류.
    pub fn provider(&self) -> Provider {
        match self {
            Self::Codex(_) => Provider::Codex,
            Self::Claude(_) => Provider::Claude,
        }
    }

    /// session의 프로세스 묶음. 트리 전체 중지에서 멈춤 신호 뒤 `Supervisor::stop_tree`에 넘긴다.
    /// Codex는 모든 session이 app-server 묶음 하나를 같이 쓴다.
    pub fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        match self {
            Self::Codex(client) => Some(client.process_group()),
            Self::Claude(client) => client.process_group(session),
        }
    }

    /// session에 provider가 적용한 설정. 적용값을 받기 전이면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        match self {
            Self::Codex(client) => client.applied_settings(session),
            Self::Claude(client) => client.applied_settings(session),
        }
    }
}

impl ProviderClient for ProviderConnection {
    /// 종류별 연결에 그대로 넘긴다.
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        match self {
            Self::Codex(client) => client.open_session(spec).await,
            Self::Claude(client) => client.open_session(spec).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.send_turn(session, text).await,
            Self::Claude(client) => client.send_turn(session, text).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다. 호출 전에 `steer_route`로 경로를 고른다.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.steer(session, text).await,
            Self::Claude(client) => client.steer(session, text).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.interrupt(session, target).await,
            Self::Claude(client) => client.interrupt(session, target).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.compact(session).await,
            Self::Claude(client) => client.compact(session).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.close_session(session).await,
            Self::Claude(client) => client.close_session(session).await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        match self {
            Self::Codex(client) => client.next_event().await,
            Self::Claude(client) => client.next_event().await,
        }
    }

    /// 종류별 연결에 그대로 넘긴다.
    fn commands(&self) -> Vec<ProviderCommand> {
        match self {
            Self::Codex(client) => client.commands(),
            Self::Claude(client) => client.commands(),
        }
    }
}

/// 턴 시작 주체 판정. session마다 하나 둔다.
#[derive(Debug, Default)]
pub(crate) struct TurnOriginTracker {
    /// Saturn이 보냈고 아직 턴 시작으로 이어지지 않은 입력 수.
    pending_user_turns: u32,
}

impl TurnOriginTracker {
    /// Saturn이 새 턴 입력을 보냈다. 끼워 넣기는 부르지 않는다.
    pub(crate) fn on_user_send(&mut self) {
        self.pending_user_turns = self.pending_user_turns.saturating_add(1);
    }

    /// 보내려던 새 턴 입력이 보내기 전에 실패했다. `on_user_send`로 센 것을 되돌린다.
    pub(crate) fn cancel_user_send(&mut self) {
        self.pending_user_turns = self.pending_user_turns.saturating_sub(1);
    }

    /// provider가 턴을 시작했다. 보낸 입력이 남아 있으면 하나 줄이고 `User`, 없으면 `ProviderWake`.
    pub(crate) fn on_turn_started(&mut self) -> TurnOrigin {
        if self.pending_user_turns == 0 {
            return TurnOrigin::ProviderWake;
        }
        self.pending_user_turns -= 1;
        TurnOrigin::User
    }
}

/// 끼워 넣기 경로를 고른다. `handle.steer_verified`가 거짓(끼워 넣기 실측 #5, #27 통과 전)이면 `Queue`.
pub fn steer_route(handle: &SessionHandle) -> SteerRoute {
    if handle.steer_verified {
        SteerRoute::Steer
    } else {
        SteerRoute::Queue
    }
}

/// 명령 목록에서 `excluded`(화면 전용 명령, Saturn session 명령이 대신하는 명령)를 뺀다. 이름 비교는 `/` 없이 대소문자 그대로.
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
            provider_session: ProviderSessionId("s".to_owned()),
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
