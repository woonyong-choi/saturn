//! 접속 하나의 읽기·쓰기 작업. 한 줄에 JSON 메시지 하나.
//!
//! 설계: docs/design/engine-lifecycle.md(여러 TUI 동시 접속).
//! 읽기 작업은 줄마다 `Request`로 해석해 서버 inbox로 보내고, 쓰기 작업은 outbox의 `Notification`을 한 줄씩 쓴다.
//! `Request::SubmitJudgeKey` 줄은 해석 실패여도 원문을 로그에 남기지 않는다.

use saturn_protocol::rpc::Notification;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::{ClientId, RpcEvent};

/// 접속 하나.
#[derive(Debug)]
pub(crate) struct Connection {
    /// 접속 id.
    id: ClientId,
    /// 소켓.
    stream: UnixStream,
}

impl Connection {
    /// 수락한 소켓으로 만든다.
    pub(crate) fn new(id: ClientId, stream: UnixStream) -> Self {
        Self { id, stream }
    }

    /// 읽기·쓰기 작업을 띄우고 알림 outbox를 돌려준다. 읽기가 끝나면 `RpcEvent::Disconnected`를 inbox로 보낸다.
    /// TODO(#89): outbox 크기와 느린 TUI 때문에 넘칠 때 처리 미정
    pub(crate) fn spawn(self, inbox: mpsc::Sender<RpcEvent>) -> mpsc::Sender<Notification> {
        todo!("#89")
    }
}
