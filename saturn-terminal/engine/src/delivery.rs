//! 입력 전달 중의 provider 요청: 요청은 연결 작업이 실행하고, engine 루프는 결과 메시지가 올 때마다 다음 단계를 잇는다.
//! 채팅마다 전달 하나만 진행해 같은 채팅의 요청 순서를 지키고, 다른 채팅의 입력과 조회, 멈춤 요청은 기다리지 않는다.
//! 설계: docs/design/providers-and-sessions.md#provider-요청-작업

use saturn_core::providers::ProviderError;
use saturn_core::queue::QueuedInput;
use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::InputState;

use crate::dispatch::{Delivery, MAX_SEND_ATTEMPTS, into_provider_error};
use crate::events::Arrival;
use crate::flow::LiveSession;
use crate::launch::ConnectionSeed;
use crate::providers::{
    Connected, ConnectionId, ProviderHandle, ProviderMsg, Reply, spawn_connect,
};
use crate::store::RunEnd;
use crate::switch::{OpenCall, OpenPlan, OpenPrep};
use crate::{Engine, EngineError};

use saturn_core::providers::SessionHandle;
use saturn_protocol::ids::RunId;

/// provider 응답을 기다리는 전달 하나.
#[derive(Debug)]
pub(crate) struct DeliveryJob {
    pub(crate) delivery: Delivery,
    pub(crate) record: QueuedInput,
    /// 보낼 때 채팅이 가진 더한 폴더. 쓰기 잠금 범위와 같은 시점의 값이라 기다리는 동안 더한 폴더는 다음 session부터 받는다.
    pub(crate) add_dirs: Vec<std::path::PathBuf>,
    /// 기다리는 동안 채팅이 멈췄다. 결과가 와도 더 보내지 않는다.
    pub(crate) is_stopped: bool,
}

impl DeliveryJob {
    pub(crate) fn new(
        delivery: Delivery,
        record: QueuedInput,
        add_dirs: Vec<std::path::PathBuf>,
    ) -> Self {
        Self {
            delivery,
            record,
            add_dirs,
            is_stopped: false,
        }
    }
}

/// 전달이 기다리는 요청.
#[derive(Debug)]
pub(crate) enum Stage {
    Connecting {
        plan: OpenPlan,
        seed: ConnectionSeed,
    },
    /// session 열기.
    Opening(OpenPrep),
    /// 이미 열린 session에 변경분 턴 보내기.
    Handoff(OpenPrep),
    /// 새 턴 보내기.
    Sending,
    Steering {
        live: LiveSession,
    },
    /// 끼워 넣기가 활성 턴 없음으로 거절돼 같은 session에 새 턴으로 보낸다. `run`은 그 턴의 실행이다.
    SteerFallback {
        live: LiveSession,
        run: RunId,
    },
}

impl Stage {
    /// 이 요청을 맡은 연결의 provider. 보내는 단계인데 열린 session이 없으면 `None`.
    fn provider(&self, job: &DeliveryJob) -> Option<Provider> {
        match self {
            Self::Connecting { plan, .. } => Some(plan.provider()),
            Self::Opening(prep) | Self::Handoff(prep) => Some(prep.provider()),
            Self::Steering { live } | Self::SteerFallback { live, .. } => Some(live.provider),
            Self::Sending => job.delivery.live.as_ref().map(|live| live.provider),
        }
    }

    /// 연결 작업이 끝나 이 요청의 결과가 오지 않을 때 대신 쓰는 결과. 보내기 전 단계는 `ConnectionLost`, 보낸 뒤
    /// 단계는 결과를 모르는 `Unknown`이다.
    fn lost_reply(&self) -> Reply {
        match self {
            Self::Connecting { .. } => Reply::Connected(Err(ProviderError::ConnectionLost)),
            Self::Opening(_) => Reply::Opened(Err(ProviderError::ConnectionLost)),
            Self::Handoff(_) => Reply::HandoffSent(Err(ProviderError::ConnectionLost)),
            Self::Sending | Self::SteerFallback { .. } => Reply::Sent(Err(ProviderError::Unknown)),
            Self::Steering { .. } => Reply::Steered(Err(ProviderError::Unknown)),
        }
    }
}

/// 채팅의 전달과 그것이 기다리는 요청.
#[derive(Debug)]
pub(crate) struct Parked {
    pub(crate) job: DeliveryJob,
    stage: Stage,
    /// 요청을 맡긴 연결의 번호. 연결을 맺는 중이면 아직 없다. 이 연결의 응답과 끊김만 이 전달을 이어 간다.
    connection: Option<ConnectionId>,
}

impl Engine {
    /// 요청을 맡기고 기다리지 않는 전달을 채팅에 걸어 둔다.
    pub(crate) fn park(&mut self, job: DeliveryJob, stage: Stage) {
        let chat = job.record.chat;
        let connection = match &stage {
            Stage::Connecting { .. } => None,
            _ => stage
                .provider(&job)
                .and_then(|provider| self.providers.get(&(chat, provider)))
                .map(ProviderHandle::id),
        };
        self.flow.deliveries.insert(
            chat,
            Parked {
                job,
                stage,
                connection,
            },
        );
    }

    /// 지금 이 채팅과 provider의 연결이 `connection`이면 참. 교체되거나 끝난 연결의 늦은 메시지를 가른다.
    fn is_current(&self, chat: ChatId, provider: Provider, connection: ConnectionId) -> bool {
        self.providers
            .get(&(chat, provider))
            .is_some_and(|handle| handle.id() == connection)
    }

    /// 이벤트, 명령 목록, 종료는 지금 연결이 보낸 것만 적용한다. 응답과 끊김은 맡긴 연결의 것인지 따로 가린다.
    fn is_from_current(&self, message: &ProviderMsg) -> bool {
        match message {
            ProviderMsg::Event {
                chat,
                provider,
                connection,
                ..
            }
            | ProviderMsg::Commands {
                chat,
                provider,
                connection,
                ..
            }
            | ProviderMsg::Closed {
                chat,
                provider,
                connection,
            } => self.is_current(*chat, *provider, *connection),
            ProviderMsg::Reply { .. } | ProviderMsg::Lost { .. } | ProviderMsg::Raw { .. } => true,
        }
    }

    /// 연결 작업이 보낸 메시지를 처리한다.
    pub(crate) async fn on_provider_msg(&mut self, message: ProviderMsg) {
        match message {
            ProviderMsg::Raw {
                chat,
                provider,
                raw,
            } => self.on_provider_raw(chat, provider, raw).await,
            other => self.on_connection_msg(other).await,
        }
    }

    /// 연결의 이벤트, 명령 목록, 종료, 응답, 끊김.
    async fn on_connection_msg(&mut self, message: ProviderMsg) {
        if !self.is_from_current(&message) {
            tracing::debug!("message from a replaced connection dropped");
            return;
        }
        match message {
            ProviderMsg::Event {
                chat,
                provider,
                event,
                ..
            } => {
                let arrival = Arrival {
                    chat,
                    provider,
                    event: Some(event),
                };
                self.on_arrival(arrival).await;
            }
            ProviderMsg::Raw { .. } => {}
            ProviderMsg::Commands {
                chat,
                provider,
                commands,
                ..
            } => self.on_commands(chat, provider, commands).await,
            ProviderMsg::Closed { chat, provider, .. } => {
                let arrival = Arrival {
                    chat,
                    provider,
                    event: None,
                };
                self.on_arrival(arrival).await;
            }
            ProviderMsg::Reply {
                chat,
                provider,
                connection,
                reply,
            } => self.on_reply(chat, provider, connection, reply).await,
            ProviderMsg::Lost {
                chat,
                provider,
                connection,
            } => self.on_task_lost(chat, provider, connection).await,
        }
    }

    /// 연결 작업이 끝나 맡긴 요청의 결과가 오지 않는다. 입력 전달이 아닌 요청은 연결 끊김으로 끝낸다. 기다리던 전달은 그 요청이 실패한 것으로 이어 가고(보내기 전
    /// 단계는 연결 끊김으로 거절, 보낸 뒤 단계는 결과를 모르는 것으로 `NeedsCheck`), 연결은 끊긴 것으로 처리한다.
    /// 끝난 연결이 이미 교체됐으면 새 연결은 그대로 두고, 그 연결에 맡겼던 전달과 요청만 끝낸다.
    async fn on_task_lost(&mut self, chat: ChatId, provider: Provider, connection: ConnectionId) {
        let waiting = self.flow.deliveries.get(&chat).is_some_and(|parked| {
            parked.connection == Some(connection)
                && parked.stage.provider(&parked.job) == Some(provider)
        });
        if waiting && let Some(Parked { job, stage, .. }) = self.flow.deliveries.remove(&chat) {
            let reply = stage.lost_reply();
            let advanced = self
                .advance_delivery(chat, provider, job, stage, reply)
                .await;
            self.warn_failure("delivery step failed", advanced);
        }
        if self.is_current(chat, provider, connection) {
            self.on_connection_closed(chat, provider).await;
        }
        self.on_calls_lost(chat, provider, Some(connection)).await;
        self.resume_chat(chat).await;
    }

    /// 응답이 그 요청이나 전달을 맡긴 연결에서 온 것인지. 다른 연결의 응답은 대기 중인 전달과 요청을 건드리지 못한다.
    /// `connection`이 없으면 연결을 맺는 요청의 결과다.
    fn is_expected_reply(
        &self,
        chat: ChatId,
        connection: Option<ConnectionId>,
        reply: &Reply,
    ) -> bool {
        let expected = match reply {
            Reply::Call { tag, .. } => self.flow.calls.get(tag).map(|call| call.connection),
            _ => self
                .flow
                .deliveries
                .get(&chat)
                .map(|parked| parked.connection),
        };
        let is_expected = expected.is_none_or(|sent_to| sent_to == connection);
        if !is_expected {
            tracing::warn!(chat = chat.0, "reply from another connection dropped");
        }
        is_expected
    }

    async fn on_reply(
        &mut self,
        chat: ChatId,
        provider: Provider,
        connection: Option<ConnectionId>,
        reply: Reply,
    ) {
        if !self.is_expected_reply(chat, connection, &reply) {
            return;
        }
        if let Reply::Call { tag, result } = reply {
            self.on_call_reply(tag, result).await;
            return;
        }
        let Some(Parked { job, stage, .. }) = self.flow.deliveries.remove(&chat) else {
            tracing::warn!(chat = chat.0, "provider reply without a waiting delivery");
            return;
        };
        let advanced = self
            .advance_delivery(chat, provider, job, stage, reply)
            .await;
        self.warn_failure("delivery step failed", advanced);
        self.resume_chat(chat).await;
    }

    /// 전달이 끝나 채팅이 비었으면 기다리던 다음 입력을 보낸다.
    async fn resume_chat(&mut self, chat: ChatId) {
        if self.flow.deliveries.contains_key(&chat) {
            return;
        }
        let next = self.dispatch_next(chat).await;
        self.warn_failure("failed to send the next input", next);
    }

    async fn advance_delivery(
        &mut self,
        chat: ChatId,
        provider: Provider,
        job: DeliveryJob,
        stage: Stage,
        reply: Reply,
    ) -> Result<(), EngineError> {
        match (stage, reply) {
            (Stage::Connecting { plan, seed }, Reply::Connected(connected)) => {
                self.on_connected(chat, provider, job, (plan, seed), connected)
                    .await
            }
            (Stage::Opening(prep), Reply::Opened(result)) => {
                self.on_opened(job, prep, result.map(Some)).await
            }
            (Stage::Handoff(prep), Reply::HandoffSent(result)) => {
                self.on_opened(job, prep, result.map(|()| None)).await
            }
            (Stage::Sending, Reply::Sent(result)) => self.on_sent(job, result).await,
            (Stage::Steering { live }, Reply::Steered(result)) => {
                self.on_steered(job, live, result).await
            }
            (Stage::SteerFallback { live, run }, Reply::Sent(result)) => {
                self.on_fallback_sent(job, (live, run), result).await
            }
            (stage, reply) => {
                tracing::warn!(?stage, ?reply, "provider reply does not match the delivery");
                Ok(())
            }
        }
    }

    async fn on_connected(
        &mut self,
        chat: ChatId,
        provider: Provider,
        job: DeliveryJob,
        (plan, seed): (OpenPlan, ConnectionSeed),
        connected: Result<Box<Connected>, ProviderError>,
    ) -> Result<(), EngineError> {
        let connected = match connected {
            Ok(connected) => *connected,
            Err(error) => return self.fail_open(job, error.into()).await,
        };
        let Connected { connection, models } = connected;
        // 그사이 `/model` 목록이 같은 연결을 먼저 맺었으면 그 연결을 쓴다
        if !self.providers.contains_key(&(chat, provider)) {
            self.attach_connection(chat, connection, seed);
            self.apply_models(provider, chat, models);
        }
        if job.is_stopped {
            return self.hold_stopped(job).await;
        }
        self.open_with_plan(job, plan).await
    }

    async fn on_sent(
        &mut self,
        job: DeliveryJob,
        result: Result<(), ProviderError>,
    ) -> Result<(), EngineError> {
        if job.is_stopped {
            return self.settle_stopped(job.delivery, result).await;
        }
        self.settle(job.delivery, result).await
    }

    async fn on_steered(
        &mut self,
        job: DeliveryJob,
        live: LiveSession,
        result: Result<(), ProviderError>,
    ) -> Result<(), EngineError> {
        match result {
            Err(ProviderError::NoActiveTurn) => self.steer_as_new_turn(job, live).await,
            Err(ProviderError::NotSent { reason }) => {
                self.return_refused_steer(&job.delivery, &reason).await
            }
            Ok(()) if job.delivery.run.is_some() => {
                // 끼워 넣어 적용한 입력은 들어간 실행에 묶어 둬야 인계 패킷이 사용자 입력으로 읽는다
                self.record_steered(&job.delivery).await
            }
            other => self.settle(job.delivery, other).await,
        }
    }

    async fn on_fallback_sent(
        &mut self,
        job: DeliveryJob,
        (live, run): (LiveSession, RunId),
        result: Result<(), ProviderError>,
    ) -> Result<(), EngineError> {
        if let Err(error) = &result
            && !matches!(error, ProviderError::Unknown)
        {
            self.runs.forget(live.agent);
            self.flow.baselines.remove(&run);
            let ended = self.store.finish_run(run, RunEnd::Failed).await;
            self.warn_failure("failed to end fallback run", ended);
        }
        self.settle(job.delivery, result).await
    }

    /// 새 session을 열거나 변경분을 보내는 요청을 맡긴다. 연결이 없으면 먼저 연결을 맡긴다.
    pub(crate) async fn start_open(
        &mut self,
        job: DeliveryJob,
        plan: OpenPlan,
    ) -> Result<(), EngineError> {
        let (chat, provider) = (job.record.chat, plan.provider());
        if self.providers.contains_key(&(chat, provider)) {
            return self.open_with_plan(job, plan).await;
        }
        match self.launch_spec(provider, chat, job.record.settings).await {
            Ok(launch) => {
                let seed = ConnectionSeed::of(&launch);
                let Some(adapter) = self.registry.get(provider).cloned() else {
                    return self
                        .fail_open(job, ProviderError::ConnectionLost.into())
                        .await;
                };
                spawn_connect(
                    chat,
                    launch,
                    adapter,
                    self.supervisor.clone(),
                    self.flow.provider_tx.clone(),
                    None,
                );
                self.park(job, Stage::Connecting { plan, seed });
                Ok(())
            }
            Err(error) => self.fail_open(job, error).await,
        }
    }

    async fn open_with_plan(
        &mut self,
        job: DeliveryJob,
        plan: OpenPlan,
    ) -> Result<(), EngineError> {
        match self
            .prepare_open(&job.record, job.add_dirs.clone(), plan)
            .await
        {
            Ok(prep) => self.run_open(job, prep).await,
            Err(error) => self.fail_open(job, error).await,
        }
    }

    async fn run_open(&mut self, job: DeliveryJob, mut prep: OpenPrep) -> Result<(), EngineError> {
        let chat = job.record.chat;
        if let Some((target, body, evidence)) = prep.packet_to_send(&job.record) {
            if !evidence.carries_dialogue(body) {
                let error = ProviderError::NotSent {
                    reason: "packet does not carry the recorded dialogue".to_owned(),
                };
                return self.fail_open(job, error.into()).await;
            }
            prep.packet = self.record_packet_attempt(target, body, evidence).await;
        }
        match prep.call() {
            OpenCall::None => self.complete_open(job, prep, None).await,
            OpenCall::Handoff(live, text) => match self.provider_mut(chat, live.provider) {
                Ok(connection) => {
                    connection.send_handoff_detached(live.provider_session, text);
                    self.park(job, Stage::Handoff(prep));
                    Ok(())
                }
                Err(error) => self.fail_open(job, error.into()).await,
            },
            OpenCall::Open(provider, spec) => match self.provider_mut(chat, provider) {
                Ok(connection) => {
                    connection.open_session_detached(spec, MAX_SEND_ATTEMPTS);
                    self.park(job, Stage::Opening(prep));
                    Ok(())
                }
                Err(error) => self.fail_open(job, error.into()).await,
            },
        }
    }

    /// session 열기나 변경분 전송의 결과. 맥락 한도 초과는 패킷을 줄여 한 번만 다시 연다.
    async fn on_opened(
        &mut self,
        job: DeliveryJob,
        prep: OpenPrep,
        result: Result<Option<SessionHandle>, ProviderError>,
    ) -> Result<(), EngineError> {
        let session = result
            .as_ref()
            .ok()
            .and_then(|opened| prep.provider_session_of(opened));
        self.settle_packet(prep.packet, &result, session).await;
        if job.is_stopped {
            return self.opened_after_stop(job, prep, result).await;
        }
        match result {
            Ok(opened) => self.complete_open(job, prep, opened).await,
            Err(ProviderError::ContextExceeded { limit_tokens }) => {
                self.reopen_reduced(job, prep, limit_tokens).await
            }
            Err(error) => self.fail_open(job, error.into()).await,
        }
    }

    /// 패킷이 맥락 한도로 거절됐다. 줄일 수 있으면 줄인 패킷으로 다시 열고, 아니면 보류한다.
    async fn reopen_reduced(
        &mut self,
        job: DeliveryJob,
        prep: OpenPrep,
        limit_tokens: Option<u64>,
    ) -> Result<(), EngineError> {
        match prep.reduced_plan(limit_tokens) {
            Some(reduced) => {
                tracing::warn!(
                    chat = job.record.chat.0,
                    "packet was over the context limit, sending a reduced one"
                );
                self.open_with_plan(job, reduced).await
            }
            None => {
                let error = ProviderError::ContextExceeded { limit_tokens };
                self.fail_open(job, error.into()).await
            }
        }
    }

    /// 기다리는 동안 채팅이 멈췄다. 연 session은 쓰지 않고 닫고, 입력은 보류한다.
    async fn opened_after_stop(
        &mut self,
        job: DeliveryJob,
        prep: OpenPrep,
        result: Result<Option<SessionHandle>, ProviderError>,
    ) -> Result<(), EngineError> {
        if let Ok(Some(handle)) = &result {
            self.close_unregistered(job.record.chat, prep.provider(), handle);
        }
        self.hold_stopped(job).await
    }

    async fn complete_open(
        &mut self,
        job: DeliveryJob,
        prep: OpenPrep,
        opened: Option<SessionHandle>,
    ) -> Result<(), EngineError> {
        match self.finish_open(&job.record, prep, opened).await {
            Ok(live) => self.begin_turn(job, live).await,
            Err(error) => self.fail_open(job, error).await,
        }
    }

    /// session이 열렸다. 작업과 실행을 기록한 뒤에 턴 전송을 맡긴다.
    async fn begin_turn(
        &mut self,
        mut job: DeliveryJob,
        live: LiveSession,
    ) -> Result<(), EngineError> {
        let chat = job.record.chat;
        job.delivery.live = Some(live.clone());
        let text = match self.input_with_recalled_context(&job.record, &live).await {
            Ok(text) => text,
            Err(error) => {
                let reason = self.failure_line(&error);
                return self.reject(job.delivery, reason).await;
            }
        };
        if let Err(reason) = self.begin_task(&mut job.delivery, &live) {
            return self.reject(job.delivery, reason).await;
        }
        let run = self
            .begin_run(chat, job.record.id, job.delivery.task, &live)
            .await;
        match run {
            Ok(run) => {
                job.delivery.run = Some(run);
                let linked = self.store.link_packet_run(job.record.id, run).await;
                self.warn_failure("failed to link a handoff packet to its run", linked);
            }
            Err(error) => {
                let reason = self.failure_line(&error);
                return self.reject(job.delivery, reason).await;
            }
        }
        match self.provider_mut(chat, live.provider) {
            Ok(connection) => {
                connection.send_turn_detached(live.provider_session, text, MAX_SEND_ATTEMPTS);
                self.park(job, Stage::Sending);
                Ok(())
            }
            Err(error) => self.settle(job.delivery, Err(error)).await,
        }
    }

    /// 활성 턴 없음은 확정 미전달이라 다시 판단하지 않고 같은 session에 새 턴으로 한 번 보낸다.
    async fn steer_as_new_turn(
        &mut self,
        job: DeliveryJob,
        live: LiveSession,
    ) -> Result<(), EngineError> {
        if job.is_stopped {
            return self.hold_stopped(job).await;
        }
        let chat = job.record.chat;
        let begun = self.begin_fallback_run(&job.record, &live).await;
        let run = match begun {
            Ok(run) => run,
            Err(error) => {
                return self
                    .settle(job.delivery, Err(into_provider_error(error)))
                    .await;
            }
        };
        match self.provider_mut(chat, live.provider) {
            Ok(connection) => {
                connection.send_turn_detached(
                    live.provider_session.clone(),
                    job.record.text.clone(),
                    1,
                );
                self.park(job, Stage::SteerFallback { live, run });
                Ok(())
            }
            Err(error) => {
                self.runs.forget(live.agent);
                self.flow.baselines.remove(&run);
                let ended = self.store.finish_run(run, RunEnd::Failed).await;
                self.warn_failure("failed to end fallback run", ended);
                self.settle(job.delivery, Err(error)).await
            }
        }
    }

    async fn begin_fallback_run(
        &mut self,
        record: &QueuedInput,
        live: &LiveSession,
    ) -> Result<RunId, EngineError> {
        let task = record
            .task
            .ok_or(saturn_core::queue::QueueError::NotFound(record.id))?;
        if let Some(previous) = self.runs.active.remove(&live.agent) {
            self.store.finish_run(previous, RunEnd::Completed).await?;
        }
        self.begin_run(record.chat, record.id, task, live).await
    }

    /// 열기를 끝내지 못했다. 맥락 한도 초과는 보류하고 그 밖은 거절한다. 멈춘 채팅이면 보류한다.
    async fn fail_open(&mut self, job: DeliveryJob, error: EngineError) -> Result<(), EngineError> {
        if job.is_stopped {
            return self.hold_stopped(job).await;
        }
        if matches!(
            error,
            EngineError::Provider(ProviderError::ContextExceeded { .. })
        ) {
            return self
                .hold_for_context(&job.delivery, ChatNotice::PacketOverflow)
                .await;
        }
        let reason = self.failure_line(&error);
        self.reject(job.delivery, reason).await
    }

    /// 멈춘 채팅의 전달은 provider에 보내지 않음이 확정이라 입력을 보류한다. 다시 이으면 처음부터 보낸다.
    async fn hold_stopped(&mut self, job: DeliveryJob) -> Result<(), EngineError> {
        let input = job.delivery.input;
        self.queue.hold_unsent(input)?;
        self.store
            .set_input_state(input, InputState::Held, None)
            .await?;
        self.notify_input(input).await;
        Ok(())
    }

    /// 턴 전송 중에 채팅이 멈췄다. 받았거나 결과를 모르면 입력은 보낸 것으로 두고, 작업은 멈춘 채로 둔다.
    /// 이을 때 보내는 확인 입력이 파일 상태부터 확인한다.
    async fn settle_stopped(
        &mut self,
        delivery: Delivery,
        result: Result<(), ProviderError>,
    ) -> Result<(), EngineError> {
        match result {
            Ok(()) | Err(ProviderError::Unknown) => self.record_applied(&delivery).await,
            Err(error) => self.settle(delivery, Err(error)).await,
        }
    }
}
