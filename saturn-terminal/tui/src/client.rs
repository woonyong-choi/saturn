//! engine 접속 클라이언트. TUI와 CLI가 함께 쓴다.

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::{
    self, ClientMessage, CodecError, Outcome, RequestId, ServerMessage,
};
use saturn_protocol::rpc::{Notification, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

/// engine `rpc::SOCKET_FILE`과 같은 값.
const SOCKET_FILE: &str = "engine.sock";

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// 호출자는 engine을 시작하고 다시 붙는다.
    #[error("engine is not running at {path}")]
    NotRunning {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("engine connection closed")]
    Closed,
    #[error("failed to decode engine message")]
    Decode(#[from] CodecError),
}

#[derive(Debug)]
pub struct EngineClient {
    socket: PathBuf,
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_request: u64,
}

impl EngineClient {
    /// `~/.saturn/engine.sock`. TODO(#49): 경로 설정 키
    pub fn default_socket() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        home.join(".saturn").join(SOCKET_FILE)
    }

    /// # Errors
    /// 소켓이 없거나 받는 engine이 없으면 `NotRunning`.
    pub async fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream =
            UnixStream::connect(socket)
                .await
                .map_err(|source| ClientError::NotRunning {
                    path: socket.to_owned(),
                    source,
                })?;
        let (read_half, writer) = stream.into_split();
        Ok(Self {
            socket: socket.to_owned(),
            lines: BufReader::new(read_half).lines(),
            writer,
            next_request: 0,
        })
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// 요청마다 연결별 번호를 붙인다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `Closed`.
    pub async fn send(&mut self, request: Request) -> Result<(), ClientError> {
        let id = RequestId(self.next_request);
        self.next_request += 1;
        let line = envelope::encode_line(&ClientMessage::new(id, request))?;
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| ClientError::Closed)
    }

    /// `Response`는 여기서 소비한다. 연결이 끝나면 `None`.
    /// cancel-safe: 줄 단위 읽기라 `select!` 안에서 취소돼도 메시지를 잃지 않는다.
    pub async fn next(&mut self) -> Option<Notification> {
        loop {
            let line = match self.lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => return None,
                Err(error) => {
                    tracing::warn!(%error, "engine read failed");
                    return None;
                }
            };
            match envelope::decode_server_line(&line) {
                Ok(ServerMessage::Notification(message)) => return Some(message.notification),
                Ok(ServerMessage::Response(response)) => {
                    if let Outcome::Err(error) = response.outcome {
                        tracing::warn!(code = error.code, message = %error.message, "engine rejected request");
                    }
                }
                Err(error) => tracing::warn!(%error, "dropped engine line"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::envelope::{
        NotificationMessage, Response, decode_client_line, encode_line,
    };
    use saturn_protocol::ids::ChatId;
    use tokio::net::UnixListener;

    use super::*;

    fn notice() -> Notification {
        Notification::ContextSize {
            chat: ChatId(1),
            tokens: Some(10),
            threshold: 100,
        }
    }

    #[tokio::test]
    async fn connect_without_engine_returns_not_running() {
        let home = tempfile::tempdir().unwrap();

        let result = EngineClient::connect(&home.path().join(SOCKET_FILE)).await;

        assert!(matches!(result, Err(ClientError::NotRunning { .. })));
    }

    #[tokio::test]
    async fn send_numbers_requests_and_next_skips_responses() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut writer) = stream.into_split();
            let mut lines = BufReader::new(read_half).lines();
            let mut ids = Vec::new();
            for _ in 0..2 {
                let line = lines.next_line().await.unwrap().unwrap();
                let message = decode_client_line(&line).unwrap();
                ids.push(message.id);
                let reply = ServerMessage::from(Response::ok(message.id));
                writer
                    .write_all(encode_line(&reply).unwrap().as_bytes())
                    .await
                    .unwrap();
            }
            let note = ServerMessage::Notification(NotificationMessage::new(notice()));
            writer
                .write_all(encode_line(&note).unwrap().as_bytes())
                .await
                .unwrap();
            ids
        });
        let mut client = EngineClient::connect(&socket).await.unwrap();

        client.send(Request::ListTasks).await.unwrap();
        client.send(Request::ListJudgeVersions).await.unwrap();
        let received = client.next().await;

        assert_eq!(received, Some(notice()));
        assert_eq!(server.await.unwrap(), vec![RequestId(0), RequestId(1)]);
        assert_eq!(client.next().await, None);
    }

    #[test]
    fn default_socket_ends_with_saturn_socket() {
        let socket = EngineClient::default_socket();

        assert!(socket.ends_with(".saturn/engine.sock"));
    }
}
