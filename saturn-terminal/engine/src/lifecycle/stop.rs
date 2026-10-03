//! 멈춤 테스트: 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 완료를 알리고, 보류는 요청 없이는 이어 가지 않는다.

use std::ffi::OsString;

use saturn_core::providers::ProviderError;
use saturn_core::sessions::memo::INTERRUPTED_RESULT;
use saturn_protocol::ids::{SubagentId, TaskLabel};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState};

use super::support::{
    Flow, idle_reply, running_reply, subagent_ended, subagent_started, text, turn_completed,
};
use super::*;
use crate::processes::{ProcessSpec, StopOutcome};
use crate::providers::test_support::Call;
use crate::stop::StopDone;

fn notices(seen: &[Notification]) -> Vec<ChatNotice> {
    seen.iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .collect()
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

fn interrupts(flow: &Flow) -> Vec<Option<SubagentId>> {
    flow.fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Interrupt { target, .. } => Some(target),
            _ => None,
        })
        .collect()
}

fn is_stopped(notice: &ChatNotice) -> bool {
    matches!(notice, ChatNotice::Stopped { .. })
}

/// 1초 뒤 스스로 끝나는 자손을 가진 프로세스 묶음.
fn slow_group(flow: &Flow) -> crate::processes::ProcessGroupId {
    let spawned = flow
        .engine
        .supervisor
        .spawn(ProcessSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".to_owned(), "/bin/sleep 1; true".to_owned()],
            workdir: flow.fixture.workdir.clone(),
            env: vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
        })
        .expect("test process should start");
    spawned.group
}

#[tokio::test]
async fn stop_signals_the_deepest_subagent_first_and_finishes_only_when_the_tree_is_idle() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(subagent_started(agent, "sub-1", None))
        .await;
    flow.claude_event(subagent_started(agent, "sub-2", Some("sub-1")))
        .await;
    let mut client = flow.client().await;

    flow.engine.stop_chat(flow.chat).await.unwrap();

    assert_eq!(
        interrupts(&flow),
        vec![
            Some(SubagentId("sub-2".to_owned())),
            Some(SubagentId("sub-1".to_owned())),
            None
        ]
    );
    flow.claude_event(turn_completed(agent)).await;
    assert!(!notices(&client.window().await).iter().any(is_stopped));
    flow.claude_event(subagent_ended(agent, "sub-2")).await;
    assert!(!notices(&client.window().await).iter().any(is_stopped));
    flow.claude_event(subagent_ended(agent, "sub-1")).await;
    let seen = client.window().await;
    assert_eq!(
        notices(&seen),
        vec![ChatNotice::Stopped {
            held: vec![TaskLabel('A')]
        }]
    );
    assert!(
        flow.engine
            .store
            .unfinished_runs()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn stop_without_a_finished_turn_signal_is_not_complete() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let mut client = flow.client().await;

    flow.engine.stop_chat(flow.chat).await.unwrap();

    assert!(notices(&client.window().await).is_empty());
    assert!(flow.engine.flow.stopping.contains_key(&flow.chat));
}

#[tokio::test]
async fn stop_is_not_complete_until_the_process_group_is_confirmed_stopped() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.fake.set_group(slow_group(&flow));
    let mut client = flow.client().await;
    let chat = flow.chat;
    flow.engine.stop_chat(chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;

    let (early, later) = drive(&mut flow.engine, async {
        let early = notices(&client.window().await);
        let later = client
            .until(|notification| match notification {
                Notification::ChatNotice { notice, .. } => Some(notice.clone()),
                _ => None,
            })
            .await;
        (early, later)
    })
    .await;

    assert!(early.is_empty(), "completion came too early: {early:?}");
    assert!(is_stopped(&later));
}

#[tokio::test]
async fn processes_left_outside_the_group_are_reported_instead_of_done() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.engine
        .on_stop_done(StopDone {
            chat: flow.chat,
            result: Ok(StopOutcome::Unconfirmed { remaining: 2 }),
        })
        .await;

    flow.claude_event(turn_completed(agent)).await;

    assert_eq!(
        notices(&client.window().await),
        vec![ChatNotice::StopUnconfirmed { remaining: 2 }]
    );
}

#[tokio::test]
async fn stop_twice_signals_once() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;

    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.engine.stop_chat(flow.chat).await.unwrap();

    assert_eq!(interrupts(&flow).len(), 1);
}

#[tokio::test]
async fn stop_with_nothing_running_holds_the_waiting_input_at_once() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let waiting = flow.accept_only("not sent yet").await;
    let mut client = flow.client().await;

    flow.engine.stop_chat(flow.chat).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Held);
    assert_eq!(
        notices(&client.window().await),
        vec![ChatNotice::Stopped {
            held: vec![TaskLabel('A')]
        }]
    );
    assert!(flow.fake.calls().is_empty());
}

#[tokio::test]
async fn stopped_work_is_not_continued_without_a_request() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    let waiting = flow.submit("also run the tests").await;

    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    flow.engine.dispatch_next(flow.chat).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Held);
    assert_eq!(turns(&flow), vec!["fix the build"]);
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().state,
        SessionState::Held
    );
}

#[tokio::test]
async fn continue_sends_held_input_and_then_a_state_check_for_the_interrupted_task() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    let waiting = flow.submit("also run the tests").await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;

    flow.engine.continue_held(flow.chat, None).await.unwrap();

    assert_eq!(turns(&flow), vec!["fix the build", "also run the tests"]);
    assert_eq!(flow.state(waiting), InputState::Applied);
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().state,
        SessionState::Open
    );
    let opens = flow
        .fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opens, 1);
    flow.claude_event(text(agent, "tests pass")).await;
    flow.claude_event(turn_completed(agent)).await;
    let sent = turns(&flow);
    assert_eq!(sent.len(), 3);
    assert!(sent[2].starts_with(&format!(
        "Previous turn result (error): {INTERRUPTED_RESULT}\n"
    )));
    assert!(!sent[2].contains("Check the current state"));
    assert!(sent[2].ends_with("Request:\nfix the build"));
}

#[tokio::test]
async fn continue_input_resumes_the_task_of_that_input() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("also run the tests").await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;

    flow.engine.continue_input(waiting).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Applied);
    let refused = flow.engine.continue_input(waiting).await.unwrap_err();
    assert!(matches!(refused, EngineError::Queue(_)));
}

#[tokio::test]
async fn close_held_cancels_unsent_input_and_ends_the_session() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    let first = flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    let task = flow.record(first).task.unwrap();
    let waiting = flow.submit("also run the tests").await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;

    flow.engine.close_held(flow.chat, task).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Cancelled);
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().state,
        SessionState::Ended
    );
    assert!(
        flow.fake
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Close { .. }))
    );
    assert!(!flow.engine.flow.live.contains_key(&agent));
}

#[tokio::test]
async fn task_with_unknown_result_is_continued_only_when_named() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.answer_send([Err(ProviderError::Unknown)]);
    let input = flow.submit("fix the build").await;
    let task = flow.record(input).task.unwrap();
    assert!(flow.engine.flow.needs_check.contains_key(&task));

    flow.engine.continue_held(flow.chat, None).await.unwrap();
    assert_eq!(turns(&flow), vec!["fix the build"]);
    flow.engine
        .continue_held(flow.chat, Some(task))
        .await
        .unwrap();

    let sent = turns(&flow);
    assert_eq!(sent.len(), 2);
    assert!(sent[1].ends_with("Request:\nfix the build"));
    assert_eq!(flow.state(input), InputState::Delivering);
    assert!(!flow.engine.flow.needs_check.contains_key(&task));
}
