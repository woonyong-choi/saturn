//! provider 어댑터 계약: 설명자, 연결 만들기, 권한 번역, 연결 하나의 동작. provider 이름은 이 파일에 없다.
//! 설계: docs/design/providers-and-sessions.md#provider-계층과-어댑터

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use saturn_core::permission::Rule;
use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ExtensionPartKind, Injectability, ModelInfo, PermissionAnswer};

use super::{AppliedSettings, LaunchSpec, PermissionLaunch};
use crate::processes::{ProcessGroupId, Supervisor};

/// engine이 지원하는 Saturn 인터페이스 판. 어댑터가 알린 판이 이 값과 다르면 등록하지 않는다.
pub(crate) const INTERFACE_VERSION: u32 = 1;

pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 열린 session의 적용값을 읽는 함수. 읽기 작업이 갱신하는 값이라 부를 때마다 읽는다.
pub(crate) type AppliedReader = Arc<dyn Fn() -> Option<AppliedSettings> + Send + Sync>;

/// 어댑터가 구현하는 동작 중 선택 항목. 빠진 항목은 구현하지 않은 것으로 보고 공통 코드가 그 동작을 요청하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Feature {
    /// 진행 중인 턴에 입력을 끼워 넣는다. 없으면 입력은 대기로 처리한다.
    Steer,
    /// 맥락 정리를 요청한다.
    Compact,
}

/// 맥락 값 중 provider마다 다른 기본값. 나머지 기본값은 모든 provider가 같다([설정](../../../../docs/design/settings.md)).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContextDefaults {
    /// 맥락 창 크기(토큰).
    pub(crate) window: u64,
    /// 캐시 쓰기 비용 배율.
    pub(crate) cache_write: f64,
}

/// 어댑터가 등록할 때 알리는 값.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Descriptor {
    pub(crate) id: Provider,
    /// 화면과 사용량에 보이는 이름.
    pub(crate) display_name: &'static str,
    /// 설치 여부를 확인하고 실행할 파일 이름.
    pub(crate) program: &'static str,
    /// 고정 모델도 이어 갈 메인 session도 없는 첫 입력과 모델 목록이 provider를 고르는 순서. 작은 값이 먼저다.
    pub(crate) order: u32,
    pub(crate) features: &'static [Feature],
    /// provider가 스스로 읽는 문서의 이름. 패킷에 넣지 않는다.
    pub(crate) instruction_doc: &'static str,
    pub(crate) interface_version: u32,
    pub(crate) context: ContextDefaults,
    /// 확장 부분을 받는 provider 형식. 어느 종류를 받는지가 `주입 가능` 판정의 기본 답이다.
    pub(crate) extensions: super::ExtensionLayout,
}

/// 권한 규칙을 provider 실행 설정으로 번역할 때 필요한 값.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PermissionInput<'a> {
    pub(crate) saturn_home: &'a Path,
    pub(crate) rules: &'a [Rule],
    /// provider 자식 프로세스에 줄 환경.
    pub(crate) env: &'a [(std::ffi::OsString, std::ffi::OsString)],
    /// 에이전트 질문 기능을 켠다.
    pub(crate) questions: bool,
    /// 이 provider가 주입 가능하다고 답한 확장 부분. 이름은 권한 번역에서 왔지만 같은 실행 설정을 만드는 입력이라
    /// 확장도 여기에 담는다.
    pub(crate) extensions: super::ExtensionInput<'a>,
}

/// provider 하나를 붙이는 어댑터. 레지스트리에 한 번 등록한다.
pub(crate) trait Adapter: Send + Sync + std::fmt::Debug {
    fn descriptor(&self) -> &Descriptor;

    /// 연결을 만든다. session을 여는 시점은 어댑터가 정한다.
    fn connect(
        &self,
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> BoxFuture<'_, Result<ProviderConnection, ProviderError>>;

    /// Saturn 권한 규칙과 확장 부분을 이 provider의 실행 설정으로 번역한다. 기본은 규칙을 번역하지 않고 질문 기능만
    /// 따르며 확장도 주입하지 않는다.
    ///
    /// # Errors
    /// 번역한 설정을 쓰지 못하면 `NotSent`.
    fn translate_permission(
        &self,
        input: PermissionInput<'_>,
    ) -> Result<PermissionLaunch, ProviderError> {
        Ok(PermissionLaunch {
            questions_disabled: !input.questions,
            ..PermissionLaunch::default()
        })
    }

    /// 이 provider에 `kind` 부분을 주입할 수 있는지. 기본은 설명자의 주입 형식이 그 종류를 받으면 `Injectable`, 아니면
    /// `Unavailable`이다. 형식만으로 답할 수 없는 어댑터는 `Unknown`을 돌려주도록 바꾼다.
    fn injectability(&self, kind: ExtensionPartKind) -> Injectability {
        if self.descriptor().extensions.accepts(kind) {
            Injectability::Injectable
        } else {
            Injectability::Unavailable
        }
    }

    /// 사용자가 이 provider에 직접 설치한 스킬, 명령, MCP 서버, 플러그인을 읽는다. 기본은 없음. `env`의 `HOME`과 같은
    /// 값에서 provider의 사용자 폴더를 찾고, 읽기만 하며 고치지 않는다. 읽지 못한 항목은 건너뛴다.
    fn direct_installs(
        &self,
        _env: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> Vec<super::DirectInstall> {
        Vec::new()
    }

    /// 설치된 provider CLI의 버전을 읽는다. 설치되지 않았거나 읽지 못하면 `None`. 기본은 실행 파일에 `--version`을
    /// 주어 첫 줄에서 버전을 꺼낸다. 다른 방식으로 읽어야 하는 어댑터가 바꾼다.
    fn read_version(
        &self,
        env: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> BoxFuture<'_, Option<String>> {
        let program = self.descriptor().program;
        let env = env.to_vec();
        Box::pin(async move { read_cli_version(program, &env).await })
    }

    /// 규칙이 연결을 시작할 때 고정되는 어댑터가 규칙 지문을 돌려준다. 지문이 연결을 시작할 때와 다르면 연결을 다시
    /// 시작한다. 규칙을 실행 중에 바꿀 수 있는 어댑터는 `None`이다.
    fn rules_fingerprint(&self, _rules: &[Rule]) -> Option<String> {
        None
    }
}

/// 버전을 읽는 데 기다리는 최대 시간. 넘으면 읽지 못한 것으로 본다. 시작이 멈추지 않게 하는 초안 값이다.
const VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// `PATH`에서 `program`을 찾아 `--version`을 실행하고 출력 첫 줄에서 버전을 꺼낸다. 자식 환경에는 `PATH`와 `HOME`만
/// 준다. 버전을 읽는 데 다른 설정이 필요 없고 router 키가 새지 않게 하기 위해서다.
async fn read_cli_version(
    program: &str,
    env: &[(std::ffi::OsString, std::ffi::OsString)],
) -> Option<String> {
    let path = env
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone())?;
    let file = std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())?;
    let mut command = tokio::process::Command::new(file);
    command
        .arg("--version")
        .env_clear()
        .envs(
            env.iter()
                .filter(|(name, _)| name == "PATH" || name == "HOME")
                .map(|(name, value)| (name, value)),
        )
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

/// 출력 첫 줄에서 숫자로 시작하는 첫 낱말을 버전으로 본다(`codex-cli 0.158.0`, `2.1.285 (Claude Code)`). 그런 낱말이
/// 없으면 첫 줄 전체를 쓰고, 빈 출력이면 `None`이다.
fn parse_version(output: &str) -> Option<String> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let word = line
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()));
    Some(word.unwrap_or(line).to_owned())
}

/// 어댑터 연결 하나의 동작. `ProviderClient`에 프로세스 묶음, 적용값, 줄 세운 입력 전송을 더한다.
pub(crate) trait AdapterConnection: ProviderClient {
    /// 그 session을 실행하는 프로세스 묶음. 모르는 session이면 `None`.
    fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId>;

    /// 모든 session이 프로세스 묶음 하나를 같이 쓰는 어댑터의 그 묶음. 연결을 통째로 닫을 때 쓴다.
    fn shared_group(&self) -> Option<ProcessGroupId> {
        None
    }

    /// 적용값을 받기 전이면 `None`.
    fn applied_settings(&self, _session: &ProviderSessionId) -> Option<AppliedSettings> {
        None
    }

    /// 연결 작업 밖에서 동기로 적용값을 읽는 함수. 알 수 없으면 `None`.
    fn applied_reader(&self, _session: &ProviderSessionId) -> Option<AppliedReader> {
        None
    }

    /// 턴 완료를 처리한 뒤 그 에이전트에 줄 세워 둔 첫 입력을 보낸다. 줄 세우지 않는 어댑터는 아무것도 하지 않는다.
    /// 응답을 기다리므로 `select` 가지 안에서 부르지 말고 가지 본문에서 끝까지 기다린다(#324).
    fn start_queued_turn(&mut self, _agent: AgentId) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// `AdapterConnection`을 dyn으로 쓰려고 `impl Future`를 상자에 담은 모양. 어댑터가 직접 구현하지 않는다.
pub(crate) trait DynConnection: Send {
    fn open_session(
        &mut self,
        spec: SessionSpec,
    ) -> BoxFuture<'_, Result<SessionHandle, ProviderError>>;
    fn send_turn<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        text: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn steer<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        text: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn interrupt<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        target: InterruptTarget,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn compact<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn answer_permission<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        request_id: &'a str,
        answer: PermissionAnswer,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn answer_input<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        request_id: &'a str,
        answer: InputAnswer,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn close_session<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
    ) -> BoxFuture<'a, Result<(), ProviderError>>;
    fn list_models(&mut self) -> BoxFuture<'_, Result<Vec<ModelInfo>, ProviderError>>;
    fn next_event(&mut self) -> BoxFuture<'_, Option<ProviderEvent>>;
    fn commands(&self) -> Vec<ProviderCommand>;
    fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId>;
    fn shared_group(&self) -> Option<ProcessGroupId>;
    fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings>;
    fn applied_reader(&self, session: &ProviderSessionId) -> Option<AppliedReader>;
    fn start_queued_turn(&mut self, agent: AgentId) -> BoxFuture<'_, ()>;
}

impl<T: AdapterConnection> DynConnection for T {
    fn open_session(
        &mut self,
        spec: SessionSpec,
    ) -> BoxFuture<'_, Result<SessionHandle, ProviderError>> {
        Box::pin(ProviderClient::open_session(self, spec))
    }

    fn send_turn<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        text: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::send_turn(self, session, text))
    }

    fn steer<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        text: &'a str,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::steer(self, session, text))
    }

    fn interrupt<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        target: InterruptTarget,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::interrupt(self, session, target))
    }

    fn compact<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::compact(self, session))
    }

    fn answer_permission<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        request_id: &'a str,
        answer: PermissionAnswer,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::answer_permission(
            self, session, request_id, answer,
        ))
    }

    fn answer_input<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
        request_id: &'a str,
        answer: InputAnswer,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::answer_input(
            self, session, request_id, answer,
        ))
    }

    fn close_session<'a>(
        &'a mut self,
        session: &'a ProviderSessionId,
    ) -> BoxFuture<'a, Result<(), ProviderError>> {
        Box::pin(ProviderClient::close_session(self, session))
    }

    fn list_models(&mut self) -> BoxFuture<'_, Result<Vec<ModelInfo>, ProviderError>> {
        Box::pin(ProviderClient::list_models(self))
    }

    fn next_event(&mut self) -> BoxFuture<'_, Option<ProviderEvent>> {
        Box::pin(ProviderClient::next_event(self))
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        ProviderClient::commands(self)
    }

    fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        AdapterConnection::process_group(self, session)
    }

    fn shared_group(&self) -> Option<ProcessGroupId> {
        AdapterConnection::shared_group(self)
    }

    fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        AdapterConnection::applied_settings(self, session)
    }

    fn applied_reader(&self, session: &ProviderSessionId) -> Option<AppliedReader> {
        AdapterConnection::applied_reader(self, session)
    }

    fn start_queued_turn(&mut self, agent: AgentId) -> BoxFuture<'_, ()> {
        Box::pin(AdapterConnection::start_queued_turn(self, agent))
    }
}

/// 어댑터가 만든 연결. 어느 어댑터의 연결인지 id를 함께 든다.
pub struct ProviderConnection {
    provider: Provider,
    inner: Box<dyn DynConnection>,
}

impl std::fmt::Debug for ProviderConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderConnection")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl ProviderConnection {
    pub(crate) fn new(provider: Provider, connection: impl AdapterConnection + 'static) -> Self {
        Self {
            provider,
            inner: Box::new(connection),
        }
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    pub(crate) fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        self.inner.process_group(session)
    }

    pub(crate) fn shared_group(&self) -> Option<ProcessGroupId> {
        self.inner.shared_group()
    }

    pub(crate) fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        self.inner.applied_settings(session)
    }

    pub(crate) fn applied_reader(&self, session: &ProviderSessionId) -> Option<AppliedReader> {
        self.inner.applied_reader(session)
    }

    pub(crate) async fn start_queued_turn(&mut self, agent: AgentId) {
        self.inner.start_queued_turn(agent).await;
    }
}

impl ProviderClient for ProviderConnection {
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        self.inner.open_session(spec).await
    }

    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        self.inner.send_turn(session, text).await
    }

    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        self.inner.steer(session, text).await
    }

    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        self.inner.interrupt(session, target).await
    }

    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        self.inner.compact(session).await
    }

    async fn answer_permission(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        self.inner
            .answer_permission(session, request_id, answer)
            .await
    }

    async fn answer_input(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        self.inner.answer_input(session, request_id, answer).await
    }

    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        self.inner.close_session(session).await
    }

    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.inner.list_models().await
    }

    /// 취소해도 이벤트를 잃지 않는다. 줄 선 입력은 보내지 않으므로 완료를 처리한 뒤 `start_queued_turn`으로 보낸다.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        self.inner.next_event().await
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        self.inner.commands()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_first_word_that_starts_with_a_digit() {
        assert_eq!(
            parse_version("codex-cli 0.158.0\n").as_deref(),
            Some("0.158.0")
        );
        assert_eq!(
            parse_version("2.1.285 (Claude Code)\n").as_deref(),
            Some("2.1.285")
        );
        assert_eq!(
            parse_version("\n  tool beta\nsecond").as_deref(),
            Some("tool beta")
        );
        assert_eq!(parse_version("  \n"), None);
    }
}
