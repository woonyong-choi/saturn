use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::ids::ChatId;

use super::*;

fn rename(chat: ChatId, name: &str) -> Request {
    Request::RenameChat {
        chat,
        name: name.to_owned(),
    }
}

fn regroup(chat: ChatId, group: Option<&str>) -> Request {
    Request::SetChatGroup {
        chat,
        group: group.map(str::to_owned),
    }
}

async fn names_in_list(client: &mut Client, id: u64) -> Vec<Option<String>> {
    client.send(id, Request::ListChats { folder: None }).await;
    let Notification::ChatList { chats } = client.notification().await else {
        panic!("expected ChatList");
    };
    assert_eq!(client.response().await, Response::ok(RequestId(id)));
    chats.into_iter().map(|item| item.name).collect()
}

async fn answer(client: &mut Client, id: u64, request: Request) -> Response {
    client.send(id, request).await;
    client.response().await
}

#[tokio::test]
async fn rename_and_group_are_saved_trimmed_and_shown_in_the_chat_list() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let (renamed, grouped, names) = drive(&mut engine, async {
        let renamed = answer(&mut client, 1, rename(chat, "  login fix  ")).await;
        let grouped = answer(&mut client, 2, regroup(chat, Some(" auth "))).await;
        (renamed, grouped, names_in_list(&mut client, 3).await)
    })
    .await;

    assert_eq!(renamed, Response::ok(RequestId(1)));
    assert_eq!(grouped, Response::ok(RequestId(2)));
    assert_eq!(names, [Some("login fix".to_owned())]);
    assert_eq!(
        engine.store.chat_labels(chat).await.unwrap(),
        (Some("login fix".to_owned()), Some("auth".to_owned()))
    );
}

#[tokio::test]
async fn blank_name_and_no_group_clear_the_labels() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    engine.store.set_chat_name(chat, Some("old")).await.unwrap();
    engine
        .store
        .set_chat_group(chat, Some("old"))
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    drive(&mut engine, async {
        answer(&mut client, 1, rename(chat, "   ")).await;
        answer(&mut client, 2, regroup(chat, None)).await;
    })
    .await;

    assert_eq!(engine.store.chat_labels(chat).await.unwrap(), (None, None));
}

#[tokio::test]
async fn unknown_chat_and_control_characters_are_refused_without_changing_anything() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    engine
        .store
        .set_chat_name(chat, Some("keep"))
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let refused = drive(&mut engine, async {
        vec![
            answer(&mut client, 1, rename(ChatId(999), "x")).await,
            answer(&mut client, 2, regroup(ChatId(999), Some("x"))).await,
            answer(&mut client, 3, rename(chat, "a\nb")).await,
            answer(&mut client, 4, regroup(chat, Some("a\tb"))).await,
        ]
    })
    .await;

    assert!(
        refused
            .iter()
            .all(|response| error_code(response) == INVALID_PARAMS)
    );
    assert_eq!(
        engine.store.chat_labels(chat).await.unwrap(),
        (Some("keep".to_owned()), None)
    );
}

#[tokio::test]
async fn rename_and_group_notify_every_attached_tui_but_not_unattached_connections() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;
    let mut command = Client::connect(&fixture.socket()).await;
    let attach_chat = Request::Attach {
        chat: Some(chat),
        workdir: fixture.workdir.display().to_string(),
        env: Vec::new(),
        overrides: Vec::new(),
        add_dirs: Vec::new(),
    };

    let (first_seen, second_seen, command_seen) = drive(&mut engine, async {
        first.attach(1, attach_chat).await;
        second.attach(1, new_chat(&fixture.workdir)).await;
        assert_eq!(
            answer(&mut command, 2, rename(chat, " login fix ")).await,
            Response::ok(RequestId(2))
        );
        assert_eq!(
            answer(&mut command, 3, regroup(chat, Some("auth"))).await,
            Response::ok(RequestId(3))
        );
        (
            first.window().await,
            second.window().await,
            command.window().await,
        )
    })
    .await;

    let labeled = |name: &str, group: Option<&str>| Notification::ChatLabeled {
        chat,
        name: Some(name.to_owned()),
        group: group.map(str::to_owned),
    };
    let expected = [
        labeled("login fix", None),
        labeled("login fix", Some("auth")),
    ];
    assert_eq!(first_seen, expected);
    assert_eq!(second_seen, expected);
    assert!(command_seen.is_empty());
}
