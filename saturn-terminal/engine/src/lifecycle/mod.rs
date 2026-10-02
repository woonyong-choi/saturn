//! engine 시작 순서, 요청 분배, 채팅 붙기, 입력 흐름 테스트. 가짜 judge 전송, 가짜 provider, 임시 폴더만 쓴다.

mod attach;
mod decision;
mod deliver;
mod intake;
mod outcomes;
mod requests;
mod sessions;
mod start;
mod support;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use saturn_protocol::envelope::{
    ClientMessage, Outcome, RequestId, Response, ServerMessage, decode_server_line, encode_line,
};
use saturn_protocol::rpc::{Notification, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::judges::test_support::{
    FakeTransport, HttpReply, KEY, TransportError, judge, ok, status,
};
use crate::judges::{ActiveJudge, SharedSecrets};
use crate::rpc::SOCKET_FILE;
use crate::secrets::{JudgeKey, KeyInput, KeySource, SecretStore, StorageMode};
use crate::{Engine, EngineError, EngineOptions, StartEnv};

const WAIT: Duration = Duration::from_secs(5);

const MODELS: &str = r#"{"models":[]}"#;

const CHECK_OK: &str = r#"{"model":"jev-1.13.0","answers":{"saturn_check":{"noul":0.6}},"usage":{"input_tokens":1,"output_tokens":1}}"#;

/// 폴더가 살아 있는 동안만 쓴다.
struct Fixture {
    root: tempfile::TempDir,
    options: EngineOptions,
    /// TUI가 붙을 때 넘기는 작업 폴더.
    workdir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workdir = root.path().join("work");
        std::fs::create_dir_all(workdir.join(".git")).unwrap();
        let options = EngineOptions {
            home: root.path().join("home"),
            run_overrides: Vec::new(),
        };
        Self {
            root,
            options,
            workdir,
        }
    }

    fn key_file(&self) -> PathBuf {
        self.root.path().join("judge.key")
    }

    fn socket(&self) -> PathBuf {
        self.options.home.join(SOCKET_FILE)
    }

    fn write_user_config(&self, content: &str) {
        std::fs::create_dir_all(&self.options.home).unwrap();
        std::fs::write(self.options.home.join("config.toml"), content).unwrap();
    }

    fn write_folder_config(&self, content: &str) {
        let dir = self.workdir.join(".saturn");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), content).unwrap();
    }

    /// 저장된 키가 있으면 그 키를 들고 시작한다.
    async fn secrets(&self, with_key: bool) -> SharedSecrets {
        let mut store = SecretStore::with_key_file(self.key_file(), StorageMode::Standard);
        if with_key {
            store
                .save(JudgeKey::new(KEY.to_owned()).unwrap(), KeySource::Stored)
                .await
                .unwrap();
        }
        Arc::new(Mutex::new(store))
    }

    async fn env(&self, with_key: bool, transport: Arc<FakeTransport>) -> StartEnv {
        let secrets = self.secrets(with_key).await;
        StartEnv {
            nested_marker: None,
            judge: Some(ActiveJudge::Remote(judge(Arc::clone(&secrets), transport))),
            secrets: Some(secrets),
            key_inputs: Some(Vec::<KeyInput>::new()),
        }
    }

    async fn start(&self, env: StartEnv) -> Result<Engine, EngineError> {
        Engine::start_with(self.options.clone(), env).await
    }

    /// judge 확인이 통과한 engine.
    async fn ready(&self) -> Engine {
        let transport = FakeTransport::new(vec![ok(MODELS), ok(CHECK_OK)]);
        self.start(self.env(true, transport).await).await.unwrap()
    }

    /// 키가 없어 judge 키를 기다리는 engine과, 키를 다시 확인할 때 쓸 전송.
    async fn waiting_for_key(&self, replies: Vec<FakeReply>) -> Engine {
        let transport = FakeTransport::new(replies);
        self.start(self.env(false, transport).await).await.unwrap()
    }
}

type FakeReply = Result<HttpReply, TransportError>;

fn check_passes() -> Vec<FakeReply> {
    vec![ok(MODELS), ok(CHECK_OK)]
}

fn key_rejected() -> FakeReply {
    status(401, "{}", Vec::new())
}

struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
}

impl Client {
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

    async fn recv(&mut self) -> ServerMessage {
        let line = timeout(WAIT, self.lines.next_line())
            .await
            .expect("engine should reply in time")
            .unwrap()
            .expect("engine should keep the connection open");
        decode_server_line(&line).unwrap()
    }

    async fn notification(&mut self) -> Notification {
        match self.recv().await {
            ServerMessage::Notification(message) => message.notification,
            ServerMessage::Response(response) => panic!("expected notification, got {response:?}"),
        }
    }

    async fn response(&mut self) -> Response {
        match self.recv().await {
            ServerMessage::Response(response) => response,
            ServerMessage::Notification(message) => {
                panic!("expected response, got {:?}", message.notification)
            }
        }
    }

    /// `Attach` 응답까지 받은 알림.
    async fn attach(&mut self, id: u64, request: Request) -> Vec<Notification> {
        self.send(id, request).await;
        let mut notifications = Vec::new();
        loop {
            match self.recv().await {
                ServerMessage::Notification(message) => notifications.push(message.notification),
                ServerMessage::Response(response) => {
                    assert_eq!(response, Response::ok(RequestId(id)));
                    return notifications;
                }
            }
        }
    }
}

fn new_chat(workdir: &Path) -> Request {
    Request::Attach {
        chat: None,
        workdir: workdir.display().to_string(),
        env: Vec::new(),
        overrides: Vec::new(),
    }
}

fn error_code(response: &Response) -> i32 {
    match &response.outcome {
        Outcome::Err(error) => error.code,
        Outcome::Ok(()) => panic!("expected error response, got ok"),
    }
}

/// `script`가 끝날 때까지 같은 작업에서 engine 요청 처리를 돌린다.
async fn drive<T>(engine: &mut Engine, script: impl Future<Output = T>) -> T {
    tokio::select! {
        biased;
        result = script => result,
        served = engine.serve() => panic!("serve should not end: {served:?}"),
    }
}
