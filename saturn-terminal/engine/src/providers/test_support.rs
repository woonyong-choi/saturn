//! 입력 흐름 테스트용 가짜 provider. 프로세스를 띄우지 않고 받은 호출을 기록하며 정해 둔 답을 낸다.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId};
use saturn_protocol::rpc::PermissionAnswer;

/// provider가 받은 호출.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Open {
        agent: AgentId,
        model: Option<String>,
        resume: Option<ProviderSessionId>,
    },
    SendTurn {
        session: ProviderSessionId,
        text: String,
    },
    Steer {
        session: ProviderSessionId,
        text: String,
    },
    AnswerPermission {
        session: ProviderSessionId,
        request_id: String,
        answer: PermissionAnswer,
    },
}

type Answer = Result<(), ProviderError>;

#[derive(Debug, Default)]
struct Script {
    calls: Vec<Call>,
    open: VecDeque<Answer>,
    send: VecDeque<Answer>,
    steer: VecDeque<Answer>,
    steer_verified: bool,
    opened: u32,
}

/// 복제본은 같은 기록과 답을 함께 쓴다. 답을 정해 두지 않은 호출은 성공한다.
#[derive(Debug, Clone)]
pub(crate) struct FakeProvider {
    provider: Provider,
    script: Arc<Mutex<Script>>,
}

impl FakeProvider {
    pub(crate) fn new(provider: Provider) -> Self {
        Self {
            provider,
            script: Arc::default(),
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
        _session: &ProviderSessionId,
        _target: InterruptTarget,
    ) -> Result<(), ProviderError> {
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
        Ok(())
    }

    async fn close_session(&mut self, _session: &ProviderSessionId) -> Result<(), ProviderError> {
        Ok(())
    }

    async fn next_event(&mut self) -> Option<ProviderEvent> {
        std::future::pending().await
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        Vec::new()
    }
}
