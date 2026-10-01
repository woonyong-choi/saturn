//! 보관 session의 마지막 턴 값 기록과 되살리기, 전송 대상 정하기, provider 전환 때 보관과 재개.
//! 설계: docs/design/providers-and-sessions.md

use std::time::SystemTime;

use saturn_core::sessions::{
    AgentRole, LastTurn, ReturnInputs, SendTarget, SessionManager, SessionRecord,
};
use saturn_protocol::ids::{ChatId, LedgerSeq, Provider, SessionId, SettingsRevision};
use saturn_protocol::state::SessionState;

use crate::store::Store;
use crate::{Engine, EngineError};

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
            budget: settings.context_budget(request.provider),
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
    async fn persist_sessions(&self, changed: SessionId) -> Result<(), EngineError> {
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
        Ok(())
    }
}
