//! 입력을 보낼 session을 정하고 연다: provider 전환 패킷, 돌아온 session의 변경분, 맥락 정리로 바꾸는 새 session.
//! 설계: docs/design/providers-and-sessions.md#provider-전환, docs/design/context-management.md

use std::time::SystemTime;

use saturn_core::providers::{ProviderClient, ProviderError, SessionSpec};
use saturn_core::queue::QueuedInput;
use saturn_core::sessions::{AgentRole, SendTarget, SessionError, SessionRecord};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, SessionId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::SessionState;

use crate::dispatch::Start;
use crate::flow::LiveSession;
use crate::handoff::{HandoffOutcome, build_handoff, others_only};
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
    /// 새 session의 첫 턴으로 보낼 패킷이나 돌아온 session에 붙일 변경분.
    handoff: Option<String>,
    /// provider를 바꿀 때 떠나는 열린 메인.
    leaving: Option<SessionId>,
    /// 메인 에이전트 번호는 session을 바꿔도 같게 이어 간다. 대기열의 작업이 에이전트 번호로 session을 가리키기 때문이다.
    agent: Option<AgentId>,
    /// 열린 session이 받은 것으로 치는 기록 번호.
    synced: LedgerSeq,
}

impl Engine {
    /// 다음 입력부터 `provider`로 보낸다. 그 provider의 session이 열리면 지운다.
    /// TODO(#168): 모델을 고르는 입력이 provider로 가는 대응이 정해지면 입력의 모델로 부른다
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
            .pick_provider(record.chat)
            .map_err(|error| PlanError::Failed(self.failure_line(&error)))?;
        let main = self.sessions.live_main(record.chat).cloned();
        let agent = match (start, role, &main) {
            (Start::Turn(agent), _, _) => Some(agent),
            (_, AgentRole::Main, Some(main)) => Some(main.agent),
            _ => None,
        };
        let plain = OpenPlan {
            provider,
            target: SendTarget::New { provider, role },
            handoff: None,
            leaving: None,
            agent,
            synced: LedgerSeq(0),
        };
        if role == AgentRole::Sub {
            return Ok(plain);
        }
        if let Some(main) = main
            .as_ref()
            .filter(|main| main.provider == provider && main.state == SessionState::Open)
        {
            return Ok(OpenPlan {
                target: SendTarget::Open(main.id),
                ..plain
            });
        }
        self.plan_handoff(record, plain, main.as_ref()).await
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
        let full = build_handoff(&rows, &budget);
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
        let outcome = match &target {
            SendTarget::Resume(id) => {
                let after = self.sessions.attach_from(*id);
                let changes = rows.into_iter().filter(|row| row.seq > after).collect();
                build_handoff(&others_only(changes, *id), &budget)
            }
            SendTarget::New { .. } | SendTarget::Open(_) => full,
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
            .filter(|main| main.provider != provider && main.state == SessionState::Open)
            .map(|main| main.id);
        Ok(OpenPlan {
            target,
            handoff,
            leaving,
            synced,
            ..plain
        })
    }

    /// 계획대로 session을 연다. 떠나는 메인은 새 session이 열린 뒤에 보관한다. 열지 못하면 떠나는 메인은 그대로다.
    ///
    /// # Errors
    /// 연결과 session 열기 실패는 `Provider`, 기록 저장 실패는 `Store`.
    pub(crate) async fn open_planned(
        &mut self,
        record: &QueuedInput,
        plan: OpenPlan,
    ) -> Result<LiveSession, EngineError> {
        self.ensure_connected(plan.provider, record.chat, record.settings)
            .await?;
        let live = match plan.target {
            SendTarget::Open(id) => self.reuse_open(record, id).await?,
            SendTarget::Resume(id) => self.resume_session(record, id, &plan).await?,
            SendTarget::New { provider, role } => {
                self.open_new_session(record, provider, role, &plan).await?
            }
        };
        self.flow.switch_to.remove(&record.chat);
        self.after_open(record.chat, &live, &plan).await?;
        Ok(live)
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
            target: SendTarget::Resume(id),
            handoff: None,
            leaving: None,
            agent,
            synced: LedgerSeq(0),
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
            self.leave_main(record.chat, plan).await?;
            if stored.state != SessionState::Open {
                self.resume_main(id).await?;
            }
            self.send_handoff_turn(record.chat, &live, plan).await?;
            return Ok(live);
        }
        let resume = stored
            .provider_session
            .clone()
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("session {} has no provider session id", id.0),
            })?;
        let spec = self.session_spec(record, stored.agent, Some(resume), plan.handoff.clone());
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
        let spec = self.session_spec(record, agent, None, plan.handoff.clone());
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
        self.notify_chat(
            chat,
            ChatNotice::ProviderSwitched {
                from: main.provider,
                to: plan.provider,
            },
        )
        .await;
        Ok(())
    }

    /// 열린 session이 받은 기록 번호를 올리고 저장한다. 패킷은 첫 턴으로 갔으니 그 턴의 완료는 작업 끝이 아니다.
    async fn after_open(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        plan: &OpenPlan,
    ) -> Result<(), EngineError> {
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
            model: None,
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
