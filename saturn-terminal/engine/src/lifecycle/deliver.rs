//! 전송 테스트: `NotSent`만 다시 보내고 `Unknown`은 `NeedsCheck`로 두며, 끼워 넣기와 취소를 다룬다.

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::{Disposition, InputState, TaskState};

use super::support::{CLIENT, Flow, idle_reply, running_reply};
use super::*;
use crate::dispatch::MAX_SEND_ATTEMPTS;
use crate::providers::ProviderConnection;
use crate::providers::test_support::{Call, FakeProvider};

fn not_sent() -> Result<(), ProviderError> {
    Err(ProviderError::NotSent {
        reason: "not sent".to_owned(),
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
async fn not_sent_is_sent_again_and_then_applied() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.answer_send([not_sent(), Ok(())]);

    let input = flow.submit("fix the build").await;

    assert_eq!(turns(&flow), vec!["fix the build", "fix the build"]);
    assert_eq!(flow.state(input), InputState::Applied);
}

#[tokio::test]
async fn not_sent_every_time_is_rejected_and_the_next_input_still_goes() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let attempts = usize::try_from(MAX_SEND_ATTEMPTS).unwrap();
    flow.fake.answer_send((0..attempts).map(|_| not_sent()));

    let first = flow.submit("first request").await;
    let second = flow.submit("second request").await;

    assert_eq!(flow.state(first), InputState::Rejected);
    assert_eq!(turns(&flow).len(), attempts + 1);
    assert_eq!(flow.state(second), InputState::Applied);
    let opens = flow
        .fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opens, 1);
    let runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].input, Some(second));
}

#[tokio::test]
async fn unknown_is_never_sent_again_and_the_task_needs_check() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.answer_send([Err(ProviderError::Unknown)]);

    let input = flow.submit("fix the build").await;

    assert_eq!(turns(&flow), vec!["fix the build"]);
    assert_eq!(flow.state(input), InputState::Delivering);
    let runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].input, Some(input));
}

#[tokio::test]
async fn open_failure_that_is_not_a_resend_case_rejects_without_sending() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.answer_open([Err(ProviderError::ConnectionLost)]);

    let input = flow.submit("fix the build").await;

    assert_eq!(flow.state(input), InputState::Rejected);
    assert!(turns(&flow).is_empty());
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
async fn open_not_sent_is_tried_again() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.answer_open([not_sent()]);

    let input = flow.submit("fix the build").await;

    assert_eq!(flow.state(input), InputState::Applied);
    let opens = flow
        .fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opens, 2);
}

#[tokio::test]
async fn steer_without_active_turn_sends_one_new_turn_without_rerouting() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    flow.fake.answer_steer([Err(ProviderError::NoActiveTurn)]);

    let second = flow.submit("also run the tests").await;

    assert_eq!(steers(&flow), vec!["also run the tests"]);
    assert_eq!(turns(&flow), vec!["fix the build", "also run the tests"]);
    assert_eq!(flow.router_calls(), 2);
    assert_eq!(flow.state(second), InputState::Applied);
    let runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].input, Some(second));
}

#[tokio::test]
async fn steer_new_turn_unknown_is_not_sent_again() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("fix the build").await;
    flow.fake.answer_steer([Err(ProviderError::NoActiveTurn)]);
    flow.fake.answer_send([Err(ProviderError::Unknown)]);

    let second = flow.submit("also run the tests").await;

    assert_eq!(turns(&flow).len(), 2);
    assert_eq!(steers(&flow).len(), 1);
    assert_eq!(flow.state(second), InputState::Delivering);
}

#[tokio::test]
async fn unverified_steer_waits_and_goes_as_a_new_turn_when_the_task_ends() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    let second = flow.submit("also run the tests").await;

    assert!(steers(&flow).is_empty());
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(
        flow.engine.queue.disposition(second),
        Some(Disposition::Queue)
    );
    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    assert_eq!(flow.state(second), InputState::Applied);
    assert_eq!(turns(&flow), vec!["fix the build", "also run the tests"]);
    let ended = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].input, Some(second));
}

#[tokio::test]
async fn finish_task_ends_the_run_and_releases_the_label() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let input = flow.submit("fix the build").await;
    let agent = flow.agent();
    let task = flow.record(input).task.unwrap();
    assert!(flow.engine.flow.tasks.label(task).is_some());

    flow.engine.finish_task(flow.chat, agent).await.unwrap();

    assert!(
        flow.engine
            .store
            .unfinished_runs()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(flow.engine.flow.tasks.label(task).is_none());
    assert!(!flow.engine.chat_is_running(flow.chat));
}

#[tokio::test]
async fn cancel_applies_only_before_the_input_is_sent() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    let sent = flow.submit("fix the build").await;
    let waiting = flow.submit("then run the tests").await;

    let refused = flow.engine.cancel_input(CLIENT, sent).await.unwrap_err();
    flow.engine.cancel_input(CLIENT, waiting).await.unwrap();

    assert!(matches!(
        refused,
        EngineError::Queue(saturn_core::queue::QueueError::AlreadySent)
    ));
    assert_eq!(flow.state(sent), InputState::Applied);
    assert_eq!(flow.state(waiting), InputState::Cancelled);
    let (entries, _) = flow
        .engine
        .store
        .recent_history(flow.chat, 10)
        .await
        .unwrap();
    assert!(entries.iter().any(|entry| matches!(
        entry,
        crate::store::HistoryEntry::Input { input, state: InputState::Cancelled, .. } if *input == waiting
    )));
}

#[tokio::test]
async fn run_as_new_task_moves_a_waiting_input_to_its_own_session() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("unrelated cleanup").await;
    assert_eq!(flow.state(waiting), InputState::Queued);

    flow.engine.run_as_new_task(CLIENT, waiting).await.unwrap();
    flow.engine.finish_task(flow.chat, agent).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Applied);
    let opens = flow
        .fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opens, 2);
}

#[tokio::test]
async fn requests_are_answered_while_a_judgment_is_in_flight() {
    let fixture = Fixture::new();
    let transport = FakeTransport::new({
        let mut script = check_passes();
        script.push(idle_reply(0.95));
        script
    });
    let mut engine = fixture
        .start(fixture.env(true, Arc::clone(&transport)).await)
        .await
        .unwrap();
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let fake = FakeProvider::new(Provider::Claude);
    engine.providers.insert(
        (chat, Provider::Claude),
        ProviderConnection::Fake(fake.clone()),
    );
    let mut client = Client::connect(&fixture.socket()).await;
    let workdir = fixture.workdir.display().to_string();
    let release = transport.hold_next_call();

    let input = drive(&mut engine, async {
        client
            .attach(
                1,
                Request::Attach {
                    chat: Some(chat),
                    workdir,
                    env: Vec::new(),
                    overrides: Vec::new(),
                    add_dirs: Vec::new(),
                },
            )
            .await;
        let submitted = client
            .attach(
                2,
                Request::SubmitInput {
                    chat,
                    client_ref: 7,
                    text: "fix the build".to_owned(),
                    skip_relation: false,
                },
            )
            .await;
        let input = submitted
            .iter()
            .find_map(|notification| match notification {
                Notification::InputAccepted { input, .. } => Some(*input),
                _ => None,
            })
            .expect("input should be accepted");
        client.attach(3, Request::CancelInput { input }).await;
        input
    })
    .await;

    release.notify_one();
    let done = timeout(WAIT, engine.flow.router_rx.recv())
        .await
        .expect("router result should arrive in time")
        .expect("result channel should stay open");
    engine.on_routed(done).await;
    assert_eq!(
        engine.queue.input(input).map(|record| record.state),
        Some(InputState::Cancelled)
    );
    assert!(engine.flow.judging.is_empty());
    assert!(fake.calls().is_empty());
}

#[tokio::test]
async fn socket_submit_reports_input_and_task_states_in_order() {
    let fixture = Fixture::new();
    let transport = FakeTransport::new({
        let mut script = check_passes();
        script.push(idle_reply(0.95));
        script
    });
    let mut engine = fixture
        .start(fixture.env(true, Arc::clone(&transport)).await)
        .await
        .unwrap();
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let fake = FakeProvider::new(Provider::Claude);
    engine.providers.insert(
        (chat, Provider::Claude),
        ProviderConnection::Fake(fake.clone()),
    );
    let mut client = Client::connect(&fixture.socket()).await;
    let workdir = fixture.workdir.display().to_string();

    let notifications = drive(&mut engine, async {
        client
            .attach(
                1,
                Request::Attach {
                    chat: Some(chat),
                    workdir,
                    env: Vec::new(),
                    overrides: Vec::new(),
                    add_dirs: Vec::new(),
                },
            )
            .await;
        let mut notifications = client
            .attach(
                2,
                Request::SubmitInput {
                    chat,
                    client_ref: 7,
                    text: "fix the build".to_owned(),
                    skip_relation: false,
                },
            )
            .await;
        // 판단은 별도 작업이라 응답 뒤에 이어서 온다
        for _ in 0..3 {
            notifications.push(client.notification().await);
        }
        notifications
    })
    .await;

    let states: Vec<String> = notifications
        .iter()
        .map(|notification| match notification {
            Notification::InputAccepted { client_ref, .. } => format!("accepted {client_ref}"),
            Notification::InputChanged { state, .. } => format!("input {state:?}"),
            Notification::TaskChanged { state, .. } => format!("task {state:?}"),
            other => format!("other {other:?}"),
        })
        .collect();
    assert_eq!(
        states,
        vec![
            "accepted 7",
            "input Judging",
            "input Delivering",
            "input Applied",
            &format!("task {:?}", TaskState::Running),
        ]
    );
    assert!(matches!(
        &fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}
