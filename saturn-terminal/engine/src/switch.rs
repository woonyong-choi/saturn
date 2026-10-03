//! 입력을 보낼 session을 정하고 연다: provider 전환 패킷, 돌아온 session의 변경분, 맥락 정리로 바꾸는 새 session.
//! 설계: docs/design/providers-and-sessions.md#provider-전환, docs/design/context-management.md

use std::time::SystemTime;

use saturn_core::providers::{ProviderClient, ProviderError, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_core::sessions::context::ContextBudget;
use saturn_core::sessions::packet::PacketSource;
use saturn_core::sessions::{AgentRole, SendTarget, SessionError, SessionRecord};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, SessionId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::SessionState;

use crate::dispatch::Start;
use crate::flow::LiveSession;
use crate::handoff::{
    Handoff, HandoffOutcome, handoff_of, handoff_source, others_only, reduce_handoff,
};
use crate::models::pinned_model_name;
use crate::sessions::SendRequest;
use crate::store::IdKind;
use crate::{Engine, EngineError};

/// 계획을 세우지 못한 이유.
#[derive(Debug)]
pub(crate) enum PlanError {
    /// 사용자에게 보일 원인 한 줄.
    Failed(String),
    /// 패킷의 고정 구역이 `P_hard`도 넘어 보내지 않는다.
    Deferred(Vec<String>),
}

/// provider를 호출하기 전에 정한 것. 계획이 서면 입력 상태는 아직 바뀌지 않았다.
#[derive(Debug)]
pub(crate) struct OpenPlan {
    provider: Provider,
    target: SendTarget,
    /// 새 session에 고른 모델. `None`이면 provider 기본값이다.
    model: Option<String>,
    /// 새 session의 첫 턴으로 보낼 패킷이나 돌아온 session에 붙일 변경분.
    handoff: Option<String>,
    /// provider를 바꿀 때 떠나는 열린 메인.
    leaving: Option<SessionId>,
    /// 메인 에이전트 번호는 session을 바꿔도 같게 이어 간다. 대기열의 작업이 에이전트 번호로 session을 가리키기 때문이다.
    agent: Option<AgentId>,
    /// 열린 session이 받은 것으로 치는 기록 번호.
    synced: LedgerSeq,
    /// 패킷이 맥락 한도로 거절되면 줄여 다시 보낼 재료. 이미 줄였거나 보낼 패킷이 없으면 `None`.
    reduction: Option<Reduction>,
}

/// 거절된 패킷을 줄여 만들 재료.
#[derive(Debug)]
struct Reduction {
    source: PacketSource,
    budget: ContextBudget,
    /// 거절된 패킷의 추정 토큰 수.
    sent_tokens: u64,
}

/// 초안. 거절 응답이 한도를 알려 주지 않을 때 줄이는 목표를 정하는 상대 provider `P_max`의 몫(백분율).
const REDUCED_TARGET_PERCENT: u64 = 50;

impl Reduction {
    /// 목표는 거절 응답이 알려 준 한도, 없으면 상대 provider `P_max`의 일부다. 어느 쪽이든 거절된 패킷보다 작아지게 반으로 줄인 값을 넘지 않는다.
    /// 고정 구역만으로 목표를 넘으면 `None`.
    fn reduce(&self, limit_tokens: Option<u64>) -> Option<Handoff> {
        let by_budget = self.budget.packet_limit() * REDUCED_TARGET_PERCENT / 100;
        let target = limit_tokens
            .unwrap_or(by_budget)
            .min(self.sent_tokens * REDUCED_TARGET_PERCENT / 100);
        reduce_handoff(&self.source, &self.budget, target)
    }
}

impl OpenPlan {
    /// 거절된 계획의 패킷을 한 번만 줄인다. 줄일 재료가 없거나 이미 줄였거나 고정 구역만으로 넘치면 `None`.
    fn reduced(self, limit_tokens: Option<u64>) -> Option<Self> {
        let handoff = self.reduction.as_ref()?.reduce(limit_tokens)?;
        Some(Self {
            handoff: Some(handoff.text),
            reduction: None,
            ..self
        })
    }
}

/// 고른 모델이 없으면 어떤 session이든 이어 쓴다.
fn keeps_model(session: &SessionRecord, model: Option<&str>) -> bool {
    model.is_none_or(|model| session.model.as_deref() == Some(model))
}

impl Engine {
    /// 다음 입력부터 `provider`로 보낸다. 그 provider의 session이 열리면 지운다. 입력에 고정한 모델이 있으면 그 모델의 provider가 먼저다.
    pub(crate) fn switch_provider(&mut self, chat: ChatId, provider: Provider) {
        self.flow.switch_to.insert(chat, provider);
    }

    /// 보낼 session을 어떻게 열지 정한다. provider를 바꾸는 입력이거나 이어 갈 메인이 없으면 기록으로 패킷을 만들고,
    /// 보관 session으로 돌아가면 그 session이 받지 못한 변경분만 만든다.
    ///
    /// # Errors
    /// 원인 한 줄은 `Failed`, 패킷의 고정 구역이 넘치면 `Deferred`.
    pub(crate) async fn plan_open(
        &self,
        record: &QueuedInput,
        start: Start,
    ) -> Result<OpenPlan, PlanError> {
        let role = match start {
            Start::Task(task) if !self.queue.is_main_task(task) => AgentRole::Sub,
            _ => AgentRole::Main,
        };
        let provider = self
            .pick_provider(record)
            .map_err(|error| PlanError::Failed(self.failure_line(&error)))?;
        let model = pinned_model_name(record);
        let main = self.sessions.live_main(record.chat).cloned();
        let agent = match (start, role, &main) {
            (Start::Turn(agent), _, _) => Some(agent),
            (_, AgentRole::Main, Some(main)) => Some(main.agent),
            _ => None,
        };
        let plain = OpenPlan {
            provider,
            model: model.clone(),
            target: SendTarget::New { provider, role },
            handoff: None,
            leaving: None,
            agent,
            synced: LedgerSeq(0),
            reduction: None,
        };
        if role == AgentRole::Sub {
            return Ok(plain);
        }
        if let Some(main) = main.as_ref().filter(|main| {
            main.provider == provider
                && main.state == SessionState::Open
                && keeps_model(main, model.as_deref())
        }) {
            return Ok(OpenPlan {
                target: SendTarget::Open(main.id),
                ..plain
            });
        }
        self.plan_handoff(record, plain, main.as_ref()).await
    }

    /// 고른 모델과 다른 모델의 session은 쓰거나 재개하지 않고 새 session을 연다. 모델이 바뀌면 새 메인 session이다.
    fn keep_pinned_model(&self, target: SendTarget, model: Option<&str>) -> SendTarget {
        let (SendTarget::Open(id) | SendTarget::Resume(id)) = target else {
            return target;
        };
        let Some(stored) = self.sessions.get(id) else {
            return target;
        };
        if keeps_model(stored, model) {
            return target;
        }
        SendTarget::New {
            provider: stored.provider,
            role: stored.role,
        }
    }

    async fn plan_handoff(
        &self,
        record: &QueuedInput,
        plain: OpenPlan,
        main: Option<&SessionRecord>,
    ) -> Result<OpenPlan, PlanError> {
        let failed = |error: EngineError| PlanError::Failed(self.failure_line(&error));
        let (chat, provider) = (record.chat, plain.provider);
        let settings = self
            .settings
            .at(&self.store, record.settings)
            .await
            .map_err(|error| failed(error.into()))?;
        let budget = settings.context_budget(provider);
        let rows = self
            .store
            .ledger_since(chat, LedgerSeq(0))
            .await
            .map_err(|error| failed(error.into()))?;
        let synced = rows.last().map_or(LedgerSeq(0), |row| row.seq);
        let pending = self.pending_work(chat, Some(record.id));
        let full_source = handoff_source(&rows, &pending);
        let full = full_source
            .as_ref()
            .map_or(HandoffOutcome::Empty, |source| handoff_of(source, &budget));
        let packet = match &full {
            HandoffOutcome::Ready(handoff) => handoff.tokens,
            HandoffOutcome::Empty | HandoffOutcome::Deferred { .. } => 0,
        };
        let request = SendRequest {
            chat,
            provider,
            role: AgentRole::Main,
            packet,
            settings: record.settings,
        };
        let target = self
            .send_target(request, SystemTime::now())
            .await
            .map_err(failed)?;
        let target = self.keep_pinned_model(target, plain.model.as_deref());
        let (source, outcome) = match &target {
            SendTarget::Resume(id) => {
                let after = self.sessions.attach_from(*id);
                let changes = rows.into_iter().filter(|row| row.seq > after).collect();
                let source = handoff_source(&others_only(changes, *id), &pending);
                let outcome = source
                    .as_ref()
                    .map_or(HandoffOutcome::Empty, |source| handoff_of(source, &budget));
                (source, outcome)
            }
            SendTarget::New { .. } | SendTarget::Open(_) => (full_source, full),
        };
        let reduction = match (&outcome, source) {
            (HandoffOutcome::Ready(handoff), Some(source)) => Some(Reduction {
                source,
                budget,
                sent_tokens: handoff.tokens,
            }),
            _ => None,
        };
        let handoff = match outcome {
            HandoffOutcome::Ready(handoff) => {
                if handoff.is_over_limit {
                    tracing::warn!(
                        chat = chat.0,
                        tokens = handoff.tokens,
                        "packet exceeds its limit"
                    );
                }
                Some(handoff.text)
            }
            HandoffOutcome::Empty => None,
            HandoffOutcome::Deferred { constraints } => {
                return Err(PlanError::Deferred(constraints));
            }
        };
        let leaving = main
            .filter(|main| {
                main.state == SessionState::Open
                    && (main.provider != provider || !keeps_model(main, plain.model.as_deref()))
            })
            .map(|main| main.id);
        Ok(OpenPlan {
            target,
            handoff,
            leaving,
            synced,
            reduction,
            ..plain
        })
    }

    /// 계획대로 session을 연다. 떠나는 메인은 새 session이 열린 뒤에 보관한다. 열지 못하면 떠나는 메인은 그대로다.
    /// 패킷이 맥락 한도로 거절되면 경쟁 구역을 줄여 한 번만 다시 연다.
    ///
    /// # Errors
    /// 연결과 session 열기 실패는 `Provider`, 기록 저장 실패는 `Store`. 줄인 패킷도 거절됐거나 고정 구역만으로 넘쳐 줄일 수 없으면 `Provider(ContextExceeded)`.
    pub(crate) async fn open_planned(
        &mut self,
        record: &QueuedInput,
        plan: OpenPlan,
    ) -> Result<LiveSession, EngineError> {
        self.ensure_connected(plan.provider, record.chat, record.settings)
            .await?;
        let (live, plan) = match self.open_target(record, &plan).await {
            Err(EngineError::Provider(ProviderError::ContextExceeded { limit_tokens })) => {
                let Some(reduced) = plan.reduced(limit_tokens) else {
                    return Err(ProviderError::ContextExceeded { limit_tokens }.into());
                };
                tracing::warn!(
                    chat = record.chat.0,
                    "packet was over the context limit, sending a reduced one"
                );
                (self.open_target(record, &reduced).await?, reduced)
            }
            other => (other?, plan),
        };
        self.flow.switch_to.remove(&record.chat);
        self.after_open(&live, &plan).await?;
        Ok(live)
    }

    async fn open_target(
        &mut self,
        record: &QueuedInput,
        plan: &OpenPlan,
    ) -> Result<LiveSession, EngineError> {
        match plan.target {
            SendTarget::Open(id) => self.reuse_open(record, id).await,
            SendTarget::Resume(id) => self.resume_session(record, id, plan).await,
            SendTarget::New { provider, role } => {
                self.open_new_session(record, provider, role, plan).await
            }
        }
    }

    /// 이 프로세스에서 이미 열어 둔 session이면 그대로 쓴다.
    async fn reuse_open(
        &mut self,
        record: &QueuedInput,
        id: SessionId,
    ) -> Result<LiveSession, EngineError> {
        let agent = self.sessions.get(id).map(|session| session.agent);
        if let Some(live) = agent.and_then(|agent| self.flow.live.get(&agent)) {
            return Ok(live.clone());
        }
        let plan = self.reopen_plan(id);
        self.resume_session(record, id, &plan).await
    }

    /// 보관한 provider session id로 다시 연다.
    fn reopen_plan(&self, id: SessionId) -> OpenPlan {
        let (provider, agent) = self
            .sessions
            .get(id)
            .map_or((Provider::Claude, None), |session| {
                (session.provider, Some(session.agent))
            });
        OpenPlan {
            provider,
            model: None,
            target: SendTarget::Resume(id),
            handoff: None,
            leaving: None,
            agent,
            synced: LedgerSeq(0),
            reduction: None,
        }
    }

    /// 열린 뒤에만 상태를 `Open`으로 바꾼다. 멈춤으로 보류됐어도 provider에 아직 열려 있으면 다시 열지 않는다.
    async fn resume_session(
        &mut self,
        record: &QueuedInput,
        id: SessionId,
        plan: &OpenPlan,
    ) -> Result<LiveSession, EngineError> {
        let stored = self
            .sessions
            .get(id)
            .cloned()
            .ok_or(SessionError::NotFound(id))?;
        let still_open = self
            .flow
            .live
            .get(&stored.agent)
            .filter(|live| live.session == id)
            .cloned();
        if let Some(live) = still_open {
            self.send_handoff_turn(record.chat, &live, plan).await?;
            self.leave_main(record.chat, plan).await?;
            if stored.state != SessionState::Open {
                self.resume_main(id).await?;
            }
            return Ok(live);
        }
        let resume = stored
            .provider_session
            .clone()
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("session {} has no provider session id", id.0),
            })?;
        let spec = self.session_spec(
            record,
            stored.agent,
            stored.model.clone(),
            Some(resume),
            plan.handoff.clone(),
        );
        let handle = self
            .open_with_retries(record.chat, stored.provider, spec)
            .await?;
        self.leave_main(record.chat, plan).await?;
        if stored.state != SessionState::Open {
            self.resume_main(id).await?;
        }
        self.count_packet_turn(stored.agent, plan.handoff.is_some());
        Ok(self.remember(stored.agent, id, stored.provider, &handle))
    }

    /// provider가 이미 열려 있는 session에는 열 때 넘길 곳이 없어 변경분을 턴으로 보낸다.
    async fn send_handoff_turn(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        plan: &OpenPlan,
    ) -> Result<(), EngineError> {
        let Some(text) = &plan.handoff else {
            return Ok(());
        };
        self.provider_mut(chat, live.provider)?
            .send_turn(&live.provider_session, text)
            .await?;
        *self.flow.packet_turns.entry(live.agent).or_insert(0) += 1;
        Ok(())
    }

    async fn open_new_session(
        &mut self,
        record: &QueuedInput,
        provider: Provider,
        role: AgentRole,
        plan: &OpenPlan,
    ) -> Result<LiveSession, EngineError> {
        let agent = match plan.agent {
            Some(agent) => agent,
            None => AgentId(self.store.allocate_id(IdKind::Agent).await?),
        };
        let id = SessionId(self.store.allocate_id(IdKind::Session).await?);
        let spec = self.session_spec(
            record,
            agent,
            plan.model.clone(),
            None,
            plan.handoff.clone(),
        );
        let handle = self.open_with_retries(record.chat, provider, spec).await?;
        let registered = match self.leave_main(record.chat, plan).await {
            Ok(()) => {
                self.register_session(SessionRecord {
                    id,
                    chat: record.chat,
                    agent,
                    role,
                    provider,
                    provider_session: Some(handle.provider_session.clone()),
                    model: plan.model.clone(),
                    state: SessionState::Open,
                    delivered: LedgerSeq(0),
                    idle_since: None,
                })
                .await
            }
            Err(error) => Err(error),
        };
        if let Err(error) = registered {
            self.close_unregistered(record.chat, provider, &handle)
                .await;
            return Err(error);
        }
        self.count_packet_turn(agent, plan.handoff.is_some());
        Ok(self.remember(agent, id, provider, &handle))
    }

    /// 떠나는 메인의 provider session을 닫고 보관한다. provider session id는 재개에 쓰려고 남긴다.
    async fn leave_main(&mut self, chat: ChatId, plan: &OpenPlan) -> Result<(), EngineError> {
        let Some(leaving) = plan.leaving else {
            return Ok(());
        };
        let Some(main) = self.sessions.get(leaving).cloned() else {
            return Ok(());
        };
        self.flow.live.retain(|_, live| live.session != leaving);
        if let (Some(provider_session), Ok(connection)) = (
            main.provider_session.as_ref(),
            self.provider_mut(chat, main.provider),
        ) && let Err(error) = connection.close_session(provider_session).await
        {
            tracing::warn!(%error, "failed to close the leaving session");
        }
        self.archive_main(leaving).await?;
        if main.provider != plan.provider {
            self.notify_chat(
                chat,
                ChatNotice::ProviderSwitched {
                    from: main.provider,
                    to: plan.provider,
                },
            )
            .await;
        }
        Ok(())
    }

    /// 열린 session이 받은 기록 번호를 올리고 저장한다. 패킷은 첫 턴으로 갔으니 그 턴의 완료는 작업 끝이 아니다.
    async fn after_open(&mut self, live: &LiveSession, plan: &OpenPlan) -> Result<(), EngineError> {
        self.sessions.mark_delivered(live.session, plan.synced);
        self.persist_sessions(live.session).await
    }

    /// 열 때 넘긴 패킷도 provider에는 턴 하나다.
    fn count_packet_turn(&mut self, agent: AgentId, sent: bool) {
        if sent {
            *self.flow.packet_turns.entry(agent).or_insert(0) += 1;
        }
    }

    /// 맥락 정리로 같은 provider의 새 session으로 이어 간다. 옛 session은 `Ended`가 된다.
    /// 턴 경계(유휴로 표시한 session)에서만 부른다.
    ///
    /// # Errors
    /// 열지 못하면 `Provider`, 턴 경계가 아니면 `Session(NotAtTurnBoundary)`, 저장 실패면 `Store`.
    pub(crate) async fn restart_session(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        packet: String,
        up_to: LedgerSeq,
    ) -> Result<(), EngineError> {
        let old = self
            .sessions
            .get(live.session)
            .cloned()
            .ok_or(SessionError::NotFound(live.session))?;
        if old.state == SessionState::Open && old.idle_since.is_none() {
            return Err(SessionError::NotAtTurnBoundary.into());
        }
        let workdir = self
            .chat_env(chat)
            .ok_or(EngineError::ChatNotAttached { chat })?
            .workdir()
            .to_path_buf();
        let id = SessionId(self.store.allocate_id(IdKind::Session).await?);
        let spec = SessionSpec {
            agent: old.agent,
            workdir,
            model: old.model.clone(),
            settings: self
                .settings
                .current()
                .ok_or(crate::settings::SettingsError::NoPreviousRevision)?,
            resume: None,
            packet: Some(packet),
            add_dirs: self.chat_dirs_of(chat),
        };
        let handle = self.open_with_retries(chat, live.provider, spec).await?;
        let record = SessionRecord {
            id,
            chat,
            agent: old.agent,
            role: old.role,
            provider: live.provider,
            provider_session: Some(handle.provider_session.clone()),
            model: old.model.clone(),
            state: SessionState::Open,
            delivered: up_to,
            idle_since: None,
        };
        if let Err(error) = self.replace_session(live.session, record).await {
            self.close_unregistered(chat, live.provider, &handle).await;
            return Err(error);
        }
        self.close_replaced(chat, live, &old).await;
        self.remember(old.agent, id, live.provider, &handle);
        self.count_packet_turn(old.agent, true);
        Ok(())
    }

    async fn close_replaced(&mut self, chat: ChatId, live: &LiveSession, old: &SessionRecord) {
        let Some(provider_session) = old.provider_session.as_ref() else {
            return;
        };
        if let Ok(connection) = self.provider_mut(chat, live.provider)
            && let Err(error) = connection.close_session(provider_session).await
        {
            tracing::warn!(%error, "failed to close the replaced session");
        }
    }
}
