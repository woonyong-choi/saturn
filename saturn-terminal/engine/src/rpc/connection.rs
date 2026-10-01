//! 접속 하나의 읽기·쓰기 작업. 해석하지 못한 줄은 inbox 대신 오류 응답을 outbox에 넣는다.
//! `SubmitJudgeKey` 줄은 해석 실패여도 원문을 로그에 남기지 않는다.

use saturn_protocol::envelope::ServerMessage;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::{ClientId, RpcEvent};

#[derive(Debug)]
pub(crate) struct Connection {
    id: ClientId,
    stream: UnixStream,
}

impl Connection {
    pub(crate) fn new(id: ClientId, stream: UnixStream) -> Self {
        Self { id, stream }
    }

    /// 읽기가 끝나면 `RpcEvent::Disconnected`를 inbox로 보낸다.
    /// TODO(#89): outbox가 넘칠 때 처리
    pub(crate) fn spawn(self, inbox: mpsc::Sender<RpcEvent>) -> mpsc::Sender<ServerMessage> {
        todo!("#89")
    }
}
