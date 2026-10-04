//! engine 접속 클라이언트. TUI와 CLI가 함께 쓴다.

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::{
    self, ClientMessage, CodecError, ErrorKind, Outcome, RequestId, ServerMessage,
};
use saturn_protocol::rpc::{ATTACH_ENV_NAMES, Notification, QueryResult, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use saturn_protocol::home::SOCKET_FILE;

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
    Rejected {
        code: i32,
        /// 옛 engine은 보내지 않는다.
        kind: Option<ErrorKind>,
        message: String,
    },
    #[error("failed to decode engine message")]
    Decode(#[from] CodecError),
}

/// engine가 보낸 것 하나. 알림이거나 조회 요청의 결과다.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification(Notification),
    /// 조회 요청의 응답 `result`. 요청을 보낸 이 접속에만 온다.
    Result(QueryResult),
    /// 요청 하나의 거절 응답. 연결은 그대로이고 다른 요청에는 영향이 없다.
    Rejected(Rejection),
}

/// engine이 요청 하나를 거절한 응답.
#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    /// 거절된 요청의 번호. 줄을 해석하지 못해 번호를 모르면 `None`.
    pub id: Option<RequestId>,
    pub code: i32,
    /// 옛 engine은 보내지 않는다.
    pub kind: Option<ErrorKind>,
    /// 사용자 입력과 비밀값이 없다.
    pub message: String,
}

impl Rejection {
    /// 연결을 끝내야 하는 오류로 바꾼다. `cli`가 종료 코드를 정하는 데 쓴다.
    pub fn into_error(self) -> ClientError {
        ClientError::Rejected {
            code: self.code,
            kind: self.kind,
            message: self.message,
        }
    }
}

#[derive(Debug)]
pub struct EngineClient {
    socket: PathBuf,
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_request: u64,
    /// 소켓 반대편 프로세스 번호. 운영체제가 알려 주지 않으면 `None`.
    peer_pid: Option<i32>,
}

/// engine이 `Version`에 답한 빌드 버전과 protocol 판.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineVersion {
    pub saturn_version: String,
    pub protocol_version: u32,
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
    /// 홈 폴더 안의 `engine.sock`. 홈 폴더는 환경 변수 `SATURN_HOME`, 없으면 `~/.saturn`.
    pub fn default_socket() -> PathBuf {
        saturn_protocol::home::from_env()
            .unwrap_or_else(|| PathBuf::from(saturn_protocol::home::DEFAULT_DIR))
            .join(SOCKET_FILE)
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
        let peer_pid = stream.peer_cred().ok().and_then(|cred| cred.pid());
        let (read_half, writer) = stream.into_split();
        Ok(Self {
            socket: socket.to_owned(),
            lines: BufReader::new(read_half).lines(),
            writer,
            next_request: 0,
            peer_pid,
        })
    }

    /// 소켓을 받는 engine의 프로세스 번호. 종료 요청을 모르는 옛 engine을 끝낼 때 쓴다.
    pub fn peer_pid(&self) -> Option<i32> {
        self.peer_pid
    }

    /// `Attach` 전에 engine의 빌드 버전과 protocol 판을 묻는다.
    ///
    /// # Errors
    /// `Version`을 모르는 옛 engine은 `Rejected`, 연결이 끊겼거나 답에 버전이 없으면 `Closed`.
    pub async fn version(&mut self) -> Result<EngineVersion, ClientError> {
        let mut found = None;
        self.call(Request::Version, |notification| {
            if let Notification::EngineVersion {
                saturn_version,
                protocol_version,
            } = notification
            {
                found = Some(EngineVersion {
                    saturn_version,
                    protocol_version,
                });
            }
        })
        .await?;
        found.ok_or(ClientError::Closed)
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// 요청마다 연결별 번호를 붙이고 그 번호를 돌려준다. 거절 응답은 이 번호로 짝짓는다.
    ///
    /// # Errors
    /// 연결이 끊겼으면 `Closed`.
    pub async fn send(&mut self, request: Request) -> Result<RequestId, ClientError> {
        let id = RequestId(self.next_request);
        self.next_request += 1;
        let line = envelope::encode_line(&ClientMessage::new(id, request))?;
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|_| ClientError::Closed)?;
        Ok(id)
    }

    /// 거절 응답은 `Incoming::Rejected`, 조회 결과는 `Incoming::Result`로 돌려준다. 연결이 끝나면 `None`.
    /// cancel-safe: 줄 단위 읽기라 `select!` 안에서 취소돼도 메시지를 잃지 않는다.
    pub async fn next(&mut self) -> Option<Incoming> {
        loop {
            match self.next_message().await? {
                ServerMessage::Notification(message) => {
                    return Some(Incoming::Notification(message.notification));
                }
                ServerMessage::Response(response) => match response.outcome {
                    Outcome::Ok(Some(result)) => return Some(Incoming::Result(result)),
                    Outcome::Ok(None) => {}
                    Outcome::Err(error) => {
                        return Some(Incoming::Rejected(Rejection {
                            id: response.id,
                            code: error.code,
                            kind: error.kind,
                            message: error.message,
                        }));
                    }
                },
            }
        }
    }

    // cost: time O(m), heap O(1), stack O(1), io m
    // vars: m = 요청 하나가 응답을 받기까지 온 알림 수
    // basis: estimate
    /// 요청 하나를 보내고 그 응답이 올 때까지 받은 알림을 `on_notification`에 넘긴다.
    /// 요청이 하나만 진행 중일 때 쓴다(`cli` 하위 명령). 응답 번호는 비교하지 않는다.
    /// 조회 요청이면 응답의 `result`를 돌려주고, 명령 요청이면 `None`이다.
    ///
    /// # Errors
    /// engine이 거절했으면 `Rejected`, 연결이 끊겼으면 `Closed`.
    pub async fn call(
        &mut self,
        request: Request,
        mut on_notification: impl FnMut(Notification),
    ) -> Result<Option<QueryResult>, ClientError> {
        self.send(request).await?;
        loop {
            match self.next_message().await.ok_or(ClientError::Closed)? {
                ServerMessage::Notification(message) => on_notification(message.notification),
                ServerMessage::Response(response) => {
                    return match response.outcome {
                        Outcome::Ok(result) => Ok(result),
                        Outcome::Err(error) => Err(ClientError::Rejected {
                            code: error.code,
                            kind: error.kind,
                            message: error.message,
                        }),
                    };
                }
            }
        }
    }

    async fn next_message(&mut self) -> Option<ServerMessage> {
        loop {
            let line = self.read_line().await?;
            match envelope::decode_server_line(&line) {
                Ok(message) => return Some(message),
                Err(error) => tracing::warn!(%error, "dropped engine line"),
            }
        }
    }

    async fn read_line(&mut self) -> Option<String> {
        match self.lines.next_line().await {
            Ok(line) => line,
            Err(error) => {
                tracing::warn!(%error, "engine read failed");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
