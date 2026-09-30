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
#[derive(Debug, Clone)]
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

    /// 보내는 순간 대상 session을 고른다. 모델이나 provider가 바뀌면 새 메인을 열고 이전 메인은 인수 뒤 종료한다.
    pub fn target_for_send(&self, chat: ChatId, provider: Provider, role: AgentRole) -> SendTarget {
        todo!("#77")
    }

    /// 새 session을 기록한다.
    ///
    /// # Errors
    /// 채팅에 열린 메인이 있으면 `MainAlreadyOpen`.
    pub fn register(&mut self, record: SessionRecord) -> Result<(), SessionError> {
        todo!("#77")
    }

    /// provider 전환이나 compaction 재시작. 턴 경계에서만 하고, 교체 중 들어온 입력은 새 session에 순서대로 보낸다.
    ///
    /// # Errors
    /// 턴 경계가 아니면 `NotAtTurnBoundary`.
    pub fn replace(&mut self, old: SessionId, new: SessionRecord) -> Result<(), SessionError> {
        todo!("#77")
    }

    /// 트리 유휴를 기록한다. `IDLE_GRACE` 뒤 `due_for_close`가 이 session을 돌려준다.
    pub fn mark_idle(&mut self, session: SessionId, now: Instant) {
        todo!("#77")
    }

    /// 유예가 지나 닫을 session. 닫은 뒤 상태는 `ClosedResumable`, provider session id는 보관한다.
    pub fn due_for_close(&self, now: Instant) -> Vec<SessionId> {
        todo!("#77")
    }

    /// 다음 입력에 첨부할 변경분의 시작 번호. 그 뒤 다른 에이전트의 결과 요약과 수정 파일 경로를 붙인다.
    /// 에이전트끼리 직접 통신하지 않고 이 번호로만 맥락을 넘긴다.
    pub fn attach_from(&self, session: SessionId) -> LedgerSeq {
        todo!("#77")
    }

    /// 전달한 마지막 기록 번호를 올린다.
    pub fn mark_delivered(&mut self, session: SessionId, seq: LedgerSeq) {
        todo!("#77")
    }

    /// 상태를 바꾼다(`Held`, `Ended` 등).
    pub fn set_state(&mut self, session: SessionId, state: SessionState) {
        todo!("#77")
    }
}
