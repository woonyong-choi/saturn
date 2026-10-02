//! engine 접속 클라이언트. TUI와 CLI가 함께 쓴다.

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::{
    self, ClientMessage, CodecError, Outcome, RequestId, ServerMessage,
};
use saturn_protocol::rpc::{ATTACH_ENV_NAMES, Notification, Request};
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
    /// `message`에는 사용자 입력과 비밀값이 없다.
    #[error("engine rejected the request ({code}): {message}")]
    Rejected { code: i32, message: String },
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

/// 이 TUI의 환경 변수 중 `Attach`로 넘길 것. 없는 변수와 문자열이 아닌 값은 뺀다.
pub fn attach_env() -> Vec<(String, String)> {
    collect_attach_env(|name| std::env::var(name).ok())
}

fn collect_attach_env(lookup: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    ATTACH_ENV_NAMES
        .iter()
        .filter_map(|name| lookup(name).map(|value| ((*name).to_owned(), value)))
        .collect()
}

impl EngineClient {
    /// `~/.saturn/engine.sock`. TODO(#235): 경로 설정 키
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
            match self.next_message().await? {
                ServerMessage::Notification(message) => return Some(message.notification),
                ServerMessage::Response(response) => {
                    if let Outcome::Err(error) = response.outcome {
                        tracing::warn!(code = error.code, message = %error.message, "engine rejected request");
                    }
                }
            }
        }
    }

    // cost: time O(m), heap O(1), stack O(1), io m
    // vars: m = 요청 하나가 응답을 받기까지 온 알림 수
    // basis: estimate
    /// 요청 하나를 보내고 그 응답이 올 때까지 받은 알림을 `on_notification`에 넘긴다.
    /// 요청이 하나만 진행 중일 때 쓴다(`cli` 하위 명령). 응답 번호는 비교하지 않는다.
    ///
    /// # Errors
    /// engine이 거절했으면 `Rejected`, 연결이 끊겼으면 `Closed`.
    pub async fn call(
        &mut self,
        request: Request,
        mut on_notification: impl FnMut(Notification),
    ) -> Result<(), ClientError> {
        self.send(request).await?;
        loop {
            match self.next_message().await.ok_or(ClientError::Closed)? {
                ServerMessage::Notification(message) => on_notification(message.notification),
                ServerMessage::Response(response) => {
                    return match response.outcome {
                        Outcome::Ok(()) => Ok(()),
                        Outcome::Err(error) => Err(ClientError::Rejected {
                            code: error.code,
                            message: error.message,
                        }),
                    };
                }
            }
        }
    }

    async fn next_message(&mut self) -> Option<ServerMessage> {
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
                Ok(message) => return Some(message),
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

    #[test]
    fn attach_env_takes_only_listed_names_and_never_the_router_key() {
        let process = [
            ("PATH", "/opt/bin:/usr/bin"),
            ("SATURN_KEY", "sk-secret"),
            ("AWS_SECRET_ACCESS_KEY", "other"),
        ];

        let env = collect_attach_env(|name| {
            process
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        });

        assert_eq!(
            env,
            vec![("PATH".to_owned(), "/opt/bin:/usr/bin".to_owned())]
        );
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
        client.send(Request::ListRouterVersions).await.unwrap();
        let received = client.next().await;

        assert_eq!(received, Some(notice()));
        assert_eq!(server.await.unwrap(), vec![RequestId(0), RequestId(1)]);
        assert_eq!(client.next().await, None);
    }

    #[tokio::test]
    async fn call_collects_notifications_until_response_and_reports_rejection() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read_half, mut writer) = stream.into_split();
            let mut lines = BufReader::new(read_half).lines();
            for rejected in [false, true] {
                let line = lines.next_line().await.unwrap().unwrap();
                let id = decode_client_line(&line).unwrap().id;
                if !rejected {
                    let note = ServerMessage::Notification(NotificationMessage::new(notice()));
                    writer
                        .write_all(encode_line(&note).unwrap().as_bytes())
                        .await
                        .unwrap();
                }
                let reply = if rejected {
                    Response::error(Some(id), -32601, "unsupported")
                } else {
                    Response::ok(id)
                };
                writer
                    .write_all(encode_line(&ServerMessage::from(reply)).unwrap().as_bytes())
                    .await
                    .unwrap();
            }
        });
        let mut client = EngineClient::connect(&socket).await.unwrap();
        let mut seen = Vec::new();

        let first = client
            .call(Request::ListTasks, |note| seen.push(note))
            .await;
        let second = client.call(Request::ListTasks, |_| {}).await;

        assert!(first.is_ok());
        assert_eq!(seen, vec![notice()]);
        assert!(matches!(
            second,
            Err(ClientError::Rejected { code: -32601, .. })
        ));
        server.await.unwrap();
    }

    #[test]
    fn default_socket_ends_with_saturn_socket() {
        let socket = EngineClient::default_socket();

        assert!(socket.ends_with(".saturn/engine.sock"));
    }
}
