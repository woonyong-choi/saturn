//! 메인·보조 에이전트의 session 수명, provider 전환, 기록 번호로 결과 전달.
//! 설계: docs/design/providers-and-sessions.md

pub mod context;
pub mod fragments;
pub mod memo;
pub mod packet;
pub mod ranking;
pub mod stamp;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq, Provider, ProviderSessionId, SessionId};
use saturn_protocol::state::SessionState;

use self::context::{ContextBudget, ReturnDecision, decide_return};

/// 트리 유휴 뒤 session을 닫기까지 기다리는 시간.
pub const IDLE_GRACE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// 채팅마다 살아 있는 메인 session은 하나다.
    #[error("chat already has an open main session")]
    MainAlreadyOpen,
    /// session 교체는 턴이 끝난 경계에서만 한다.
    #[error("session switch is only allowed at a turn boundary")]
    NotAtTurnBoundary,
    #[error("session not found: {0:?}")]
    NotFound(SessionId),
    #[error("session id already exists: {0:?}")]
    DuplicateId(SessionId),
    #[error("replacement must belong to the same chat and role")]
    InvalidReplacement,
    #[error("invalid session state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        from: SessionState,
        to: SessionState,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// 채팅마다 하나.
    Main,
    /// 끝나면 결과를 전달하고 바로 종료한다.
    Sub,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: SessionId,
    pub chat: ChatId,
    pub agent: AgentId,
    pub role: AgentRole,
    pub provider: Provider,
    /// 닫은 뒤 재개에 쓴다.
    pub provider_session: Option<ProviderSessionId>,
    pub state: SessionState,
    /// 다음 입력 때 이 번호 뒤의 변경분만 첨부한다.
    pub delivered: LedgerSeq,
    /// `IDLE_GRACE`가 지나면 닫는다.
    pub idle_since: Option<Instant>,
}

/// `New`는 새 session을 열어 패킷과 함께 보낸다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendTarget {
    Open(SessionId),
    Resume(SessionId),
    New { provider: Provider, role: AgentRole },
}

/// 보관 session으로 돌아갈 때 재개 판정에 쓰는 마지막 턴의 값. 저장은 engine이 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastTurn {
    /// 마지막 활성 맥락 `A`(토큰).
    pub active: u64,
    pub ended_at: SystemTime,
}

/// `budget`은 돌아갈 provider의 것이고, `packet`은 그 provider에 넘길 패킷 크기(토큰)다.
#[derive(Debug, Clone, Copy)]
pub struct ReturnInputs {
    pub budget: ContextBudget,
    pub packet: u64,
    pub now: SystemTime,
}

#[derive(Debug, Default)]
pub struct SessionManager {
    sessions: Vec<SessionRecord>,
    last_turns: HashMap<SessionId, LastTurn>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 열린 메인이 같은 provider면 열린 session에 보낸다. provider를 바꾸는 입력이면 돌아갈 provider의 보관 session을 `decide_return`으로 재개와 새 session으로 가르고, 마지막 턴 값이 없으면 재개한다. 보조 에이전트는 작업마다 새 session을 연다.
    pub fn target_for_send(
        &self,
        chat: ChatId,
        provider: Provider,
        role: AgentRole,
        inputs: &ReturnInputs,
    ) -> SendTarget {
        let new = SendTarget::New { provider, role };
        if role == AgentRole::Sub {
            return new;
        }
        let Some(main) = self.live_main(chat) else {
            return new;
        };
        if main.provider == provider {
            return match main.state {
                SessionState::Open => SendTarget::Open(main.id),
                SessionState::ClosedResumable | SessionState::Held
                    if main.provider_session.is_some() =>
                {
                    SendTarget::Resume(main.id)
                }
                _ => new,
            };
        }
        let Some(archived) = self.archived_main(chat, provider) else {
            return new;
        };
        match self.decide_resume(archived.id, inputs) {
            ReturnDecision::Resume => SendTarget::Resume(archived.id),
            ReturnDecision::NewSession => new,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 같은 session의 이전 값은 덮어쓴다.
    pub fn record_last_turn(&mut self, session: SessionId, last_turn: LastTurn) {
        self.last_turns.insert(session, last_turn);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub fn last_turn(&self, session: SessionId) -> Option<LastTurn> {
        self.last_turns.get(&session).copied()
    }

    // cost: time O(s), heap O(1) amortized, stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 보관(`ClosedResumable`) 메인을 넣으면 같은 채팅·provider의 더 오래된 보관 session은 `Ended`로 둔다.
    ///
    /// # Errors
    /// 중복 ID면 `DuplicateId`, 열린 메인이 있는데 열린 메인을 넣으면 `MainAlreadyOpen`.
    pub fn register(&mut self, record: SessionRecord) -> Result<(), SessionError> {
        if self.get(record.id).is_some() {
            return Err(SessionError::DuplicateId(record.id));
        }
        if record.role == AgentRole::Main
            && record.state == SessionState::Open
            && self.open_main(record.chat).is_some()
        {
            return Err(SessionError::MainAlreadyOpen);
        }
        let (id, chat, provider) = (record.id, record.chat, record.provider);
        let is_archive =
            record.role == AgentRole::Main && record.state == SessionState::ClosedResumable;
        self.sessions.push(record);
        if is_archive {
            self.end_older_archives(chat, provider, id);
        }
        Ok(())
    }

    // cost: time O(s), heap O(1) amortized, stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 새 session의 전달 번호는 이전 session이 받은 번호보다 작아지지 않는다.
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`, 중복 ID면 `DuplicateId`, 채팅·역할이 다르면 `InvalidReplacement`, 턴 경계가 아니면 `NotAtTurnBoundary`, 다른 열린 메인이 있는데 열린 메인을 넣으면 `MainAlreadyOpen`.
    pub fn replace(&mut self, old: SessionId, mut new: SessionRecord) -> Result<(), SessionError> {
        let index = self.index_of(old)?;
        let previous = &self.sessions[index];
        if self.get(new.id).is_some() {
            return Err(SessionError::DuplicateId(new.id));
        }
        if previous.chat != new.chat || previous.role != new.role {
            return Err(SessionError::InvalidReplacement);
        }
        let is_turn_running = previous.state == SessionState::Open && previous.idle_since.is_none();
        if is_turn_running {
            return Err(SessionError::NotAtTurnBoundary);
        }
        if previous.state == SessionState::Ended {
            return Err(SessionError::NotFound(old));
        }
        let has_other_open_main = self.sessions.iter().any(|session| {
            session.id != old
                && session.chat == new.chat
                && session.role == AgentRole::Main
                && session.state == SessionState::Open
        });
        if new.role == AgentRole::Main && new.state == SessionState::Open && has_other_open_main {
            return Err(SessionError::MainAlreadyOpen);
        }
        let previous = &mut self.sessions[index];
        previous.state = SessionState::Ended;
        previous.idle_since = None;
        new.delivered = new.delivered.max(previous.delivered);
        let (id, chat, provider) = (new.id, new.chat, new.provider);
        let is_archive = new.role == AgentRole::Main && new.state == SessionState::ClosedResumable;
        self.sessions.push(new);
        if is_archive {
            self.end_older_archives(chat, provider, id);
        }
        Ok(())
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 없는 session이면 무시한다.
    pub fn mark_idle(&mut self, session: SessionId, now: Instant) {
        if let Some(record) = self.find_mut(session) {
            record.idle_since = Some(now);
        }
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 없는 session이면 무시한다.
    pub fn mark_busy(&mut self, session: SessionId) {
        if let Some(record) = self.find_mut(session) {
            record.idle_since = None;
        }
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 돌려준 번호 자체는 이미 받은 것이고, 없는 session이면 `LedgerSeq(0)`이다.
    pub fn attach_from(&self, session: SessionId) -> LedgerSeq {
        self.sessions
            .iter()
            .find(|record| record.id == session)
            .map_or_else(LedgerSeq::default, |record| record.delivered)
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 더 작은 번호로는 내리지 않고, 없는 session이면 무시한다.
    pub fn mark_delivered(&mut self, session: SessionId, seq: LedgerSeq) {
        if let Some(record) = self.find_mut(session) {
            record.delivered = record.delivered.max(seq);
        }
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 같은 상태면 아무것도 하지 않고, `Open`이 아니게 되면 유예 시계를 지운다. 메인이 `ClosedResumable`이 되면 같은 채팅·provider의 더 오래된 보관 session은 `Ended`로 둔다.
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`, session 상태표에 없는 전이면 `InvalidTransition`, 다른 열린 메인이 있는데 메인을 `Open`으로 돌리면 `MainAlreadyOpen`.
    pub fn set_state(
        &mut self,
        session: SessionId,
        state: SessionState,
    ) -> Result<(), SessionError> {
        let index = self.index_of(session)?;
        let record = &self.sessions[index];
        let from = record.state;
        if from == state {
            return Ok(());
        }
        if !can_move(from, state) {
            return Err(SessionError::InvalidTransition { from, to: state });
        }
        let (chat, provider, is_main) =
            (record.chat, record.provider, record.role == AgentRole::Main);
        let other_open = self.open_main(chat).is_some_and(|open| open.id != session);
        if is_main && state == SessionState::Open && other_open {
            return Err(SessionError::MainAlreadyOpen);
        }
        let record = &mut self.sessions[index];
        record.state = state;
        if state != SessionState::Open {
            record.idle_since = None;
        }
        if is_main && state == SessionState::ClosedResumable {
            self.end_older_archives(chat, provider, session);
        }
        Ok(())
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    pub fn get(&self, session: SessionId) -> Option<&SessionRecord> {
        self.sessions.iter().find(|record| record.id == session)
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    /// 열린 메인이 있으면 그 메인, 없으면 `Ended`가 아닌 가장 나중 메인. engine이 이어 갈 provider를 정하는 데 쓴다.
    /// 열린 메인을 먼저 보는 것은 provider를 오간 뒤 등록 순서가 열린 순서와 달라지기 때문이다.
    pub fn live_main(&self, chat: ChatId) -> Option<&SessionRecord> {
        self.open_main(chat).or_else(|| {
            self.sessions.iter().rev().find(|session| {
                session.chat == chat
                    && session.role == AgentRole::Main
                    && session.state != SessionState::Ended
            })
        })
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    fn open_main(&self, chat: ChatId) -> Option<&SessionRecord> {
        self.sessions.iter().find(|session| {
            session.chat == chat
                && session.role == AgentRole::Main
                && session.state == SessionState::Open
        })
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    fn archived_main(&self, chat: ChatId, provider: Provider) -> Option<&SessionRecord> {
        self.sessions.iter().rev().find(|session| {
            session.chat == chat
                && session.role == AgentRole::Main
                && session.provider == provider
                && matches!(
                    session.state,
                    SessionState::ClosedResumable | SessionState::Held
                )
                && session.provider_session.is_some()
        })
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn decide_resume(&self, session: SessionId, inputs: &ReturnInputs) -> ReturnDecision {
        let Some(last) = self.last_turns.get(&session) else {
            return ReturnDecision::Resume;
        };
        let since_last_turn = inputs
            .now
            .duration_since(last.ended_at)
            .unwrap_or(Duration::ZERO);
        decide_return(&inputs.budget, since_last_turn, last.active, inputs.packet)
    }

    // cost: time O(s), heap O(1), stack O(1)
    // vars: s = session 수
    // basis: estimate
    fn end_older_archives(&mut self, chat: ChatId, provider: Provider, keep: SessionId) {
        for session in &mut self.sessions {
            let is_older_archive = session.id != keep
                && session.chat == chat
                && session.role == AgentRole::Main
                && session.provider == provider
                && session.state == SessionState::ClosedResumable;
            if is_older_archive {
                session.state = SessionState::Ended;
                session.idle_since = None;
                self.last_turns.remove(&session.id);
            }
        }
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
    const NOW_SECS: u64 = 1_000_000;

    fn inputs(packet: u64) -> ReturnInputs {
        ReturnInputs {
            budget: ContextBudget {
                t_abs: 100_000,
                safety_percent: 60,
                window: 200_000,
                cache_read: 0.1,
                cache_write: 1.25,
                cache_ttl: Duration::from_secs(300),
            },
            packet,
            now: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS),
        }
    }

    fn last_turn(active: u64, secs_ago: u64) -> LastTurn {
        LastTurn {
            active,
            ended_at: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS - secs_ago),
        }
    }

    /// Claude가 열림, Codex 보관(id 1)인 채팅. Codex로 돌아가는 판정을 시험한다.
    fn archived_codex(active: u64, secs_ago: u64) -> SessionManager {
        let mut manager = manager_with(vec![
            record(1, Provider::Codex, SessionState::ClosedResumable),
            record(2, Provider::Claude, SessionState::Open),
        ]);
        manager.record_last_turn(SessionId(1), last_turn(active, secs_ago));
        manager
    }

    fn return_target(manager: &SessionManager, packet: u64) -> SendTarget {
        manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(packet))
    }

    fn new_codex() -> SendTarget {
        SendTarget::New {
            provider: Provider::Codex,
            role: AgentRole::Main,
        }
    }

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

        let target =
            manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main, &inputs(50_000));

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

        let target =
            manager.target_for_send(CHAT, Provider::Claude, AgentRole::Main, &inputs(50_000));

        assert_eq!(target, SendTarget::Open(SessionId(1)));
    }

    #[test]
    fn target_for_send_closed_session_returns_resume() {
        let manager = manager_with(vec![record(
            1,
            Provider::Codex,
            SessionState::ClosedResumable,
        )]);

        let target =
            manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

        assert_eq!(target, SendTarget::Resume(SessionId(1)));
    }

    #[test]
    fn target_for_send_closed_without_provider_id_returns_new() {
        let mut closed = record(1, Provider::Codex, SessionState::ClosedResumable);
        closed.provider_session = None;
        let manager = manager_with(vec![closed]);

        let target =
            manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

        assert!(matches!(target, SendTarget::New { .. }));
    }

    #[test]
    fn target_for_send_other_provider_returns_new() {
        let manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let target =
            manager.target_for_send(CHAT, Provider::Codex, AgentRole::Main, &inputs(50_000));

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

        let target =
            manager.target_for_send(CHAT, Provider::Claude, AgentRole::Sub, &inputs(50_000));

        assert_eq!(
            target,
            SendTarget::New {
                provider: Provider::Claude,
                role: AgentRole::Sub
            }
        );
    }

    #[test]
    fn register_second_open_main_returns_error() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Open)]);

        let result = manager.register(record(2, Provider::Codex, SessionState::Open));

        assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
    }

    #[test]
    fn register_duplicate_id_returns_error() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Ended)]);

        let result = manager.register(record(1, Provider::Codex, SessionState::Open));

        assert!(matches!(
            result,
            Err(SessionError::DuplicateId(SessionId(1)))
        ));
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
    fn replace_other_chat_keeps_original_session() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);
        let mut other_chat = record(2, Provider::Codex, SessionState::Open);
        other_chat.chat = ChatId(2);

        let result = manager.replace(SessionId(1), other_chat);

        assert!(matches!(result, Err(SessionError::InvalidReplacement)));
        assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
    }

    #[test]
    fn replace_duplicate_id_keeps_original_session() {
        let mut manager = manager_with(vec![record(1, Provider::Claude, SessionState::Held)]);

        let result = manager.replace(SessionId(1), record(1, Provider::Codex, SessionState::Open));

        assert!(matches!(
            result,
            Err(SessionError::DuplicateId(SessionId(1)))
        ));
        assert_eq!(manager.get(SessionId(1)).unwrap().state, SessionState::Held);
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

    #[test]
    fn target_for_send_warm_below_threshold_resumes_archive() {
        let manager = archived_codex(99_999, 300);

        assert_eq!(
            return_target(&manager, 50_000),
            SendTarget::Resume(SessionId(1))
        );
    }

    #[test]
    fn target_for_send_warm_at_threshold_returns_new() {
        let manager = archived_codex(100_000, 10);

        assert_eq!(return_target(&manager, 50_000), new_codex());
    }

    #[test]
    fn target_for_send_expired_smaller_packet_returns_new() {
        let manager = archived_codex(80_000, 301);

        assert_eq!(return_target(&manager, 79_999), new_codex());
    }

    #[test]
    fn target_for_send_expired_packet_not_smaller_resumes_archive() {
        let manager = archived_codex(80_000, 301);

        assert_eq!(
            return_target(&manager, 80_000),
            SendTarget::Resume(SessionId(1))
        );
    }

    #[test]
    fn target_for_send_archive_without_last_turn_resumes() {
        let manager = manager_with(vec![
            record(1, Provider::Codex, SessionState::ClosedResumable),
            record(2, Provider::Claude, SessionState::Open),
        ]);

        assert_eq!(
            return_target(&manager, 50_000),
            SendTarget::Resume(SessionId(1))
        );
    }

    #[test]
    fn live_main_prefers_the_open_main_over_a_later_registered_archive() {
        let manager = manager_with(vec![
            record(1, Provider::Codex, SessionState::Open),
            record(2, Provider::Claude, SessionState::ClosedResumable),
        ]);

        assert_eq!(
            manager.live_main(CHAT).map(|main| main.id),
            Some(SessionId(1))
        );
        assert_eq!(
            return_target(&manager, 50_000),
            SendTarget::Open(SessionId(1))
        );
    }

    #[test]
    fn target_for_send_archive_without_provider_id_returns_new() {
        let mut archive = record(1, Provider::Codex, SessionState::ClosedResumable);
        archive.provider_session = None;
        let manager = manager_with(vec![
            archive,
            record(2, Provider::Claude, SessionState::Open),
        ]);

        assert_eq!(return_target(&manager, 50_000), new_codex());
    }

    #[test]
    fn target_for_send_clock_before_last_turn_counts_as_warm() {
        let mut manager = archived_codex(10_000, 0);
        manager.record_last_turn(
            SessionId(1),
            LastTurn {
                active: 150_000,
                ended_at: SystemTime::UNIX_EPOCH + Duration::from_secs(NOW_SECS + 60),
            },
        );

        assert_eq!(return_target(&manager, 50_000), new_codex());
    }

    // cost: time O(s), heap O(s), stack O(1), alloc 1
    // vars: s = 테스트 session 수
    // basis: estimate
    fn states(manager: &SessionManager) -> Vec<SessionState> {
        manager
            .sessions
            .iter()
            .map(|session| session.state)
            .collect()
    }

    #[test]
    fn provider_switches_keep_one_open_and_one_archive_per_provider() {
        let mut manager = manager_with(vec![record(1, Provider::Codex, SessionState::Open)]);
        manager
            .set_state(SessionId(1), SessionState::ClosedResumable)
            .unwrap();
        manager
            .register(record(2, Provider::Claude, SessionState::Open))
            .unwrap();
        manager
            .set_state(SessionId(2), SessionState::ClosedResumable)
            .unwrap();
        manager
            .register(record(3, Provider::Codex, SessionState::Open))
            .unwrap();

        manager
            .set_state(SessionId(3), SessionState::ClosedResumable)
            .unwrap();

        assert_eq!(
            states(&manager),
            vec![
                SessionState::Ended,
                SessionState::ClosedResumable,
                SessionState::ClosedResumable
            ]
        );
    }

    #[test]
    fn register_second_archive_ends_older_one_and_drops_its_last_turn() {
        let mut manager = manager_with(vec![record(
            1,
            Provider::Codex,
            SessionState::ClosedResumable,
        )]);
        manager.record_last_turn(SessionId(1), last_turn(1, 1));

        manager
            .register(record(2, Provider::Codex, SessionState::ClosedResumable))
            .unwrap();

        assert_eq!(
            states(&manager),
            vec![SessionState::Ended, SessionState::ClosedResumable]
        );
        assert_eq!(manager.last_turn(SessionId(1)), None);
    }

    #[test]
    fn replace_with_archive_ends_older_archive_of_same_provider() {
        let mut manager = manager_with(vec![
            record(1, Provider::Codex, SessionState::ClosedResumable),
            record(2, Provider::Claude, SessionState::Open),
        ]);
        manager.mark_idle(SessionId(2), Instant::now());

        manager
            .replace(
                SessionId(2),
                record(3, Provider::Codex, SessionState::ClosedResumable),
            )
            .unwrap();

        assert_eq!(
            states(&manager),
            vec![
                SessionState::Ended,
                SessionState::Ended,
                SessionState::ClosedResumable
            ]
        );
    }

    #[test]
    fn archive_cap_keeps_held_session_and_other_provider() {
        let mut manager = manager_with(vec![
            record(1, Provider::Codex, SessionState::Held),
            record(2, Provider::Claude, SessionState::ClosedResumable),
        ]);

        manager
            .register(record(3, Provider::Codex, SessionState::ClosedResumable))
            .unwrap();

        assert_eq!(
            states(&manager),
            vec![
                SessionState::Held,
                SessionState::ClosedResumable,
                SessionState::ClosedResumable
            ]
        );
    }

    #[test]
    fn set_state_resume_beside_open_main_returns_error() {
        let mut manager = archived_codex(1, 1);

        let result = manager.set_state(SessionId(1), SessionState::Open);

        assert!(matches!(result, Err(SessionError::MainAlreadyOpen)));
    }
}
