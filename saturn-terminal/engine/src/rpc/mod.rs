//! 사용자당 engine 잠금과 Unix 소켓 JSON-RPC 서버. 설계: docs/design/engine-lifecycle.md
//! TODO(#46): 메서드 이름과 목록 확정
//! 잠금 없이 소켓을 열지 않는다.

mod connection;
mod lock;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use saturn_protocol::envelope::{CodecError, RequestId, Response, ServerMessage};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{Notification, Request};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

pub use lock::EngineLock;

/// `~/.saturn/` 아래. TUI `EngineClient::default_socket`과 같은 경로.
/// TODO(#89): 값 미정, 초안 `engine.sock`
/// TODO(#49): 경로 설정 키
pub const SOCKET_FILE: &str = "engine.sock";

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    /// cli는 기존 engine 소켓에 붙는다.
    #[error("engine already running: {lock}")]
    AlreadyRunning { lock: PathBuf },
    #[error("failed to lock engine: {path}")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to bind engine socket: {path}")]
    Bind {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("client connection failed")]
    Io(#[from] std::io::Error),
    /// 그 줄만 버리고 연결은 유지한다.
    #[error("failed to decode client message")]
    Decode(#[from] CodecError),
    #[error("client not connected: {0:?}")]
    ClientGone(ClientId),
}

/// engine 실행 안에서만 유효.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

#[derive(Debug)]
pub enum RpcEvent {
    /// 아직 `Request::Attach` 전.
    Connected(ClientId),
    Request(ClientId, RequestId, Request),
    /// `Request::Detach` 포함.
    Disconnected(ClientId),
    /// `Disconnected` 바로 뒤에 한 번 온다.
    LastDetached,
}

#[derive(Debug)]
struct ClientHandle {
    outbox: mpsc::Sender<ServerMessage>,
    /// `Attach` 전이면 `None`.
    chat: Option<ChatId>,
}

/// 잠금을 소유해 서버가 사는 동안 잠금이 유지된다.
#[derive(Debug)]
pub struct RpcServer {
    socket: PathBuf,
    lock: EngineLock,
    listener: UnixListener,
    clients: HashMap<ClientId, ClientHandle>,
    inbox: mpsc::Receiver<RpcEvent>,
    inbox_tx: mpsc::Sender<RpcEvent>,
    /// TUI가 없을 때 온 허가 요청. 다시 붙으면 가장 먼저 보낸다.
    held_permissions: Vec<Notification>,
    next_client: u64,
}

impl RpcServer {
    /// 남은 소켓 파일은 잠금을 잡은 뒤에만 지운다.
    /// TODO(#89): 소켓 파일 권한, 초안 0600
    pub async fn bind(home: &Path, lock: EngineLock) -> Result<Self, RpcError> {
        todo!("#89")
    }

    /// 마지막 클라이언트가 떨어지면 `LastDetached`를 한 번 더 돌려준다.
    pub async fn next_event(&mut self) -> Option<RpcEvent> {
        todo!("#89")
    }

    /// 순서: `StartInfo` → `HistoryChunk` → 보관한 허가 요청(접수 순서).
    pub async fn greet(
        &mut self,
        client: ClientId,
        chat: ChatId,
        start: Notification,
        history: Notification,
    ) -> Result<(), RpcError> {
        todo!("#89")
    }

    /// 요청마다 한 번.
    pub async fn respond(&self, client: ClientId, response: Response) -> Result<(), RpcError> {
        todo!("#89")
    }

    pub async fn send_to(
        &self,
        client: ClientId,
        notification: Notification,
    ) -> Result<(), RpcError> {
        todo!("#89")
    }

    /// `chat`이 `None`이면 모든 클라이언트. 끊긴 클라이언트는 건너뛴다.
    pub async fn broadcast(&self, chat: Option<ChatId>, notification: Notification) {
        todo!("#89")
    }

    /// 붙은 클라이언트가 없으면 보관하고 다음 `greet`에서 가장 먼저 보낸다.
    pub async fn offer_permission(&mut self, chat: ChatId, request: Notification) {
        todo!("#89")
    }

    /// 다른 클라이언트에 `PermissionResolved`를 보내 창을 지우게 한다.
    pub async fn resolve_permission(&mut self, answered_by: ClientId, request_id: &str) {
        todo!("#89")
    }

    /// 0이면 TUI 없음(background).
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    pub async fn close(self) {
        todo!("#89")
    }
}
