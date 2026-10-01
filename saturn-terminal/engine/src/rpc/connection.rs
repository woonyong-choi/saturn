//! 접속 하나의 읽기·쓰기 작업. 해석하지 못한 줄은 inbox 대신 오류 응답을 outbox에 넣는다.
//! `SubmitJudgeKey` 줄은 해석 실패여도 원문을 로그에 남기지 않는다.

use std::sync::Arc;

use saturn_protocol::envelope::{self, CodecError, ServerMessage};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                tracing::debug!(client = id.0, %error, "client read failed");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        match envelope::decode_client_line(&line) {
            Ok(message) => {
                let event = RpcEvent::Request(id, message.id, message.request);
                if inbox.send(event).await.is_err() {
                    return; // 서버가 이미 닫혔다
                }
            }
            Err(error) => {
                if let CodecError::Decode { kind, column, .. } = &error {
                    tracing::warn!(client = id.0, %kind, column, "dropped client line");
                }
                let _ = replies.try_send(error.to_response().into()); // 넘치면 쓰기 쪽이 곧 끊긴다
            }
        }
    }
    let _ = inbox.send(RpcEvent::Disconnected(id)).await; // 서버가 이미 닫혔다
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
        let line = match envelope::encode_line(&message) {
            Ok(line) => line,
            Err(error) => {
                tracing::error!(%error, "failed to encode server message");
                continue;
            }
        };
        if write_half.write_all(line.as_bytes()).await.is_err() {
            break;
        }
    }
    let _ = write_half.shutdown().await; // 상대가 이미 끊었다
}
