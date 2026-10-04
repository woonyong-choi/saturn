use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{PruneSkipReason, PruneSkipped};
use saturn_protocol::state::InputState;

use super::chats::chat_with_input;
use super::*;

/// 사용자 설정에 정리 기준을 둔 engine.
async fn ready_with_retention(fixture: &Fixture) -> Engine {
    fixture.write_user_config("retention.max_age_days = 1\n");
    fixture.ready().await
}

/// 3일 전에 쓰고 끝낸 채팅.
async fn old_chat(engine: &Engine, folder: &str, text: &str) -> ChatId {
    let chat = chat_with_input(engine, folder, text).await;
    finish_inputs(engine, chat).await;
    engine.store.age_chat(chat, 3).await;
    chat
}

async fn finish_inputs(engine: &Engine, chat: ChatId) {
    for (input, new, _) in engine.store.open_inputs().await.unwrap() {
        if new.chat == chat {
            engine
                .store
                .set_input_state(input, InputState::Applied, None)
                .await
                .unwrap();
        }
    }
}

struct Report {
    chats: Vec<ChatId>,
    skipped: Vec<PruneSkipped>,
    rows: u64,
    is_preview: bool,
}

async fn prune(engine: &mut Engine, client: &mut Client, id: u64, yes: bool) -> Report {
    drive(engine, async {
        client.send(id, Request::Prune { yes }).await;
        let mut report = None;
        loop {
            match client.recv().await {
                ServerMessage::Notification(message) => match message.notification {
                    Notification::PrunePreview {
                        chats,
                        skipped,
                        rows,
                    } => report = Some((chats, skipped, rows, true)),
                    Notification::Pruned {
                        chats,
                        skipped,
                        rows,
                    } => report = Some((chats, skipped, rows, false)),
                    _ => {}
                },
                ServerMessage::Response(response) => {
                    assert_eq!(response, Response::ok(RequestId(id)));
                    let (chats, skipped, rows, is_preview) =
                        report.expect("a report should arrive before the response");
                    return Report {
                        chats: chats.into_iter().map(|item| item.chat).collect(),
                        skipped,
                        rows,
                        is_preview,
                    };
                }
            }
        }
    })
    .await
}

#[tokio::test]
async fn prune_without_yes_previews_old_chats_and_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let recent = chat_with_input(&engine, "/work/recent", "new work").await;
    let open = chat_with_input(&engine, "/work/open", "still going").await;
    engine.store.age_chat(open, 3).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let report = prune(&mut engine, &mut client, 1, false).await;

    assert!(report.is_preview);
    assert_eq!(report.chats, [old]);
    assert_eq!(
        report.skipped,
        [PruneSkipped {
            chat: open,
            reasons: vec![PruneSkipReason::OpenInput]
        }]
    );
    assert!(report.rows > 0);
    for chat in [old, recent, open] {
        assert!(engine.store.chat_workdir(chat).await.is_ok(), "{chat:?}");
    }
    assert!(engine.store.tombstone(old).await.unwrap().is_none());
}

#[tokio::test]
async fn prune_with_yes_deletes_only_the_old_finished_chats_and_reports_them() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let recent = chat_with_input(&engine, "/work/recent", "new work").await;
    let open = chat_with_input(&engine, "/work/open", "still going").await;
    engine.store.age_chat(open, 3).await;
    let mut client = Client::connect(&fixture.socket()).await;
    let preview = prune(&mut engine, &mut client, 1, false).await;

    let report = prune(&mut engine, &mut client, 2, true).await;

    assert!(!report.is_preview);
    assert_eq!(report.chats, [old]);
    assert_eq!(report.rows, preview.rows);
    assert_eq!(
        report
            .skipped
            .iter()
            .map(|item| item.chat)
            .collect::<Vec<_>>(),
        [open]
    );
    assert!(engine.store.chat_workdir(old).await.is_err());
    assert!(engine.store.tombstone(old).await.unwrap().is_some());
    assert!(engine.store.chat_workdir(recent).await.is_ok());
    assert!(engine.store.chat_workdir(open).await.is_ok());
}

#[tokio::test]
async fn prune_keeps_a_chat_a_tui_is_attached_to() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut viewer = Client::connect(&fixture.socket()).await;
    let mut client = Client::connect(&fixture.socket()).await;
    drive(&mut engine, async {
        viewer
            .attach(
                1,
                Request::Attach {
                    chat: Some(old),
                    workdir: fixture.workdir.display().to_string(),
                    env: Vec::new(),
                    overrides: Vec::new(),
                    add_dirs: Vec::new(),
                },
            )
            .await;
    })
    .await;
    engine.store.age_chat(old, 3).await;

    let preview = prune(&mut engine, &mut client, 1, false).await;
    let deleted = prune(&mut engine, &mut client, 2, true).await;

    assert!(preview.chats.is_empty() && deleted.chats.is_empty());
    for report in [&preview, &deleted] {
        assert_eq!(
            report.skipped,
            [PruneSkipped {
                chat: old,
                reasons: vec![PruneSkipReason::Attached]
            }]
        );
    }
    assert!(engine.store.chat_workdir(old).await.is_ok());
}

#[tokio::test]
async fn prune_without_a_retention_setting_is_refused_and_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut client = Client::connect(&fixture.socket()).await;

    let refused = drive(&mut engine, async {
        client.send(1, Request::Prune { yes: true }).await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&refused), INVALID_PARAMS);
    assert!(engine.store.chat_workdir(old).await.is_ok());
}
