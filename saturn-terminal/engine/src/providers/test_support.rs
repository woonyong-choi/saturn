//! 입력 흐름 테스트용 가짜 provider. 프로세스를 띄우지 않고 받은 호출을 기록하며 정해 둔 답과 이벤트를 낸다.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId, SubagentId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use tokio::sync::mpsc;

use crate::processes::ProcessGroupId;

/// provider가 받은 호출.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    Open {
        agent: AgentId,
        model: Option<String>,
        resume: Option<ProviderSessionId>,
        packet: Option<String>,
        add_dirs: Vec<PathBuf>,
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
    opened: u32,
    group: Option<ProcessGroupId>,
}

/// 복제본은 같은 기록과 답을 함께 쓴다. 답을 정해 두지 않은 호출은 성공한다.
#[derive(Debug, Clone)]
pub(crate) struct FakeProvider {
    provider: Provider,
    script: Arc<Mutex<Script>>,
    events_tx: mpsc::UnboundedSender<ProviderEvent>,
    events_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ProviderEvent>>>,
}

impl FakeProvider {
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
        let mut script = self.lock();
        script.calls.push(Call::Open {
            agent: spec.agent,
            model: spec.model.clone(),
            resume: spec.resume.clone(),
            packet: spec.packet.clone(),
            add_dirs: spec.add_dirs.clone(),
        });
        script.open.pop_front().unwrap_or(Ok(()))?;
        script.opened += 1;
        let provider_session = spec
            .resume
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
        let mut script = self.lock();
        script.calls.push(Call::AnswerPermission {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
        });
        script.answer_permission.pop_front().unwrap_or(Ok(()))
    }

    async fn answer_input(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        let mut script = self.lock();
        script.calls.push(Call::AnswerInput {
            session: session.clone(),
            request_id: request_id.to_owned(),
            answer,
        });
        script.answer_input.pop_front().unwrap_or(Ok(()))
    }

    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        self.lock().calls.push(Call::Close {
            session: session.clone(),
        });
        Ok(())
    }

    async fn next_event(&mut self) -> Option<ProviderEvent> {
        self.events_rx.lock().await.recv().await
    }

    /// 정해 둔 모델 없이 provider마다 모델 하나를 돌려준다.
    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        let model = format!("fake-{:?}", self.provider).to_lowercase();
        Ok(vec![ModelInfo {
            choice: ModelChoice {
                provider: self.provider,
                model: model.clone(),
            },
            name: model,
        }])
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        Vec::new()
    }
}
