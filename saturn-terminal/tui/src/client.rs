//! engine 접속 클라이언트. TUI와 CLI가 함께 쓴다. Unix 소켓 위 JSON-RPC 한 줄에 메시지 하나.
//!
//! 설계: docs/design/engine-lifecycle.md(engine 시작, 여러 TUI 동시 접속).

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::CodecError;
use saturn_protocol::rpc::{Notification, Request};

/// engine 접속 오류.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// 소켓이 없거나 engine이 응답하지 않는다. 호출자는 engine을 시작하고 다시 붙는다.
    #[error("engine is not running at {path}")]
    NotRunning { path: PathBuf },
    /// 연결이 끊겼다.
    #[error("engine connection closed")]
    Closed,
    /// 메시지를 해석하지 못했다.
    #[error("failed to decode engine message")]
    Decode(#[from] CodecError),
}

/// engine과의 연결 하나.
#[derive(Debug)]
pub struct EngineClient {
    socket: PathBuf,
}

impl EngineClient {
    /// 사용자 소켓 경로(초안 `~/.saturn/engine.sock`, engine `rpc::SOCKET_FILE`과 같은 값). TODO(#49): 경로 설정 키
    pub fn default_socket() -> PathBuf {
        todo!("#89")
    }

    /// 소켓에 붙는다.
    ///
    /// # Errors
    /// engine이 없으면 `NotRunning`.
    pub async fn connect(socket: &Path) -> Result<Self, ClientError> {
        todo!("#89")
    }

    /// 요청 하나를 `ClientMessage` 봉투에 연결별 번호를 붙여 보낸다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `Closed`.
    pub async fn send(&mut self, request: Request) -> Result<(), ClientError> {
        todo!("#89")
    }

    /// 다음 알림을 기다린다. 요청의 `Response`는 번호로 짝지어 여기서 소비한다. 연결이 끝나면 `None`.
    pub async fn next(&mut self) -> Option<Notification> {
        todo!("#89")
    }
}
