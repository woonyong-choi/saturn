//! 채팅 폴더 추가 등록 테스트: 더한 폴더는 기록에 남아 이어 열어도 유지되고 provider session을 열 때 넘어가며, 설정 파일은 읽지 않는다.

use std::path::{Path, PathBuf};

use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ChatNotice, Notification, Request};

use super::support::{CLIENT, Flow, idle_reply};
use super::*;
use crate::providers::test_support::Call;
use crate::rpc::ClientId;

/// 링크를 푼 절대 경로. engine이 저장하는 값과 같다.
fn made_dir(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn attach_with(chat: Option<ChatId>, workdir: &Path, add_dirs: &[&Path]) -> Request {
    Request::Attach {
        chat,
        workdir: workdir.display().to_string(),
        env: Vec::new(),
        overrides: Vec::new(),
        add_dirs: add_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect(),
    }
}

fn start_info_dirs(notifications: &[Notification]) -> Vec<String> {
    notifications
        .iter()
        .find_map(|notification| match notification {
            Notification::StartInfo { added_dirs, .. } => Some(added_dirs.clone()),
            _ => None,
        })
        .expect("attach should send start info")
}

fn attached_chat(notifications: &[Notification]) -> ChatId {
    notifications
        .iter()
        .find_map(|notification| match notification {
            Notification::HistoryChunk { chat, .. } => Some(*chat),
            _ => None,
        })
        .expect("attach should send history")
}

fn folder_notices(notifications: &[Notification]) -> Vec<(String, bool)> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice:
                    ChatNotice::FolderAdded {
                        path,
                        applies_from_next_session,
                    },
                ..
            } => Some((path.clone(), *applies_from_next_session)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn add_dir_from_attach_is_saved_and_kept_when_the_chat_is_opened_again() {
    let fixture = Fixture::new();
    let extra = made_dir(fixture.root.path(), "extra");
    let mut engine = fixture.ready().await;
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;

    let (opened, reopened) = drive(&mut engine, async {
        let opened = first
            .attach(1, attach_with(None, &fixture.workdir, &[&extra]))
            .await;
        let chat = attached_chat(&opened);
        let reopened = second
            .attach(1, attach_with(Some(chat), &fixture.workdir, &[]))
            .await;
        (opened, reopened)
    })
    .await;

    let chat = attached_chat(&opened);
    assert_eq!(start_info_dirs(&opened), vec![extra.display().to_string()]);
    assert_eq!(
        start_info_dirs(&reopened),
        vec![extra.display().to_string()]
    );
    assert_eq!(engine.store.chat_dirs(chat).await.unwrap(), vec![extra]);
}

#[tokio::test]
async fn add_dir_is_read_from_records_not_from_memory_when_a_chat_is_reopened() {
    let fixture = Fixture::new();
    let extra = made_dir(fixture.root.path(), "extra");
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    engine.store.add_chat_dir(chat, &extra).await.unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client
            .attach(1, attach_with(Some(chat), &fixture.workdir, &[]))
            .await
    })
    .await;

    assert_eq!(
        start_info_dirs(&received),
        vec![extra.display().to_string()]
    );
    assert_eq!(engine.chat_dirs_of(chat), vec![extra]);
}

#[tokio::test]
async fn add_dir_reaches_the_session_spec_of_every_provider() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let extra = made_dir(flow.fixture.root.path(), "extra");
    flow.engine
        .add_dir(CLIENT, flow.chat, &extra.display().to_string())
        .await
        .unwrap();

    let input = flow.submit("fix the build").await;

    let opened: Vec<Vec<PathBuf>> = flow
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { add_dirs, .. } => Some(add_dirs),
            _ => None,
        })
        .collect();
    assert_eq!(opened, vec![vec![extra.clone()]]);
    let record = flow.record(input);
    let other_provider_spec =
        flow.engine
            .session_spec(&record, flow.agent(), None, None, Some("packet".to_owned()));
    assert_eq!(other_provider_spec.add_dirs, vec![extra]);
}

#[tokio::test]
async fn add_dir_while_a_session_is_open_applies_from_the_next_session() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let mut client = flow.client().await;
    let extra = made_dir(flow.fixture.root.path(), "extra");
    let opens = || {
        flow.fake
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Open { .. }))
            .count()
    };
    let before = opens();

    flow.engine
        .add_dir(CLIENT, flow.chat, &extra.display().to_string())
        .await
        .unwrap();

    assert_eq!(opens(), before);
    assert_eq!(
        folder_notices(&client.window().await),
        vec![(extra.display().to_string(), true)]
    );
}

#[tokio::test]
async fn add_dir_without_an_open_session_applies_at_once() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    let extra = made_dir(flow.fixture.root.path(), "extra");

    flow.engine
        .add_dir(CLIENT, flow.chat, &extra.display().to_string())
        .await
        .unwrap();

    assert_eq!(
        folder_notices(&client.window().await),
        vec![(extra.display().to_string(), false)]
    );
}

#[tokio::test]
async fn add_dir_twice_or_the_base_folder_changes_nothing() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    let extra = made_dir(flow.fixture.root.path(), "extra");
    let text = extra.display().to_string();
    let base = flow.fixture.workdir.display().to_string();

    for path in [&text, &text, &base] {
        flow.engine.add_dir(CLIENT, flow.chat, path).await.unwrap();
    }

    assert_eq!(flow.engine.chat_dirs_of(flow.chat), vec![extra.clone()]);
    assert_eq!(
        flow.engine.store.chat_dirs(flow.chat).await.unwrap(),
        vec![extra]
    );
    assert_eq!(folder_notices(&client.window().await).len(), 1);
}

#[tokio::test]
async fn add_dir_refuses_other_clients_and_paths_that_are_not_folders() {
    let mut flow = Flow::new(Vec::new()).await;
    let root = flow.fixture.root.path().to_path_buf();
    let file = root.join("a-file");
    std::fs::write(&file, "x").unwrap();
    let extra = made_dir(&root, "extra");

    let other_client = flow
        .engine
        .add_dir(ClientId(2), flow.chat, &extra.display().to_string())
        .await
        .unwrap_err();
    let mut refused = Vec::new();
    for path in [
        "relative/dir".to_owned(),
        root.join("missing").display().to_string(),
        file.display().to_string(),
    ] {
        refused.push(flow.engine.add_dir(CLIENT, flow.chat, &path).await);
    }

    assert!(matches!(other_client, EngineError::ChatNotAttached { .. }));
    assert!(
        refused
            .iter()
            .all(|result| matches!(result, Err(EngineError::InvalidFolder { .. })))
    );
    assert!(flow.engine.chat_dirs_of(flow.chat).is_empty());
}

#[tokio::test]
async fn add_dir_with_a_bad_attach_path_creates_no_chat() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;
    let missing = fixture.root.path().join("missing");

    let response = drive(&mut engine, async {
        client
            .send(1, attach_with(None, &fixture.workdir, &[missing.as_path()]))
            .await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
    assert!(engine.store.chat_workdir(ChatId(1)).await.is_err());
}

#[tokio::test]
async fn add_dir_settings_file_is_not_read() {
    let fixture = Fixture::new();
    let extra = made_dir(fixture.root.path(), "extra");
    std::fs::create_dir_all(extra.join(".git")).unwrap();
    std::fs::create_dir_all(extra.join(".saturn")).unwrap();
    std::fs::write(
        extra.join(".saturn/config.toml"),
        "tui.on_exit = \"stop\"\n[permission.shell]\n\"*\" = \"allow\"\n",
    )
    .unwrap();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client
            .attach(1, attach_with(None, &fixture.workdir, &[&extra]))
            .await
    })
    .await;

    assert!(
        received
            .iter()
            .all(|notification| !matches!(notification, Notification::FolderTrustRequested { .. }))
    );
    let revision = engine.settings.current().unwrap();
    let settings = engine.settings.at(&engine.store, revision).await.unwrap();
    assert_eq!(
        settings.on_exit(),
        saturn_protocol::state::OnExit::Background
    );
    assert!(settings.permission().rules.is_empty());
}

#[tokio::test]
async fn add_dir_session_spec_gets_nothing_for_a_chat_without_added_folders() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;

    flow.submit("fix the build").await;

    let opened = flow.fake.calls().into_iter().find_map(|call| match call {
        Call::Open { add_dirs, .. } => Some(add_dirs),
        _ => None,
    });
    assert_eq!(opened, Some(Vec::new()));
}
