//! provider 연결 구현: 종류별 생성, 공통 실행 준비, 끼워 넣기 경로 선택, 명령 목록 거르기.
//! 설계: docs/design/providers-and-sessions.md
//! TODO(#34): 상시 연결을 열 수 없을 때 한 번 실행 경로로 대체할지. 정해지기 전에는 `ConnectionLost`
//! TODO(#38): 추론과 도구 실행이 오래 걸릴 때 타임아웃을 둘지와 값. 정해지기 전에는 기다리기만 한다

mod claude;
mod codex;
mod codex_home;
mod codex_permission;
#[cfg(test)]
pub(crate) mod test_support;
mod tool_detail;

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{ProviderEvent, TurnOrigin};
use saturn_protocol::ids::{Provider, ProviderSessionId, SettingsRevision};
use saturn_protocol::rpc::PermissionAnswer;

use crate::processes::{ProcessGroupId, Supervisor};
use crate::secrets::Masker;

pub use claude::ClaudeClient;
pub use codex::CodexClient;
pub use codex_home::{
    HomeError, HomeInput, PreparedHome, prepare as prepare_codex_home, rules_fingerprint,
};

pub const STEER_PENDING_NOTICE: &str = "바로 반영: 준비 중";

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
    /// Claude만 `--settings`로 넘기고 Codex는 무시한다.
    pub hook_settings: Option<serde_json::Value>,
    /// Saturn 규칙을 번역한 provider 실행 설정.
    pub permission: PermissionLaunch,
    /// provider stderr와 오류 문구를 로그에 남기기 전에 가린다.
    pub masker: Masker,
}

/// Saturn 권한 규칙을 provider 실행 설정으로 번역한 결과. Claude의 `ask` 목록은 `hook_settings`에 합쳐 넘긴다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PermissionLaunch {
    /// Codex 전용 `CODEX_HOME`. 없으면 환경의 값을 그대로 쓴다.
    pub codex_home: Option<PathBuf>,
    /// 첫 턴 전에 준비를 확인할 Codex MCP 서버. 비면 확인하지 않는다.
    pub mcp_servers: Vec<String>,
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
    /// `T_hard`(토큰). `ContextBudget::hard_limit` 값.
    pub auto_compact_tokens: u64,
}

/// provider 설정을 바꾸는 명령도 막지 않고 이 값을 읽어 기록한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedSettings {
    /// 보고하지 않으면 `None`.
    pub model: Option<String>,
    /// provider가 쓴 문자열 그대로. 보고하지 않으면 `None`.
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerRoute {
    Steer,
    /// 실측 전 provider라 대기로 바꾼다.
    Queue,
}

/// `ProviderClient`는 `impl Future`를 돌려 dyn으로 못 쓰므로 enum으로 나눈다.
#[derive(Debug)]
pub enum ProviderConnection {
    Codex(CodexClient),
    Claude(ClaudeClient),
    /// 입력 흐름 테스트가 실제 provider 없이 보낸 내용과 답을 정한다.
    #[cfg(test)]
    #[allow(private_interfaces)]
    Fake(test_support::FakeProvider),
}

impl ProviderConnection {
    /// Codex는 app-server를 바로 띄우고, Claude는 session을 열 때 띄운다.
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

    pub fn provider(&self) -> Provider {
        match self {
            Self::Codex(_) => Provider::Codex,
            Self::Claude(_) => Provider::Claude,
            #[cfg(test)]
            Self::Fake(client) => client.provider(),
        }
    }

    /// Codex는 모든 session이 app-server 묶음 하나를 같이 쓴다.
    pub fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        match self {
            Self::Codex(client) => Some(client.process_group()),
            Self::Claude(client) => client.process_group(session),
            #[cfg(test)]
            Self::Fake(client) => client.group(),
        }
    }

    /// 모든 session이 프로세스 묶음 하나를 같이 쓰는 provider의 그 묶음. 연결을 통째로 닫을 때 쓴다.
    pub fn shared_group(&self) -> Option<ProcessGroupId> {
        match self {
            Self::Codex(client) => Some(client.process_group()),
            Self::Claude(_) => None,
            #[cfg(test)]
            Self::Fake(client) => client.group(),
        }
    }

    /// 적용값을 받기 전이면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        match self {
            Self::Codex(client) => client.applied_settings(session),
            Self::Claude(client) => client.applied_settings(session),
            #[cfg(test)]
            Self::Fake(_) => None,
        }
    }
}

impl ProviderClient for ProviderConnection {
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        match self {
            Self::Codex(client) => client.open_session(spec).await,
            Self::Claude(client) => client.open_session(spec).await,
            #[cfg(test)]
            Self::Fake(client) => client.open_session(spec).await,
        }
    }

    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.send_turn(session, text).await,
            Self::Claude(client) => client.send_turn(session, text).await,
            #[cfg(test)]
            Self::Fake(client) => client.send_turn(session, text).await,
        }
    }

    /// 호출 전에 `steer_route`로 경로를 고른다.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.steer(session, text).await,
            Self::Claude(client) => client.steer(session, text).await,
            #[cfg(test)]
            Self::Fake(client) => client.steer(session, text).await,
        }
    }

    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.interrupt(session, target).await,
            Self::Claude(client) => client.interrupt(session, target).await,
            #[cfg(test)]
            Self::Fake(client) => client.interrupt(session, target).await,
        }
    }

    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.compact(session).await,
            Self::Claude(client) => client.compact(session).await,
            #[cfg(test)]
            Self::Fake(client) => client.compact(session).await,
        }
    }

    async fn answer_permission(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.answer_permission(session, request_id, answer).await,
            Self::Claude(client) => client.answer_permission(session, request_id, answer).await,
            #[cfg(test)]
            Self::Fake(client) => client.answer_permission(session, request_id, answer).await,
        }
    }

    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.close_session(session).await,
            Self::Claude(client) => client.close_session(session).await,
            #[cfg(test)]
            Self::Fake(client) => client.close_session(session).await,
        }
    }

    async fn next_event(&mut self) -> Option<ProviderEvent> {
        match self {
            Self::Codex(client) => client.next_event().await,
            Self::Claude(client) => client.next_event().await,
            #[cfg(test)]
            Self::Fake(client) => client.next_event().await,
        }
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        match self {
            Self::Codex(client) => client.commands(),
            Self::Claude(client) => client.commands(),
            #[cfg(test)]
            Self::Fake(client) => client.commands(),
        }
    }
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

/// 고정 모델도 현재 provider도 없는 첫 입력을 설치된 앞쪽 provider로 보낸다([#168](https://github.com/woonyong-choi/saturn/issues/168) 결정).
pub const FIRST_INPUT_ORDER: [Provider; 2] = [Provider::Claude, Provider::Codex];

/// 설정에 실행 파일 경로가 없을 때 `PATH`에서 찾는 이름.
pub fn program_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => codex::PROGRAM,
        Provider::Claude => claude::PROGRAM,
    }
}

/// `env`의 `PATH`에서 실행 권한이 있는 파일을 찾는다. `PATH`가 없으면 없는 것으로 본다.
pub fn is_installed(provider: Provider, env: &[(OsString, OsString)]) -> bool {
    let Some((_, path)) = env.iter().find(|(name, _)| name == "PATH") else {
        return false;
    };
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program_name(provider)))
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}

/// 사용량 화면처럼 사용자에게 provider를 이름으로 보일 때 쓴다.
pub fn display_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => codex::DISPLAY_NAME,
        Provider::Claude => claude::DISPLAY_NAME,
    }
}

/// `handle.steer_verified`가 거짓(끼워 넣기 실측 #5, #27 통과 전)이면 `Queue`.
pub fn steer_route(handle: &SessionHandle) -> SteerRoute {
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
            provider_session: ProviderSessionId("s".to_owned()),
            steer_verified,
        };

        assert_eq!(steer_route(&handle(false)), SteerRoute::Queue);
        assert_eq!(steer_route(&handle(true)), SteerRoute::Steer);
    }

    #[test]
    fn installed_means_executable_file_on_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(program_name(Provider::Claude));
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        let path =
            |dir: &std::path::Path| vec![(OsString::from("PATH"), dir.as_os_str().to_owned())];

        let plain_file = is_installed(Provider::Claude, &path(dir.path()));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let executable = is_installed(Provider::Claude, &path(dir.path()));

        assert!(!plain_file);
        assert!(executable);
        assert!(!is_installed(Provider::Codex, &path(dir.path())));
        assert!(!is_installed(Provider::Claude, &[]));
    }

    #[test]
    fn first_input_prefers_claude_over_codex() {
        assert_eq!(FIRST_INPUT_ORDER, [Provider::Claude, Provider::Codex]);
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
