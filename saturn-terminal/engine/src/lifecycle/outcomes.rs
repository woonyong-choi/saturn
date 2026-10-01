use std::time::Instant;

use saturn_core::judges::calibration::{AskedAnswer, Signal};
use saturn_protocol::ids::{ChatId, JudgmentId};

use super::*;
use crate::outcomes::{OBSERVE_INPUTS, OBSERVE_WINDOW};
use crate::store::{StoreError, test_judgment};

async fn recorded_judgment(
    engine: &Engine,
    workdir: &Path,
    asked_with: Option<f64>,
) -> (ChatId, JudgmentId) {
    let chat = engine
        .store
        .create_chat(workdir.to_path_buf())
        .await
        .unwrap();
    let mut judgment = test_judgment(chat);
    judgment.asked_with = asked_with;
    let id = engine
        .store
        .record_judgment(&judgment)
        .await
        .unwrap()
        .unwrap();
    (chat, id)
}

async fn recorded_signals(engine: &Engine) -> Vec<Signal> {
    engine
        .store
        .observations()
        .await
        .unwrap()
        .into_iter()
        .map(|observation| observation.signal)
        .collect()
}

#[tokio::test]
async fn settle_after_three_inputs_records_confirmed_signal() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (chat, judgment) = recorded_judgment(&engine, &fixture.workdir, None).await;
    let start = Instant::now();
    engine.watch_judgment(chat, judgment, start);
    engine.note_reaction(judgment, Signal::Wrong);

    for _ in 0..OBSERVE_INPUTS {
        engine.note_input_accepted(chat, start).await.unwrap();
    }

    assert_eq!(recorded_signals(&engine).await, vec![Signal::Wrong]);
    assert!(!engine.signals.is_watching(judgment));
}

#[tokio::test]
async fn settle_before_observation_ends_leaves_signal_empty() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (chat, judgment) = recorded_judgment(&engine, &fixture.workdir, None).await;
    let start = Instant::now();
    engine.watch_judgment(chat, judgment, start);
    engine.note_reaction(judgment, Signal::Missed);
    for _ in 0..OBSERVE_INPUTS - 1 {
        engine.note_input_accepted(chat, start).await.unwrap();
    }

    engine
        .settle_signals(start + OBSERVE_WINDOW - Duration::from_secs(1))
        .await
        .unwrap();

    assert!(recorded_signals(&engine).await.is_empty());
    assert!(engine.signals.is_watching(judgment));
}

#[tokio::test]
async fn settle_after_ten_minutes_without_reaction_records_unconfirmed() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (chat, judgment) = recorded_judgment(&engine, &fixture.workdir, None).await;
    let start = Instant::now();
    engine.watch_judgment(chat, judgment, start);

    engine.settle_signals(start + OBSERVE_WINDOW).await.unwrap();

    assert_eq!(recorded_signals(&engine).await, vec![Signal::Unconfirmed]);
}

#[tokio::test]
async fn settle_forgets_judgment_deleted_during_observation() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (chat, _) = recorded_judgment(&engine, &fixture.workdir, None).await;
    let start = Instant::now();
    engine.watch_judgment(chat, JudgmentId(999), start);

    engine.settle_signals(start + OBSERVE_WINDOW).await.unwrap();

    assert!(!engine.signals.is_watching(JudgmentId(999)));
}

#[tokio::test]
async fn answer_feedback_records_answer_in_judgment() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (_, judgment) = recorded_judgment(&engine, &fixture.workdir, Some(0.2)).await;
    engine
        .store
        .record_signal(judgment, Signal::Unconfirmed)
        .await
        .unwrap();

    engine.answer_feedback(judgment, false).await.unwrap();

    let observations = engine.store.observations().await.unwrap();
    assert_eq!(observations[0].asked_answer, Some(AskedAnswer::Wrong));
}

#[tokio::test]
async fn answer_feedback_for_unasked_judgment_returns_not_found() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (_, judgment) = recorded_judgment(&engine, &fixture.workdir, None).await;

    let error = engine.answer_feedback(judgment, true).await.unwrap_err();

    assert!(matches!(
        error,
        EngineError::Store(StoreError::NotFound { .. })
    ));
}

#[tokio::test]
async fn answer_feedback_request_is_answered_through_socket() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let (_, judgment) = recorded_judgment(&engine, &fixture.workdir, Some(0.2)).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let response = drive(&mut engine, async {
        client
            .send(
                1,
                Request::AnswerFeedback {
                    judgment,
                    correct: true,
                },
            )
            .await;
        client.response().await
    })
    .await;

    assert_eq!(response, Response::ok(RequestId(1)));
}
