//! provider 이벤트: 기록한 뒤에만 화면과 상태에 반영하고, 허가 요청을 TUI에 올려 답을 provider로 돌려준다.
//! 설계: docs/design/providers-and-sessions.md#이벤트-수신과-변환, docs/design/permissions.md

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::future::{Future, poll_fn};
#[cfg(test)]
use std::pin::pin;
#[cfg(test)]
use std::task::Poll;

use saturn_core::agents::TreeStatus;
use saturn_core::permission::Verdict;
use saturn_core::providers::InterruptTarget;
#[cfg(test)]
use saturn_core::providers::ProviderClient;
use saturn_core::providers::ProviderError;
use saturn_protocol::event::{PermissionCall, ProviderEvent};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, Provider, RunId, SubagentId, TaskId,
};
use saturn_protocol::rpc::{ChatNotice, Notification, PermissionAnswer};
use saturn_protocol::state::{EffectScope, SessionState, TaskState};

use crate::flow::{LiveSession, NeedsCheck};
#[cfg(test)]
use crate::providers::ProviderConnection;
use crate::rpc::ClientId;
use crate::store::NewRun;
use crate::{Engine, EngineError};

/// 답을 기다리는 허가 요청이 속한 곳.
#[derive(Debug, Clone)]
pub(crate) struct PendingPermission {
    pub(crate) chat: ChatId,
    pub(crate) agent: AgentId,
    pub(crate) provider: Provider,
    pub(crate) task: TaskId,
    /// provider가 그 연결에서 붙인 요청 ID. 답은 이 ID로 돌려준다.
    pub(crate) provider_request: String,
    /// `항상 허용` 답을 저장할 호출. 규칙으로 읽지 못한 요청은 `None`.
    pub(crate) call: Option<PermissionCall>,
}

/// provider가 올린 허가 요청 한 건.
#[derive(Clone, Copy)]
struct PermissionRequest<'a> {
    id: &'a str,
    summary: &'a str,
    reason: &'a str,
    call: Option<&'a PermissionCall>,
}

/// 연결 하나가 낸 것. 연결이 끝났으면 `event`가 `None`이다.
pub(crate) struct Arrival {
    pub(crate) chat: ChatId,
    pub(crate) provider: Provider,
    pub(crate) event: Option<ProviderEvent>,
}

/// 연결이 없으면 영원히 기다린다. 이벤트를 꺼내기만 하므로 취소해도 이벤트를 잃지 않는다(응답을 기다리는 일을 여기에 넣지 않는다).
#[cfg(test)]
pub(crate) async fn next_arrival(
    providers: &mut HashMap<(ChatId, Provider), ProviderConnection>,
) -> Arrival {
    poll_fn(|cx| {
        for ((chat, provider), connection) in providers.iter_mut() {
            let mut next = pin!(connection.next_event());
            if let Poll::Ready(event) = next.as_mut().poll(cx) {
                return Poll::Ready(Arrival {
                    chat: *chat,
                    provider: *provider,
                    event,
                });
            }
        }
        Poll::Pending
    })
    .await
}

/// 완료를 처리한 뒤 줄 세워 둔 다음 입력을 보낸다. 전송은 응답을 기다리므로 이벤트 수신(`next_arrival`)에 묶으면 안 된다.
/// 수신 쪽 Future는 `select`에서 언제든 버려지고, 버려지면 이미 꺼낸 완료 이벤트와 전송이 함께 사라진다(#324).
#[cfg(test)]
pub(crate) async fn start_queued_turn(
    providers: &mut HashMap<(ChatId, Provider), ProviderConnection>,
    chat: ChatId,
    provider: Provider,
    agent: AgentId,
) {
    if let Some(connection) = providers.get_mut(&(chat, provider)) {
        connection.start_queued_turn(agent).await;
    }
}

impl Engine {
    pub(crate) async fn on_arrival(&mut self, arrival: Arrival) {
        let Arrival {
            chat,
            provider,
            event,
        } = arrival;
        let Some(event) = event else {
            self.on_connection_closed(chat, provider).await;
            return;
        };
        let completed = match &event {
            ProviderEvent::TurnCompleted { agent, .. } => Some(*agent),
            _ => None,
        };
        if let Err(error) = self.on_provider_event(provider, event).await {
            tracing::warn!(error = %self.failure_line(&error), "provider event not handled");
        }
        if let Some(agent) = completed
            && let Some(connection) = self.providers.get(&(chat, provider))
        {
            // 쓰기가 막힐 수 있어 루프가 기다리지 않고 연결 작업이 끝까지 쓴다
            connection.start_queued_turn_detached(agent);
        }
    }

    /// 이벤트는 처리 전에 먼저 기록한다. 기록하지 못하면 화면에도 상태에도 반영하지 않는다.
    ///
    /// # Errors
    /// 열린 session이 없는 에이전트의 이벤트는 버리고 `Ok`, 붙일 실행이 없으면 `NoRun`, 기록 실패면 `Store`.
    pub(crate) async fn on_provider_event(
        &mut self,
        provider: Provider,
        event: ProviderEvent,
    ) -> Result<(), EngineError> {
        if let ProviderEvent::CacheWindow { ttl_secs, .. } = event {
            self.store.record_cache_ttl(provider, ttl_secs).await?;
            return Ok(());
        }
        let agent = event.agent();
        let live = self
            .flow
            .live
            .get(&agent)
            .filter(|live| live.provider == provider)
            .cloned();
        let Some(live) = live else {
            tracing::warn!(
                agent = agent.0,
                "event from an agent without an open session"
            );
            return Ok(());
        };
        let chat = self.session_chat(live.session)?;
        if self.block_interrupted_subagent(chat, &live, &event).await {
            return Ok(());
        }
        let event = self.mark_packet_reply(event);
        let run = self.run_for_event(chat, &live, &event).await?;
        let seq = self.record_event(run, chat, &live, &event).await?;
        if let Some(seq) = seq {
            self.sessions.mark_delivered(live.session, seq);
        }
        let status = self.agents.on_event(&event);
        self.apply_event(chat, &live, event, status).await
    }

    /// 패킷 턴의 완료 신호 전에 온 메인 글은 패킷에 대한 답이다. 사용자 입력의 답과 섞이지 않도록 `PacketReply`로 바꿔 기록한다.
    fn mark_packet_reply(&self, event: ProviderEvent) -> ProviderEvent {
        match event {
            ProviderEvent::Text {
                agent,
                subagent: None,
                text,
            } if self.flow.packet_turns.contains_key(&agent) => {
                ProviderEvent::PacketReply { agent, text }
            }
            other => other,
        }
    }

    /// 크래시로 끊긴 하위 에이전트의 이벤트가 provider에서 다시 왔으면 기록하지 않고 막는다. 그 에이전트를 멈추고 화면에 알린다.
    /// Saturn이 시작하지 않은 하위 에이전트라 provider가 업데이트로 동작을 바꿔 다시 실행한 것이다. 이미 막은 하위 에이전트의
    /// 이벤트는 알림 없이 버린다.
    async fn block_interrupted_subagent(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        event: &ProviderEvent,
    ) -> bool {
        let Some(subagent) = event_subagent(event) else {
            return false;
        };
        let is_watched = self
            .flow
            .interrupted_watch
            .get(&live.agent)
            .is_some_and(|ids| ids.contains(subagent));
        if !is_watched {
            return false;
        }
        if !self
            .flow
            .interrupted_blocked
            .insert((live.agent, subagent.clone()))
        {
            return true;
        }
        tracing::warn!(agent = live.agent.0, subagent = %subagent.0, "an interrupted subagent was started again by the provider");
        if self.runs.active.contains_key(&live.agent) {
            let stopped = self.stop_chat(chat).await;
            self.warn_failure(
                "failed to stop after an interrupted subagent returned",
                stopped,
            );
        } else if let Some(connection) = self.providers.get(&(chat, live.provider)) {
            connection.interrupt_tree_detached(
                live.provider_session.clone(),
                vec![InterruptTarget::Subagent(subagent.clone())],
            );
        }
        self.notify_chat(
            chat,
            ChatNotice::InterruptedSubagentReturned {
                provider: live.provider,
            },
        )
        .await;
        true
    }

    async fn record_event(
        &self,
        run: RunId,
        chat: ChatId,
        live: &LiveSession,
        event: &ProviderEvent,
    ) -> Result<Option<LedgerSeq>, EngineError> {
        match event {
            ProviderEvent::Usage(report) => {
                self.store.record_usage(run, live.session, report).await?;
                Ok(None)
            }
            _ => Ok(Some(self.store.append_event(run, chat, event).await?)),
        }
    }

    /// 진행 중인 실행에 붙인다. 실행이 없을 때 메인 출력이 오면 입력 없이 provider가 시작한 턴이고,
    /// 그 밖의 이벤트는 가장 나중 실행에 붙인다.
    async fn run_for_event(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        event: &ProviderEvent,
    ) -> Result<RunId, EngineError> {
        if let Some(run) = self.runs.active.get(&live.agent) {
            return Ok(*run);
        }
        let no_run = EngineError::NoRun { agent: live.agent };
        let is_held = self
            .sessions
            .get(live.session)
            .is_some_and(|session| session.state == SessionState::Held);
        if starts_turn(event) && !is_held {
            let task = *self.flow.last_task.get(&live.agent).ok_or(no_run)?;
            return self.begin_wake_run(chat, live, task).await;
        }
        self.flow.last_run.get(&live.agent).copied().ok_or(no_run)
    }

    async fn begin_wake_run(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        task: TaskId,
    ) -> Result<RunId, EngineError> {
        let run = self
            .store
            .start_run(&NewRun {
                input: None,
                task,
                agent: live.agent,
                session: live.session,
                provider: live.provider,
                effect_scope: EffectScope::NetworkPossible,
            })
            .await?;
        self.runs.active.insert(live.agent, run);
        self.runs.chat_of.insert(live.agent, chat);
        self.runs.task_of.insert(live.agent, task);
        self.flow.last_run.insert(live.agent, run);
        self.sessions.mark_busy(live.session);
        Ok(run)
    }

    async fn apply_event(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        event: ProviderEvent,
        status: TreeStatus,
    ) -> Result<(), EngineError> {
        match &event {
            ProviderEvent::PermissionRequested {
                request_id,
                summary,
                reason,
                call,
                ..
            } => {
                let request = PermissionRequest {
                    id: request_id,
                    summary,
                    reason,
                    call: call.as_ref(),
                };
                self.on_permission_request(chat, live, request).await;
            }
            ProviderEvent::InputRequested {
                request_id,
                request,
                ..
            } => self.offer_input(chat, live, request_id, request).await,
            ProviderEvent::ContextSize { tokens, .. } => {
                self.on_context_size(chat, live, *tokens).await;
            }
            ProviderEvent::StreamLost { .. } => self.on_stream_lost(chat, live).await,
            _ => self.forward_event(live.agent, &event).await,
        }
        match event {
            ProviderEvent::TurnCompleted { .. } | ProviderEvent::SubagentEnded { .. } => {
                self.after_tree_change(chat, live, &event, status).await
            }
            _ => Ok(()),
        }
    }

    async fn forward_event(&self, agent: AgentId, event: &ProviderEvent) {
        let Some(chat) = self.runs.chat_of.get(&agent).copied() else {
            return;
        };
        let Some(task) = self.runs.task_of.get(&agent).copied() else {
            return;
        };
        let notification = Notification::TaskEvent {
            task,
            event: event.clone(),
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }

    /// 부모 턴이 끝났는데 subagent가 남았으면 `AnsweredTreeRunning`으로 보이고, 트리가 유휴가 되면 턴 끝이다.
    /// 멈추는 중인 채팅은 멈춤 완료 확인으로 넘긴다.
    async fn after_tree_change(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        event: &ProviderEvent,
        status: TreeStatus,
    ) -> Result<(), EngineError> {
        let is_turn_completed = matches!(event, ProviderEvent::TurnCompleted { .. });
        if self.flow.stopping.contains_key(&chat) {
            if is_turn_completed {
                self.confirm_stopped_agent(chat, live.agent);
            }
            return self.check_stop_done(chat).await;
        }
        if is_turn_completed && self.take_packet_turn(live.agent) {
            return Ok(());
        }
        match status {
            TreeStatus::Running => Ok(()),
            TreeStatus::AnsweredTreeRunning => {
                self.notify_running_task(chat, live, TaskState::AnsweredTreeRunning)
                    .await;
                Ok(())
            }
            TreeStatus::TreeIdle if self.runs.active.contains_key(&live.agent) => {
                self.on_turn_end(chat, live.agent).await
            }
            TreeStatus::TreeIdle => Ok(()),
        }
    }

    /// 새 session의 첫 턴으로 보낸 패킷의 완료 신호면 참이고, 작업 끝으로 보지 않는다.
    fn take_packet_turn(&mut self, agent: AgentId) -> bool {
        let Some(pending) = self.flow.packet_turns.get_mut(&agent) else {
            return false;
        };
        *pending -= 1;
        if *pending == 0 {
            self.flow.packet_turns.remove(&agent);
        }
        true
    }

    pub(crate) async fn notify_running_task(
        &self,
        chat: ChatId,
        live: &LiveSession,
        state: TaskState,
    ) {
        if let Some(task) = self.runs.task_of.get(&live.agent).copied() {
            self.notify_task(chat, task, state, Some(live.provider), None)
                .await;
        }
    }

    async fn on_context_size(&mut self, chat: ChatId, live: &LiveSession, tokens: Option<u64>) {
        self.flow.context_tokens.insert(live.agent, tokens);
        let threshold = match self.context_budget(live.provider).await {
            Ok(budget) => budget.threshold(),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read context budget");
                return;
            }
        };
        let notification = Notification::ContextSize {
            chat,
            tokens,
            threshold,
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }

    /// 완료 신호 없이 흐름이 끝났다. 관찰이 끊겼으므로 효과 범위를 `Unobserved`로 기록하고, 자동으로 이어 가지 않고
    /// 결과 확인 필요로 둔다. 실행 기록은 열어 둔다.
    /// TODO(#90): 결과를 모르는 작업을 사용자가 푸는 방법. 지금은 `/continue <작업>`이 확인 입력을 보낸다
    async fn on_stream_lost(&mut self, chat: ChatId, live: &LiveSession) {
        self.clear_permissions(live.agent).await;
        self.flow.live.remove(&live.agent);
        self.confirm_stopped_agent(chat, live.agent);
        if self.flow.stopping.contains_key(&chat) {
            let checked = self.check_stop_done(chat).await;
            self.warn_failure("failed to finish stop", checked);
            return;
        }
        if let Some(run) = self.runs.active.get(&live.agent).copied() {
            let recorded = self
                .store
                .set_effect_scope(run, EffectScope::Unobserved)
                .await;
            self.warn_failure("failed to record lost stream", recorded);
        }
        self.mark_needs_check(chat, live).await;
    }

    /// 실행 기록에서 입력을 찾을 수 있을 때만 결과 확인 필요로 둔다.
    async fn mark_needs_check(&mut self, chat: ChatId, live: &LiveSession) {
        let (Some(task), Some(input)) = (
            self.runs.task_of.get(&live.agent).copied(),
            self.run_input(live.agent).await,
        ) else {
            return;
        };
        self.flow.needs_check.insert(
            task,
            NeedsCheck {
                chat,
                agent: live.agent,
                input,
            },
        );
        self.notify_task(chat, task, TaskState::NeedsCheck, Some(live.provider), None)
            .await;
    }

    /// 에이전트의 진행 중인 실행을 연 입력.
    pub(crate) async fn run_input(&self, agent: AgentId) -> Option<InputId> {
        let run = self.runs.active.get(&agent).copied()?;
        let runs = self.store.unfinished_runs().await.ok()?;
        runs.into_iter()
            .find(|record| record.id == run)
            .and_then(|record| record.input)
    }

    /// 연결이 끝났다. 그 연결의 열린 session은 닫히고, 진행 중인 실행은 흐름이 끊긴 것으로 다룬다.
    pub(crate) async fn on_connection_closed(&mut self, chat: ChatId, provider: Provider) {
        self.providers.remove(&(chat, provider));
        self.flow.questions_of_connection.remove(&(chat, provider));
        self.flow.stale_connections.remove(&(chat, provider));
        if provider == Provider::Codex {
            self.flow.rules_of_connection.remove(&chat);
        }
        let lost: Vec<LiveSession> = self
            .flow
            .live
            .values()
            .filter(|live| live.provider == provider)
            .filter(|live| {
                self.session_chat(live.session)
                    .is_ok_and(|owner| owner == chat)
            })
            .cloned()
            .collect();
        for live in lost {
            let event = ProviderEvent::StreamLost { agent: live.agent };
            if let Err(error) = self.on_provider_event(provider, event).await {
                tracing::warn!(error = %self.failure_line(&error), "failed to handle closed connection");
                self.flow.live.remove(&live.agent);
            }
        }
    }

    /// Saturn 규칙이 `allow`나 `deny`로 판정하면 사용자에게 묻지 않고 바로 답한다. `ask`이거나 답이 provider에
    /// 닿지 않으면 TUI로 올린다.
    async fn on_permission_request(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        request: PermissionRequest<'_>,
    ) {
        let answer = match self.router_permission(chat, live.agent, request.call).await {
            Verdict::Allow => Some(PermissionAnswer::AllowOnce),
            Verdict::Deny => Some(PermissionAnswer::Deny { note: None }),
            Verdict::Ask => None,
        };
        if let Some(answer) = answer
            && self.answer_by_rule(chat, live, request.id, answer).await
        {
            return;
        }
        self.offer_permission(chat, live, request).await;
    }

    /// provider가 답을 받았으면 참. 받지 못했으면 사용자에게 물어야 하므로 거짓.
    #[expect(
        clippy::cognitive_complexity,
        reason = "성공과 실패 로그 매크로 둘이 점수를 올리고 흐름은 단순하다"
    )]
    async fn answer_by_rule(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> bool {
        let is_allow = answer != PermissionAnswer::Deny { note: None };
        match self.send_rule_answer(chat, live, request_id, answer).await {
            Ok(()) => {
                tracing::debug!(
                    request = request_id,
                    is_allow,
                    "permission answered by rule"
                );
                true
            }
            Err(error) => {
                tracing::warn!(request = request_id, %error, "rule answer not sent, asking the user");
                false
            }
        }
    }

    async fn send_rule_answer(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        self.provider_mut(chat, live.provider)?
            .answer_permission(&live.provider_session, request_id, answer)
            .await
    }

    /// 허가 요청을 기록 뒤에 TUI로 올린다. 답이 올 때까지 provider는 그 호출에서 멈춰 있고 작업 시계도 멈춘다.
    async fn offer_permission(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        request: PermissionRequest<'_>,
    ) {
        let PermissionRequest {
            id: request_id,
            summary,
            reason,
            call,
        } = request;
        let Some(task) = self.runs.task_of.get(&live.agent).copied() else {
            tracing::warn!(request = request_id, "permission request without a task");
            return;
        };
        let label = self.flow.tasks.assign(task);
        let waiting = self.waiting_in_chat(chat);
        let engine_request = self.flow.issue_request_id();
        self.flow.permissions.insert(
            engine_request.clone(),
            PendingPermission {
                chat,
                agent: live.agent,
                provider: live.provider,
                task,
                provider_request: request_id.to_owned(),
                call: call.cloned(),
            },
        );
        let request = Notification::PermissionRequested {
            task,
            label,
            provider: live.provider,
            request_id: engine_request,
            summary: summary.to_owned(),
            reason: reason.to_owned(),
            waiting,
        };
        self.rpc.offer_permission(chat, request).await;
        self.notify_task(
            chat,
            task,
            TaskState::AwaitingPermission,
            Some(live.provider),
            None,
        )
        .await;
    }

    /// 사용자 답을 provider 값으로 넘긴다. 다른 TUI의 창은 지운다. 규칙으로 읽은 호출의 `항상 허용`은 Saturn이
    /// 저장해 판정하므로 provider에는 이번만 허용으로 보낸다.
    ///
    /// # Errors
    /// 묻지 않은 요청이면 `UnexpectedAnswer`, 답한 TUI가 그 요청의 채팅에 붙어 있지 않으면 `ChatNotAttached`, 열린
    /// session이 없으면 `Provider(NotSent)`. provider가 받지 못했으면 요청을 그대로 두어 다시 답할 수 있다.
    pub(crate) async fn answer_permission(
        &mut self,
        client: ClientId,
        request_id: String,
        answer: PermissionAnswer,
    ) -> Result<(), EngineError> {
        let pending = self
            .flow
            .permissions
            .get(&request_id)
            .cloned()
            .ok_or(EngineError::UnexpectedAnswer { what: "permission" })?;
        self.require_attached(client, pending.chat)?;
        let live =
            self.flow
                .live
                .get(&pending.agent)
                .cloned()
                .ok_or_else(|| ProviderError::NotSent {
                    reason: "no open session for the permission request".to_owned(),
                })?;
        let is_saved_here = answer == PermissionAnswer::AllowAlways && pending.call.is_some();
        let sent = if is_saved_here {
            PermissionAnswer::AllowOnce
        } else {
            answer
        };
        self.provider_mut(pending.chat, pending.provider)?
            .answer_permission(&live.provider_session, &pending.provider_request, sent)
            .await?;
        self.flow.permissions.remove(&request_id);
        if let (true, Some(call)) = (is_saved_here, &pending.call) {
            self.save_always_allow(pending.chat, pending.agent, call)
                .await;
        }
        self.rpc.resolve_permission(client, &request_id).await;
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
        Ok(())
    }

    // cost: time O(p + i), heap O(1), stack O(1)
    // vars: p = 대기 허가 요청 수, i = 대기 입력 요청 수
    // basis: estimate
    /// 채팅에서 답을 기다리는 허가 요청과 입력 요청 수.
    pub(crate) fn waiting_in_chat(&self, chat: ChatId) -> u32 {
        let permissions = self
            .flow
            .permissions
            .values()
            .filter(|pending| pending.chat == chat)
            .count();
        let inputs = self
            .flow
            .inputs
            .values()
            .filter(|pending| pending.chat == chat)
            .count();
        u32::try_from(permissions + inputs).unwrap_or(u32::MAX)
    }

    // cost: time O(p + i), heap O(1), stack O(1)
    // vars: p = 대기 허가 요청 수, i = 대기 입력 요청 수
    // basis: estimate
    /// 작업이 아직 기다리는 것이 있으면 그 상태. 허가 요청이 먼저이고, 없으면 `None`.
    pub(crate) fn waiting_state(&self, task: TaskId) -> Option<TaskState> {
        if self
            .flow
            .permissions
            .values()
            .any(|pending| pending.task == task)
        {
            return Some(TaskState::AwaitingPermission);
        }
        self.flow
            .inputs
            .values()
            .any(|pending| pending.task == task)
            .then_some(TaskState::AwaitingInput)
    }

    /// 턴이 끝났거나 흐름이 끊겨 더는 답할 수 없는 허가 요청과 입력 요청의 창을 지운다.
    pub(crate) async fn clear_permissions(&mut self, agent: AgentId) {
        self.clear_inputs(agent).await;
        let ended: Vec<String> = self
            .flow
            .permissions
            .iter()
            .filter(|(_, pending)| pending.agent == agent)
            .map(|(request_id, _)| request_id.clone())
            .collect();
        for request_id in ended {
            self.flow.permissions.remove(&request_id);
            self.rpc.withdraw_permission(&request_id).await;
        }
    }
}

/// 입력 없이 provider가 시작한 턴도 첫 메인 출력으로 시작을 안다.
fn starts_turn(event: &ProviderEvent) -> bool {
    matches!(
        event,
        ProviderEvent::Text { subagent: None, .. } | ProviderEvent::ToolCall { subagent: None, .. }
    )
}

/// 이벤트가 속한 하위 에이전트. 메인 에이전트의 이벤트면 `None`.
fn event_subagent(event: &ProviderEvent) -> Option<&SubagentId> {
    match event {
        ProviderEvent::Text { subagent, .. }
        | ProviderEvent::ToolCall { subagent, .. }
        | ProviderEvent::ToolResult { subagent, .. } => subagent.as_ref(),
        ProviderEvent::SubagentStarted { subagent, .. }
        | ProviderEvent::SubagentEnded { subagent, .. }
        | ProviderEvent::SubagentInterrupted { subagent, .. } => Some(subagent),
        ProviderEvent::Usage(report) => report.subagent.as_ref(),
        _ => None,
    }
}
