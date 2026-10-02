use saturn_protocol::envelope::{NotificationMessage, Response, decode_client_line, encode_line};
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
