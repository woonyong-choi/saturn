//! 입력 흐름 테스트용 가짜 provider. 프로세스를 띄우지 않고 받은 호출을 기록하며 정해 둔 답과 이벤트를 낸다.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId, SettingsRevision, SubagentId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use tokio::sync::{Semaphore, mpsc};

use crate::processes::{ProcessGroupId, Supervisor};
use crate::providers::{
    Adapter, AdapterConnection, BoxFuture, ContextDefaults, Descriptor, ExtensionLayout, Feature,
    INTERFACE_VERSION, LaunchSpec, ProviderConnection,
};

/// 실제 어댑터 연결과 가짜 app-server를 쓰는 시험이 가져다 쓴다.
pub(crate) use super::codex::CodexClient;
/// 응답을 보내지 않을 수도 있는 가짜 app-server를 띄우는 실행 설정.
pub(crate) use super::codex::tests::launch as fake_codex_launch;

/// 시험이 이름으로 부르는 어댑터 id. 어댑터 밖 공통 코드는 이 이름을 쓰지 않는다.
pub(crate) const CODEX: Provider = Provider::from_static("codex");
pub(crate) const CLAUDE: Provider = Provider::from_static("claude");

/// provider가 받은 호출.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    Open {
        agent: AgentId,
        model: Option<String>,
        resume: Option<ProviderSessionId>,
        packet: Option<String>,
        add_dirs: Vec<PathBuf>,
        interrupted_children: Vec<SubagentId>,
        settings: SettingsRevision,
    },
    SendTurn {
        session: ProviderSessionId,
        text: String,
    },
    Steer {
        session: ProviderSessionId,
        text: String,
    },
    /// `target`이 `None`이면 메인.
    Interrupt {
        session: ProviderSessionId,
        target: Option<SubagentId>,
    },
    AnswerPermission {
        session: ProviderSessionId,
        request_id: String,
        answer: PermissionAnswer,
    },
    AnswerInput {
        session: ProviderSessionId,
        request_id: String,
        answer: InputAnswer,
    },
    Close {
        session: ProviderSessionId,
    },
}

/// 시험이 풀어 줄 때까지 멈춰 세울 수 있는 호출. 느린 provider를 시간 맞추기 없이 흉내 낸다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Stall {
    /// 어댑터의 `connect`.
    Connect,
    Open,
    Answer,
    Close,
    Models,
}

type Answer = Result<(), ProviderError>;

#[derive(Debug, Default)]
struct Script {
    calls: Vec<Call>,
    open: VecDeque<Answer>,
    send: VecDeque<Answer>,
    steer: VecDeque<Answer>,
    answer_permission: VecDeque<Answer>,
    answer_input: VecDeque<Answer>,
    steer_verified: bool,
    /// 호출을 받으면 패닉한다.
    panic_on_open: bool,
    /// 재개 요청에도 이 번호로 새로 열었다고 답한다.
    reopen_as: Option<ProviderSessionId>,
    panic_on_send: bool,
    stalls: HashMap<Stall, Arc<Semaphore>>,
    opened: u32,
    group: Option<ProcessGroupId>,
    commands: Vec<ProviderCommand>,
}

/// 복제본은 같은 기록과 답을 함께 쓴다. 답을 정해 두지 않은 호출은 성공한다.
#[derive(Debug, Clone)]
pub(crate) struct FakeProvider {
    provider: Provider,
    script: Arc<Mutex<Script>>,
    events_tx: mpsc::UnboundedSender<ProviderEvent>,
    events_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ProviderEvent>>>,
}

impl AdapterConnection for FakeProvider {
    fn process_group(&self, _session: &ProviderSessionId) -> Option<ProcessGroupId> {
        self.group()
    }

    fn shared_group(&self) -> Option<ProcessGroupId> {
        self.group()
    }
}

/// 가짜 어댑터의 설명자. 시험이 값을 바꿔 쓴다.
pub(crate) fn fake_descriptor(id: Provider) -> Descriptor {
    Descriptor {
        id,
        display_name: "fake",
        program: "fake-agent",
        order: 100,
        features: &[Feature::Steer, Feature::Compact],
        instruction_doc: "FAKE.md",
        interface_version: INTERFACE_VERSION,
        context: ContextDefaults {
            window: 50_000,
            cache_write: 2.0,
        },
        extensions: ExtensionLayout::NONE,
    }
}

/// 설명자와 연결 하나로 이루어진 어댑터. 공통 코드를 고치지 않고 provider를 붙이는 시험에 쓴다.
#[derive(Debug)]
pub(crate) struct FakeAdapter {
    pub(crate) descriptor: Descriptor,
    pub(crate) provider: FakeProvider,
}

impl Adapter for FakeAdapter {
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    fn connect(
        &self,
        _launch: LaunchSpec,
        _supervisor: Supervisor,
    ) -> BoxFuture<'_, Result<ProviderConnection, ProviderError>> {
        Box::pin(async move {
            self.provider.stalled(Stall::Connect).await;
            Ok(self.provider.connection())
        })
    }
}

impl FakeProvider {
    /// 이 가짜를 가리키는 연결. 복제본이라 호출 기록과 답은 이 가짜와 함께 쓴다.
    pub(crate) fn connection(&self) -> ProviderConnection {
        ProviderConnection::new(self.provider, self.clone())
    }

    pub(crate) fn new(provider: Provider) -> Self {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        Self {
            provider,
            script: Arc::default(),
            events_tx,
            events_rx: Arc::new(tokio::sync::Mutex::new(events_rx)),
        }
    }

    pub(crate) fn provider(&self) -> Provider {
        self.provider
    }

    /// 끼워 넣기 실측을 통과한 provider처럼 연다.
    pub(crate) fn verify_steer(&self) {
        self.lock().steer_verified = true;
    }

    /// 재개 요청을 받으면 기록이 없어 새로 열었다는 듯 `session` 번호로 답한다.
    pub(crate) fn reopen_as(&self, session: &str) {
        self.lock().reopen_as = Some(ProviderSessionId(session.to_owned()));
    }

    /// 다음 `open_session`에서 연결 작업이 패닉하게 한다.
    pub(crate) fn panic_on_open(&self) {
        self.lock().panic_on_open = true;
    }

    /// 다음 `send_turn`에서 연결 작업이 패닉하게 한다.
    pub(crate) fn panic_on_send(&self) {
        self.lock().panic_on_send = true;
    }

    /// 이 종류의 호출을 `release`까지 멈춰 세운다. 호출 기록은 멈추기 전에 남는다.
    pub(crate) fn stall(&self, call: Stall) {
        self.lock().stalls.insert(call, Arc::new(Semaphore::new(0)));
    }

    /// 멈춰 세운 호출과 이후 호출을 모두 보낸다.
    pub(crate) fn release(&self, call: Stall) {
        if let Some(gate) = self.lock().stalls.remove(&call) {
            gate.add_permits(Semaphore::MAX_PERMITS >> 1);
        }
    }

    async fn stalled(&self, call: Stall) {
        let gate = self.lock().stalls.get(&call).cloned();
        if let Some(gate) = gate {
            // 닫지 않는 세마포어라 얻기 실패는 없다
            if let Ok(permit) = gate.acquire().await {
                permit.forget();
            }
        }
    }

    pub(crate) fn answer_open(&self, answers: impl IntoIterator<Item = Answer>) {
        self.lock().open.extend(answers);
    }

    pub(crate) fn answer_send(&self, answers: impl IntoIterator<Item = Answer>) {
        self.lock().send.extend(answers);
    }

    pub(crate) fn answer_steer(&self, answers: impl IntoIterator<Item = Answer>) {
        self.lock().steer.extend(answers);
    }

    pub(crate) fn answer_permission_with(&self, answers: impl IntoIterator<Item = Answer>) {
        self.lock().answer_permission.extend(answers);
    }

    pub(crate) fn answer_input_with(&self, answers: impl IntoIterator<Item = Answer>) {
        self.lock().answer_input.extend(answers);
    }

    /// 이 연결이 알릴 명령 목록. 연결이 이미 돌고 있어도 다음 요청이나 이벤트 처리 뒤에 알려진다.
    pub(crate) fn set_commands(&self, commands: Vec<ProviderCommand>) {
        self.lock().commands = commands;
    }

    /// 멈춤 때 중지할 프로세스 묶음.
    pub(crate) fn set_group(&self, group: ProcessGroupId) {
        self.lock().group = Some(group);
    }

    pub(crate) fn group(&self) -> Option<ProcessGroupId> {
        self.lock().group
    }

    /// 연결이 이벤트를 읽는 순서로 내보낸다.
    pub(crate) fn emit(&self, event: ProviderEvent) {
        let _ = self.events_tx.send(event); // 받는 쪽이 이미 사라진 시험은 이벤트가 필요 없다
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ProviderClient for FakeProvider {
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        {
            let mut script = self.lock();
            script.calls.push(Call::Open {
                agent: spec.agent,
                model: spec.model.clone(),
                resume: spec.resume.clone(),
                packet: spec.packet.clone(),
                add_dirs: spec.add_dirs.clone(),
                interrupted_children: spec.interrupted_children.clone(),
                settings: spec.settings,
            });
            assert!(!script.panic_on_open, "fake provider panics on open");
        }
        self.stalled(Stall::Open).await;
        let mut script = self.lock();
        script.open.pop_front().unwrap_or(Ok(()))?;
        script.opened += 1;
        let provider_session = spec
            .resume
            .map(|resumed| script.reopen_as.clone().unwrap_or(resumed))
            .unwrap_or_else(|| ProviderSessionId(format!("fake-session-{}", script.opened)));
        Ok(SessionHandle {
            provider_session,
            steer_verified: script.steer_verified,
        })
    }

    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let mut script = self.lock();
        script.calls.push(Call::SendTurn {
            session: session.clone(),
            text: text.to_owned(),
        });
        assert!(!script.panic_on_send, "fake provider panics on send");
        script.send.pop_front().unwrap_or(Ok(()))
    }

    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let mut script = self.lock();
        script.calls.push(Call::Steer {
            session: session.clone(),
            text: text.to_owned(),
        });
        script.steer.pop_front().unwrap_or(Ok(()))
    }

    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        self.lock().calls.push(Call::Interrupt {
            session: session.clone(),
            target: match target {
                InterruptTarget::Subagent(subagent) => Some(subagent),
                InterruptTarget::Main => None,
            },
        });
        Ok(())
    }

    async fn compact(&mut self, _session: &ProviderSessionId) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn answer_permission(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        self.lock().calls.push(Call::AnswerPermission {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
        });
        self.stalled(Stall::Answer).await;
        self.lock().answer_permission.pop_front().unwrap_or(Ok(()))
    }

    async fn answer_input(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        self.lock().calls.push(Call::AnswerInput {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
        });
        self.stalled(Stall::Answer).await;
        self.lock().answer_input.pop_front().unwrap_or(Ok(()))
    }

    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        self.lock().calls.push(Call::Close {
            session: session.clone(),
        });
        self.stalled(Stall::Close).await;
        Ok(())
    }

    async fn next_event(&mut self) -> Option<ProviderEvent> {
        self.events_rx.lock().await.recv().await
    }

    /// 정해 둔 모델 없이 provider마다 모델 하나를 돌려준다.
    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.stalled(Stall::Models).await;
        let model = format!("fake-{}", self.provider);
        Ok(vec![ModelInfo {
            choice: ModelChoice {
                provider: self.provider,
                model: model.clone(),
            },
            name: model,
        }])
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        self.lock().commands.clone()
    }
}
