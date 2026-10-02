//! 바로 보내기 테스트: judge를 부르지 않고 끼워 넣기를 시도하고, 안 되면 대기열 맨 앞에서 다음 차례를 기다린다.

use saturn_protocol::state::{Disposition, InputState};

use super::support::{CLIENT, Flow, idle_reply, judge_down, running_reply};
use super::*;
use crate::providers::test_support::Call;

fn turns(flow: &Flow) -> Vec<String> {
    flow.fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SendTurn { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn steers(flow: &Flow) -> Vec<String> {
    flow.fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Steer { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn send_now_steers_into_the_running_turn_without_calling_the_judge() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    let waiting = flow.submit("also run the tests").await;
    assert_eq!(flow.state(waiting), InputState::Queued);
    let before = flow.judge_calls();

    flow.engine.send_now(CLIENT, waiting).await.unwrap();
    flow.settle().await;

    assert_eq!(flow.judge_calls(), before);
    assert_eq!(steers(&flow), vec!["also run the tests"]);
    assert_eq!(flow.state(waiting), InputState::Applied);
}

#[tokio::test]
async fn send_now_does_not_need_the_judge_to_be_up() {
    let mut flow = Flow::new(
        [idle_reply(0.95), running_reply(0.95, "continues", "queue")]
            .into_iter()
            .chain(judge_down())
            .collect(),
    )
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    let waiting = flow.submit("also run the tests").await;

    flow.engine.send_now(CLIENT, waiting).await.unwrap();
    flow.settle().await;

    assert_eq!(steers(&flow), vec!["also run the tests"]);
    assert_eq!(flow.state(waiting), InputState::Applied);
}

#[tokio::test]
async fn send_now_that_cannot_steer_goes_first_in_the_queue() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let earlier = flow.submit("write the docs").await;
    let later = flow.submit("also run the tests").await;
    assert_eq!(
        flow.engine
            .queue
            .inputs_in_state(flow.chat, InputState::Queued),
        vec![earlier, later]
    );

    flow.engine.send_now(CLIENT, later).await.unwrap();
    flow.settle().await;

    assert!(steers(&flow).is_empty());
    assert_eq!(flow.state(later), InputState::Queued);
    assert_eq!(
        flow.engine
            .queue
            .inputs_in_state(flow.chat, InputState::Queued),
        vec![later, earlier]
    );
    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    assert_eq!(
        turns(&flow),
        vec!["fix the build", "also run the tests"],
        "the sent-now input takes the next turn"
    );
    assert_eq!(flow.state(later), InputState::Applied);
    assert_eq!(flow.state(earlier), InputState::Queued);
}

#[tokio::test]
async fn send_now_without_verified_steer_goes_back_to_waiting() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let waiting = flow.submit("also run the tests").await;

    flow.engine.send_now(CLIENT, waiting).await.unwrap();

    assert_eq!(
        flow.engine.queue.disposition(waiting),
        Some(Disposition::Queue),
        "without verified steer the input goes back to waiting, in front"
    );
}

#[tokio::test]
async fn send_now_on_a_sent_input_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let sent = flow.submit("fix the build").await;

    let error = flow.engine.send_now(CLIENT, sent).await.unwrap_err();

    assert!(matches!(error, EngineError::Queue(_)));
    assert_eq!(flow.judge_calls(), 1);
}
