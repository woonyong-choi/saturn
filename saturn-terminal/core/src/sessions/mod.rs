//! 메인 에이전트와 보조 에이전트의 session 수명, provider 전환, 기록 번호로 다른 에이전트 결과 전달.
//!
//! 설계: docs/design/providers-and-sessions.md(메인과 보조 에이전트 수명, provider 전환, session 상태).
//! 맥락 크기 판정과 패킷 구성은 `context`.

pub mod context;

use std::time::{Duration, Instant};

use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, ProviderSessionId, SessionId};
use saturn_protocol::state::SessionState;

/// 트리 유휴 뒤 session을 닫기까지 기다리는 시간.
pub const IDLE_GRACE: Duration = Duration::from_secs(5 * 60);

/// session 규칙 위반.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// 채팅에 이미 열린 메인 session이 있다. 채팅마다 살아 있는 메인 session은 하나다.
    #[error("chat already has an open main session")]
    MainAlreadyOpen,
    /// 턴 경계가 아니다. session 교체는 턴이 끝난 경계에서만 한다.
    #[error("session switch is only allowed at a turn boundary")]
    NotAtTurnBoundary,
    /// 없는 session.
    #[error("session not found: {0:?}")]
    NotFound(SessionId),
    /// session 상태표에 없는 전이.
    #[error("invalid session state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        /// 지금 상태.
        from: SessionState,
        /// 요청한 상태.
        to: SessionState,
    },
}

/// 에이전트 역할.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// 채팅마다 하나.
    Main,
    /// judge가 무관한 작업으로 판단해 띄운 보조 에이전트. 끝나면 결과를 전달하고 바로 종료한다.
    Sub,
}

/// session 기록 하나.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Saturn session id.
    pub id: SessionId,
    /// 소속 채팅.
    pub chat: ChatId,
    /// 에이전트.
    pub agent: AgentId,
    /// 역할.
    pub role: AgentRole,
    /// provider.
    pub provider: Provider,
    /// provider session id. 닫은 뒤 재개에 쓴다.
    pub provider_session: Option<ProviderSessionId>,
    /// 상태.
    pub state: SessionState,
    /// 이 session이 전달받은 마지막 기록 번호. 다음 입력 때 이 뒤의 변경분만 첨부한다.
    pub delivered: LedgerSeq,
    /// 트리 유휴가 된 시각. `IDLE_GRACE`가 지나면 닫는다.
    pub idle_since: Option<Instant>,
}

/// 보내는 순간 대상 session을 고르는 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendTarget {
    /// 열린 session에 보낸다.
    Open(SessionId),
    /// 닫힌 session을 보관한 id로 재개한 뒤 보낸다.
    Resume(SessionId),
    /// 새 session을 열고 패킷과 함께 보낸다(provider 전환이나 compaction 재시작).
    New { provider: Provider, role: AgentRole },
}

/// 채팅별 session 기록을 관리한다. 대기열은 session이 아니라 채팅에 둔다(교체 중 입력이 옛 session을 가리키지 않게).
#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: Vec<SessionRecord>,
}

impl SessionManager {
    /// 빈 관리자.
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 보내는 순간 대상 session을 고른다. 모델이나 provider가 바뀌면 새 메인을 열고 이전 메인은 인수 뒤 종료한다.
    /// 메인은 살아 있는 메인이 같은 provider면 열린 것은 그대로, 닫혔거나 보류된 것은 보관한 id로 재개한다.
    /// 보관한 id가 없거나 provider가 다르면 새 session이다. 보조 에이전트는 작업마다 새 session을 연다.
    pub fn target_for_send(&self, chat: ChatId, provider: Provider, role: AgentRole) -> SendTarget {
        let new = SendTarget::New { provider, role };
        if role == AgentRole::Sub {
            return new;
        }
        let Some(main) = self.live_main(chat) else {
            return new;
        };
        if main.provider != provider {
            return new;
        }
        match main.state {
            SessionState::Open => SendTarget::Open(main.id),
            SessionState::ClosedResumable | SessionState::Held
                if main.provider_session.is_some() =>
            {
                SendTarget::Resume(main.id)
            }
            _ => new,
        }
    }

    // cost: time O(s), heap O(1) amortized, stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 새 session을 기록한다.
    ///
    /// # Errors
    /// 채팅에 열린 메인이 있으면 `MainAlreadyOpen`. 끝나지 않은 메인(열림, 닫힘·재개 가능, 보류)이 있으면 열린 것으로 본다.
    pub fn register(&mut self, record: SessionRecord) -> Result<(), SessionError> {
        if record.role == AgentRole::Main
            && record.state != SessionState::Ended
            && self.live_main(record.chat).is_some()
        {
            return Err(SessionError::MainAlreadyOpen);
        }
        self.sessions.push(record);
        Ok(())
    }

    // cost: time O(s), heap O(1) amortized, stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// provider 전환이나 compaction 재시작. 턴 경계에서만 하고, 교체 중 들어온 입력은 새 session에 순서대로 보낸다.
    /// 이전 session은 `Ended`가 된다. 새 session의 전달 번호는 이전 session이 받은 번호보다 작아지지 않는다
    /// (패킷이 그 번호까지의 기록을 담으므로, 교체 뒤에는 그 뒤 변경분만 넘긴다).
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`, 턴 경계(트리 유휴, 닫힘, 보류)가 아니면 `NotAtTurnBoundary`,
    /// 이전 session 말고 다른 살아 있는 메인이 있으면 `MainAlreadyOpen`.
    pub fn replace(&mut self, old: SessionId, mut new: SessionRecord) -> Result<(), SessionError> {
        let index = self.index_of(old)?;
        let previous = &self.sessions[index];
        let is_turn_running = previous.state == SessionState::Open && previous.idle_since.is_none();
        if is_turn_running {
            return Err(SessionError::NotAtTurnBoundary);
        }
        if previous.state == SessionState::Ended {
            return Err(SessionError::NotFound(old));
        }
        let has_other_main = self.sessions.iter().any(|session| {
            session.id != old
                && session.chat == new.chat
                && session.role == AgentRole::Main
                && session.state != SessionState::Ended
        });
        if new.role == AgentRole::Main && has_other_main {
            return Err(SessionError::MainAlreadyOpen);
        }
        let previous = &mut self.sessions[index];
        previous.state = SessionState::Ended;
        previous.idle_since = None;
        new.delivered = new.delivered.max(previous.delivered);
        self.sessions.push(new);
        Ok(())
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 트리 유휴를 기록한다. `IDLE_GRACE` 뒤 `due_for_close`가 이 session을 돌려준다. 없는 session이면 무시한다.
    pub fn mark_idle(&mut self, session: SessionId, now: Instant) {
        if let Some(record) = self.find_mut(session) {
            record.idle_since = Some(now);
        }
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 턴을 시작한다. 유예 시계를 멈춘다. 없는 session이면 무시한다.
    pub fn mark_busy(&mut self, session: SessionId) {
        if let Some(record) = self.find_mut(session) {
            record.idle_since = None;
        }
    }

    // cost: time O(s), heap O(k), stack O(1), alloc 1
    // vars: s = session 수, k = 닫을 session 수
    // basis: estimate
    /// 유예가 지나 닫을 session. 닫은 뒤 상태는 `ClosedResumable`, provider session id는 보관한다.
    pub fn due_for_close(&self, now: Instant) -> Vec<SessionId> {
        self.sessions
            .iter()
            .filter(|session| session.state == SessionState::Open)
            .filter(|session| {
                session
                    .idle_since
                    .is_some_and(|since| now.saturating_duration_since(since) >= IDLE_GRACE)
            })
            .map(|session| session.id)
            .collect()
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 다음 입력에 첨부할 변경분의 시작 번호. 그 뒤 다른 에이전트의 결과 요약과 수정 파일 경로를 붙인다.
    /// 에이전트끼리 직접 통신하지 않고 이 번호로만 맥락을 넘긴다. 돌려준 번호 자체는 이미 받은 것이다.
    /// 없는 session이면 처음 번호(`LedgerSeq(0)`)를 준다.
    pub fn attach_from(&self, session: SessionId) -> LedgerSeq {
        self.sessions
            .iter()
            .find(|record| record.id == session)
            .map_or_else(LedgerSeq::default, |record| record.delivered)
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 전달한 마지막 기록 번호를 올린다. 더 작은 번호로는 내리지 않는다. 없는 session이면 무시한다.
    pub fn mark_delivered(&mut self, session: SessionId, seq: LedgerSeq) {
        if let Some(record) = self.find_mut(session) {
            record.delivered = record.delivered.max(seq);
        }
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 상태를 바꾼다(`Held`, `Ended` 등). 같은 상태로 바꾸면 아무것도 하지 않는다. `Open`이 아니게 되면 유예 시계를 지운다.
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`, session 상태표에 없는 전이면 `InvalidTransition`.
    pub fn set_state(
        &mut self,
        session: SessionId,
        state: SessionState,
    ) -> Result<(), SessionError> {
        let index = self.index_of(session)?;
        let record = &mut self.sessions[index];
        let from = record.state;
        if from == state {
            return Ok(());
        }
        if !can_move(from, state) {
            return Err(SessionError::InvalidTransition { from, to: state });
        }
        record.state = state;
        if state != SessionState::Open {
            record.idle_since = None;
        }
        Ok(())
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// session 기록 하나.
    pub fn get(&self, session: SessionId) -> Option<&SessionRecord> {
        self.sessions.iter().find(|record| record.id == session)
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 채팅의 끝나지 않은 메인 session.
    fn live_main(&self, chat: ChatId) -> Option<&SessionRecord> {
        self.sessions.iter().rev().find(|session| {
            session.chat == chat
                && session.role == AgentRole::Main
                && session.state != SessionState::Ended
        })
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    fn index_of(&self, session: SessionId) -> Result<usize, SessionError> {
        self.sessions
            .iter()
            .position(|record| record.id == session)
            .ok_or(SessionError::NotFound(session))
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    fn find_mut(&mut self, session: SessionId) -> Option<&mut SessionRecord> {
        self.sessions.iter_mut().find(|record| record.id == session)
    }
}

/// session 상태표의 전이인지.
fn can_move(from: SessionState, to: SessionState) -> bool {
    use SessionState::{ClosedResumable, Ended, Held, Open};
    matches!(
        (from, to),
        (Open, ClosedResumable | Held | Ended)
            | (ClosedResumable, Open | Ended)
            | (Held, Open | Ended)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: ChatId = ChatId(1);

    fn record(id: u64, provider: Provider, state: SessionState) -> SessionRecord {
        SessionRecord {
            id: SessionId(id),
            chat: CHAT,
            agent: AgentId(id),
            role: AgentRole::Main,
            provider,
            provider_session: Some(ProviderSessionId(format!("p{id}"))),
            state,
            delivered: LedgerSeq(0),
            idle_since: None,
        }
    }

    // cost: time O(s²), heap O(s), stack O(1)
    // vars: s = 테스트 session 수
    // basis: estimate
    fn manager_with(records: Vec<SessionRecord>) -> SessionManager {
        let mut manager = SessionManager::new();
        for record in records {
            manager.register(record).unwrap();
        }
        manager
    }

    #[test]
    fn target_for_send_without_session_returns_new() {
        let manager = SessionManager::new();

        let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main);

        assert_eq!(
            target,
            SendTarget::New {
                provider: Provider::Claude,
                role: AgentRole::Main
            }
        );
    }

    #[test]
    fn target_for_send_open_same_provider_returns_open() {
        let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main);

        assert_eq!(target, SendTarget::Open(SessionId(1)));
    }

    #[test]
    fn target_for_send_closed_session_returns_resume() {
        let manager = manager_with(vec![record(
            1,
            Provider::Codex,
            SessionState::ClosedResumable,
        )]);

        let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main);

        assert_eq!(target, SendTarget::Resume(SessionId(1)));
    }

    #[test]
    fn target_for_send_closed_without_provider_id_returns_new() {
        let mut closed = record(1, Provider::Codex, SessionState::ClosedResumable);
        closed.provider_session = None;
        let manager = manager_with(vec![closed]);

        let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main);

        assert!(matches!(target, SendTarget::New { .. }));
    }

    #[test]
    fn target_for_send_other_provider_returns_new() {
        let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let target = manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main);

        assert_eq!(
            target,
            SendTarget::New {
                provider: Provider::Codex,
                role: AgentRole::Main
            }
        );
    }

    #[test]
    fn target_for_send_sub_always_returns_new() {
        let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let target = manager.target_for_send(CHAT, Provider::Claude, AgentRole::Sub);

        assert_eq!(
            target,
            SendTarget::New {
                provider: Provider::Claude,
                role: AgentRole::Sub
            }
        );
    }

    #[test]
    fn register_second_live_main_returns_error() {
        let mut manager = manager_with(vec![record(
            1,
            Provider::Claude,
            SessionState::ClosedResumable,
        )]);

        let result = manager.register(record(2, Provider::Codex, SessionState::Open));

        assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
    }

    #[test]
    fn register_sub_beside_main_succeeds() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        let mut sub = record(2, Provider::Codex, SessionState::Open);
        sub.role = AgentRole::Sub;

        let result = manager.register(sub);

        assert!(result.is_ok());
    }

    #[test]
    fn replace_during_turn_returns_not_at_turn_boundary() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let result = manager.replace(SessionId(1), record(2, Provider::Codex, SessionState::Open));

        assert!(matches!(result, Err(SessionError::NotAtTurnBoundary)));
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = 테스트 session 수
    // basis: estimate
    #[test]
    fn replace_keeps_one_live_session_per_chat() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        manager.mark_idle(SessionId(1), Instant::now());

        manager
            .replace(SessionId(1), record(2, Provider::Codex, SessionState::Open))
            .unwrap();

        let live: Vec<SessionId> = manager
            .sessions
            .iter()
            .filter(|session| session.chat == CHAT && session.state != SessionState::Ended)
            .map(|session| session.id)
            .collect();
        assert_eq!(live, vec![SessionId(2)]);
    }

    #[test]
    fn replace_attaches_only_after_previous_delivered() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        manager.mark_delivered(SessionId(1), LedgerSeq(40));
        manager.mark_idle(SessionId(1), Instant::now());

        manager
            .replace(SessionId(1), record(2, Provider::Codex, SessionState::Open))
            .unwrap();

        assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(40));
    }

    #[test]
    fn replace_keeps_packet_seq_when_newer() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);
        manager.mark_delivered(SessionId(1), LedgerSeq(40));
        let mut new = record(2, Provider::Codex, SessionState::Open);
        new.delivered = LedgerSeq(55);

        manager.replace(SessionId(1), new).unwrap();

        assert_eq!(manager.attach_from(SessionId(2)), LedgerSeq(55));
    }

    #[test]
    fn replace_unknown_session_returns_not_found() {
        let mut manager = SessionManager::new();

        let result = manager.replace(SessionId(9), record(2, Provider::Codex, SessionState::Open));

        assert!(matches!(result, Err(SessionError::NotFound(SessionId(9)))));
    }

    #[test]
    fn due_for_close_waits_for_idle_grace() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        let idle = Instant::now();
        manager.mark_idle(SessionId(1), idle);

        let early = manager.due_for_close(idle + IDLE_GRACE - Duration::from_secs(1));
        let due = manager.due_for_close(idle + IDLE_GRACE);

        assert!(early.is_empty());
        assert_eq!(due, vec![SessionId(1)]);
    }

    #[test]
    fn due_for_close_busy_session_is_not_due() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        let idle = Instant::now();
        manager.mark_idle(SessionId(1), idle);

        manager.mark_busy(SessionId(1));

        assert!(manager.due_for_close(idle + IDLE_GRACE).is_empty());
    }

    #[test]
    fn due_for_close_closed_session_is_not_due() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        let idle = Instant::now();
        manager.mark_idle(SessionId(1), idle);

        manager
            .set_state(SessionId(1), SessionState::ClosedResumable)
            .unwrap();

        assert!(manager.due_for_close(idle + IDLE_GRACE).is_empty());
        assert!(
            manager
                .get(SessionId(1))
                .unwrap()
                .provider_session
                .is_some()
        );
    }

    #[test]
    fn mark_delivered_never_lowers() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);
        manager.mark_delivered(SessionId(1), LedgerSeq(10));

        manager.mark_delivered(SessionId(1), LedgerSeq(3));

        assert_eq!(manager.attach_from(SessionId(1)), LedgerSeq(10));
    }

    #[test]
    fn attach_from_unknown_session_returns_start() {
        let manager = SessionManager::new();

        assert_eq!(manager.attach_from(SessionId(1)), LedgerSeq(0));
    }

    #[test]
    fn set_state_follows_transition_table() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        manager.set_state(SessionId(1), SessionState::Held).unwrap();
        manager.set_state(SessionId(1), SessionState::Open).unwrap();
        manager
            .set_state(SessionId(1), SessionState::Ended)
            .unwrap();

        assert_eq!(
            manager.get(SessionId(1)).unwrap().state,
            SessionState::Ended
        );
    }

    #[test]
    fn set_state_from_ended_returns_error() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Ended)]);

        let result = manager.set_state(SessionId(1), SessionState::Open);

        assert!(matches!(
            result,
            Err(SessionError::InvalidTransition {
                from: SessionState::Ended,
                to: SessionState::Open
            })
        ));
    }

    #[test]
    fn set_state_closed_to_held_returns_error() {
        let mut manager = manager_with(vec![record(
            1,
            Provider::Claude,
            SessionState::ClosedResumable,
        )]);

        let result = manager.set_state(SessionId(1), SessionState::Held);

        assert!(result.is_err());
    }
}
