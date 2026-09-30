//! provider 연결 공통 규격. 구현은 `saturn-engine`의 `providers::{codex, claude}`.
//!
//! 설계: docs/design/providers-and-sessions.md(provider 연결, 입력 전달, 세션 닫기와 재개).
//! provider 고유 메서드 이름(`turn/steer` 등)은 구현 안에서만 쓰고 여기서는 Saturn 용어만 쓴다.

use std::future::Future;

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId, SettingsRevision, SubagentId};

/// provider 연결 오류.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// 보내기 전에 실패했다. 전달되지 않은 것이 확정이라 다시 보내도 된다.
    #[error("provider rejected before send: {reason}")]
    NotSent { reason: String },
    /// 보낸 뒤 결과를 모른다. 다시 보내지 않고 사용자 확인으로 넘긴다.
    #[error("provider result unknown after send")]
    Unknown,
    /// 끼워 넣기할 활성 턴이 없다. 같은 session의 새 턴으로 보낸다.
    #[error("no active turn to steer")]
    NoActiveTurn,
    /// provider 프로세스나 연결이 끊겼다.
    #[error("provider connection lost")]
    ConnectionLost,
}

/// 새 session이나 재개에 필요한 값. 사용자 provider 설정에 값이 없을 때만 Saturn 기본값을 인자로 넣는다.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// 이 session을 쓰는 Saturn 에이전트.
    pub agent: AgentId,
    /// 작업 폴더.
    pub workdir: std::path::PathBuf,
    /// 모델. `None`이면 provider 기본값.
    pub model: Option<String>,
    /// 입력 접수 때 고정한 설정 번호.
    pub settings: SettingsRevision,
    /// 재개할 provider session id. `None`이면 새로 연다.
    pub resume: Option<ProviderSessionId>,
    /// 첫 턴에 넣을 패킷 본문. 새 session으로 이어 갈 때만.
    pub packet: Option<String>,
}

/// 열린 session 손잡이.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    /// provider가 준 session id. 닫은 뒤 재개에 쓴다.
    pub provider_session: ProviderSessionId,
    /// 이 연결이 끼워 넣기 실측을 통과했는지. 아니면 끼워 넣기를 대기로 바꾼다.
    pub steer_verified: bool,
}

/// 멈춤 신호 대상.
#[derive(Debug, Clone)]
pub enum InterruptTarget {
    /// 추적된 subagent 하나.
    Subagent(SubagentId),
    /// 메인 턴.
    Main,
}

/// provider 명령이나 스킬 하나. `/` 팝업과 `$` 목록에 쓴다.
#[derive(Debug, Clone)]
pub struct ProviderCommand {
    /// 이름(`/` 없이).
    pub name: String,
    /// 설명.
    pub description: String,
    /// 스킬이면 참.
    pub is_skill: bool,
}

/// provider 연결 하나. session 여러 개를 다룰 수 있다(Codex app-server 하나에 thread 여러 개).
///
/// 모든 메서드는 보내기 전 실패와 보낸 뒤 불명을 `ProviderError`로 구분해야 한다.
pub trait ProviderClient: Send {
    /// session을 열거나 재개한다.
    ///
    /// # Errors
    /// 연결 실패는 `ConnectionLost`, 재개 불가는 `NotSent`.
    fn open_session(
        &mut self,
        spec: SessionSpec,
    ) -> impl Future<Output = Result<SessionHandle, ProviderError>> + Send;

    /// 새 턴으로 입력을 보낸다(Codex `turn/start`, Claude 스트림 입력).
    ///
    /// # Errors
    /// `NotSent`면 다시 보내도 되고 `Unknown`이면 보내지 않는다.
    fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 진행 중인 턴에 입력을 더한다(Codex `turn/steer`, Claude 스트림 입력 추가).
    ///
    /// # Errors
    /// 활성 턴이 없으면 `NoActiveTurn`. 호출자는 다시 판단하지 않고 `send_turn`으로 보낸다.
    fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 멈춤 신호를 보낸다. 트리 전체 중지는 호출자가 subagent부터 차례로 부른다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `ConnectionLost`. 호출자는 프로세스 묶음 중지로 넘어간다.
    fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// provider 압축을 요청한다(Codex `thread/compact/start`, Claude `/compact`).
    ///
    /// # Errors
    /// provider가 거절하면 `NotSent`.
    fn compact(
        &mut self,
        session: &ProviderSessionId,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// session을 닫는다. provider session id는 호출자가 보관해 재개에 쓴다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `ConnectionLost`.
    fn close_session(
        &mut self,
        session: &ProviderSessionId,
    ) -> impl Future<Output = Result<(), ProviderError>> + Send;

    /// 다음 이벤트를 기다린다. 연결이 끝나면 `None`.
    fn next_event(&mut self) -> impl Future<Output = Option<ProviderEvent>> + Send;

    /// provider 명령과 스킬 목록. 화면 전용 명령과 Saturn 세션 명령이 대신하는 명령은 뺀다.
    fn commands(&self) -> Vec<ProviderCommand>;
}
