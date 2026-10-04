//! `run_plain` 본체를 가짜 engine과 파이프 입력으로 시험한다.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use saturn_protocol::envelope::{
    ErrorKind, NotificationMessage, Response, ServerMessage, decode_client_line, encode_line,
};
use saturn_protocol::rpc::Notification;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

use super::*;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn options(home: &TempDir) -> RunOptions {
    RunOptions {
        chat: None,
        lang: Some(Lang::Ko),
        workdir: "/work".into(),
        overrides: Vec::new(),
        add_dirs: Vec::new(),
        history: home.path().join("history"),
        child: None,
        plain: Some(true),
    }
}

/// `Attach`에는 채팅을 알리고 응답한 뒤, 그 뒤 요청마다 `reply`가 정한 응답을 보낸다. 연결은 닫지 않는다.
fn fake_engine(
    home: &TempDir,
    reply: impl Fn(&Request, saturn_protocol::envelope::RequestId) -> Option<Response> + Send + 'static,
) -> (std::path::PathBuf, tokio::task::JoinHandle<Vec<Request>>) {
    let socket = home.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut writer) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        let mut seen = Vec::new();
        while let Some(line) = lines.next_line().await.unwrap() {
            let message = decode_client_line(&line).unwrap();
            let response = if matches!(message.request, Request::Attach { .. }) {
                let chunk = Notification::HistoryChunk {
                    chat: ChatId(7),
                    entries: Vec::new(),
                    oldest: None,
                    has_more: false,
                };
                let note = ServerMessage::Notification(NotificationMessage::new(chunk));
                writer
                    .write_all(encode_line(&note).unwrap().as_bytes())
                    .await
                    .unwrap();
                Some(Response::ok(message.id))
            } else {
                reply(&message.request, message.id)
            };
            let detach = matches!(message.request, Request::Detach);
            seen.push(message.request);
            if let Some(response) = response {
                let line = encode_line(&ServerMessage::from(response)).unwrap();
                // 클라이언트가 `Detach` 뒤 먼저 닫아도 시험은 요청 기록만 본다
                let _ = writer.write_all(line.as_bytes()).await;
            }
            if detach {
                break;
            }
        }
        seen
    });
    (socket, server)
}

async fn run_with_input(
    socket: &std::path::Path,
    home: &TempDir,
    input: &'static str,
) -> (Result<(), TuiError>, String) {
    let mut client = EngineClient::connect(socket).await.unwrap();
    let out = Captured::default();
    let output = PlainOutput::new(out.clone(), Lang::Ko);
    let stdin = BufReader::new(input.as_bytes()).lines();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        plain_session(&mut client, options(home), stdin, output),
    )
    .await
    .expect("plain mode kept waiting after the rejection");
    (result, out.text())
}

#[tokio::test]
async fn plain_ends_with_the_rejection_when_the_only_input_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let (socket, server) = fake_engine(&home, |request, id| match request {
        Request::SubmitInput { .. } => Some(Response::error_of_kind(
            Some(id),
            -32602,
            Some(ErrorKind::Failed),
            "chat is closed",
        )),
        _ => Some(Response::ok(id)),
    });

    let (result, text) = run_with_input(&socket, &home, "hello\n").await;

    assert!(matches!(
        result,
        Err(TuiError::Client(ClientError::Rejected {
            kind: Some(ErrorKind::Failed),
            ..
        }))
    ));
    assert!(text.contains("입력을 접수하지 못했습니다: chat is closed"));
    server.await.unwrap();
}

#[tokio::test]
async fn plain_refused_attach_ends_instead_of_waiting_for_a_chat() {
    let home = tempfile::tempdir().unwrap();
    let socket = home.path().join("engine.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut writer) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        let line = lines.next_line().await.unwrap().unwrap();
        let id = decode_client_line(&line).unwrap().id;
        let reply = Response::error(Some(id), -32602, "bad folder");
        writer
            .write_all(encode_line(&ServerMessage::from(reply)).unwrap().as_bytes())
            .await
            .unwrap();
        // 연결은 닫지 않고 둔다
        while lines.next_line().await.unwrap().is_some() {}
    });

    let (result, text) = run_with_input(&socket, &home, "hello\n").await;

    assert!(matches!(
        result,
        Err(TuiError::Client(ClientError::Rejected { code: -32602, .. }))
    ));
    assert!(text.contains("engine이 요청을 거절했습니다: bad folder"));
    drop(server);
}

#[tokio::test]
async fn plain_keeps_waiting_for_an_accepted_input_when_another_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let (socket, _server) = fake_engine(&home, |request, id| match request {
        Request::SubmitInput { client_ref: 2, .. } => {
            Some(Response::error(Some(id), -32602, "second refused"))
        }
        _ => Some(Response::ok(id)),
    });
    let mut client = EngineClient::connect(&socket).await.unwrap();
    let out = Captured::default();
    let output = PlainOutput::new(out.clone(), Lang::Ko);
    let stdin = BufReader::new("one\ntwo\n".as_bytes()).lines();

    let outcome = tokio::time::timeout(
        Duration::from_millis(500),
        plain_session(&mut client, options(&home), stdin, output),
    )
    .await;

    // 첫 입력의 실행 결과는 오지 않았으므로 끝나지 않고 기다려야 한다
    assert!(outcome.is_err());
    assert!(out.text().contains("second refused"));
}
