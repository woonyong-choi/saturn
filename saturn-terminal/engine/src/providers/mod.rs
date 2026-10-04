//! provider 연결 구현: 종류별 생성, 공통 실행 준비, 끼워 넣기 경로 선택, 명령 목록 거르기.
//! 설계: docs/design/providers-and-sessions.md

mod claude;
mod claude_input;
mod codex;
mod codex_home;
mod codex_input;
mod codex_permission;
#[cfg(test)]
pub(crate) mod test_support;
mod tool_detail;
mod worker;

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{ProviderEvent, TurnOrigin};
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId, SettingsRevision};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};

use crate::processes::{ProcessGroupId, Supervisor};
use crate::secrets::Masker;

/// 어댑터 설명자가 생기기 전까지 이름으로 가르는 자리가 쓰는 provider id.
pub(crate) const CODEX: Provider = codex::ID;
pub(crate) const CLAUDE: Provider = claude::ID;

pub(crate) use claude::ClaudeClient;
pub use claude::{HookInputError, run_pre_tool_use};
pub(crate) use codex::CodexClient;
/// 응답을 보내지 않을 수도 있는 가짜 app-server를 띄우는 실행 설정.
#[cfg(test)]
pub(crate) use codex::tests::launch as fake_codex_launch;
pub(crate) use codex_home::{
    HomeInput, prepare as prepare_codex_home, rules_fingerprint, rules_of_home,
};
pub(crate) use worker::{Connected, ProviderHandle, ProviderMsg, Reply, spawn_connect};

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
    /// 권한 모드 `full`이라 에이전트 질문 기능을 뺀다. 기본(거짓)은 묻는다. Codex는 `CODEX_HOME` 생성 설정으로,
    /// Claude는 `--disallowedTools AskUserQuestion`으로 적용한다.
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
pub struct AppliedSettings {
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
            CODEX => Ok(Self::Codex(CodexClient::start(launch, supervisor).await?)),
            CLAUDE => Ok(Self::Claude(ClaudeClient::new(launch, supervisor))),
            _ => Err(ProviderError::ConnectionLost),
        }
    }

    pub fn provider(&self) -> Provider {
        match self {
            Self::Codex(_) => CODEX,
            Self::Claude(_) => CLAUDE,
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

    /// 적용값을 부를 때마다 읽는 함수. 연결 작업 밖에서 동기로 읽으려고 session을 열 때 받아 둔다.
    pub(crate) fn applied_reader(
        &self,
        session: &ProviderSessionId,
    ) -> Option<worker::AppliedReader> {
        match self {
            Self::Codex(client) => Some(client.applied_reader(session)),
            Self::Claude(client) => client.applied_reader(session),
            #[cfg(test)]
            Self::Fake(_) => None,
        }
    }

    /// 턴 완료를 처리한 뒤 그 에이전트에 줄 세워 둔 첫 입력을 보낸다. 줄 세우지 않는 연결은 아무것도 하지 않는다.
    /// 응답을 기다리므로 `select` 가지 안에서 부르지 말고 가지 본문에서 끝까지 기다린다(#324).
    pub(crate) async fn start_queued_turn(&mut self, agent: AgentId) {
        match self {
            Self::Codex(client) => client.start_queued_turn(agent).await,
            Self::Claude(client) => client.start_queued_turn(agent).await,
            #[cfg(test)]
            Self::Fake(_) => {}
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

    async fn answer_input(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        match self {
            Self::Codex(client) => client.answer_input(session, request_id, answer).await,
            Self::Claude(client) => client.answer_input(session, request_id, answer).await,
            #[cfg(test)]
            Self::Fake(client) => client.answer_input(session, request_id, answer).await,
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

    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        match self {
            Self::Codex(client) => client.list_models().await,
            Self::Claude(client) => client.list_models().await,
            #[cfg(test)]
            Self::Fake(client) => client.list_models().await,
        }
    }

    /// 취소해도 이벤트를 잃지 않는다. 줄 선 입력은 보내지 않으므로 완료를 처리한 뒤 `start_queued_turn`으로 보낸다.
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
pub(crate) const FIRST_INPUT_ORDER: [Provider; 2] = [CLAUDE, CODEX];

/// 설정에 실행 파일 경로가 없을 때 `PATH`에서 찾는 이름.
/// 모르는 provider면 `None`.
pub(crate) fn program_name(provider: Provider) -> Option<&'static str> {
    match provider {
        CODEX => Some(codex::PROGRAM),
        CLAUDE => Some(claude::PROGRAM),
        _ => None,
    }
}

/// `env`의 `PATH`에서 실행 권한이 있는 파일을 찾는다. `PATH`가 없으면 없는 것으로 본다.
pub(crate) fn is_installed(provider: Provider, env: &[(OsString, OsString)]) -> bool {
    let Some((_, path)) = env.iter().find(|(name, _)| name == "PATH") else {
        return false;
    };
    let Some(program) = program_name(provider) else {
        return false;
    };
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}

/// 사용량 화면처럼 사용자에게 provider를 이름으로 보일 때 쓴다.
pub(crate) fn display_name(provider: Provider) -> &'static str {
    match provider {
        CODEX => codex::DISPLAY_NAME,
        CLAUDE => claude::DISPLAY_NAME,
        other => other.as_str(),
    }
}

/// 입력 접수 기록에 남기는 고정 모델 글 `<provider>/<model>`. 같은 모델 이름이 두 provider에 있어도 구분된다.
pub(crate) fn pinned_text(choice: &ModelChoice) -> String {
    format!("{}/{}", choice.provider, choice.model)
}

/// `pinned_text`가 만든 글을 되돌린다. provider 접두사가 없으면 `None`.
pub(crate) fn parse_pinned(text: &str) -> Option<ModelChoice> {
    let (name, model) = text.split_once('/')?;
    let provider = Provider::from_stored(name).ok()?;
    Some(ModelChoice {
        provider,
        model: model.to_owned(),
    })
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
            provider_session: ProviderSessionId("s".to_owned()),
            steer_verified,
        };

        assert_eq!(steer_route(&handle(false)), SteerRoute::Queue);
        assert_eq!(steer_route(&handle(true)), SteerRoute::Steer);
    }

    #[test]
    fn installed_means_executable_file_on_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join(program_name(CLAUDE).unwrap());
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        let path =
            |dir: &std::path::Path| vec![(OsString::from("PATH"), dir.as_os_str().to_owned())];

        let plain_file = is_installed(CLAUDE, &path(dir.path()));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let executable = is_installed(CLAUDE, &path(dir.path()));

        assert!(!plain_file);
        assert!(executable);
        assert!(!is_installed(CODEX, &path(dir.path())));
        assert!(!is_installed(CLAUDE, &[]));
    }

    #[test]
    fn pinned_text_keeps_the_old_provider_prefix() {
        let choice = parse_pinned("codex/gpt-x").unwrap();

        assert_eq!(choice.provider, CODEX);
        assert_eq!(choice.model, "gpt-x");
        assert_eq!(pinned_text(&choice), "codex/gpt-x");
        assert_eq!(parse_pinned("claude/a/b").unwrap().model, "a/b");
        assert_eq!(parse_pinned("no-prefix"), None);
    }

    #[test]
    fn unknown_provider_has_no_executable() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("gemini");
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = vec![(OsString::from("PATH"), dir.path().as_os_str().to_owned())];

        assert!(!is_installed(Provider::parse("gemini").unwrap(), &path));
        assert_eq!(program_name(Provider::parse("gemini").unwrap()), None);
    }

    #[test]
    fn first_input_prefers_claude_over_codex() {
        assert_eq!(FIRST_INPUT_ORDER, [CLAUDE, CODEX]);
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
