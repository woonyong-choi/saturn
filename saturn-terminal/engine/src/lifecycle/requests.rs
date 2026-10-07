use saturn_protocol::envelope::{INVALID_PARAMS, METHOD_NOT_FOUND};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{QueryResult, UsageRange};

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
        let usage = client
            .query(
                3,
                Request::Usage {
                    scope: UsageRange::Chat,
                    folder: None,
                },
            )
            .await;
        (unattached, usage)
    })
    .await;

    assert_eq!(error_code(&unattached), INVALID_PARAMS);
    assert_eq!(
        usage,
        QueryResult::Usage {
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
        let usage = reader.query(3, request(&folder)).await;
        (other_folder, usage)
    })
    .await;

    assert_eq!(error_code(&other_folder), INVALID_PARAMS);
    assert_eq!(
        usage,
        QueryResult::Usage {
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
        let found = reader.query(2, ask(&folder)).await;
        let empty = reader.query(3, ask("/nowhere")).await;
        ((chat, found), empty)
    })
    .await;

    assert_eq!(
        found.1,
        QueryResult::LatestChat {
            chat: Some(found.0)
        }
    );
    assert_eq!(empty, QueryResult::LatestChat { chat: None });
}

fn set_recording(chat: u64) -> Request {
    Request::SetRecording {
        chat: ChatId(chat),
        on: false,
    }
}

/// `dir` 아래(하위 폴더 포함)에서 `needle`이 든 파일. 로그 폴더도 본다.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_containing(&path, needle));
        } else if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            if bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
            {
                found.push(path);
            }
        }
    }
    found
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

#[tokio::test]
async fn query_result_goes_only_to_the_connection_that_asked() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut asker = Client::connect(&fixture.socket()).await;
    let mut other = Client::connect(&fixture.socket()).await;

    let (usage, tasks, history, heard) = drive(&mut engine, async {
        let attached = asker.attach(1, new_chat(&fixture.workdir)).await;
        let Some(Notification::HistoryChunk { chat, .. }) = attached.get(1) else {
            panic!("expected HistoryChunk, got {attached:?}");
        };
        let chat = *chat;
        other
            .attach(
                2,
                Request::Attach {
                    chat: Some(chat),
                    workdir: fixture.workdir.display().to_string(),
                    env: Vec::new(),
                    overrides: Vec::new(),
                    add_dirs: Vec::new(),
                },
            )
            .await;
        asker.window().await;
        let usage = Request::Usage {
            scope: UsageRange::Chat,
            folder: None,
        };
        asker.send(3, usage).await;
        asker.send(4, Request::ListTasks).await;
        asker
            .send(
                5,
                Request::LoadHistory {
                    chat,
                    before: None,
                    limit: 10,
                },
            )
            .await;
        let usage = asker.response().await;
        let tasks = asker.response().await;
        let history = asker.response().await;
        (usage, tasks, history, other.window().await)
    })
    .await;

    assert!(matches!(
        usage.outcome,
        Outcome::Ok(Some(QueryResult::Usage { .. }))
    ));
    assert!(matches!(
        tasks.outcome,
        Outcome::Ok(Some(QueryResult::Tasks { .. }))
    ));
    assert!(matches!(
        history.outcome,
        Outcome::Ok(Some(QueryResult::History { .. }))
    ));
    assert!(heard.is_empty(), "other connection heard {heard:?}");
}

// docs/design/router-key-security.md: 키가 든 오류도 실제 engine 로그 파일에는 가린 채로 남는다
#[tokio::test]
async fn engine_log_file_never_holds_the_router_key() {
    let fixture = Fixture::new();
    let engine = fixture.ready().await;
    let log = crate::engine_log::EngineLog::start(&fixture.options.home).unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(log)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    engine.warn_failure(
        "provider call failed",
        Err::<(), _>(std::io::Error::other(format!("rejected key {KEY}"))),
    );
    drop(guard);

    let logs = fixture.options.home.join("logs");
    let written = std::fs::read_dir(&logs)
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(written.contains("provider call failed"), "{written}");
    assert!(written.contains("rejected key [redacted]"), "{written}");
    assert!(files_containing(&fixture.options.home, KEY).is_empty());
}
