//! 끼워 넣기 거절 테스트: provider가 거절(`NotSent`)한 입력은 다시 끼워 넣지 않고 대기열 맨 앞에서 다음 차례에 간다.

use saturn_core::providers::ProviderError;
use saturn_protocol::state::{Disposition, InputState};

use super::support::{CLIENT, Flow, idle_reply, running_reply};
use crate::providers::test_support::Call;

fn not_sent() -> Result<(), ProviderError> {
    Err(ProviderError::NotSent {
        reason: "steer refused".to_owned(),
    })
}

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
async fn refused_steer_is_not_sent_again_and_is_not_rejected() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    flow.fake.answer_steer([not_sent()]);

    let second = flow.submit("also run the tests").await;

    assert_eq!(steers(&flow), vec!["also run the tests"]);
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(
        flow.engine.queue.disposition(second),
        Some(Disposition::Queue)
    );
    let stored = flow.engine.store.open_inputs().await.unwrap();
    assert!(
        stored
            .iter()
            .any(|(id, _, state)| *id == second && *state == InputState::Queued)
    );
}

#[tokio::test]
async fn refused_steer_goes_to_the_front_and_takes_the_next_turn() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("write the docs").await;
    flow.fake.answer_steer([not_sent()]);

    let refused = flow.submit("also run the tests").await;

    assert_eq!(
        flow.engine
            .queue
            .inputs_in_state(flow.chat, InputState::Queued),
        vec![refused, waiting]
    );
    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    assert_eq!(turns(&flow), vec!["fix the build", "also run the tests"]);
    assert_eq!(flow.state(refused), InputState::Applied);
    assert_eq!(flow.state(waiting), InputState::Queued);
}

#[tokio::test]
async fn refused_steer_through_send_now_also_goes_to_the_front() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let earlier = flow.submit("write the docs").await;
    let later = flow.submit("also run the tests").await;
    flow.fake.answer_steer([not_sent()]);

    flow.engine.send_now(CLIENT, later).await.unwrap();
    flow.settle().await;

    assert_eq!(steers(&flow), vec!["also run the tests"]);
    assert_eq!(flow.state(later), InputState::Queued);
    assert_eq!(
        flow.engine
            .queue
            .inputs_in_state(flow.chat, InputState::Queued),
        vec![later, earlier]
    );
    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    assert_eq!(turns(&flow), vec!["fix the build", "also run the tests"]);
}
