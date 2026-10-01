use saturn_protocol::envelope::{INVALID_PARAMS, METHOD_NOT_FOUND};
use saturn_protocol::ids::ChatId;

use super::*;
use crate::{JUDGE_KEY_REQUIRED, JudgeGate};

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
async fn requests_wait_for_judge_key_and_key_is_not_recorded() {
    let fixture = Fixture::new();
    let mut replies = vec![key_rejected()];
    replies.extend(check_passes());
    let mut engine = fixture.waiting_for_key(replies).await;
    let chat = engine
        .store
        .create_chat(fixture.options.workdir.clone())
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    drive(&mut engine, async {
        client.send(1, set_recording(chat.0)).await;
        let refused = client.response().await;
        assert_eq!(refused.id, Some(RequestId(1)));
        assert_eq!(error_code(&refused), JUDGE_KEY_REQUIRED);

        client
            .send(
                2,
                Request::SubmitJudgeKey {
                    key: "sk-wrong-0000".to_owned(),
                },
            )
            .await;
        assert!(matches!(
            client.notification().await,
            Notification::JudgeKeyRequired { .. }
        ));
        let wrong = client.response().await;
        assert_eq!(wrong.id, Some(RequestId(2)));
        assert!(!format!("{wrong:?}").contains("sk-wrong-0000"));

        client
            .send(
                3,
                Request::SubmitJudgeKey {
                    key: KEY.to_owned(),
                },
            )
            .await;
        assert_eq!(client.response().await, Response::ok(RequestId(3)));
        client.send(4, set_recording(chat.0)).await;
        assert_eq!(client.response().await, Response::ok(RequestId(4)));
    })
    .await;

    assert_eq!(engine.judge_gate, JudgeGate::Open);
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
        .create_chat(fixture.options.workdir.clone())
        .await
        .unwrap();
    let export = fixture.root.path().join("judgments.jsonl");
    let mut client = Client::connect(&fixture.socket()).await;

    let responses = drive(&mut engine, async {
        client.send(1, set_recording(chat.0)).await;
        client.send(2, set_recording(999)).await;
        client
            .send(
                3,
                Request::RenameChat {
                    chat,
                    name: "build".to_owned(),
                },
            )
            .await;
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
                Request::SubmitJudgeKey {
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
