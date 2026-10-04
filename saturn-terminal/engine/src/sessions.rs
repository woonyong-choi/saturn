//! 보관 session의 마지막 턴 값 기록과 되살리기, 전송 대상 정하기, provider 전환 때 보관과 재개.
//! 설계: docs/design/providers-and-sessions.md

use std::time::{Duration, SystemTime};

use saturn_core::sessions::context::{ContextBudget, DEFAULT_CACHE_TTL};
use saturn_core::sessions::{
    AgentRole, LastTurn, ReturnInputs, SendTarget, SessionError, SessionManager, SessionRecord,
};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, SessionId, SettingsRevision};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::SessionState;

use crate::settings::{ContextMode, SettingsError};
use crate::store::Store;
use crate::{ClientId, Engine, EngineError};

/// 입력 하나를 어느 session에 보낼지 정하는 데 쓰는 값.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SendRequest {
    pub(crate) chat: ChatId,
    pub(crate) provider: Provider,
    pub(crate) role: AgentRole,
    /// 돌아갈 provider에 넘길 패킷 크기(토큰).
    pub(crate) packet: u64,
    /// 입력이 접수 때 고정한 설정 번호.
    pub(crate) settings: SettingsRevision,
}

// cost: time O(a), heap O(a), stack O(1), io 1
// vars: a = 보관 session 수
// basis: estimate
/// 끝나지 않은 메인과 마지막 턴 값을 `SessionManager`에 되살린다. 크래시로 남은 `Open` 메인의 복구는 크래시 복구(#150)가 맡는다.
///
/// # Errors
/// 읽기 실패면 `Store`, 저장된 session이 규칙에 어긋나면 `Session`.
pub(crate) async fn restore_sessions(store: &Store) -> Result<SessionManager, EngineError> {
    let mut sessions = SessionManager::new();
    for (record, last_turn) in store.live_mains().await? {
        let id = record.id;
        sessions.register(record)?;
        if let Some(last_turn) = last_turn {
            sessions.record_last_turn(id, last_turn);
        }
    }
    Ok(sessions)
}

impl Engine {
    /// 기록 저장소에 먼저 쓰고, 성공하면 `sessions`에 알린다.
    ///
    /// # Errors
    /// 저장 실패면 `Store`이고 `sessions`는 바뀌지 않는다.
    pub(crate) async fn finish_turn(
        &mut self,
        session: SessionId,
        last_turn: LastTurn,
    ) -> Result<(), EngineError> {
        self.store.record_last_turn(session, last_turn).await?;
        self.sessions.record_last_turn(session, last_turn);
        Ok(())
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 붙는 TUI에 그 채팅 메인 session의 맥락 사용량을 알린다. 열려 있는 session이 이번 턴 값을 알면 그 값을, 아니면 기록에서
    /// 되살린 마지막 턴 값을 쓴다. 둘 다 모르면 보내지 않아 TUI가 `미확인`으로 둔다. 기준값을 읽지 못하면 로그만 남긴다.
    pub(crate) async fn send_chat_context(&self, client: ClientId, chat: ChatId) {
        let Some(main) = self.sessions.live_main(chat) else {
            return;
        };
        let live_tokens = self
            .flow
            .live
            .values()
            .find(|live| live.session == main.id)
            .and_then(|live| self.flow.context_tokens.get(&live.agent).copied().flatten());
        let Some(tokens) =
            live_tokens.or_else(|| self.sessions.last_turn(main.id).map(|last| last.active))
        else {
            return;
        };
        let threshold = match self.context_budget(chat, main.agent, main.provider).await {
            Ok(budget) => budget.threshold(),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read context budget");
                return;
            }
        };
        let notification = Notification::ContextSize {
            chat,
            tokens: Some(tokens),
            threshold,
        };
        self.send(client, notification).await;
    }

    // cost: time O(s), heap O(1), stack O(1), io 1
    // vars: s = session 수
    // basis: estimate
    /// 돌아갈 provider의 설정과 패킷 크기로 `target_for_send`의 재개 판정을 부른다.
    ///
    /// # Errors
    /// 설정 번호를 읽지 못하면 `Settings`.
    pub(crate) async fn send_target(
        &self,
        request: SendRequest,
        now: SystemTime,
    ) -> Result<SendTarget, EngineError> {
        let settings = self.settings.at(&self.store, request.settings).await?;
        let inputs = ReturnInputs {
            budget: ContextBudget {
                cache_ttl: self.cache_ttl(request.provider).await?,
                ..settings.context_budget(
                    request.provider,
                    self.registry.context_defaults(request.provider),
                )
            },
            packet: request.packet,
            now,
        };
        Ok(self
            .sessions
            .target_for_send(request.chat, request.provider, request.role, &inputs))
    }

    /// provider를 바꿀 때 떠나는 메인을 `replace` 대신 보관한다. 같은 provider의 더 오래된 보관 session은 `Ended`가 된다.
    ///
    /// # Errors
    /// 없는 session이거나 상태표에 없는 전이면 `Session`, 저장 실패면 `Store`.
    pub(crate) async fn archive_main(&mut self, leaving: SessionId) -> Result<(), EngineError> {
        self.sessions
            .set_state(leaving, SessionState::ClosedResumable)?;
        self.persist_sessions(leaving).await
    }

    /// 새 session을 `register`하고 저장한다. 같은 provider의 더 오래된 보관 session은 `Ended`가 된다.
    ///
    /// # Errors
    /// 중복 ID이거나 열린 메인이 이미 있으면 `Session`, 저장 실패면 `Store`.
    pub(crate) async fn register_session(
        &mut self,
        record: SessionRecord,
    ) -> Result<(), EngineError> {
        let id = record.id;
        self.sessions.register(record)?;
        self.persist_sessions(id).await
    }

    /// 맥락 정리로 `old`를 `Ended`로 두고 같은 채팅의 새 session으로 바꾼다. 턴 경계에서만 된다.
    ///
    /// # Errors
    /// 턴 경계가 아니거나 없는 session이면 `Session`, 저장 실패면 `Store`.
    pub(crate) async fn replace_session(
        &mut self,
        old: SessionId,
        record: SessionRecord,
    ) -> Result<(), EngineError> {
        let id = record.id;
        self.sessions.replace(old, record)?;
        self.persist_sessions(id).await
    }

    /// 입력 없이 에이전트와 채팅의 설정을 읽는 곳이 쓰는 번호. 에이전트가 가장 나중에 시작한 입력에 고정한 번호이고,
    /// 시작한 입력이 없으면(engine을 다시 켠 뒤 등) 그 채팅에서 마지막에 적용한 번호다.
    ///
    /// # Errors
    /// 번호가 없으면 `Settings`.
    pub(crate) fn revision_of_agent(
        &self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<SettingsRevision, EngineError> {
        Ok(self
            .flow
            .settings_of
            .get(&agent)
            .copied()
            .or(self.settings.latest_of(chat))
            .ok_or(SettingsError::NoPreviousRevision)?)
    }

    /// 그 에이전트의 입력에 고정한 설정으로 잰다. 턴이 끝난 session의 맥락 정리를 판정할 때 쓴다.
    ///
    /// # Errors
    /// 설정 번호를 읽지 못하면 `Settings`.
    pub(crate) async fn context_budget(
        &self,
        chat: ChatId,
        agent: AgentId,
        provider: Provider,
    ) -> Result<ContextBudget, EngineError> {
        let revision = self.revision_of_agent(chat, agent)?;
        let budget = self
            .settings
            .at(&self.store, revision)
            .await?
            .context_budget(provider, self.registry.context_defaults(provider));
        Ok(ContextBudget {
            cache_ttl: self.cache_ttl(provider).await?,
            ..budget
        })
    }

    /// 그 에이전트의 입력에 고정한 설정의 정리 모드 `context.mode`.
    ///
    /// # Errors
    /// 설정 번호를 읽지 못하면 `Settings`.
    pub(crate) async fn context_mode(
        &self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<ContextMode, EngineError> {
        let revision = self.revision_of_agent(chat, agent)?;
        Ok(self
            .settings
            .at(&self.store, revision)
            .await?
            .context_mode())
    }

    /// provider가 마지막으로 알려 준 캐시 유지 시간. 연결이 닫혔거나 engine을 다시 시작해도 기록 저장소에서 읽는다.
    /// 알려 준 적이 없을 때만 `DEFAULT_CACHE_TTL`.
    pub(crate) async fn cache_ttl(&self, provider: Provider) -> Result<Duration, EngineError> {
        Ok(self
            .store
            .cache_ttl_secs(provider)
            .await?
            .map_or(DEFAULT_CACHE_TTL, Duration::from_secs))
    }

    /// session 기록에서 채팅을 찾는다.
    ///
    /// # Errors
    /// 모르는 session이면 `Session(NotFound)`.
    pub(crate) fn session_chat(&self, session: SessionId) -> Result<ChatId, EngineError> {
        self.sessions
            .get(session)
            .map(|record| record.chat)
            .ok_or_else(|| SessionError::NotFound(session).into())
    }

    /// `Resume` 판정을 받은 보관 session을 `Open`으로 돌리고, 붙이기 시작할 기록 번호를 돌려준다.
    ///
    /// # Errors
    /// 다른 열린 메인이 있으면 `Session`(떠나는 메인을 먼저 보관한다), 저장 실패면 `Store`.
    pub(crate) async fn resume_main(
        &mut self,
        session: SessionId,
    ) -> Result<LedgerSeq, EngineError> {
        self.sessions.set_state(session, SessionState::Open)?;
        self.persist_sessions(session).await?;
        Ok(self.sessions.attach_from(session))
    }

    // cost: time O(s), heap O(s), stack O(1), io 1
    // vars: s = 채팅의 session 수
    // basis: estimate
    /// `changed`의 채팅에서 `sessions`가 아는 session의 현재 상태를 한 거래로 저장한다. 같은 채팅의 다른 session이 `Ended`로 바뀐 것까지 함께 남기기 위해서다.
    pub(crate) async fn persist_sessions(&self, changed: SessionId) -> Result<(), EngineError> {
        let Some(chat) = self.sessions.get(changed).map(|record| record.chat) else {
            return Ok(());
        };
        let mut ids: Vec<SessionId> = self
            .store
            .sessions(chat)
            .await?
            .into_iter()
            .map(|record| record.id)
            .collect();
        if !ids.contains(&changed) {
            ids.push(changed);
        }
        let records: Vec<SessionRecord> = ids
            .into_iter()
            .filter_map(|id| self.sessions.get(id).cloned())
            .collect();
        self.store.upsert_sessions(&records).await?;
        for record in records
            .iter()
            .filter(|record| record.state == SessionState::Ended)
        {
            self.store
                .delete_interrupted_subagents(record.agent)
                .await?;
        }
        Ok(())
    }
}
