//! 대기열 맨 앞 입력을 provider session에 보내고 결과를 입력과 작업 상태에 반영한다.
//! 설계: docs/design/input-handling.md, docs/design/providers-and-sessions.md

use saturn_core::providers::ProviderError;
use saturn_core::queue::{QueuedInput, SendAction};
use saturn_protocol::ids::{AgentId, ChatId, InputId, Provider, RunId, TaskId};
use saturn_protocol::rpc::{Alert, ChatNotice};
use saturn_protocol::state::{EffectScope, InputState, TaskState};

use crate::delivery::{DeliveryJob, Stage};
use crate::flow::{LiveSession, NeedsCheck};
use crate::providers::ProviderHandle;
use crate::store::{NewRun, RunEnd};
use crate::switch::PlanError;
use crate::{Engine, EngineError};

/// 초안. 같은 입력을 보내기 전에 확정된 실패(`NotSent`)로 시도하는 최대 횟수. 처음 시도를 포함한다.
pub(crate) const MAX_SEND_ATTEMPTS: u32 = 3;

/// 보내려는 입력이 어느 에이전트의 어떤 턴이 되는지.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Start {
    /// 진행 중인 턴에 끼워 넣는다.
    Steer(AgentId),
    /// 쉬는 에이전트에 새 턴으로 보낸다.
    Turn(AgentId),
    /// 새 작업을 시작한다.
    Task(TaskId),
}

/// 한 번의 전달이 지나온 단계. 실패하면 이 값으로 되돌릴 것을 정한다.
#[derive(Debug)]
pub(crate) struct Delivery {
    pub(crate) chat: ChatId,
    pub(crate) input: InputId,
    pub(crate) start: Start,
    pub(crate) task: TaskId,
    /// session을 연 뒤에만 있다.
    pub(crate) live: Option<LiveSession>,
    /// 작업을 `Running`으로 시작한 뒤에만 참.
    pub(crate) is_task_started: bool,
    pub(crate) run: Option<RunId>,
}

impl Engine {
    /// 보낼 것이 없을 때까지 반복한다.
    ///
    /// # Errors
    /// `deliver`와 같다. 오류가 나면 남은 입력은 다음 호출 때 보낸다.
    pub(crate) async fn dispatch_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        loop {
            // 앞선 전달이 provider 응답을 기다리는 채팅은 끝날 때까지 건너뛴다. 같은 채팅의 순서를 지키기 위해서다
            let busy: Vec<ChatId> = self.flow.deliveries.keys().copied().collect();
            let Some(action) = self.queue.next_to_send_except(&busy) else {
                break;
            };
            let input = match action {
                SendAction::Steer { input, .. }
                | SendAction::NewTurn { input, .. }
                | SendAction::NewTask { input, .. } => input,
            };
            let owner = self.queue.input(input).map_or(chat, |record| record.chat);
            self.deliver(owner, action).await?;
        }
        for input in std::mem::take(&mut self.flow.applied) {
            let is_waiting = self
                .queue
                .input(input)
                .is_some_and(|record| record.state == InputState::Queued);
            if is_waiting {
                self.notify_input(input).await;
            }
        }
        Ok(())
    }

    /// `NotSent`만 다시 보내고, `Unknown`이면 작업을 `NeedsCheck`로 두어 사용자 확인으로 넘긴다.
    ///
    /// # Errors
    /// 입력을 모르면 `Queue`. 보내기 실패는 입력 상태로 반영하고 오류로 돌려주지 않는다.
    pub(crate) async fn deliver(
        &mut self,
        chat: ChatId,
        action: SendAction,
    ) -> Result<(), EngineError> {
        match action {
            SendAction::Steer { input, agent } => self.deliver_steer(chat, input, agent).await,
            SendAction::NewTurn { input, agent } => {
                self.deliver_new(chat, input, Start::Turn(agent)).await
            }
            SendAction::NewTask { input, task } => {
                self.deliver_new(chat, input, Start::Task(task)).await
            }
        }
    }

    /// 끼워 넣기 실측 전 provider는 대기로 바꾸고, 그 밖에는 진행 중인 턴에 보낸다.
    async fn deliver_steer(
        &mut self,
        chat: ChatId,
        input: InputId,
        agent: AgentId,
    ) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        let live = self.flow.live.get(&agent).cloned();
        let Some(live) = live.filter(|live| live.steer_verified) else {
            let provider = self.flow.live.get(&agent).map(|live| live.provider);
            return self.defer_steer(chat, input, provider).await;
        };
        let mut delivery = self.delivery(&record, Start::Steer(agent))?;
        delivery.live = Some(live.clone());
        delivery.run = self.runs.active.get(&agent).copied();
        self.mark_delivering(&delivery).await?;
        let job = DeliveryJob::new(delivery, record);
        match self.provider_mut(chat, live.provider) {
            Ok(connection) => {
                connection.steer_detached(live.provider_session.clone(), job.record.text.clone());
                self.park(job, Stage::Steering { live });
                Ok(())
            }
            Err(error) => self.settle(job.delivery, Err(error)).await,
        }
    }

    /// provider가 끼워 넣기를 거절했다(보내지 않음이 확정). 다시 끼워 넣지 않고 입력을 대기열 맨 앞으로 되돌려
    /// 현재 작업이 끝난 뒤 다음 차례에 보낸다. 입력을 거절로 끝내지 않는다([#60](https://github.com/woonyong-choi/saturn/issues/60) 결정).
    /// 충돌 입력이면 큐가 `ConfirmStop` 사유를 달아 알림이 사용자에게 멈추고 실행할지 묻는다([#36](https://github.com/woonyong-choi/saturn/issues/36) 결정).
    pub(crate) async fn return_refused_steer(
        &mut self,
        delivery: &Delivery,
        reason: &str,
    ) -> Result<(), EngineError> {
        let input = delivery.input;
        tracing::warn!(input = input.0, %reason, "steer was refused, moving the input to the front of the queue");
        self.queue.return_refused_steer(input)?;
        let written = self
            .store
            .set_input_state(input, InputState::Queued, None)
            .await;
        self.warn_failure("failed to record the returned input", written);
        self.notify_input(input).await;
        Ok(())
    }

    /// 끼워 넣기를 대기로 바꾸고 TUI에 `바로 반영 준비 중`을 보인다.
    async fn defer_steer(
        &mut self,
        chat: ChatId,
        input: InputId,
        provider: Option<Provider>,
    ) -> Result<(), EngineError> {
        let is_asking = self.queue.defer_steer(input)?;
        if is_asking {
            self.notify_input(input).await;
        } else if let Some(provider) = provider {
            self.notify_alert(chat, Alert::SteerNotReady { provider })
                .await;
        }
        self.flow.applied.push(input);
        Ok(())
    }

    async fn deliver_new(
        &mut self,
        chat: ChatId,
        input: InputId,
        start: Start,
    ) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        let delivery = self.delivery(&record, start)?;
        let plan = match self.plan_open(&record, start).await {
            Err(PlanError::Deferred(constraints)) => {
                let notice = ChatNotice::ContextDeferred { constraints };
                return self.hold_for_context(&delivery, notice).await;
            }
            Err(PlanError::Failed(reason)) => Err(reason),
            Ok(plan) => Ok(plan),
        };
        if let Err(error) = self.mark_delivering(&delivery).await {
            self.release_task(&delivery);
            return Err(error);
        }
        let job = DeliveryJob::new(delivery, record);
        match plan {
            Ok(plan) => self.start_open(job, plan).await,
            Err(reason) => self.reject(job.delivery, reason).await,
        }
    }

    /// 패킷의 고정 구역이 `P_hard`도 넘거나 줄인 패킷도 맥락 한도로 거절되면 보내지 않는다. 입력은 작업과 함께 보류하고
    /// `notice`를 보인다. 사용자가 `/continue`로 다시 시도한다.
    pub(crate) async fn hold_for_context(
        &mut self,
        delivery: &Delivery,
        notice: ChatNotice,
    ) -> Result<(), EngineError> {
        self.queue.hold_unsent(delivery.input)?;
        self.store
            .set_input_state(delivery.input, InputState::Held, None)
            .await?;
        self.notify_input(delivery.input).await;
        self.notify_chat(delivery.chat, notice).await;
        self.notify_task(delivery.chat, delivery.task, TaskState::Held, None, None)
            .await;
        Ok(())
    }

    fn delivery(&mut self, record: &QueuedInput, start: Start) -> Result<Delivery, EngineError> {
        let task = match start {
            Start::Task(task) => task,
            Start::Steer(_) | Start::Turn(_) => record
                .task
                .ok_or(saturn_core::queue::QueueError::NotFound(record.id))?,
        };
        self.flow.tasks.assign(task);
        Ok(Delivery {
            chat: record.chat,
            input: record.id,
            start,
            task,
            live: None,
            is_task_started: matches!(start, Start::Steer(_) | Start::Turn(_)),
            run: None,
        })
    }

    /// 새 작업이면 열린 session의 에이전트로 작업을 시작하고, 새 턴이면 같은 에이전트인지 확인한다.
    pub(crate) fn begin_task(
        &mut self,
        delivery: &mut Delivery,
        live: &LiveSession,
    ) -> Result<(), String> {
        match delivery.start {
            Start::Task(task) => {
                self.queue
                    .start_task(task, live.agent)
                    .map_err(|error| self.failure_line(&error))?;
                delivery.is_task_started = true;
            }
            Start::Turn(agent) if agent != live.agent => {
                return Err("session belongs to another agent".to_owned());
            }
            Start::Turn(_) | Start::Steer(_) => {}
        }
        Ok(())
    }

    /// 기록 저장소에 `Delivering`을 먼저 쓰고, 그 뒤에만 provider로 보낸다. 쓰기에 실패하면 보내지 않고 거절한다.
    pub(crate) async fn mark_delivering(&mut self, delivery: &Delivery) -> Result<(), EngineError> {
        let input = delivery.input;
        self.queue.set_state(input, InputState::Delivering)?;
        let written = self
            .store
            .set_input_state(input, InputState::Delivering, None)
            .await;
        if let Err(error) = written {
            self.queue.set_state(input, InputState::Rejected)?;
            self.notify_input(input).await;
            return Err(error.into());
        }
        self.notify_input(input).await;
        Ok(())
    }

    /// 턴 시작은 provider에 보내기 전에 기록한다.
    pub(crate) async fn begin_run(
        &mut self,
        chat: ChatId,
        input: InputId,
        task: TaskId,
        live: &LiveSession,
    ) -> Result<RunId, EngineError> {
        let run = self
            .store
            .start_run(&NewRun {
                input: Some(input),
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
        self.flow.last_task.insert(live.agent, task);
        if let Some(record) = self.queue.input(input) {
            self.flow.settings_of.insert(live.agent, record.settings);
        }
        self.sessions.mark_busy(live.session);
        Ok(run)
    }

    pub(crate) fn provider_mut(
        &mut self,
        chat: ChatId,
        provider: Provider,
    ) -> Result<&mut ProviderHandle, ProviderError> {
        self.providers
            .get_mut(&(chat, provider))
            .ok_or_else(|| ProviderError::NotSent {
                reason: "provider is not connected".to_owned(),
            })
    }

    /// 보낸 결과를 입력과 작업 상태에 반영한다.
    pub(crate) async fn settle(
        &mut self,
        delivery: Delivery,
        result: Result<(), ProviderError>,
    ) -> Result<(), EngineError> {
        match result {
            Ok(()) => self.mark_applied(delivery).await,
            Err(ProviderError::Unknown) => {
                self.needs_check(&delivery).await;
                Ok(())
            }
            Err(error) => {
                let reason = self.failure_line(&error);
                self.reject(delivery, reason).await
            }
        }
    }

    /// provider가 받았다. 작업 끝은 `finish_task`가 알린다.
    pub(crate) async fn mark_applied(&mut self, delivery: Delivery) -> Result<(), EngineError> {
        self.record_applied(&delivery).await?;
        if !matches!(delivery.start, Start::Steer(_)) {
            let provider = delivery.live.as_ref().map(|live| live.provider);
            self.notify_task(
                delivery.chat,
                delivery.task,
                TaskState::Running,
                provider,
                None,
            )
            .await;
        }
        Ok(())
    }

    /// 입력을 받은 것으로 기록하고 알린다. 작업 상태는 건드리지 않는다.
    pub(crate) async fn record_applied(&mut self, delivery: &Delivery) -> Result<(), EngineError> {
        let input = delivery.input;
        self.queue.set_state(input, InputState::Applied)?;
        self.store
            .set_input_state(input, InputState::Applied, None)
            .await?;
        self.notify_input(input).await;
        Ok(())
    }

    /// 보낸 뒤 결과를 모른다. 다시 보내지 않고 입력은 `Delivering`으로 두며 실행 기록도 열어 둔다.
    /// 사용자는 `/continue <작업>`으로 확인 입력을 보내 이어 간다(`continue_held`).
    async fn needs_check(&mut self, delivery: &Delivery) {
        tracing::warn!(
            input = delivery.input.0,
            "turn result is unknown, needs check"
        );
        let provider = delivery.live.as_ref().map(|live| live.provider);
        if let Some(live) = &delivery.live {
            self.flow.needs_check.insert(
                delivery.task,
                NeedsCheck {
                    chat: delivery.chat,
                    agent: live.agent,
                    input: delivery.input,
                },
            );
        }
        self.notify_task(
            delivery.chat,
            delivery.task,
            TaskState::NeedsCheck,
            provider,
            None,
        )
        .await;
    }

    /// 보내기 전에 확정된 실패가 끝내 이어졌다. 입력은 `Rejected`, 시작하려던 작업은 닫는다.
    pub(crate) async fn reject(
        &mut self,
        delivery: Delivery,
        reason: String,
    ) -> Result<(), EngineError> {
        tracing::warn!(input = delivery.input.0, %reason, "input was not delivered");
        self.queue.set_state(delivery.input, InputState::Rejected)?;
        let written = self
            .store
            .set_input_state(delivery.input, InputState::Rejected, None)
            .await;
        self.warn_failure("failed to record rejected input", written);
        self.end_failed_run(&delivery).await;
        self.release_task(&delivery);
        self.notify_input(delivery.input).await;
        let provider = delivery.live.as_ref().map(|live| live.provider);
        self.notify_task(
            delivery.chat,
            delivery.task,
            TaskState::Failed,
            provider,
            Some(reason),
        )
        .await;
        if matches!(delivery.start, Start::Task(_)) {
            self.flow.tasks.release(delivery.task);
        }
        Ok(())
    }

    async fn end_failed_run(&mut self, delivery: &Delivery) {
        let Some(run) = delivery.run else {
            return;
        };
        if let Some(live) = &delivery.live {
            self.runs.forget(live.agent);
        }
        let ended = self.store.finish_run(run, RunEnd::Failed).await;
        self.warn_failure("failed to end run", ended);
    }

    /// 시작하지 못한 새 작업은 닫고, 에이전트가 붙은 작업은 끝내 쓰기 잠금을 푼다.
    pub(crate) fn release_task(&mut self, delivery: &Delivery) {
        match (delivery.start, &delivery.live) {
            (Start::Steer(_), _) => {}
            (Start::Turn(agent), _) => self.queue.finish_task(agent),
            (Start::Task(_), Some(live)) if delivery.is_task_started => {
                self.queue.finish_task(live.agent);
            }
            (Start::Task(task), _) => self.queue.abandon_task(task),
        }
    }

    /// 작업 끝을 대기열에 알리고, 그 작업이 쥐고 있던 쓰기 차례를 기다리던 입력을 이어서 보낸다.
    ///
    /// # Errors
    /// 실행 기록을 끝내지 못하면 `Store`, 이어 보내는 중의 오류는 `dispatch_next`와 같다.
    #[cfg(test)]
    pub(crate) async fn finish_task(
        &mut self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<(), EngineError> {
        self.end_task(chat, agent).await?;
        self.dispatch_next(chat).await
    }

    /// `finish_task`에서 이어 보내기만 뺀 것. 턴 끝이 맥락 정리를 판정한 뒤에 이어 보낼 때 쓴다.
    ///
    /// # Errors
    /// 실행 기록을 끝내지 못하면 `Store`.
    pub(crate) async fn end_task(
        &mut self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<(), EngineError> {
        self.queue.finish_task(agent);
        let task = self.runs.task_of.remove(&agent);
        self.runs.chat_of.remove(&agent);
        if let Some(run) = self.runs.active.remove(&agent) {
            self.store.finish_run(run, RunEnd::Completed).await?;
        }
        if let Some(task) = task {
            let provider = self.flow.live.get(&agent).map(|live| live.provider);
            self.notify_task(chat, task, TaskState::Done, provider, None)
                .await;
            self.flow.tasks.release(task);
        }
        Ok(())
    }

    /// 로그와 알림에 남길 원인 한 줄. router 키와 같은 문자열은 가린다.
    pub(crate) fn failure_line(&self, error: &dyn std::error::Error) -> String {
        crate::masked_chain(&self.masker, error)
    }

    /// 흐름을 이어 가는 실패를 원인 한 줄과 함께 경고로 남긴다.
    pub(crate) fn warn_failure<T>(&self, what: &str, result: Result<T, impl std::error::Error>) {
        if let Err(error) = result {
            tracing::warn!(error = %self.failure_line(&error), "{what}");
        }
    }
}

pub(crate) fn into_provider_error(error: EngineError) -> ProviderError {
    match error {
        EngineError::Provider(error) => error,
        other => ProviderError::NotSent {
            reason: other.to_string(),
        },
    }
}
