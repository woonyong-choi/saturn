//! 테스트용 가짜 engine. 소켓 하나에 접속 하나를 받아 정해 둔 답을 요청 순서대로 돌려준다.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use saturn_protocol::envelope::{
    NotificationMessage, Response, ServerMessage, decode_client_line, encode_line,
};
use saturn_protocol::rpc::{Notification, Request};
use saturn_tui::client::EngineClient;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::task::JoinHandle;

/// 요청 하나에 대한 알림들과 응답.
#[derive(Debug, Clone, Default)]
pub(crate) struct Reply {
    notifications: Vec<Notification>,
    error: Option<(i32, String)>,
}

impl Reply {
    pub(crate) fn ok() -> Self {
        Self::default()
    }

    pub(crate) fn with(notifications: Vec<Notification>) -> Self {
        Self {
            notifications,
            error: None,
        }
    }

    pub(crate) fn error(code: i32, message: &str) -> Self {
        Self {
            notifications: Vec::new(),
            error: Some((code, message.to_owned())),
        }
    }
}

#[derive(Debug)]
pub(crate) struct FakeEngine {
    home: tempfile::TempDir,
    received: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

impl FakeEngine {
    pub(crate) fn start(replies: Vec<Reply>) -> Self {
        let home = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(home.path().join("engine.sock")).unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&received);
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut writer) = stream.into_split();
            let mut lines = BufReader::new(read_half).lines();
            for reply in replies {
                let Some(line) = lines.next_line().await.unwrap() else {
                    return;
                };
                let message = decode_client_line(&line).unwrap();
                log.lock().unwrap().push(message.request);
                for notification in reply.notifications {
                    let note = ServerMessage::Notification(NotificationMessage::new(notification));
                    writer
                        .write_all(encode_line(&note).unwrap().as_bytes())
                        .await
                        .unwrap();
                }
                let response = match reply.error {
                    None => Response::ok(message.id),
                    Some((code, text)) => Response::error(Some(message.id), code, text),
                };
                writer
                    .write_all(
                        encode_line(&ServerMessage::from(response))
                            .unwrap()
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        Self {
            home,
            received,
            task,
        }
    }

    pub(crate) fn socket(&self) -> PathBuf {
        self.home.path().join("engine.sock")
    }

    pub(crate) async fn client(&self) -> EngineClient {
        EngineClient::connect(&self.socket()).await.unwrap()
    }

    /// 모든 답을 보낸 뒤의 받은 요청.
    pub(crate) async fn finish(self) -> Vec<Request> {
        self.task.await.unwrap();
        self.received.lock().unwrap().clone()
    }
}
