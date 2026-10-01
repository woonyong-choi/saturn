//! engine 접속 클라이언트. TUI와 CLI가 함께 쓴다.

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::CodecError;
use saturn_protocol::rpc::{Notification, Request};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// 호출자는 engine을 시작하고 다시 붙는다.
    #[error("engine is not running at {path}")]
    NotRunning { path: PathBuf },
    #[error("engine connection closed")]
    Closed,
    #[error("failed to decode engine message")]
    Decode(#[from] CodecError),
}

#[derive(Debug)]
pub struct EngineClient {
    socket: PathBuf,
}

impl EngineClient {
    /// engine `rpc::SOCKET_FILE`과 같은 경로. TODO(#49): 경로 설정 키
    pub fn default_socket() -> PathBuf {
        todo!("#89")
    }

    pub async fn connect(socket: &Path) -> Result<Self, ClientError> {
        todo!("#89")
    }

    /// 요청마다 연결별 번호를 붙인다.
    pub async fn send(&mut self, request: Request) -> Result<(), ClientError> {
        todo!("#89")
    }

    /// `Response`는 여기서 소비한다. 연결이 끝나면 `None`.
    pub async fn next(&mut self) -> Option<Notification> {
        todo!("#89")
    }
}
