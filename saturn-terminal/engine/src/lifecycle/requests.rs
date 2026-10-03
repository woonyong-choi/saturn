use saturn_protocol::envelope::{INVALID_PARAMS, METHOD_NOT_FOUND};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::UsageRange;

use super::*;
use crate::{ROUTER_KEY_REQUIRED, RouterGate};

#[tokio::test]
async fn usage_request_answers_rows_for_attached_chat() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let (unattached, usage) = drive(&mut engine, async {
        client
            .send(
                1,
                Request::Usage {
                    scope: UsageRange::Chat,
                    folder: None,
                },
            )
            .await;
        let unattached = client.response().await;
        client.attach(2, new_chat(&fixture.workdir)).await;
        client
            .send(
                3,
                Request::Usage {
                    scope: UsageRange::Chat,
                    folder: None,
                },
            )
            .await;
        let usage = client.notification().await;
        assert_eq!(client.response().await, Response::ok(RequestId(3)));
        (unattached, usage)
    })
    .await;

    assert_eq!(error_code(&unattached), INVALID_PARAMS);
    assert_eq!(
        usage,
        Notification::Usage {
            range: UsageRange::Chat,
            rows: Vec::new(),
        }
    );
}

#[tokio::test]
async fn usage_request_without_attachment_reads_the_latest_chat_of_the_folder() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut owner = Client::connect(&fixture.socket()).await;
    let mut reader = Client::connect(&fixture.socket()).await;
    let folder = fixture.workdir.display().to_string();

    let (other_folder, usage) = drive(&mut engine, async {
        owner.attach(1, new_chat(&fixture.workdir)).await;
        let request = |folder: &str| Request::Usage {
            scope: UsageRange::Chat,
            folder: Some(folder.to_owned()),
        };
        reader.send(2, request("/nowhere")).await;
        let other_folder = reader.response().await;
        reader.send(3, request(&folder)).await;
        let usage = reader.notification().await;
        assert_eq!(reader.response().await, Response::ok(RequestId(3)));
        (other_folder, usage)
    })
    .await;

    assert_eq!(error_code(&other_folder), INVALID_PARAMS);
    assert_eq!(
        usage,
        Notification::Usage {
            range: UsageRange::Chat,
            rows: Vec::new(),
        }
    );
}

#[tokio::test]
async fn latest_chat_request_answers_the_latest_chat_of_the_folder_without_attachment() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut owner = Client::connect(&fixture.socket()).await;
    let mut reader = Client::connect(&fixture.socket()).await;
    let folder = fixture.workdir.display().to_string();

    let (found, empty) = drive(&mut engine, async {
        let attached = owner.attach(1, new_chat(&fixture.workdir)).await;
        let Some(Notification::HistoryChunk { chat, .. }) = attached.get(1) else {
            panic!("expected HistoryChunk, got {attached:?}");
        };
        let chat = *chat;
        let ask = |folder: &str| Request::LatestChat {
            folder: folder.to_owned(),
        };
        reader.send(2, ask(&folder)).await;
        let found = reader.notification().await;
        assert_eq!(reader.response().await, Response::ok(RequestId(2)));
        reader.send(3, ask("/nowhere")).await;
        let empty = reader.notification().await;
        assert_eq!(reader.response().await, Response::ok(RequestId(3)));
        ((chat, found), empty)
    })
    .await;

    assert_eq!(
        found.1,
        Notification::LatestChat {
            chat: Some(found.0)
        }
    );
    assert_eq!(empty, Notification::LatestChat { chat: None });
}

fn set_recording(chat: u64) -> Request {
    Request::SetRecording {
        chat: ChatId(chat),
        on: false,
    }
}

fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .filter(|path| {
            let bytes = std::fs::read(path).unwrap();
            bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        })
        .collect()
}

#[tokio::test]
async fn requests_wait_for_router_key_and_key_is_not_recorded() {
    let fixture = Fixture::new();
    let mut replies = vec![key_rejected()];
    replies.extend(check_passes());
    let mut engine = fixture.waiting_for_key(replies).await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    drive(&mut engine, async {
        client.send(1, set_recording(chat.0)).await;
        let refused = client.response().await;
        assert_eq!(refused.id, Some(RequestId(1)));
        assert_eq!(error_code(&refused), ROUTER_KEY_REQUIRED);

        client
            .send(
                2,
                Request::SubmitRouterKey {
                    key: "sk-wrong-0000".to_owned(),
                },
            )
            .await;
        assert!(matches!(
            client.notification().await,
            Notification::RouterKeyRequired { .. }
        ));
        let wrong = client.response().await;
        assert_eq!(wrong.id, Some(RequestId(2)));
        assert!(!format!("{wrong:?}").contains("sk-wrong-0000"));

        client
            .send(
                3,
                Request::SubmitRouterKey {
                    key: KEY.to_owned(),
                },
            )
            .await;
        assert_eq!(client.response().await, Response::ok(RequestId(3)));
        client.send(4, set_recording(chat.0)).await;
        assert_eq!(client.response().await, Response::ok(RequestId(4)));
    })
    .await;

    assert_eq!(engine.router_gate, RouterGate::Open);
    assert_eq!(std::fs::read_to_string(fixture.key_file()).unwrap(), KEY);
    assert!(files_containing(&fixture.options.home, KEY).is_empty());
    assert!(files_containing(&fixture.options.home, "sk-wrong-0000").is_empty());
}

#[tokio::test]
async fn requests_each_get_one_response_in_order() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let export = fixture.root.path().join("judgments.jsonl");
    let mut client = Client::connect(&fixture.socket()).await;

    let responses = drive(&mut engine, async {
        client.send(1, set_recording(chat.0)).await;
        client.send(2, set_recording(999)).await;
        client.send(3, Request::ListRouterVersions).await;
        client
            .send(
                4,
                Request::ExportJudgments {
                    path: export.display().to_string(),
                },
            )
            .await;
        client
            .send(
                5,
                Request::SubmitRouterKey {
                    key: KEY.to_owned(),
                },
            )
            .await;
        let mut responses = Vec::new();
        for _ in 0..5 {
            responses.push(client.response().await);
        }
        responses
    })
    .await;

    let ids: Vec<Option<RequestId>> = responses.iter().map(|response| response.id).collect();
    assert_eq!(
        ids,
        (1..=5).map(|id| Some(RequestId(id))).collect::<Vec<_>>()
    );
    assert_eq!(responses[0], Response::ok(RequestId(1)));
    assert_eq!(error_code(&responses[1]), INVALID_PARAMS);
    assert_eq!(error_code(&responses[2]), METHOD_NOT_FOUND);
    assert_eq!(responses[3], Response::ok(RequestId(4)));
    assert!(export.exists());
    assert_eq!(error_code(&responses[4]), INVALID_PARAMS);
}
