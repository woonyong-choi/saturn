//! 사용자당 engine 잠금과 Unix 소켓 JSON-RPC 서버. 설계: docs/design/engine-lifecycle.md
//! 잠금 없이 소켓을 열지 않는다.

mod connection;
mod lock;

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use saturn_protocol::envelope::{CodecError, RequestId, Response, ServerMessage};
use saturn_protocol::ids::{ChatId, ConstraintAskId};
use saturn_protocol::rpc::{Notification, Request};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use crate::passes::PassGate;
use crate::processes::Supervisor;
use connection::{Connection, Outbox};
pub(crate) use lock::EngineLock;
#[cfg(test)]
pub(crate) use lock::LOCK_FILE;
use saturn_core::passes::Grant;

/// 홈 폴더 아래. TUI `EngineClient::default_socket`과 같은 이름.
pub(crate) const SOCKET_FILE: &str = saturn_protocol::home::SOCKET_FILE;

/// 초안. 소켓을 같은 사용자만 열 수 있게 한다.
const SOCKET_MODE: u32 = 0o600;

const INBOX_CAPACITY: usize = 1024;

/// 닫기 전에 쌓인 메시지를 다 쓰기를 기다리는 시간의 위쪽 한계. 초안.
const FLUSH_LIMIT: Duration = Duration::from_secs(1);

const FLUSH_POLL: Duration = Duration::from_millis(10);

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
pub(crate) enum RpcEvent {
    /// 아직 `Request::Attach` 전.
    Connected(ClientId),
    Request(ClientId, RequestId, Request),
    /// 연결 작업이 출입증과 상한을 확인해 허용한 `Request::AttachChild`.
    Child(ClientId, RequestId, Grant),
    /// `Request::Detach` 포함. `Detach`의 응답은 서버가 보낸다.
    Disconnected(ClientId),
    /// `Disconnected` 바로 뒤에 한 번 온다.
    LastDetached,
}

#[derive(Debug)]
struct ClientHandle {
    outbox: Outbox,
    /// `Attach` 전이면 `None`.
    chat: Option<ChatId>,
}

/// 답을 기다리는 허가 요청. 나중에 붙는 TUI도 받도록 답이 올 때까지 둔다.
#[derive(Debug)]
struct PendingPermission {
    chat: ChatId,
    request_id: String,
    notification: Notification,
}

/// 잠금을 소유해 서버가 사는 동안 잠금이 유지된다.
#[derive(Debug)]
pub(crate) struct RpcServer {
    socket: PathBuf,
    lock: EngineLock,
    listener: UnixListener,
    clients: HashMap<ClientId, ClientHandle>,
    inbox: mpsc::Receiver<RpcEvent>,
    inbox_tx: mpsc::Sender<RpcEvent>,
    /// 연결 작업이 출입증을 확인하는 관문. 요청 처리 루프와 공유한다.
    gate: PassGate,
    /// 접속한 프로세스가 provider 묶음의 자손인지 확인한다.
    supervisor: Supervisor,
    pending_permissions: Vec<PendingPermission>,
    next_client: u64,
    last_detached_due: bool,
}

/// 보관한 요청 목록에서 확인을 찾는 열쇠. provider 요청 ID와 겹치지 않게 접두사를 둔다.
fn constraint_ask_key(ask: ConstraintAskId) -> String {
    format!("constraint-ask-{}", ask.0)
}

impl RpcServer {
    /// 남은 소켓 파일은 잠금을 잡은 뒤에만 지운다.
    ///
    /// # Errors
    /// 소켓 파일을 지우거나 열거나 권한을 바꾸지 못하면 `Bind`.
    pub(crate) async fn bind(
        home: &Path,
        lock: EngineLock,
        gate: PassGate,
        supervisor: Supervisor,
    ) -> Result<Self, RpcError> {
        let socket = home.join(SOCKET_FILE);
        let bind_error = |source| RpcError::Bind {
            path: socket.clone(),
            source,
        };
        match std::fs::remove_file(&socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(bind_error(error)),
        }
        let listener = UnixListener::bind(&socket).map_err(bind_error)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(SOCKET_MODE))
            .map_err(bind_error)?;
        let (inbox_tx, inbox) = mpsc::channel(INBOX_CAPACITY);
        Ok(Self {
            socket,
            lock,
            listener,
            clients: HashMap::new(),
            inbox,
            inbox_tx,
            gate,
            supervisor,
            pending_permissions: Vec::new(),
            next_client: 0,
            last_detached_due: false,
        })
    }

    /// 마지막 클라이언트가 떨어지면 `LastDetached`를 한 번 더 돌려준다.
    /// cancel-safe: `select!` 안에서 불러도 일을 잃지 않는다.
    pub(crate) async fn next_event(&mut self) -> Option<RpcEvent> {
        loop {
            if self.last_detached_due {
                self.last_detached_due = false;
                return Some(RpcEvent::LastDetached);
            }
            tokio::select! {
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, _)) => return Some(self.accept(stream)),
                    Err(error) => tracing::warn!(%error, "failed to accept client"),
                },
                event = self.inbox.recv() => {
                    if let Some(event) = self.filter(event?) {
                        return Some(event);
                    }
                }
            }
        }
    }

    /// 순서: `StartInfo` → `HistoryChunk` → 답을 기다리는 허가 요청(접수 순서).
    ///
    /// # Errors
    /// 클라이언트가 이미 끊겼으면 `ClientGone`.
    pub(crate) async fn greet(
        &mut self,
        client: ClientId,
        chat: ChatId,
        start: Notification,
        history: Notification,
    ) -> Result<(), RpcError> {
        let handle = self
            .clients
            .get_mut(&client)
            .ok_or(RpcError::ClientGone(client))?;
        handle.chat = Some(chat);
        let permissions = self
            .pending_permissions
            .iter()
            .filter(|pending| pending.chat == chat)
            .map(|pending| pending.notification.clone());
        for notification in [start, history].into_iter().chain(permissions) {
            self.push(client, notification.into())?;
        }
        Ok(())
    }

    /// 요청마다 한 번.
    ///
    /// # Errors
    /// 클라이언트가 이미 끊겼으면 `ClientGone`.
    pub(crate) async fn respond(
        &self,
        client: ClientId,
        response: Response,
    ) -> Result<(), RpcError> {
        self.push(client, response.into())
    }

    /// # Errors
    /// 클라이언트가 이미 끊겼으면 `ClientGone`.
    pub(crate) async fn send_to(
        &self,
        client: ClientId,
        notification: Notification,
    ) -> Result<(), RpcError> {
        self.push(client, notification.into())
    }

    /// `chat`이 `None`이면 모든 클라이언트. 끊긴 클라이언트는 건너뛴다.
    pub(crate) async fn broadcast(&self, chat: Option<ChatId>, notification: Notification) {
        self.broadcast_except(chat, None, &notification);
    }

    /// 채팅에 붙은 모든 클라이언트. 붙지 않은 접속(`saturn prune` 같은 명령)은 건너뛴다.
    pub(crate) async fn broadcast_attached(&self, notification: Notification) {
        let targets = self
            .clients
            .iter()
            .filter(|(_, handle)| handle.chat.is_some());
        for (id, _) in targets {
            let _ = self.push(*id, notification.clone().into()); // 끊긴 클라이언트는 건너뛴다
        }
    }

    /// 붙은 클라이언트가 없어도 답이 올 때까지 두고 다음 `greet`에서 보낸다.
    pub(crate) async fn offer_permission(&mut self, chat: ChatId, request: Notification) {
        let Notification::PermissionRequested { request_id, .. } = &request else {
            tracing::error!("offer_permission called with a non-permission notification");
            return;
        };
        let request_id = request_id.clone();
        self.offer(chat, request_id, request);
    }

    /// 입력 요청도 허가 요청처럼 답이 올 때까지 두고 다음 `greet`에서 보낸다.
    pub(crate) async fn offer_input(&mut self, chat: ChatId, request: Notification) {
        let Notification::InputRequested { request_id, .. } = &request else {
            tracing::error!("offer_input called with a non-input notification");
            return;
        };
        let request_id = request_id.clone();
        self.offer(chat, request_id, request);
    }

    fn offer(&mut self, chat: ChatId, request_id: String, request: Notification) {
        self.broadcast_except(Some(chat), None, &request);
        self.pending_permissions.push(PendingPermission {
            chat,
            request_id,
            notification: request,
        });
    }

    /// 등록 확인도 허가 요청처럼 답이 올 때까지 두고 다음 `greet`에서 보낸다.
    pub(crate) async fn offer_constraint_ask(
        &mut self,
        chat: ChatId,
        ask: ConstraintAskId,
        request: Notification,
    ) {
        self.offer(chat, constraint_ask_key(ask), request);
    }

    /// 다른 클라이언트에 `ConstraintAskResolved`를 보내 창을 지우게 한다.
    pub(crate) async fn resolve_constraint_ask(
        &mut self,
        answered_by: ClientId,
        ask: ConstraintAskId,
    ) {
        let resolved = Notification::ConstraintAskResolved { ask };
        self.resolve(answered_by, &constraint_ask_key(ask), &resolved);
    }

    /// 답 없이 닫힌 확인(대상 제약이 바뀜)의 창을 모든 클라이언트에서 지운다. 모르는 확인이면 아무것도 하지 않는다.
    pub(crate) async fn withdraw_constraint_ask(&mut self, ask: ConstraintAskId) {
        let resolved = Notification::ConstraintAskResolved { ask };
        self.withdraw(&constraint_ask_key(ask), &resolved);
    }

    /// 다른 클라이언트에 `PermissionResolved`를 보내 창을 지우게 한다.
    pub(crate) async fn resolve_permission(&mut self, answered_by: ClientId, request_id: &str) {
        let resolved = Notification::PermissionResolved {
            request_id: request_id.to_owned(),
        };
        self.resolve(answered_by, request_id, &resolved);
    }

    /// 다른 클라이언트에 `InputResolved`를 보내 창을 지우게 한다.
    pub(crate) async fn resolve_input(&mut self, answered_by: ClientId, request_id: &str) {
        let resolved = Notification::InputResolved {
            request_id: request_id.to_owned(),
        };
        self.resolve(answered_by, request_id, &resolved);
    }

    /// 답 없이 끝난 요청(턴이 끝났거나 흐름이 끊김)의 창을 모든 클라이언트에서 지운다. 모르는 요청이면 아무것도 하지 않는다.
    pub(crate) async fn withdraw_permission(&mut self, request_id: &str) {
        let resolved = Notification::PermissionResolved {
            request_id: request_id.to_owned(),
        };
        self.withdraw(request_id, &resolved);
    }

    /// 답 없이 끝난 입력 요청의 창을 모든 클라이언트에서 지운다. 모르는 요청이면 아무것도 하지 않는다.
    pub(crate) async fn withdraw_input(&mut self, request_id: &str) {
        let resolved = Notification::InputResolved {
            request_id: request_id.to_owned(),
        };
        self.withdraw(request_id, &resolved);
    }

    fn resolve(&mut self, answered_by: ClientId, request_id: &str, resolved: &Notification) {
        let chat = self.take_pending(request_id);
        self.broadcast_except(chat, Some(answered_by), resolved);
    }

    fn withdraw(&mut self, request_id: &str, resolved: &Notification) {
        if let Some(chat) = self.take_pending(request_id) {
            self.broadcast_except(Some(chat), None, resolved);
        }
    }

    // cost: time O(q), heap O(1), stack O(1)
    // vars: q = 보관한 요청 수
    // basis: estimate
    fn take_pending(&mut self, request_id: &str) -> Option<ChatId> {
        let index = self
            .pending_permissions
            .iter()
            .position(|pending| pending.request_id == request_id)?;
        Some(self.pending_permissions.remove(index).chat)
    }

    /// 0이면 TUI 없음(background).
    pub(crate) fn client_count(&self) -> usize {
        self.clients.len()
    }

    /// 쌓인 메시지를 쓸 시간을 준 뒤 모든 접속을 끊고 소켓 파일을 지운다. 잠금은 그대로 쥔다.
    /// 소켓 파일이 없어지므로 새 접속은 `NotRunning`을 받는다.
    pub(crate) async fn close_connections(&mut self) {
        let deadline = tokio::time::Instant::now() + FLUSH_LIMIT;
        while tokio::time::Instant::now() < deadline
            && self
                .clients
                .values()
                .any(|handle| handle.outbox.sender.capacity() < handle.outbox.sender.max_capacity())
        {
            tokio::time::sleep(FLUSH_POLL).await;
        }
        for handle in self.clients.values() {
            handle.outbox.kill.notify_one();
        }
        self.clients.clear();
        match std::fs::remove_file(&self.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%error, path = %self.socket.display(), "failed to remove engine socket");
            }
        }
    }

    pub(crate) async fn close(mut self) {
        self.close_connections().await;
        drop(self.listener);
        drop(self.lock);
    }

    fn accept(&mut self, stream: tokio::net::UnixStream) -> RpcEvent {
        let id = ClientId(self.next_client);
        self.next_client += 1;
        let outbox = Connection::new(id, stream).spawn(
            self.inbox_tx.clone(),
            self.gate.clone(),
            self.supervisor.clone(),
        );
        self.clients.insert(id, ClientHandle { outbox, chat: None });
        RpcEvent::Connected(id)
    }

    /// 끊긴 뒤 늦게 온 일은 버리고, `Detach`는 응답한 뒤 끊김으로 바꾼다.
    fn filter(&mut self, event: RpcEvent) -> Option<RpcEvent> {
        match event {
            RpcEvent::Request(client, id, Request::Detach) => {
                let _ = self.push(client, Response::ok(id).into()); // 이미 끊겼으면 응답할 곳이 없다
                self.disconnect(client)
            }
            RpcEvent::Request(client, ..) if !self.clients.contains_key(&client) => None,
            RpcEvent::Child(client, _, grant) if !self.clients.contains_key(&client) => {
                self.gate.abandon(&grant); // 허용받고 끊긴 접속의 자리는 다음 요청에 준다
                None
            }
            RpcEvent::Disconnected(client) => self.disconnect(client),
            other => Some(other),
        }
    }

    /// 강제 종료하지 않는다. 쌓인 응답(`Detach`의 응답 포함)을 다 쓴 뒤 쓰기 작업이 끝난다.
    fn disconnect(&mut self, client: ClientId) -> Option<RpcEvent> {
        self.clients.remove(&client)?;
        self.last_detached_due = self.clients.is_empty();
        Some(RpcEvent::Disconnected(client))
    }

    /// 느린 클라이언트의 outbox가 넘치면 기다리지 않고 그 연결을 끊는다.
    fn push(&self, client: ClientId, message: ServerMessage) -> Result<(), RpcError> {
        let handle = self
            .clients
            .get(&client)
            .ok_or(RpcError::ClientGone(client))?;
        match handle.outbox.sender.try_send(message) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                tracing::warn!(client = client.0, "client outbox full, disconnecting");
                handle.outbox.kill.notify_one();
                Err(RpcError::ClientGone(client))
            }
            Err(TrySendError::Closed(_)) => Err(RpcError::ClientGone(client)),
        }
    }

    fn broadcast_except(
        &self,
        chat: Option<ChatId>,
        skip: Option<ClientId>,
        notification: &Notification,
    ) {
        let targets = self
            .clients
            .iter()
            .filter(|(id, handle)| Some(**id) != skip && (chat.is_none() || handle.chat == chat));
        for (id, _) in targets {
            let _ = self.push(*id, notification.clone().into()); // 끊긴 클라이언트는 건너뛴다
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use saturn_protocol::envelope::{ClientMessage, Outcome, ServerMessage, encode_line};
    use saturn_protocol::ids::{TaskId, TaskLabel};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
    use tokio::net::UnixStream;
    use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
    use tokio::time::timeout;

    use super::*;

    struct TestClient {
        lines: Lines<BufReader<OwnedReadHalf>>,
        writer: OwnedWriteHalf,
    }

    impl TestClient {
        async fn connect(socket: &Path) -> Self {
            let (read_half, writer) = UnixStream::connect(socket).await.unwrap().into_split();
            Self {
                lines: BufReader::new(read_half).lines(),
                writer,
            }
        }

        async fn send(&mut self, id: u64, request: Request) {
            let line = encode_line(&ClientMessage::new(RequestId(id), request)).unwrap();
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        async fn send_raw(&mut self, line: &str) {
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        async fn recv(&mut self) -> ServerMessage {
            let line = timeout(Duration::from_secs(5), self.lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            saturn_protocol::envelope::decode_server_line(&line).unwrap()
        }
    }

    async fn server() -> (tempfile::TempDir, RpcServer) {
        let home = tempfile::tempdir().unwrap();
        let lock = EngineLock::acquire(home.path()).unwrap();
        let gate = PassGate::new(saturn_core::passes::PassLimits::DEFAULT);
        let server = RpcServer::bind(home.path(), lock, gate, Supervisor::new())
            .await
            .unwrap();
        (home, server)
    }

    impl RpcEvent {
        fn disconnected(&self) -> Option<ClientId> {
            match self {
                RpcEvent::Disconnected(id) => Some(*id),
                _ => None,
            }
        }
    }

    async fn event(server: &mut RpcServer) -> RpcEvent {
        timeout(Duration::from_secs(5), server.next_event())
            .await
            .unwrap()
            .unwrap()
    }

    async fn connected(server: &mut RpcServer, home: &Path) -> (ClientId, TestClient) {
        let client = TestClient::connect(&home.join(SOCKET_FILE)).await;
        let RpcEvent::Connected(id) = event(server).await else {
            panic!("expected Connected");
        };
        (id, client)
    }

    fn notice(chat: u64) -> Notification {
        Notification::ContextSize {
            chat: ChatId(chat),
            tokens: None,
            threshold: 1,
        }
    }

    fn permission(request_id: &str) -> Notification {
        Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: crate::providers::test_support::CODEX,
            request_id: request_id.into(),
            summary: "rm".into(),
            reason: "cleanup".into(),
            waiting: 0,
        }
    }

    fn method(message: ServerMessage) -> Notification {
        match message {
            ServerMessage::Notification(message) => message.notification,
            ServerMessage::Response(response) => panic!("expected notification, got {response:?}"),
        }
    }

    // #460
    #[tokio::test]
    async fn a_client_that_never_reads_is_dropped_once_its_outbox_overflows() {
        let (home, mut server) = server().await;
        let (id, _silent) = connected(&mut server, home.path()).await;
        let big = Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: crate::providers::test_support::CODEX,
            request_id: "r".into(),
            summary: "x".repeat(64 * 1024),
            reason: String::new(),
            waiting: 0,
        };

        for _ in 0..(super::connection::OUTBOX_CAPACITY * 4) {
            server.broadcast(None, big.clone()).await;
            tokio::task::yield_now().await;
        }

        assert_eq!(event(&mut server).await.disconnected(), Some(id));
        assert_eq!(server.client_count(), 0);
        assert!(matches!(event(&mut server).await, RpcEvent::LastDetached));
    }

    #[tokio::test]
    async fn bind_sets_owner_only_socket_mode() {
        let (home, _server) = server().await;

        let mode = std::fs::metadata(home.path().join(SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();

        assert_eq!(mode & 0o777, SOCKET_MODE);
    }

    #[tokio::test]
    async fn bind_replaces_stale_socket_file() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(SOCKET_FILE), b"stale").unwrap();
        let lock = EngineLock::acquire(home.path()).unwrap();

        let gate = PassGate::new(saturn_core::passes::PassLimits::DEFAULT);
        let server = RpcServer::bind(home.path(), lock, gate, Supervisor::new()).await;

        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn second_engine_is_refused_while_server_holds_lock() {
        let (home, _server) = server().await;

        let second = EngineLock::acquire(home.path());

        assert!(matches!(second, Err(RpcError::AlreadyRunning { .. })));
    }

    #[tokio::test]
    async fn requests_arrive_with_client_and_request_id() {
        let (home, mut server) = server().await;
        let (id, mut client) = connected(&mut server, home.path()).await;

        client.send(9, Request::ListTasks).await;

        let RpcEvent::Request(from, request_id, Request::ListTasks) = event(&mut server).await
        else {
            panic!("expected ListTasks");
        };
        assert_eq!((from, request_id), (id, RequestId(9)));
    }

    // #572: 표지를 지우고 붙은 provider 자손은 출입증 없는 요청을 거절당하고, 바깥 접속은 그대로 받는다
    #[tokio::test]
    async fn provider_descendant_without_a_pass_is_rejected() {
        use crate::processes::ProcessSpec;
        use tokio::io::AsyncBufReadExt;

        let (home, mut server) = server().await;
        let line = encode_line(&ClientMessage::new(RequestId(1), Request::ListTasks)).unwrap();
        let script = "import os, socket, sys\n\
            s = socket.socket(socket.AF_UNIX)\n\
            s.connect(sys.argv[1])\n\
            s.sendall(os.environ['LINE'].encode())\n\
            print(s.makefile().readline().strip())";
        let spec = ProcessSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "python3 -c \"$SCRIPT\" \"$SOCKET\"; true".into(),
            ],
            workdir: home.path().to_owned(),
            env: [
                ("PATH", std::env::var("PATH").unwrap_or_default()),
                ("LINE", line),
                ("SCRIPT", script.to_owned()),
                (
                    "SOCKET",
                    home.path().join(SOCKET_FILE).display().to_string(),
                ),
            ]
            .map(|(key, value)| (key.into(), value.into()))
            .into(),
        };
        let spawned = server.supervisor.spawn(spec).unwrap();
        let mut reply = BufReader::new(spawned.io.stdout).lines();

        let mut requests = 0;
        let answer = timeout(Duration::from_secs(10), async {
            loop {
                tokio::select! {
                    line = reply.next_line() => break line.unwrap().unwrap(),
                    event = server.next_event() => {
                        requests += usize::from(matches!(event, Some(RpcEvent::Request(..))));
                    }
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(requests, 0, "the request should not reach the engine");
        assert!(answer.contains("-32002"), "unexpected reply: {answer}");
        assert!(answer.contains("need a pass"), "unexpected reply: {answer}");
    }

    #[tokio::test]
    async fn respond_reaches_only_that_client() {
        let (home, mut server) = server().await;
        let (first, mut first_client) = connected(&mut server, home.path()).await;
        let (_, mut second_client) = connected(&mut server, home.path()).await;

        server
            .respond(first, Response::ok(RequestId(4)))
            .await
            .unwrap();
        server.broadcast(None, notice(1)).await;

        assert_eq!(
            first_client.recv().await,
            ServerMessage::Response(Response::ok(RequestId(4)))
        );
        assert_eq!(method(first_client.recv().await), notice(1));
        assert_eq!(method(second_client.recv().await), notice(1));
    }

    #[tokio::test]
    async fn broadcast_to_chat_skips_other_chats() {
        let (home, mut server) = server().await;
        let (first, mut first_client) = connected(&mut server, home.path()).await;
        let (second, mut second_client) = connected(&mut server, home.path()).await;
        server
            .greet(first, ChatId(1), notice(0), notice(0))
            .await
            .unwrap();
        server
            .greet(second, ChatId(2), notice(0), notice(0))
            .await
            .unwrap();
        for client in [&mut first_client, &mut second_client] {
            client.recv().await;
            client.recv().await;
        }

        server.broadcast(Some(ChatId(2)), notice(2)).await;
        server.broadcast(None, notice(3)).await;

        assert_eq!(method(first_client.recv().await), notice(3));
        assert_eq!(method(second_client.recv().await), notice(2));
    }

    #[tokio::test]
    async fn greet_sends_start_history_then_pending_permissions() {
        let (home, mut server) = server().await;
        server.offer_permission(ChatId(1), permission("p1")).await;
        server
            .offer_permission(ChatId(2), permission("other"))
            .await;
        let (id, mut client) = connected(&mut server, home.path()).await;

        server
            .greet(id, ChatId(1), notice(10), notice(11))
            .await
            .unwrap();

        assert_eq!(method(client.recv().await), notice(10));
        assert_eq!(method(client.recv().await), notice(11));
        assert_eq!(method(client.recv().await), permission("p1"));
        server.broadcast(None, notice(12)).await;
        assert_eq!(method(client.recv().await), notice(12));
    }

    #[tokio::test]
    async fn resolve_permission_clears_other_clients_and_pending() {
        let (home, mut server) = server().await;
        let (first, mut first_client) = connected(&mut server, home.path()).await;
        let (second, mut second_client) = connected(&mut server, home.path()).await;
        server
            .greet(first, ChatId(1), notice(0), notice(0))
            .await
            .unwrap();
        server
            .greet(second, ChatId(1), notice(0), notice(0))
            .await
            .unwrap();
        for client in [&mut first_client, &mut second_client] {
            client.recv().await;
            client.recv().await;
        }
        server.offer_permission(ChatId(1), permission("p1")).await;
        assert_eq!(method(first_client.recv().await), permission("p1"));
        assert_eq!(method(second_client.recv().await), permission("p1"));

        server.resolve_permission(first, "p1").await;
        server.broadcast(None, notice(5)).await;

        assert_eq!(method(first_client.recv().await), notice(5));
        assert_eq!(
            method(second_client.recv().await),
            Notification::PermissionResolved {
                request_id: "p1".into()
            }
        );
        let (late, mut late_client) = connected(&mut server, home.path()).await;
        server
            .greet(late, ChatId(1), notice(0), notice(1))
            .await
            .unwrap();
        late_client.recv().await;
        late_client.recv().await;
        server.broadcast(None, notice(6)).await;
        assert_eq!(method(late_client.recv().await), notice(6));
    }

    #[tokio::test]
    async fn broken_line_gets_error_response_and_keeps_connection() {
        let (home, mut server) = server().await;
        let (_, mut client) = connected(&mut server, home.path()).await;

        client
            .send_raw("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"Nope\"}\n")
            .await;
        client.send(4, Request::ListTasks).await;

        let ServerMessage::Response(response) = client.recv().await else {
            panic!("expected error response");
        };
        assert_eq!(response.id, Some(RequestId(3)));
        assert!(matches!(response.outcome, Outcome::Err(_)));
        assert!(matches!(
            event(&mut server).await,
            RpcEvent::Request(_, RequestId(4), Request::ListTasks)
        ));
    }

    #[tokio::test]
    async fn detach_responds_and_reports_last_detached() {
        let (home, mut server) = server().await;
        let (id, mut client) = connected(&mut server, home.path()).await;

        client.send(1, Request::Detach).await;

        assert!(matches!(event(&mut server).await, RpcEvent::Disconnected(gone) if gone == id));
        assert!(matches!(event(&mut server).await, RpcEvent::LastDetached));
        assert_eq!(
            client.recv().await,
            ServerMessage::Response(Response::ok(RequestId(1)))
        );
        assert_eq!(server.client_count(), 0);
    }

    #[tokio::test]
    async fn last_detached_waits_for_every_client() {
        let (home, mut server) = server().await;
        let (first, first_client) = connected(&mut server, home.path()).await;
        let (second, second_client) = connected(&mut server, home.path()).await;

        drop(first_client);
        assert!(matches!(event(&mut server).await, RpcEvent::Disconnected(gone) if gone == first));
        drop(second_client);

        assert!(matches!(event(&mut server).await, RpcEvent::Disconnected(gone) if gone == second));
        assert!(matches!(event(&mut server).await, RpcEvent::LastDetached));
    }

    #[tokio::test]
    async fn send_to_gone_client_returns_client_gone() {
        let (_home, server) = server().await;

        let result = server.send_to(ClientId(42), notice(1)).await;

        assert!(matches!(result, Err(RpcError::ClientGone(ClientId(42)))));
    }

    #[tokio::test]
    async fn close_removes_socket_and_releases_lock() {
        let (home, server) = server().await;

        server.close().await;

        assert!(!home.path().join(SOCKET_FILE).exists());
        assert!(EngineLock::acquire(home.path()).is_ok());
    }
}
