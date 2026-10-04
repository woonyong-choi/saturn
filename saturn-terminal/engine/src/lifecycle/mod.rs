//! engine 시작 순서, 요청 분배, 채팅 붙기, 입력 흐름 테스트. 가짜 router 전송, 가짜 provider, 임시 폴더만 쓴다.

mod add_dir;
mod agent_questions;
mod attach;
mod chat_labels;
mod chats;
mod child_sessions;
mod commands;
mod conflict_steer;
mod crash_recovery;
mod decision;
mod deliver;
mod events;
mod exit;
mod fake_provider;
mod inputs;
mod intake;
mod live_settings;
mod model;
mod outcomes;
mod packet_overflow;
mod permissions;
mod provider_stall;
mod prune;
mod read_only_steer;
mod requests;
mod restore_inputs;
mod send_now;
mod sessions;
mod settings_watch;
mod start;
mod steer_rejected;
mod stop;
mod support;
mod switch_round_trip;
mod target_model;
mod tasks;
mod turn_end;
mod upgrade;
mod versions;
mod write_scope;

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

use crate::routers::test_support::{
    FakeTransport, HttpReply, KEY, TransportError, ok, router, status,
};
use crate::routers::{ActiveRouter, SharedSecrets};
use crate::rpc::SOCKET_FILE;
use crate::secrets::{KeyInput, KeySource, RouterKey, SecretStore, StorageMode};
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
            after_upgrade: false,
        };
        Self {
            root,
            options,
            workdir,
        }
    }

    fn key_file(&self) -> PathBuf {
        self.root.path().join("router.key")
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
                .save(RouterKey::new(KEY.to_owned()).unwrap(), KeySource::Stored)
                .await
                .unwrap();
        }
        Arc::new(Mutex::new(store))
    }

    async fn env(&self, with_key: bool, transport: Arc<FakeTransport>) -> StartEnv {
        let secrets = self.secrets(with_key).await;
        StartEnv {
            nested_marker: None,
            router: Some(ActiveRouter::Remote(router(
                Arc::clone(&secrets),
                transport,
            ))),
            secrets: Some(secrets),
            key_inputs: Some(Vec::<KeyInput>::new()),
        }
    }

    async fn start(&self, env: StartEnv) -> Result<Engine, EngineError> {
        Engine::start_with(self.options.clone(), env).await
    }

    /// router 확인이 통과한 engine.
    async fn ready(&self) -> Engine {
        let transport = FakeTransport::new(vec![ok(MODELS), ok(CHECK_OK)]);
        self.start(self.env(true, transport).await).await.unwrap()
    }

    /// 키가 없어 router 키를 기다리는 engine과, 키를 다시 확인할 때 쓸 전송.
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

    /// `pick`이 값을 돌려주는 알림까지 읽고, 그 값을 돌려준다. 그 전에 온 알림은 버린다.
    async fn until<T>(&mut self, mut pick: impl FnMut(&Notification) -> Option<T>) -> T {
        loop {
            let notification = self.notification().await;
            if let Some(found) = pick(&notification) {
                return found;
            }
        }
    }

    /// 짧은 시간 동안 도착한 알림을 모두 읽는다. 아무것도 오지 않아야 하는 시험은 이 목록이 비어 있는지 본다.
    async fn window(&mut self) -> Vec<Notification> {
        let mut seen = Vec::new();
        while let Ok(line) = timeout(Duration::from_millis(300), self.lines.next_line()).await {
            let line = line
                .unwrap()
                .expect("engine should keep the connection open");
            match decode_server_line(&line).unwrap() {
                ServerMessage::Notification(message) => seen.push(message.notification),
                ServerMessage::Response(response) => panic!("unexpected response {response:?}"),
            }
        }
        seen
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
        add_dirs: Vec::new(),
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
