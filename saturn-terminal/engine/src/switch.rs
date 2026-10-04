//! 입력을 보낼 session을 정하고 연다: provider 전환 패킷, 돌아온 session의 변경분, 맥락 정리로 바꾸는 새 session.
//! 설계: docs/design/providers-and-sessions.md#provider-전환, docs/design/context-management.md

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use saturn_core::providers::{ProviderError, SessionHandle, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_core::sessions::context::{ContextBudget, ReturnDecision, decide_return};
use saturn_core::sessions::packet::PacketSource;
use saturn_core::sessions::{AgentRole, LastTurn, SendTarget, SessionError, SessionRecord};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, SessionId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::SessionState;

use crate::calls::CallKind;
use crate::dispatch::{MAX_SEND_ATTEMPTS, Start};
use crate::flow::LiveSession;
use crate::handoff::{
    Handoff, HandoffOutcome, changes_of_others, handoff_of, handoff_source, others_only,
    reduce_handoff,
};
use crate::models::pinned_model_name;
use crate::sessions::SendRequest;
use crate::settings::ContextMode;
use crate::store::IdKind;
use crate::{Engine, EngineError};

/// 캐시 유지 시간을 넘겨 쉰 열린 메인. session, 마지막 턴 값, 마지막 턴 뒤 쉰 시간이다.
type StaleMain = (SessionId, LastTurn, Duration);

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

/// 새 session 열기를 기다리는 맥락 정리. 열리면 옛 session을 바꾼다.
#[derive(Debug)]
pub(crate) struct Restart {
    live: LiveSession,
    old: SessionRecord,
    /// 새 session 번호.
    id: SessionId,
    spec: SessionSpec,
    /// 패킷이 맥락 한도로 거절되면 줄일 재료.
    reduction: Option<Reduction>,
    up_to: LedgerSeq,
    /// 줄인 패킷으로 다시 열었다. 한 번만 줄인다.
    is_reduced: bool,
}

/// 거절된 패킷을 줄여 만들 재료.
#[derive(Debug)]
pub(crate) struct Reduction {
    pub(crate) source: PacketSource,
    pub(crate) budget: ContextBudget,
    /// 거절된 패킷의 추정 토큰 수.
    pub(crate) sent_tokens: u64,
}

/// 초안. 거절 응답이 한도를 알려 주지 않을 때 줄이는 목표를 정하는 상대 provider `P_max`의 몫(백분율).
const REDUCED_TARGET_PERCENT: u64 = 50;

impl Reduction {
    /// 목표는 거절 응답이 알려 준 한도, 없으면 상대 provider `P_max`의 일부다. 어느 쪽이든 거절된 패킷보다 작아지게 반으로 줄인 값을 넘지 않는다.
    /// 고정 구역만으로 목표를 넘으면 `None`.
    pub(crate) fn reduce(&self, limit_tokens: Option<u64>) -> Option<Handoff> {
        let by_budget = self.budget.packet_limit() * REDUCED_TARGET_PERCENT / 100;
        let target = limit_tokens
            .unwrap_or(by_budget)
            .min(self.sent_tokens * REDUCED_TARGET_PERCENT / 100);
        reduce_handoff(&self.source, &self.budget, target)
    }
}

impl OpenPlan {
    pub(crate) fn provider(&self) -> Provider {
        self.provider
    }

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

/// provider를 부르기 전에 정해 둔 열기 방법.
#[derive(Debug)]
pub(crate) struct OpenPrep {
    plan: OpenPlan,
    kind: OpenKind,
}

#[derive(Debug)]
enum OpenKind {
    /// 이 프로세스에서 이미 열려 있어 provider를 부르지 않는다.
    Live(LiveSession),
    /// provider에 열려 있는 session에 변경분(`text`)을 턴으로 보낸다. 없으면 부르지 않는다.
    Handoff {
        live: LiveSession,
        stored: SessionRecord,
        text: Option<String>,
    },
    Resume {
        stored: SessionRecord,
        spec: SessionSpec,
    },
    New {
        provider: Provider,
        role: AgentRole,
        agent: AgentId,
        id: SessionId,
        spec: SessionSpec,
    },
}

/// provider에 보낼 열기 요청.
pub(crate) enum OpenCall {
    None,
    /// 변경분 턴을 보낸다.
    Handoff(LiveSession, String),
    Open(Provider, SessionSpec),
}

impl OpenPrep {
    pub(crate) fn provider(&self) -> Provider {
        self.plan.provider
    }

    /// 이 준비가 provider에 요청할 것.
    pub(crate) fn call(&self) -> OpenCall {
        match &self.kind {
            OpenKind::Live(_) | OpenKind::Handoff { text: None, .. } => OpenCall::None,
            OpenKind::Handoff {
                live,
                text: Some(text),
                ..
            } => OpenCall::Handoff(live.clone(), text.clone()),
            OpenKind::Resume { stored, spec } => OpenCall::Open(stored.provider, spec.clone()),
            OpenKind::New { provider, spec, .. } => OpenCall::Open(*provider, spec.clone()),
        }
    }

    /// 패킷이 맥락 한도로 거절됐을 때 줄인 패킷으로 다시 열 계획. 줄일 수 없으면 `None`.
    pub(crate) fn reduced_plan(self, limit_tokens: Option<u64>) -> Option<OpenPlan> {
        self.plan.reduced(limit_tokens)
    }
}

/// 보낼 패킷 글. 고정 구역이 넘치면 보내지 않는다.
fn packet_text(chat: ChatId, outcome: HandoffOutcome) -> Result<Option<String>, PlanError> {
    match outcome {
        HandoffOutcome::Ready(handoff) => {
            if handoff.is_over_limit {
                tracing::warn!(
                    chat = chat.0,
                    tokens = handoff.tokens,
                    "packet exceeds its limit"
                );
            }
            Ok(Some(handoff.text))
        }
        HandoffOutcome::Empty => Ok(None),
        HandoffOutcome::Deferred { constraints } => Err(PlanError::Deferred(constraints)),
    }
}

/// 새 session을 열 때 떠나는 열린 메인. 쉬었다 돌아와 같은 provider와 모델로 새로 열 때도 옛 session을 닫는다.
fn leaving_main(
    main: Option<&SessionRecord>,
    plan: &OpenPlan,
    is_idle_return: bool,
) -> Option<SessionId> {
    main.filter(|main| {
        matches!(
            main.state,
            SessionState::Open | SessionState::ClosedResumable
        ) && (is_idle_return
            || main.provider != plan.provider
            || !keeps_model(main, plan.model.as_deref()))
    })
    .map(|main| main.id)
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
        let model = pinned_model_name(&self.registry, record);
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
                && main.state == SessionState::ClosedResumable
                && keeps_model(main, model.as_deref())
        }) {
            // 유휴로 닫은 메인도 캐시 유지 시간을 넘겼으면 열려 있을 때와 같은 유휴 복귀 판정을 받는다
            let stale = self
                .idle_return_input(record, main)
                .await
                .map_err(|error| PlanError::Failed(self.failure_line(&error)))?;
            if stale.is_some() {
                return self.plan_handoff(record, plain, Some(main), stale).await;
            }
        }
        if let Some(main) = main.as_ref().filter(|main| {
            main.provider == provider
                && main.state == SessionState::Open
                && keeps_model(main, model.as_deref())
        }) {
            let stale = self
                .idle_return_input(record, main)
                .await
                .map_err(|error| PlanError::Failed(self.failure_line(&error)))?;
            if stale.is_none() {
                return Ok(OpenPlan {
                    target: SendTarget::Open(main.id),
                    ..plain
                });
            }
            return self.plan_handoff(record, plain, Some(main), stale).await;
        }
        self.plan_handoff(record, plain, main.as_ref(), None).await
    }

    /// 열린 메인의 마지막 턴 뒤 쉰 시간이 캐시 유지 시간을 넘었으면 유휴 복귀 판정에 쓸 마지막 턴 값과 쉰 시간.
    /// `provider` 모드는 `sessions`가 compaction을 판정하지 않고, 트리가 유휴가 아니거나 마지막 턴 값을 모르면 판정하지 않는다.
    async fn idle_return_input(
        &self,
        record: &QueuedInput,
        main: &SessionRecord,
    ) -> Result<Option<StaleMain>, EngineError> {
        let settings = self.settings.at(&self.store, record.settings).await?;
        if settings.context_mode() == ContextMode::Provider || !self.agents.is_tree_idle(main.agent)
        {
            return Ok(None);
        }
        let Some(last) = self.sessions.last_turn(main.id) else {
            return Ok(None);
        };
        let since = SystemTime::now()
            .duration_since(last.ended_at)
            .unwrap_or(Duration::ZERO);
        let ttl = self.cache_ttl(main.provider).await?;
        Ok((since > ttl).then_some((main.id, last, since)))
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
        stale: Option<StaleMain>,
    ) -> Result<OpenPlan, PlanError> {
        let failed = |error: EngineError| PlanError::Failed(self.failure_line(&error));
        let (chat, provider) = (record.chat, plain.provider);
        let settings = self
            .settings
            .at(&self.store, record.settings)
            .await
            .map_err(|error| failed(error.into()))?;
        let budget = settings.context_budget(provider, self.registry.context_defaults(provider));
        let rows = self
            .store
            .ledger_since(chat, LedgerSeq(0))
            .await
            .map_err(|error| failed(error.into()))?;
        let steers = self
            .store
            .steered_inputs(chat)
            .await
            .map_err(|error| failed(error.into()))?;
        let changes = self
            .store
            .run_changes(chat)
            .await
            .map_err(|error| failed(error.into()))?;
        let synced = rows.last().map_or(LedgerSeq(0), |row| row.seq);
        let pending = self.pending_work(chat, Some(record.id));
        let full_source = handoff_source(
            &rows,
            &steers,
            &changes,
            &pending,
            &self.registry.instruction_docs(),
            budget.rrf_k,
        );
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
            .handoff_target(request, stale, (&budget, packet))
            .await
            .map_err(failed)?;
        if stale.is_some() && matches!(target, SendTarget::Open(_)) {
            return Ok(OpenPlan { target, ..plain });
        }
        let target = self.keep_pinned_model(target, plain.model.as_deref());
        let (source, outcome) = match &target {
            SendTarget::Resume(id) => {
                let after = self.sessions.attach_from(*id);
                let newer = rows.into_iter().filter(|row| row.seq > after).collect();
                let source = handoff_source(
                    &others_only(newer, *id),
                    &steers,
                    &changes_of_others(changes, *id, after),
                    &pending,
                    &self.registry.instruction_docs(),
                    budget.rrf_k,
                );
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
        let handoff = packet_text(chat, outcome)?;
        let leaving = leaving_main(main, &plain, stale.is_some());
        Ok(OpenPlan {
            target,
            handoff,
            leaving,
            synced,
            reduction,
            ..plain
        })
    }

    /// 패킷을 보낼 session. 쉰 열린 메인이면 유휴 복귀 판정으로 그 메인을 그대로 쓰거나 새 session을 연다.
    async fn handoff_target(
        &self,
        request: SendRequest,
        stale: Option<StaleMain>,
        (budget, packet): (&ContextBudget, u64),
    ) -> Result<SendTarget, EngineError> {
        let Some((open, last, since)) = stale else {
            return self.send_target(request, SystemTime::now()).await;
        };
        let budget = ContextBudget {
            cache_ttl: self.cache_ttl(request.provider).await?,
            ..*budget
        };
        Ok(match decide_return(&budget, since, last.active, packet) {
            ReturnDecision::Resume => SendTarget::Open(open),
            ReturnDecision::NewSession => SendTarget::New {
                provider: request.provider,
                role: AgentRole::Main,
            },
        })
    }

    /// 계획대로 session을 열기 위해 provider를 부르기 전까지 준비한다. 새 session의 번호를 받고 열 때 넘길 값을 정한다.
    ///
    /// # Errors
    /// 번호를 받지 못하면 `Store`, 보관 session을 찾지 못하거나 provider session id가 없으면 `Session`이나 `Provider`.
    pub(crate) async fn prepare_open(
        &mut self,
        record: &QueuedInput,
        add_dirs: Vec<PathBuf>,
        plan: OpenPlan,
    ) -> Result<OpenPrep, EngineError> {
        match plan.target.clone() {
            SendTarget::Open(id) => {
                let agent = self.sessions.get(id).map(|session| session.agent);
                if let Some(live) = agent.and_then(|agent| self.flow.live.get(&agent)) {
                    // 이 프로세스에서 이미 열어 둔 session이면 그대로 쓴다
                    let live = live.clone();
                    return Ok(OpenPrep {
                        plan,
                        kind: OpenKind::Live(live),
                    });
                }
                let reopened = self.reopen_plan(id)?;
                self.prepare_resume(record, add_dirs, id, reopened)
            }
            SendTarget::Resume(id) => self.prepare_resume(record, add_dirs, id, plan),
            SendTarget::New { provider, role } => {
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
                    add_dirs,
                );
                Ok(OpenPrep {
                    plan,
                    kind: OpenKind::New {
                        provider,
                        role,
                        agent,
                        id,
                        spec,
                    },
                })
            }
        }
    }

    /// 보관한 provider session id로 다시 열 준비를 한다. 멈춤으로 보류됐어도 provider에 아직 열려 있으면 다시 열지 않는다.
    fn prepare_resume(
        &self,
        record: &QueuedInput,
        add_dirs: Vec<PathBuf>,
        id: SessionId,
        plan: OpenPlan,
    ) -> Result<OpenPrep, EngineError> {
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
            // provider에 이미 열려 있는 session에는 열 때 넘길 곳이 없어 변경분을 턴으로 보낸다
            let text = plan.handoff.clone();
            return Ok(OpenPrep {
                plan,
                kind: OpenKind::Handoff { live, stored, text },
            });
        }
        let resume = stored
            .provider_session
            .clone()
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("session {} has no provider session id", id.0),
            })?;
        let mut spec = self.session_spec(
            record,
            stored.agent,
            stored.model.clone(),
            Some(resume),
            plan.handoff.clone(),
            add_dirs,
        );
        spec.interrupted_children = self
            .flow
            .interrupted_to_clean
            .get(&stored.agent)
            .cloned()
            .unwrap_or_default();
        Ok(OpenPrep {
            plan,
            kind: OpenKind::Resume { stored, spec },
        })
    }

    /// provider 응답을 받은 뒤의 상태 변경을 한다. 떠나는 메인은 새 session이 열린 뒤에 보관하고, 열지 못하면 그대로다.
    /// `opened`는 session을 연 요청의 결과이고, provider를 부르지 않은 경우는 `None`이다.
    ///
    /// # Errors
    /// 기록 저장 실패는 `Store`.
    pub(crate) async fn finish_open(
        &mut self,
        record: &QueuedInput,
        prep: OpenPrep,
        opened: Option<SessionHandle>,
    ) -> Result<LiveSession, EngineError> {
        let OpenPrep { plan, kind } = prep;
        let chat = record.chat;
        let live = match (kind, opened) {
            (OpenKind::Live(live), _) => live,
            (OpenKind::Handoff { live, stored, text }, _) => {
                if text.is_some() {
                    *self.flow.packet_turns.entry(live.agent).or_insert(0) += 1;
                }
                self.leave_main(chat, &plan).await?;
                if stored.state != SessionState::Open {
                    self.resume_main(stored.id).await?;
                }
                live
            }
            (OpenKind::Resume { stored, spec }, Some(handle)) => {
                self.flow.session_dirs.insert(stored.agent, spec.add_dirs);
                // provider가 끊긴 자식 정리를 시도한 뒤라 다시 넘기지 않는다
                if self
                    .flow
                    .interrupted_to_clean
                    .remove(&stored.agent)
                    .is_some()
                {
                    let marked = self.store.mark_interrupted_cleaned(stored.agent).await;
                    self.warn_failure("failed to record cleaned interrupted subagents", marked);
                }
                self.leave_main(chat, &plan).await?;
                if stored.state != SessionState::Open {
                    self.resume_main(stored.id).await?;
                }
                self.count_packet_turn(stored.agent, plan.handoff.is_some());
                self.remember(stored.agent, stored.id, stored.provider, &handle)
            }
            (
                OpenKind::New {
                    provider,
                    role,
                    agent,
                    id,
                    spec,
                },
                Some(handle),
            ) => {
                let live = self
                    .register_opened(record, &plan, (provider, role, agent, id), &handle)
                    .await?;
                self.flow.session_dirs.insert(agent, spec.add_dirs);
                live
            }
            (OpenKind::Resume { .. } | OpenKind::New { .. }, None) => {
                return Err(ProviderError::ConnectionLost.into());
            }
        };
        self.flow.switch_to.remove(&chat);
        self.after_open(&live, &plan).await?;
        Ok(live)
    }

    /// 연 새 session을 기록에 등록한다. 기록하지 못하면 provider 쪽도 닫는다.
    async fn register_opened(
        &mut self,
        record: &QueuedInput,
        plan: &OpenPlan,
        (provider, role, agent, id): (Provider, AgentRole, AgentId, SessionId),
        handle: &SessionHandle,
    ) -> Result<LiveSession, EngineError> {
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
            self.close_unregistered(record.chat, provider, handle);
            return Err(error);
        }
        self.count_packet_turn(agent, plan.handoff.is_some());
        Ok(self.remember(agent, id, provider, handle))
    }

    /// 보관한 provider session id로 다시 연다.
    ///
    /// # Errors
    /// 모르는 session이면 `Session(NotFound)`.
    fn reopen_plan(&self, id: SessionId) -> Result<OpenPlan, EngineError> {
        let session = self.sessions.get(id).ok_or(SessionError::NotFound(id))?;
        Ok(OpenPlan {
            provider: session.provider,
            model: None,
            target: SendTarget::Resume(id),
            handoff: None,
            leaving: None,
            agent: Some(session.agent),
            synced: LedgerSeq(0),
            reduction: None,
        })
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
        // 유휴로 이미 닫은 session은 provider에 다시 닫으라고 하지 않는다
        let is_open = main.state == SessionState::Open;
        if let (true, Some(provider_session), Ok(connection)) = (
            is_open,
            main.provider_session.as_ref(),
            self.provider_mut(chat, main.provider),
        ) {
            // 닫는 요청은 기다리지 않는다. 같은 연결의 뒤따르는 요청은 줄 선 순서대로 나간다
            connection.close_session_detached(provider_session.clone());
        }
        if main.provider == plan.provider && keeps_model(&main, plan.model.as_deref()) {
            // 같은 provider와 모델의 새 session으로 바꾸는 유휴 복귀라 되돌아갈 보관 session이 없다
            self.sessions.set_state(leaving, SessionState::Ended)?;
            self.persist_sessions(leaving).await?;
        } else {
            self.archive_main(leaving).await?;
        }
        if main.provider != plan.provider {
            self.notify_chat(
                chat,
                ChatNotice::ProviderSwitched {
                    from: main.provider,
                    to: plan.provider,
                },
            )
            .await;
            self.tell_parts_left_behind(chat, main.provider, plan.provider)
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

    /// 맥락 정리로 같은 provider의 새 session으로 이어 가는 첫 단계. 새 session 열기를 연결 작업에 맡기고 기다리지 않는다.
    /// 열리면 `finish_restart`가 옛 session을 `Ended`로 바꾸고 이어 간다. 그동안 그 채팅의 다음 입력은 보내지 않는다.
    /// 턴 경계(유휴로 표시한 session)에서만 부른다.
    ///
    /// # Errors
    /// 턴 경계가 아니면 `Session(NotAtTurnBoundary)`, 연결이 없으면 `Provider(NotSent)`, 저장 실패면 `Store`.
    pub(crate) async fn restart_session(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        packet: String,
        reduction: Option<Reduction>,
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
            settings: self.revision_of_agent(chat, old.agent)?,
            resume: None,
            packet: Some(packet),
            add_dirs: self.chat_dirs_of(chat),
            interrupted_children: Vec::new(),
        };
        self.provider_mut(chat, live.provider)?;
        let restart = Restart {
            live: live.clone(),
            old,
            id,
            spec,
            reduction,
            up_to,
            is_reduced: false,
        };
        self.flow.restarting.insert(chat);
        self.submit_restart(chat, restart);
        Ok(())
    }

    /// 새 session 열기를 맡긴다. 연결이 있는지 호출자가 먼저 확인한다.
    fn submit_restart(&mut self, chat: ChatId, restart: Restart) {
        let (provider, spec) = (restart.live.provider, restart.spec.clone());
        self.start_call(
            (chat, provider),
            CallKind::Restart(Box::new(restart)),
            |connection, tag| connection.open_session_call(tag, spec, MAX_SEND_ATTEMPTS),
        );
    }

    /// 새 session 열기의 결과. 패킷이 맥락 한도로 거절되면 `reduction`으로 줄여 한 번만 다시 연다. 옛 session은 그대로
    /// 열려 있다. 끝나면 기다리던 입력을 보낸다. 맥락 정리 오류는 로그만 남기고 입력은 보낸다.
    pub(crate) async fn finish_restart(
        &mut self,
        chat: ChatId,
        restart: Restart,
        opened: Result<SessionHandle, ProviderError>,
    ) {
        let restarted = match opened {
            Ok(handle) => self.replace_with_new(chat, &restart, &handle).await,
            Err(ProviderError::ContextExceeded { limit_tokens }) if !restart.is_reduced => {
                match self.reopen_restart_reduced(chat, restart, limit_tokens) {
                    Ok(()) => return,
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error.into()),
        };
        self.end_restart(chat, restarted).await;
    }

    /// 줄인 패킷으로 새 session 열기를 다시 맡긴다.
    ///
    /// # Errors
    /// 줄일 재료가 없거나 고정 구역만으로 목표를 넘으면 `Provider(ContextExceeded)`, 연결이 없으면 `Provider(NotSent)`.
    fn reopen_restart_reduced(
        &mut self,
        chat: ChatId,
        mut restart: Restart,
        limit_tokens: Option<u64>,
    ) -> Result<(), EngineError> {
        let reduced = restart
            .reduction
            .take()
            .and_then(|reduction| reduction.reduce(limit_tokens))
            .ok_or(ProviderError::ContextExceeded { limit_tokens })?;
        tracing::warn!(
            chat = chat.0,
            "packet was over the context limit, sending a reduced one"
        );
        restart.spec.packet = Some(reduced.text);
        restart.is_reduced = true;
        self.provider_mut(chat, restart.live.provider)?;
        self.submit_restart(chat, restart);
        Ok(())
    }

    /// 맥락 정리를 끝내고 결과를 알린 뒤 기다리던 입력을 보낸다. 줄인 패킷도 맥락 한도로 거절되면 옛 session을 그대로
    /// 두고 `PacketOverflow`를 알린다.
    async fn end_restart(&mut self, chat: ChatId, restarted: Result<(), EngineError>) {
        self.flow.restarting.remove(&chat);
        match restarted {
            Ok(()) => self.notify_chat(chat, ChatNotice::Compacted).await,
            Err(EngineError::Provider(ProviderError::ContextExceeded { .. })) => {
                self.notify_chat(chat, ChatNotice::PacketOverflow).await;
            }
            Err(error) => {
                tracing::warn!(chat = chat.0, error = %self.failure_line(&error), "context compaction skipped");
            }
        }
        self.restart_stale_connections(chat).await;
        let next = self.dispatch_next(chat).await;
        self.warn_failure("failed to send the next input", next);
    }

    /// 연 새 session으로 옛 session을 바꿔 기록하고 옛 session을 닫는다. 기록하지 못하면 새 session을 닫는다.
    async fn replace_with_new(
        &mut self,
        chat: ChatId,
        restart: &Restart,
        handle: &SessionHandle,
    ) -> Result<(), EngineError> {
        let (live, old) = (&restart.live, &restart.old);
        let record = SessionRecord {
            id: restart.id,
            chat,
            agent: old.agent,
            role: old.role,
            provider: live.provider,
            provider_session: Some(handle.provider_session.clone()),
            model: old.model.clone(),
            state: SessionState::Open,
            delivered: restart.up_to,
            idle_since: None,
        };
        if let Err(error) = self.replace_session(live.session, record).await {
            self.close_unregistered(chat, live.provider, handle);
            return Err(error);
        }
        self.close_replaced(chat, live, old);
        self.flow
            .session_dirs
            .insert(old.agent, restart.spec.add_dirs.clone());
        self.remember(old.agent, restart.id, live.provider, handle);
        self.count_packet_turn(old.agent, true);
        Ok(())
    }

    fn close_replaced(&mut self, chat: ChatId, live: &LiveSession, old: &SessionRecord) {
        let Some(provider_session) = old.provider_session.as_ref() else {
            return;
        };
        if let Ok(connection) = self.provider_mut(chat, live.provider) {
            connection.close_session_detached(provider_session.clone());
        }
    }
}
