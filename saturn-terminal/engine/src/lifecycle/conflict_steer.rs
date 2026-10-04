//! 충돌 입력 테스트: 하던 작업과 반대되는 입력은 끼워 넣고, provider가 받지 않으면 사용자에게 멈출지 묻는다.
//! 설계: docs/design/input-handling.md#충돌-입력

use saturn_core::providers::ProviderError;
use saturn_core::queue::QueueError;
use saturn_protocol::state::{InputState, QueueReason};

use super::support::{CLIENT, Flow, idle_reply, running_reply, turn_completed};
use crate::EngineError;
use crate::providers::test_support::Call;

const CONFLICT: &str = "stop that and use pytest instead";

fn not_sent() -> Result<(), ProviderError> {
    Err(ProviderError::NotSent {
        reason: "steer refused".to_owned(),
    })
}

fn calls_of(flow: &Flow, pick: fn(Call) -> Option<String>) -> Vec<String> {
    flow.fake.calls().into_iter().filter_map(pick).collect()
}

fn turns(flow: &Flow) -> Vec<String> {
    calls_of(flow, |call| match call {
        Call::SendTurn { text, .. } => Some(text),
        _ => None,
    })
}

fn steers(flow: &Flow) -> Vec<String> {
    calls_of(flow, |call| match call {
        Call::Steer { text, .. } => Some(text),
        _ => None,
    })
}

fn interrupts(flow: &Flow) -> usize {
    flow.fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Interrupt { .. }))
        .count()
}

/// 작업이 실행 중일 때 충돌로 판단된 입력이 끼워 넣기를 거절당한 상태.
async fn refused_conflict(flow: &mut Flow) -> saturn_protocol::ids::InputId {
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    flow.fake.answer_steer([not_sent()]);
    flow.submit(CONFLICT).await
}

#[tokio::test]
async fn conflict_input_is_steered_into_the_running_turn() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;

    let second = flow.submit(CONFLICT).await;

    assert_eq!(steers(&flow), vec![CONFLICT]);
    assert_eq!(turns(&flow), vec!["fix the build"]);
    assert_eq!(flow.state(second), InputState::Applied);
    assert_eq!(interrupts(&flow), 0);
}

#[tokio::test]
async fn refused_conflict_steer_asks_whether_to_stop_and_does_not_stop() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
    ])
    .await;

    let second = refused_conflict(&mut flow).await;

    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(flow.record(second).reason, Some(QueueReason::ConfirmStop));
    assert!(flow.engine.queue.awaits_stop(second));
    assert_eq!(interrupts(&flow), 0);
}

#[tokio::test]
async fn conflict_steer_to_a_provider_without_steer_asks_whether_to_stop() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;

    let second = flow.submit(CONFLICT).await;

    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(flow.record(second).reason, Some(QueueReason::ConfirmStop));
    assert!(steers(&flow).is_empty());
}

#[tokio::test]
async fn answering_wait_keeps_the_input_in_front_for_the_next_turn() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
    ])
    .await;
    let second = refused_conflict(&mut flow).await;
    let agent = flow.agent();

    flow.engine
        .answer_stop_confirm(CLIENT, second, false)
        .await
        .unwrap();

    flow.settle().await;

    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(flow.record(second).reason, None);
    assert_eq!(interrupts(&flow), 0);
    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    flow.settle().await;
    assert_eq!(turns(&flow), vec!["fix the build", CONFLICT]);
    assert_eq!(flow.state(second), InputState::Applied);
}

#[tokio::test]
async fn answering_stop_stops_the_chat_and_then_runs_the_input() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
    ])
    .await;
    let second = refused_conflict(&mut flow).await;
    let agent = flow.agent();

    flow.engine
        .answer_stop_confirm(CLIENT, second, true)
        .await
        .unwrap();

    flow.settle().await;

    assert_eq!(interrupts(&flow), 1);
    assert_eq!(flow.state(second), InputState::Held);
    assert_eq!(turns(&flow), vec!["fix the build"]);
    flow.claude_event(turn_completed(agent)).await;
    assert_eq!(flow.state(second), InputState::Applied);
    assert!(turns(&flow).contains(&CONFLICT.to_owned()));
}

#[tokio::test]
async fn answering_without_a_question_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let first = flow.submit("fix the build").await;

    let answered = flow.engine.answer_stop_confirm(CLIENT, first, true).await;

    assert!(matches!(
        answered,
        Err(EngineError::Queue(QueueError::NotAwaitingStop(_)))
    ));
    assert_eq!(interrupts(&flow), 0);
}

// #456
#[tokio::test]
async fn applied_steer_is_kept_with_its_run_and_a_refused_one_is_not() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "conflicts", "queue"),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    let applied = flow.submit(CONFLICT).await;
    flow.fake.answer_steer([not_sent()]);
    let refused = flow.submit("also bump the version").await;

    let steered = flow.engine.store.steered_inputs(flow.chat).await.unwrap();

    assert_eq!(flow.state(applied), InputState::Applied);
    assert_eq!(flow.state(refused), InputState::Queued);
    assert_eq!(
        steered.iter().map(|steer| steer.input).collect::<Vec<_>>(),
        vec![applied]
    );
}
