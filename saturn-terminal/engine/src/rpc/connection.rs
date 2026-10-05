//! 접속 하나의 읽기·쓰기 작업. 해석하지 못한 줄은 inbox 대신 오류 응답을 outbox에 넣는다.
//! `SubmitRouterKey` 줄은 해석 실패여도 원문을 로그에 남기지 않는다.

use std::sync::Arc;

use saturn_core::passes::{Reject, Waiter};
use saturn_core::permission::Mode;
use saturn_protocol::envelope::{
    self, CodecError, ErrorKind, INVALID_PARAMS, RequestId, Response, ServerMessage,
};
use saturn_protocol::rpc::{CHILD_REJECTED, Notification, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc};

use super::{ClientId, RpcEvent};
use crate::passes::{Entry, PassGate, Waited};
use crate::processes::Supervisor;

/// 초안. 넘치면 그 클라이언트를 끊는다. 다시 붙으면 기록으로 화면을 되살린다.
pub(crate) const OUTBOX_CAPACITY: usize = 1024;

#[derive(Debug)]
pub(crate) struct Connection {
    id: ClientId,
    stream: UnixStream,
}

/// 쓰는 쪽과 강제 종료 신호.
#[derive(Debug, Clone)]
pub(crate) struct Outbox {
    pub(crate) sender: mpsc::Sender<ServerMessage>,
    pub(crate) kill: Arc<Notify>,
}

impl Connection {
    pub(crate) fn new(id: ClientId, stream: UnixStream) -> Self {
        Self { id, stream }
    }

    /// 읽기가 끝나면 `RpcEvent::Disconnected`를 inbox로 보낸다.
    pub(crate) fn spawn(
        self,
        inbox: mpsc::Sender<RpcEvent>,
        gate: PassGate,
        supervisor: Supervisor,
    ) -> Outbox {
        let peer = self.stream.peer_cred().ok().and_then(|cred| cred.pid());
        let (sender, receiver) = mpsc::channel(OUTBOX_CAPACITY);
        let kill = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (read_half, write_half) = self.stream.into_split();
        tokio::spawn(write_loop(
            write_half,
            receiver,
            Arc::clone(&kill),
            Arc::clone(&closed),
        ));
        tokio::spawn(read_loop(
            self.id,
            read_half,
            inbox,
            sender.clone(),
            (gate, closed),
            Peer {
                pid: peer,
                supervisor,
            },
        ));
        Outbox { sender, kill }
    }
}

/// 소켓 상대 프로세스. 번호를 모르면(`None`) provider 쪽으로 본다.
struct Peer {
    pid: Option<i32>,
    supervisor: Supervisor,
}

impl Peer {
    /// Saturn이 띄운 provider 묶음의 자손이면 참. 환경 변수 표지는 자손이 지울 수 있어 보지 않는다.
    async fn is_provider_side(&self, client: ClientId) -> bool {
        let provider_side = match self.pid.and_then(|pid| u32::try_from(pid).ok()) {
            Some(pid) => self.supervisor.is_provider_side(pid).await,
            None => true,
        };
        tracing::debug!(client = client.0, provider_side, "client peer checked");
        provider_side
    }
}

async fn read_loop(
    id: ClientId,
    read_half: tokio::net::unix::OwnedReadHalf,
    inbox: mpsc::Sender<RpcEvent>,
    replies: mpsc::Sender<ServerMessage>,
    (gate, closed): (PassGate, Arc<Notify>),
    peer: Peer,
) {
    let restricted = peer.is_provider_side(id).await;
    let mut lines = BufReader::new(read_half).lines();
    let mut line_in = Line {
        id,
        inbox: &inbox,
        replies: &replies,
        gate: &gate,
        queued: Vec::new(),
        restricted,
    };
    // 쓰기 쪽이 끝나면(종료 신호, 쓰기 실패) 읽기도 멈춰 이 연결을 정리한다
    while let Some(line) = next_line(&mut lines, id, &closed).await {
        if line.trim().is_empty() {
            continue;
        }
        if !line_in.forward(&line).await {
            return; // 서버가 이미 닫혔다
        }
    }
    for waiter in line_in.queued {
        gate.cancel(waiter); // 끊긴 연결의 대기 요청은 자리를 받지 않는다
    }
    let _ = inbox.send(RpcEvent::Disconnected(id)).await; // 서버가 이미 닫혔다
}

async fn next_line(
    lines: &mut Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    id: ClientId,
    closed: &Notify,
) -> Option<String> {
    let read = tokio::select! {
        read = lines.next_line() => Some(read),
        () = closed.notified() => None,
    };
    match read? {
        Ok(line) => line,
        Err(error) => {
            tracing::debug!(client = id.0, %error, "client read failed");
            None
        }
    }
}

/// 연결 하나가 읽은 줄을 처리하는 데 쓰는 값.
struct Line<'a> {
    id: ClientId,
    inbox: &'a mpsc::Sender<RpcEvent>,
    replies: &'a mpsc::Sender<ServerMessage>,
    gate: &'a PassGate,
    /// 이 연결이 대기열에 세워 둔 하위 접속 요청.
    queued: Vec<Waiter>,
    /// provider 자손의 접속. 출입증이 있는 요청만 받고, `AttachChild`가 통과한 뒤에는 하위 접속으로 다룬다.
    restricted: bool,
}

/// 출입증을 담은 요청. 출입증의 유효 여부는 각 요청의 처리에서 확인한다.
fn carries_pass(request: &Request) -> bool {
    matches!(
        request,
        Request::AttachChild { .. } | Request::EvidenceSearch { .. } | Request::EvidenceRead { .. }
    )
}

impl Line<'_> {
    /// 서버가 이미 닫혀 요청을 넘기지 못하면 거짓.
    async fn forward(&mut self, line: &str) -> bool {
        match envelope::decode_client_line(line) {
            Ok(message) if self.restricted && !carries_pass(&message.request) => {
                let reason = "connections from provider processes need a pass";
                let _ = self.replies.try_send(
                    Response::error_of_kind(
                        Some(message.id),
                        CHILD_REJECTED,
                        Some(ErrorKind::Failed),
                        reason,
                    )
                    .into(),
                ); // 넘치면 쓰기 쪽이 곧 끊긴다
                true
            }
            Ok(message) => match message.request {
                Request::AttachChild { pass, mode } => {
                    self.admit_child(message.id, &pass, mode.as_deref()).await
                }
                request => {
                    let event = RpcEvent::Request(self.id, message.id, request);
                    self.inbox.send(event).await.is_ok()
                }
            },
            Err(error) => {
                if let CodecError::Decode { kind, column, .. } = &error {
                    tracing::warn!(client = self.id.0, %kind, column, "dropped client line");
                }
                let _ = self.replies.try_send(error.to_response().into()); // 넘치면 쓰기 쪽이 곧 끊긴다
                true
            }
        }
    }

    /// 출입증과 상한은 요청 처리 루프를 거치지 않고 여기서 확인한다. 거절은 바로 답하고, 상한이 차면 기다리는 동안에도
    /// 이 연결의 다음 줄을 계속 읽는다.
    async fn admit_child(&mut self, request: RequestId, pass: &str, mode: Option<&str>) -> bool {
        let wanted = match mode.map(Mode::parse) {
            None => None,
            Some(Some(mode)) => Some(mode),
            Some(None) => {
                let message = "unknown permission mode";
                let _ = self
                    .replies
                    .try_send(Response::error(Some(request), INVALID_PARAMS, message).into()); // 넘치면 쓰기 쪽이 곧 끊긴다
                return true;
            }
        };
        match self.gate.request(pass, wanted) {
            Entry::Rejected(reject) => {
                let message = reject_message(reject);
                let _ = self.replies.try_send(
                    Response::error_of_kind(
                        Some(request),
                        CHILD_REJECTED,
                        Some(ErrorKind::Failed),
                        message,
                    )
                    .into(),
                ); // 넘치면 쓰기 쪽이 곧 끊긴다
                true
            }
            Entry::Admitted(grant) => {
                self.restricted = false;
                let event = RpcEvent::Child(self.id, request, grant);
                self.inbox.send(event).await.is_ok()
            }
            Entry::Queued {
                position,
                waiter,
                wait,
            } => {
                self.restricted = false;
                let queued = Notification::ChildQueued { position };
                let _ = self.replies.try_send(queued.into()); // 넘치면 쓰기 쪽이 곧 끊긴다
                self.queued.push(waiter);
                tokio::spawn(await_place(
                    self.id,
                    request,
                    wait,
                    self.inbox.clone(),
                    self.replies.clone(),
                    self.gate.clone(),
                ));
                true
            }
        }
    }
}

/// 자리가 날 때까지 기다렸다가 요청 처리 루프에 넘긴다. 이 연결의 읽기는 막지 않는다.
async fn await_place(
    id: ClientId,
    request: RequestId,
    wait: tokio::sync::oneshot::Receiver<Waited>,
    inbox: mpsc::Sender<RpcEvent>,
    replies: mpsc::Sender<ServerMessage>,
    gate: PassGate,
) {
    match wait.await {
        Ok(Waited::Granted(grant)) => {
            if let Err(error) = inbox.send(RpcEvent::Child(id, request, grant)).await
                && let RpcEvent::Child(_, _, grant) = error.0
            {
                gate.abandon(&grant); // 서버가 이미 닫혔다
            }
        }
        Ok(Waited::Cancelled) | Err(_) => {
            let message = "parent was stopped before a place opened";
            let _ = replies.try_send(
                Response::error_of_kind(
                    Some(request),
                    CHILD_REJECTED,
                    Some(ErrorKind::Failed),
                    message,
                )
                .into(),
            ); // 연결이 이미 끊겼을 수 있다
        }
    }
}

fn reject_message(reject: Reject) -> String {
    match reject {
        Reject::UnknownPass => "pass is unknown or revoked".to_owned(),
        Reject::DepthExceeded { max } => format!("child depth limit {max} exceeded"),
        Reject::ModeAboveParent { parent } => {
            format!("requested mode is above the parent mode {}", parent.name())
        }
    }
}

/// 종료 신호는 기다리는 동안과 소켓에 쓰는 동안 모두 받는다. 줄 중간에서 끊으면 그 연결은 닫아 이어 쓰지 않는다.
/// 끝나면 `closed`로 읽기 쪽에 알려 연결을 함께 정리한다.
async fn write_loop(
    mut write_half: tokio::net::unix::OwnedWriteHalf,
    mut receiver: mpsc::Receiver<ServerMessage>,
    kill: Arc<Notify>,
    closed: Arc<Notify>,
) {
    loop {
        let message = tokio::select! {
            message = receiver.recv() => message,
            () = kill.notified() => None,
        };
        let Some(message) = message else { break };
        let Some(line) = encode_logged(&message) else {
            continue;
        };
        let written = tokio::select! {
            written = write_half.write_all(line.as_bytes()) => written.is_ok(),
            () = kill.notified() => false,
        };
        if !written {
            break;
        }
    }
    let _ = write_half.shutdown().await; // 상대가 이미 끊었다
    closed.notify_one();
}

fn encode_logged(message: &ServerMessage) -> Option<String> {
    envelope::encode_line(message)
        .inspect_err(|error| tracing::error!(%error, "failed to encode server message"))
        .ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    // #460
    #[tokio::test]
    async fn kill_interrupts_a_write_blocked_by_a_client_that_does_not_read() {
        let (stream, _unread_peer) = UnixStream::pair().unwrap();
        let (_read, writer) = stream.into_split();
        let (sender, receiver) = mpsc::channel(1);
        let kill = Arc::new(Notify::new());
        sender
            .send(Response::error(None, -1, "x".repeat(8 * 1024 * 1024)).into())
            .await
            .unwrap();
        let mut task = tokio::spawn(write_loop(
            writer,
            receiver,
            Arc::clone(&kill),
            Arc::new(Notify::new()),
        ));
        tokio::time::sleep(Duration::from_millis(250)).await;

        kill.notify_one();

        let finished = tokio::time::timeout(Duration::from_secs(2), &mut task).await;
        task.abort();
        assert!(
            finished.is_ok(),
            "the kill signal should end a blocked write"
        );
    }
}
