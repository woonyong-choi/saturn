//! TUI·CLI 접속: 사용자당 engine 잠금, Unix 소켓 JSON-RPC 수락, 여러 TUI 동시 접속, 요청 전달, 알림 방송, 허가 요청 보관.
//!
//! 설계: docs/design/engine-lifecycle.md(사용자당 engine 하나, engine 시작 순서, 여러 TUI 동시 접속, TUI 종료 뒤 동작).
//! 메시지 정본은 `saturn_protocol::rpc`(`Request`, `Notification`)다. 한 줄에 JSON 메시지 하나.
//! TODO(#46): 메서드 이름과 목록 확정. 지금은 `Request` variant 이름을 그대로 쓴다
//! TODO(#75): JSON-RPC 2.0 봉투(`id`, 응답 짝짓기)는 protocol에 두고 여기서 그것을 쓴다
//!
//! 흐름:
//! 1. `EngineLock::acquire`로 사용자당 잠금을 잡는다(engine 시작 1단계). 이미 잡혀 있으면 두 번째 engine은 끝난다.
//! 2. judge 확인까지 끝나면 `RpcServer::bind`로 소켓을 연다(5단계). 잠금 없이 소켓을 열지 않는다.
//! 3. `RpcServer::next_event`가 접속, 요청, 끊김을 하나씩 돌려준다. 요청 처리는 engine(`Engine::handle_request`)이 한다.
//! 4. 붙을 때 `greet`가 `StartInfo` → 최근 기록 → 보관한 허가 요청 순서로 보낸다.
//! 5. 마지막 TUI가 떨어지면 `RpcEvent::LastDetached`를 한 번 내고, engine이 `on_exit`를 적용한다.

mod connection;
mod lock;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{Notification, Request};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

pub use lock::EngineLock;

/// 소켓 파일 이름. `~/.saturn/` 아래에 둔다. TUI `EngineClient::default_socket`과 같은 경로. 이름은 초안이다(설계에 없음).
/// TODO(#89): 값 미정, 초안 `engine.sock`
/// TODO(#49): 경로 설정 키
pub const SOCKET_FILE: &str = "engine.sock";

/// 접속, 잠금, 메시지 오류.
#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    /// 다른 engine이 이미 잠금을 잡고 있다. 이 engine은 끝나고 cli는 기존 engine 소켓에 붙는다.
    #[error("engine already running: {lock}")]
    AlreadyRunning {
        /// 잠금 파일 경로.
        lock: PathBuf,
    },
    /// 잠금 파일을 만들거나 잠그지 못했다.
    #[error("failed to lock engine: {path}")]
    Lock {
        /// 잠금 파일 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// 소켓을 열지 못했다.
    #[error("failed to bind engine socket: {path}")]
    Bind {
        /// 소켓 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// 접속 읽기·쓰기 실패.
    #[error("client connection failed")]
    Io(#[from] std::io::Error),
    /// 요청을 해석하지 못했다. 그 줄만 버리고 연결은 유지한다. 원문은 로그에 남기지 않는다(키가 들어 있을 수 있다).
    #[error("failed to decode client message")]
    Decode(#[from] serde_json::Error),
    /// 이미 끊긴 클라이언트.
    #[error("client not connected: {0:?}")]
    ClientGone(ClientId),
}

/// 접속 하나의 id. engine 실행 안에서만 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

/// `next_event`가 돌려주는 일 하나.
#[derive(Debug)]
pub enum RpcEvent {
    /// 새 접속. 아직 채팅에 붙지 않았다(`Request::Attach`를 기다린다).
    Connected(ClientId),
    /// 요청 하나. 받은 순서대로 돌려준다.
    Request(ClientId, Request),
    /// 접속이 끊겼다(`Request::Detach` 포함).
    Disconnected(ClientId),
    /// 마지막 클라이언트가 떨어졌다. `Disconnected` 바로 뒤에 한 번 온다. engine이 `on_exit`를 적용한다.
    LastDetached,
}

/// 접속한 클라이언트 하나.
#[derive(Debug)]
struct ClientHandle {
    /// 알림을 쓰는 쪽. 쓰기 작업이 한 줄씩 소켓에 쓴다.
    outbox: mpsc::Sender<Notification>,
    /// 붙은 채팅. `Attach` 전이면 `None`.
    chat: Option<ChatId>,
}

/// 소켓 서버. engine에 하나. 잠금을 소유해 서버가 사는 동안 잠금이 유지된다.
#[derive(Debug)]
pub struct RpcServer {
    /// 소켓 경로.
    socket: PathBuf,
    /// 사용자당 engine 잠금.
    lock: EngineLock,
    /// 접속 수락.
    listener: UnixListener,
    /// 접속 중인 클라이언트.
    clients: HashMap<ClientId, ClientHandle>,
    /// 연결별 읽기 작업이 보내는 요청과 끊김.
    inbox: mpsc::Receiver<RpcEvent>,
    /// 새 연결의 읽기 작업에 나눠 줄 보내는 쪽.
    inbox_tx: mpsc::Sender<RpcEvent>,
    /// TUI가 없을 때 온 허가 요청. 다시 붙으면 가장 먼저 보낸다. 답을 받으면 뺀다.
    held_permissions: Vec<Notification>,
    /// 다음 `ClientId`.
    next_client: u64,
}

impl RpcServer {
    /// 잠금을 넘겨받아 `home/engine.sock`을 연다. 남은 소켓 파일은 잠금을 잡은 뒤에만 지운다(다른 engine의 소켓이 아니다).
    /// 소켓 파일 권한은 초안이다(설계에 없음). TODO(#89): 값 미정, 초안 0600
    ///
    /// # Errors
    /// 소켓을 열지 못하면 `Bind`.
    pub async fn bind(home: &Path, lock: EngineLock) -> Result<Self, RpcError> {
        todo!("#89")
    }

    /// 다음 일을 기다린다. 새 접속 수락과 연결별 요청·끊김을 한곳에서 돌려준다.
    /// 접속을 받으면 `Connection`으로 읽기·쓰기 작업을 띄운다. 마지막 클라이언트가 떨어지면 `LastDetached`를 한 번 더 돌려준다.
    pub async fn next_event(&mut self) -> Option<RpcEvent> {
        todo!("#89")
    }

    /// 접속 직후 순서대로 보낸다: `StartInfo` → 최근 기록(`HistoryChunk`) → 보관한 허가 요청(`PermissionRequested`, 접수 순서).
    /// 채팅 붙기도 여기서 기록한다.
    ///
    /// # Errors
    /// 클라이언트가 이미 끊겼으면 `ClientGone`.
    pub async fn greet(
        &mut self,
        client: ClientId,
        chat: ChatId,
        start: Notification,
        history: Notification,
    ) -> Result<(), RpcError> {
        todo!("#89")
    }

    /// 클라이언트 하나에 알림을 보낸다(요청의 답).
    ///
    /// # Errors
    /// 클라이언트가 이미 끊겼으면 `ClientGone`.
    pub async fn send_to(
        &self,
        client: ClientId,
        notification: Notification,
    ) -> Result<(), RpcError> {
        todo!("#89")
    }

    /// 채팅에 붙은 모든 클라이언트에 보낸다. `chat`이 `None`이면 모든 클라이언트. 끊긴 클라이언트는 건너뛴다.
    pub async fn broadcast(&self, chat: Option<ChatId>, notification: Notification) {
        todo!("#89")
    }

    /// 허가 요청을 방송한다. 붙은 클라이언트가 없으면 보관하고 다음 `greet`에서 가장 먼저 보낸다.
    pub async fn offer_permission(&mut self, chat: ChatId, request: Notification) {
        todo!("#89")
    }

    /// 허가 요청에 답이 왔다. 보관에서 빼고 다른 클라이언트에 `PermissionResolved`를 보내 창을 지우게 한다.
    pub async fn resolve_permission(&mut self, answered_by: ClientId, request_id: &str) {
        todo!("#89")
    }

    /// 접속 중인 클라이언트 수. 0이면 TUI 없음(background)이다.
    pub fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// 소켓 파일을 지우고 잠금을 푼다. engine 종료 마지막 단계.
    pub async fn close(self) {
        todo!("#89")
    }
}
