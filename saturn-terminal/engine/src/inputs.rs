//! provider 입력 요청: 기록한 이벤트를 TUI에 올려 답을 provider로 돌려준다. 허가 요청과 같은 방식으로 답이 올
//! 때까지 요청을 보관하고 작업을 `허가 기다림`으로 보인다.
//! 설계: docs/design/permissions.md#입력-요청

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{AgentId, ChatId, Provider, TaskId};
use saturn_protocol::input::{InputAnswer, InputRequest};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::TaskState;

use crate::calls::{CallKind, Responder};
use crate::flow::LiveSession;
use crate::rpc::ClientId;
use crate::{Engine, EngineError};

/// 답을 기다리는 입력 요청이 속한 곳.
#[derive(Debug, Clone)]
pub(crate) struct PendingInput {
    pub(crate) chat: ChatId,
    pub(crate) agent: AgentId,
    pub(crate) provider: Provider,
    pub(crate) task: TaskId,
    /// provider가 그 연결에서 붙인 요청 ID. 답은 이 ID로 돌려준다.
    pub(crate) provider_request: String,
}

/// 사용자 답을 provider가 받는 중인 입력 요청.
#[derive(Debug)]
pub(crate) struct InputAnswering {
    responder: Responder,
    client: ClientId,
    /// engine이 발급한 요청 ID.
    request_id: String,
    pending: PendingInput,
}

impl Engine {
    // cost: time O(p + i), heap O(f), stack O(1), io 2
    // vars: p = 대기 허가 요청 수, i = 대기 입력 요청 수, f = 요청 크기
    // basis: estimate
    /// 입력 요청을 TUI로 올린다. 답이 올 때까지 provider는 그 호출에서 멈춰 있고 작업 시계도 멈춘다. TUI가 붙어
    /// 있지 않으면 보관했다가 붙을 때 보낸다.
    pub(crate) async fn offer_input(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        request_id: &str,
        request: &InputRequest,
    ) {
        let Some(task) = self.runs.task_of.get(&live.agent).copied() else {
            tracing::warn!(request = request_id, "input request without a task");
            return;
        };
        let label = self.flow.tasks.assign(task);
        let waiting = self.waiting_in_chat(chat);
        let engine_request = self.flow.issue_request_id();
        self.flow.inputs.insert(
            engine_request.clone(),
            PendingInput {
                chat,
                agent: live.agent,
                provider: live.provider,
                task,
                provider_request: request_id.to_owned(),
            },
        );
        let notification = Notification::InputRequested {
            task,
            label,
            provider: live.provider,
            request_id: engine_request,
            request: request.clone(),
            waiting,
        };
        self.rpc.offer_input(chat, notification).await;
        self.notify_task(
            chat,
            task,
            TaskState::AwaitingInput,
            Some(live.provider),
            None,
        )
        .await;
    }

    /// 사용자 답을 provider 값으로 넘긴다. 다른 TUI의 창은 지운다. 답은 연결 작업이 보내고, `responder`에는 provider가
    /// 받은 뒤에 응답한다.
    ///
    /// 응답 오류: 묻지 않은 요청이거나 같은 요청의 답이 이미 가는 중이면 `UnexpectedAnswer`, 답한 TUI가 그 요청의
    /// 채팅에 붙어 있지 않으면 `ChatNotAttached`, 열린 session이 없으면 `Provider(NotSent)`. provider가 받지
    /// 못했으면 요청을 그대로 두어 다시 답할 수 있다.
    pub(crate) async fn answer_input(
        &mut self,
        client: ClientId,
        request_id: String,
        answer: InputAnswer,
        responder: Responder,
    ) {
        let (pending, live) = match self.check_input_answer(client, &request_id) {
            Ok(checked) => checked,
            Err(error) => return self.respond(responder, Err(error)).await,
        };
        self.flow.answering.insert(request_id.clone());
        let route = (pending.chat, pending.provider);
        let provider_request = pending.provider_request.clone();
        let answering = InputAnswering {
            responder,
            client,
            request_id,
            pending,
        };
        self.start_call(route, CallKind::Input(answering), |connection, tag| {
            connection.answer_input_call(tag, live.provider_session, provider_request, answer);
        });
    }

    fn check_input_answer(
        &self,
        client: ClientId,
        request_id: &str,
    ) -> Result<(PendingInput, LiveSession), EngineError> {
        let pending = self
            .flow
            .inputs
            .get(request_id)
            .filter(|_| !self.flow.answering.contains(request_id))
            .cloned()
            .ok_or(EngineError::UnexpectedAnswer { what: "input" })?;
        self.require_attached(client, pending.chat)?;
        let live =
            self.flow
                .live
                .get(&pending.agent)
                .cloned()
                .ok_or_else(|| ProviderError::NotSent {
                    reason: "no open session for the input request".to_owned(),
                })?;
        if !self
            .providers
            .contains_key(&(pending.chat, pending.provider))
        {
            return Err(ProviderError::NotSent {
                reason: "provider is not connected".to_owned(),
            }
            .into());
        }
        Ok((pending, live))
    }

    /// provider가 사용자 답을 받았거나 받지 못했다. 받았으면 요청을 지우고 창을 닫는다.
    pub(crate) async fn finish_input_answer(
        &mut self,
        answering: InputAnswering,
        result: Result<(), ProviderError>,
    ) {
        let InputAnswering {
            responder,
            client,
            request_id,
            pending,
        } = answering;
        self.flow.answering.remove(&request_id);
        if let Err(error) = result {
            return self.respond(responder, Err(error.into())).await;
        }
        // 기다리는 동안 턴이 끝나 요청이 이미 지워졌으면 창과 작업 상태는 그대로 둔다
        if self.flow.inputs.remove(&request_id).is_some() {
            self.rpc.resolve_input(client, &request_id).await;
            let state = self
                .waiting_state(pending.task)
                .unwrap_or(TaskState::Running);
            self.notify_task(
                pending.chat,
                pending.task,
                state,
                Some(pending.provider),
                None,
            )
            .await;
        }
        self.respond(responder, Ok(())).await;
    }

    // cost: time O(i), heap O(i), stack O(1), alloc 1
    // vars: i = 대기 입력 요청 수
    // basis: estimate
    /// 턴이 끝났거나 흐름이 끊겨 더는 답할 수 없는 요청의 창을 지운다.
    pub(crate) async fn clear_inputs(&mut self, agent: AgentId) {
        let ended: Vec<String> = self
            .flow
            .inputs
            .iter()
            .filter(|(_, pending)| pending.agent == agent)
            .map(|(request_id, _)| request_id.clone())
            .collect();
        for request_id in ended {
            self.flow.inputs.remove(&request_id);
            self.rpc.withdraw_input(&request_id).await;
        }
    }
}
