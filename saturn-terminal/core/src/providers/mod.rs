//! provider 연결 공통 규격. provider 고유 이름은 쓰지 않는다.
//! 설계: docs/design/providers-and-sessions.md

use std::future::Future;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId, SettingsRevision, SubagentId};
use saturn_protocol::rpc::PermissionAnswer;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// 전달되지 않은 것이 확정이라 다시 보내도 된다.
    #[error("provider rejected before send: {reason}")]
    NotSent { reason: String },
    /// 다시 보내지 않고 사용자 확인으로 넘긴다.
    #[error("provider result unknown after send")]
    Unknown,
    /// 같은 session의 새 턴으로 보낸다.
    #[error("no active turn to steer")]
    NoActiveTurn,
    #[error("provider connection lost")]
    ConnectionLost,
}

/// 사용자 provider 설정에 값이 없을 때만 Saturn 기본값을 넣는다.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub agent: AgentId,
    pub workdir: std::path::PathBuf,
    /// `None`이면 provider 기본값.
    pub model: Option<String>,
    /// 입력 접수 때 고정한 값.
    pub settings: SettingsRevision,
    /// `None`이면 새로 연다.
    pub resume: Option<ProviderSessionId>,
    /// 새 session으로 이어 갈 때만 첫 턴에 넣는다.
    pub packet: Option<String>,
    /// 채팅에 더한 폴더. session을 열 때 provider에 넘기고, 이미 열린 session에는 반영하지 않는다.
    pub add_dirs: Vec<std::path::PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub provider_session: ProviderSessionId,
    /// 끼워 넣기 실측을 통과하지 않았으면 끼워 넣기를 대기로 바꾼다.
    pub steer_verified: bool,
}

#[derive(Debug, Clone)]
pub enum InterruptTarget {
    Subagent(SubagentId),
    Main,
}

#[derive(Debug, Clone)]
pub struct ProviderCommand {
    /// `/` 없이.
    pub name: String,
    pub description: String,
    pub is_skill: bool,
}

/// 모든 메서드는 보내기 전 실패(`NotSent`)와 보낸 뒤 불명(`Unknown`)을 구분해야 한다.
pub trait ProviderClient: Send {
    /// # Errors
    /// 연결 실패는 `ConnectionLost`, 재개 불가는 `NotSent`.
    fn open_session(
        &mut self,
        spec: SessionSpec,
    ) -> impl Future<Output = Result<SessionHandle, ProviderError>> + Send;

    fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 진행 중인 턴에 입력을 더한다.
    ///
    /// # Errors
    /// 활성 턴이 없으면 `NoActiveTurn`이고 호출자는 다시 판단하지 않고 `send_turn`으로 보낸다.
    fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 트리 전체 중지는 호출자가 subagent부터 차례로 부른다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `ConnectionLost`이고 호출자는 프로세스 묶음 중지로 넘어간다.
    fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// # Errors
    /// provider가 거절하면 `NotSent`.
    fn compact(
        &mut self,
        session: &ProviderSessionId,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// `PermissionRequested`의 `request_id`에 사용자 답을 provider 값으로 바꿔 돌려준다. 답이 갈 때까지
    /// provider는 그 호출에서 멈춰 있고, 답한 뒤 이어진다. `AllowAlways`를 보낼 값이 provider에 없으면
    /// `AllowOnce`로 보낸다.
    ///
    /// # Errors
    /// 모르는 요청(이미 답했거나 끝난 요청)이면 `NotSent`이고, 쓰기에 실패하면 구현이 정한다.
    fn answer_permission(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// provider session id는 호출자가 보관해 재개에 쓴다.
    fn close_session(
        &mut self,
        session: &ProviderSessionId,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 연결이 끝나면 `None`.
    fn next_event(&mut self) -> impl Future<Output = Option<ProviderEvent>> + Send;

    /// 화면 전용 명령과 Saturn 세션 명령이 대신하는 명령은 뺀다.
    fn commands(&self) -> Vec<ProviderCommand>;
}
