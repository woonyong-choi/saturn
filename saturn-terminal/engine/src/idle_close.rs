//! 유휴 session 닫기: TUI 부착과 상관없이 트리 유휴 뒤 유예가 지난 메인 session을 닫아 보관한다.
//! 설계: docs/design/providers-and-sessions.md#session-닫기와-재개

use std::time::Instant;

use saturn_protocol::ids::SessionId;
use saturn_protocol::state::SessionState;

use crate::{Engine, EngineError};

impl Engine {
    /// 트리 유휴 뒤 유예(`idle_grace`)가 지난 메인 session을 닫는다. engine 전체의 백그라운드 종료와는 따로 돈다.
    /// 한 session의 실패는 경고로 남기고 나머지를 이어서 닫는다.
    pub(crate) async fn close_idle_sessions(&mut self, now: Instant) {
        let expired = self.sessions.idle_expired(now, self.idle_grace, |agent| {
            self.agents.is_tree_idle(agent)
        });
        for session in expired {
            if self.has_pending_work(session) {
                continue;
            }
            let closed = self.close_idle_session(session).await;
            self.warn_failure("failed to close idle session", closed);
        }
    }

    /// 채팅에 보내기 전 입력, 응답을 기다리는 전달, 멈추는 중인 일이 있거나 그 에이전트가 허가·입력 답이나 보류를 기다리면 참.
    fn has_pending_work(&self, session: SessionId) -> bool {
        let Some(record) = self.sessions.get(session) else {
            return true;
        };
        let (chat, agent) = (record.chat, record.agent);
        self.continuing_work(chat) > 0
            || self.flow.stopping.contains_key(&chat)
            || self.flow.permissions.values().any(|p| p.agent == agent)
            || self.flow.inputs.values().any(|input| input.agent == agent)
            || self
                .flow
                .held
                .values()
                .any(|held| held.agent == Some(agent))
    }

    /// provider session을 닫고 `ClosedResumable`로 보관한다. provider session ID와 전달 기록 번호는 그대로 남는다.
    /// 같은 연결을 쓰는 다른 session과 다른 채팅은 건드리지 않는다.
    async fn close_idle_session(&mut self, session: SessionId) -> Result<(), EngineError> {
        let record = self
            .sessions
            .get(session)
            .cloned()
            .ok_or(saturn_core::sessions::SessionError::NotFound(session))?;
        self.sessions
            .set_state(session, SessionState::ClosedResumable)?;
        self.persist_sessions(session).await?;
        let is_live = self
            .flow
            .live
            .get(&record.agent)
            .is_some_and(|live| live.session == session);
        if is_live {
            self.flow.live.remove(&record.agent);
        }
        if let (Some(provider_session), Ok(connection)) = (
            record.provider_session.as_ref(),
            self.provider_mut(record.chat, record.provider),
        ) {
            // 닫는 요청은 기다리지 않는다. 같은 연결의 뒤따르는 요청은 줄 선 순서대로 나간다
            connection.close_session_detached(provider_session.clone());
        }
        Ok(())
    }
}
