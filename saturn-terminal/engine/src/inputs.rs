//! provider 입력 요청: 기록한 이벤트를 TUI에 올려 답을 provider로 돌려준다. 허가 요청과 같은 방식으로 답이 올
//! 때까지 요청을 보관하고 작업을 `허가 기다림`으로 보인다.
//! 설계: docs/design/permissions.md#입력-요청

use saturn_core::providers::{ProviderClient, ProviderError};
use saturn_protocol::ids::{AgentId, ChatId, Provider, TaskId};
use saturn_protocol::input::{InputAnswer, InputRequest};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::TaskState;

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
        self.flow.inputs.insert(
            request_id.to_owned(),
            PendingInput {
                chat,
                agent: live.agent,
                provider: live.provider,
                task,
            },
        );
        let notification = Notification::InputRequested {
            task,
            label,
            provider: live.provider,
            request_id: request_id.to_owned(),
            request: request.clone(),
            waiting,
        };
        self.rpc.offer_input(chat, notification).await;
        self.notify_task(
            chat,
            task,
            TaskState::AwaitingPermission,
            Some(live.provider),
            None,
        )
        .await;
    }

    // cost: time O(p + i), heap O(1), stack O(1), io 2
    // vars: p = 대기 허가 요청 수, i = 대기 입력 요청 수
    // basis: estimate
    /// 사용자 답을 provider 값으로 넘긴다. 다른 TUI의 창은 지운다.
    ///
    /// # Errors
    /// 묻지 않은 요청이면 `UnexpectedAnswer`, 열린 session이 없으면 `Provider(NotSent)`. provider가 받지 못했으면
    /// 요청을 그대로 두어 다시 답할 수 있다.
    pub(crate) async fn answer_input(
        &mut self,
        client: ClientId,
        request_id: String,
        answer: InputAnswer,
    ) -> Result<(), EngineError> {
        let pending = self
            .flow
            .inputs
            .get(&request_id)
            .cloned()
            .ok_or(EngineError::UnexpectedAnswer { what: "input" })?;
        let live =
            self.flow
                .live
                .get(&pending.agent)
                .cloned()
                .ok_or_else(|| ProviderError::NotSent {
                    reason: "no open session for the input request".to_owned(),
                })?;
        self.provider_mut(pending.chat, pending.provider)?
            .answer_input(&live.provider_session, &request_id, answer)
            .await?;
        self.flow.inputs.remove(&request_id);
        self.rpc.resolve_input(client, &request_id).await;
        if !self.is_task_waiting(pending.task) {
            self.notify_task(
                pending.chat,
                pending.task,
                TaskState::Running,
                Some(pending.provider),
                None,
            )
            .await;
        }
        Ok(())
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
