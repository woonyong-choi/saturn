//! 접속 하나의 읽기·쓰기 작업. 해석하지 못한 줄은 inbox 대신 오류 응답을 outbox에 넣는다.
//! `SubmitRouterKey` 줄은 해석 실패여도 원문을 로그에 남기지 않는다.

use std::sync::Arc;

use saturn_protocol::envelope::{self, CodecError, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::sync::{Notify, mpsc};

use super::{ClientId, RpcEvent};

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
    pub(crate) fn spawn(self, inbox: mpsc::Sender<RpcEvent>) -> Outbox {
        let (sender, receiver) = mpsc::channel(OUTBOX_CAPACITY);
        let kill = Arc::new(Notify::new());
        let (read_half, write_half) = self.stream.into_split();
        tokio::spawn(write_loop(write_half, receiver, Arc::clone(&kill)));
        tokio::spawn(read_loop(self.id, read_half, inbox, sender.clone()));
        Outbox { sender, kill }
    }
}

async fn read_loop(
    id: ClientId,
    read_half: tokio::net::unix::OwnedReadHalf,
    inbox: mpsc::Sender<RpcEvent>,
    replies: mpsc::Sender<ServerMessage>,
) {
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = next_line(&mut lines, id).await {
        if line.trim().is_empty() {
            continue;
        }
        if !forward_line(id, &line, &inbox, &replies).await {
            return; // 서버가 이미 닫혔다
        }
    }
    let _ = inbox.send(RpcEvent::Disconnected(id)).await; // 서버가 이미 닫혔다
}

async fn next_line(
    lines: &mut Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    id: ClientId,
) -> Option<String> {
    match lines.next_line().await {
        Ok(line) => line,
        Err(error) => {
            tracing::debug!(client = id.0, %error, "client read failed");
            None
        }
    }
}

/// 서버가 이미 닫혀 요청을 넘기지 못하면 거짓.
async fn forward_line(
    id: ClientId,
    line: &str,
    inbox: &mpsc::Sender<RpcEvent>,
    replies: &mpsc::Sender<ServerMessage>,
) -> bool {
    match envelope::decode_client_line(line) {
        Ok(message) => {
            let event = RpcEvent::Request(id, message.id, message.request);
            inbox.send(event).await.is_ok()
        }
        Err(error) => {
            if let CodecError::Decode { kind, column, .. } = &error {
                tracing::warn!(client = id.0, %kind, column, "dropped client line");
            }
            let _ = replies.try_send(error.to_response().into()); // 넘치면 쓰기 쪽이 곧 끊긴다
            true
        }
    }
}

async fn write_loop(
    mut write_half: tokio::net::unix::OwnedWriteHalf,
    mut receiver: mpsc::Receiver<ServerMessage>,
    kill: Arc<Notify>,
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
        if write_half.write_all(line.as_bytes()).await.is_err() {
            break;
        }
    }
    let _ = write_half.shutdown().await; // 상대가 이미 끊었다
}

fn encode_logged(message: &ServerMessage) -> Option<String> {
    envelope::encode_line(message)
        .inspect_err(|error| tracing::error!(%error, "failed to encode server message"))
        .ok()
}
