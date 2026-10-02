//! 판단 적용 테스트: 적용 직전 채팅 revision을 비교하고, 어긋나면 한 번만 다시 판단한다.

use saturn_core::judges::RouteDecision;
use saturn_protocol::ids::{ChatRevision, InputId};
use saturn_protocol::state::{Disposition, InputState};

use super::support::{CLIENT, Flow, idle_reply, judge_down, running_reply};
use crate::providers::test_support::Call;

fn decision(flow: &Flow, revision: ChatRevision, disposition: Disposition) -> RouteDecision {
    RouteDecision {
        revision,
        settings: flow.engine.settings.current().unwrap(),
        disposition,
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: Vec::new(),
    }
}

/// 다른 입력의 판단 적용으로 채팅 revision을 올린다.
async fn bump_revision(flow: &mut Flow, other: InputId) {
    let revision = flow.engine.queue.revision(flow.chat);
    let decision = decision(flow, revision, Disposition::Queue);
    flow.engine
        .apply_decision(other, decision, false)
        .await
        .unwrap();
}

/// 지금까지의 판단 기록을 JSONL로 내보낸 내용.
async fn exported(flow: &Flow) -> String {
    let path = flow.fixture.root.path().join("judgments.jsonl");
    flow.engine.store.export_judgments(&path).await.unwrap();
    std::fs::read_to_string(path).unwrap()
}

#[tokio::test]
async fn revision_conflict_supersedes_old_judgment_and_rejudges_once() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let first = flow.accept_only("first").await;
    let second = flow.accept_only("second").await;
    let stale = flow.judge_now(first, false).await;
    bump_revision(&mut flow, second).await;

    flow.engine
        .apply_decision(first, stale, false)
        .await
        .unwrap();
    flow.settle().await;

    assert_eq!(flow.judge_calls(), 2);
    assert_eq!(flow.state(first), InputState::Applied);
    assert_eq!(
        flow.engine.queue.disposition(first),
        Some(Disposition::Queue)
    );
    let lines = exported(&flow).await;
    let outcomes: Vec<&str> = lines
        .lines()
        .map(|line| {
            if line.contains("Superseded") {
                "Superseded"
            } else {
                "Ok"
            }
        })
        .collect();
    assert_eq!(outcomes, vec!["Superseded", "Ok"]);
}

#[tokio::test]
async fn second_conflict_puts_input_in_queue_without_another_judge_call() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let first = flow.accept_only("first").await;
    let second = flow.accept_only("second").await;
    let stale = flow.judge_now(first, false).await;
    bump_revision(&mut flow, second).await;

    flow.engine
        .apply_decision(first, stale, true)
        .await
        .unwrap();

    assert_eq!(flow.judge_calls(), 1);
    assert_eq!(flow.state(first), InputState::Queued);
    assert_eq!(flow.engine.queue.disposition(first), None);
    assert!(exported(&flow).await.contains("Superseded"));
}

#[tokio::test]
async fn matching_revision_applies_without_rejudging() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let input = flow.accept_only("only").await;
    let fresh = flow.judge_now(input, false).await;

    flow.engine
        .apply_decision(input, fresh, false)
        .await
        .unwrap();

    assert_eq!(flow.judge_calls(), 1);
    assert_eq!(flow.state(input), InputState::Queued);
}

#[tokio::test]
async fn pinned_model_input_waits_without_judge_while_task_runs() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("first request").await;

    let pinned = flow.submit_with("use that model", Some("m"), false).await;

    assert_eq!(flow.judge_calls(), 1);
    assert_eq!(flow.state(pinned), InputState::Queued);
    assert_eq!(
        flow.engine.queue.disposition(pinned),
        Some(Disposition::Queue)
    );
}

#[tokio::test]
async fn skip_relation_input_waits_without_judge_while_task_runs() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("first request").await;

    let waiting = flow.submit_with("later", None, true).await;

    assert_eq!(flow.judge_calls(), 1);
    assert_eq!(flow.state(waiting), InputState::Queued);
}

#[tokio::test]
async fn judge_failure_while_running_steers_the_current_agent() {
    let mut flow = Flow::new(
        std::iter::once(idle_reply(0.95))
            .chain(judge_down())
            .collect(),
    )
    .await;
    flow.fake.verify_steer();
    flow.submit("first request").await;

    let second = flow.submit("also this").await;

    assert_eq!(flow.state(second), InputState::Applied);
    assert!(matches!(
        flow.fake.calls().last(),
        Some(Call::Steer { text, .. }) if text == "also this"
    ));
    assert!(exported(&flow).await.contains("NoResponse"));
}

#[tokio::test]
async fn judge_failure_while_idle_sends_to_the_current_agent_not_the_queue() {
    let mut flow = Flow::new(judge_down()).await;

    let input = flow.submit("first request").await;

    assert_eq!(flow.state(input), InputState::Applied);
    assert!(matches!(
        flow.fake.calls().last(),
        Some(Call::SendTurn { text, .. }) if text == "first request"
    ));
}

#[tokio::test]
async fn stop_while_judging_holds_the_input_and_drops_the_late_judgment() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let release = flow.transport.hold_next_call();
    flow.engine
        .submit_input(
            CLIENT,
            flow.chat,
            1,
            "fix the build".to_owned(),
            None,
            false,
        )
        .await
        .unwrap();
    let (input, _) = flow.engine.queue.next_to_judge(flow.chat).unwrap();
    assert!(flow.is_judging());
    let before = flow.engine.queue.revision(flow.chat);

    flow.engine.queue.stop(flow.chat);
    assert_eq!(flow.state(input), InputState::Held);
    release.notify_one();
    flow.settle().await;

    assert_eq!(flow.state(input), InputState::Held);
    assert_ne!(flow.engine.queue.revision(flow.chat), before);
    assert!(flow.fake.calls().is_empty());
    assert!(exported(&flow).await.contains("Superseded"));
}

#[tokio::test]
async fn relation_answer_to_new_task_waits_for_the_write_turn_then_starts() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.1, "independent", "spawn"),
    ])
    .await;
    flow.submit("first request").await;
    let agent = flow.agent();

    let second = flow.submit("something else").await;
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(
        flow.record(second).reason,
        Some(saturn_protocol::state::QueueReason::WriteTurn)
    );
    flow.engine.finish_task(flow.chat, agent).await.unwrap();

    assert_eq!(flow.state(second), InputState::Applied);
    let opens = flow
        .fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opens, 2);
}
