use saturn_protocol::ids::{ChatId, SettingsRevision};
use saturn_protocol::rpc::ChatListItem;

use super::*;
use crate::store::NewInput;

pub(super) async fn chat_with_input(engine: &Engine, folder: &str, text: &str) -> ChatId {
    let chat = engine
        .store
        .create_chat(PathBuf::from(folder))
        .await
        .unwrap();
    engine
        .store
        .accept_input(&NewInput {
            chat,
            text: text.to_owned(),
            settings: SettingsRevision(1),
            permission: saturn_core::queue::Permission::Write,
            workdir: PathBuf::from(folder),
            pinned_model: None,
            skip_relation: false,
        })
        .await
        .unwrap();
    chat
}

async fn chat_list(client: &mut Client, id: u64, folder: Option<&str>) -> Vec<ChatListItem> {
    client
        .send(
            id,
            Request::ListChats {
                folder: folder.map(str::to_owned),
            },
        )
        .await;
    let Notification::ChatList { chats } = client.notification().await else {
        panic!("expected ChatList");
    };
    assert_eq!(client.response().await, Response::ok(RequestId(id)));
    chats
}

#[tokio::test]
async fn chat_list_answers_the_chats_of_a_folder_or_of_every_folder_without_attachment() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut reader = Client::connect(&fixture.socket()).await;
    let here = chat_with_input(&engine, "/work/a", "fix login\nsecond line").await;
    let there = chat_with_input(&engine, "/work/b", "write docs").await;

    let (in_folder, in_other, everywhere, nowhere) = drive(&mut engine, async {
        (
            chat_list(&mut reader, 1, Some("/work/a")).await,
            chat_list(&mut reader, 2, Some("/work/b")).await,
            chat_list(&mut reader, 3, None).await,
            chat_list(&mut reader, 4, Some("/nowhere")).await,
        )
    })
    .await;

    assert_eq!(in_folder.len(), 1);
    assert_eq!(in_folder[0].chat, here);
    assert_eq!(in_folder[0].folder, "/work/a");
    assert_eq!(
        in_folder[0].preview.as_deref(),
        Some("fix login\nsecond line")
    );
    assert_eq!(
        in_other.iter().map(|item| item.chat).collect::<Vec<_>>(),
        [there]
    );
    assert_eq!(everywhere.len(), 2);
    assert!(nowhere.is_empty());
}

#[tokio::test]
async fn chat_list_is_answered_while_waiting_for_the_router_key() {
    let fixture = Fixture::new();
    let mut engine = fixture.waiting_for_key(Vec::new()).await;
    let mut reader = Client::connect(&fixture.socket()).await;
    let chat = chat_with_input(&engine, "/work/a", "fix login").await;

    let chats = drive(&mut engine, chat_list(&mut reader, 1, None)).await;

    assert_eq!(
        chats.iter().map(|item| item.chat).collect::<Vec<_>>(),
        [chat]
    );
}
