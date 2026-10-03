//! provider 요청 하나가 응답하지 않거나 느려도 engine의 다른 요청 처리가 멈추지 않는다(#325, #352).
//! 실제 Codex 연결이 가짜 app-server와 말하고, 시험은 소켓 요청과 응답만 본다.

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{ChatNotice, ModelChoice, Notification};
use saturn_protocol::state::{InputState, TaskState};

use super::support::{Flow, idle_reply};
use super::*;
use crate::chat_env::ChatEnv;
use crate::providers::ProviderConnection;
use crate::providers::test_support::{Call, FakeProvider};
use crate::providers::{CodexClient, OPEN_REPLY_TIMEOUT, fake_codex_launch};
use std::path::PathBuf;
use std::time::Instant;

/// 시험이 기다리는 시간을 줄인다. 실제 값은 `providers::REPLY_TIMEOUT`.
const SHORT_REPLY_TIMEOUT: Duration = Duration::from_secs(1);

/// 바로 돌아와야 하는 요청의 제한보다 느린 `thread/start` 응답 시간(ms). 열기는 긴 제한으로 기다려 성공해야 한다.
const SLOW_OPEN_MS: &str = "1500";

/// 시작 요청이 느린 동안 engine이 다른 요청에 답해야 하는 시간. 시작 요청(`SLOW_START_MS`)이 끝나기 전이어야 한다.
const PROMPT: Duration = Duration::from_secs(1);

/// 시작 요청 `thread/start`가 걸리는 시간(ms). 제한 시간(60초)보다 훨씬 짧고 `PROMPT`보다 길다.
const SLOW_START_MS: &str = "5000";

fn slow_open() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    vec![("FAKE_SLOW_OPEN_MS".into(), SLOW_OPEN_MS.into())]
}

fn input_state(notification: &Notification) -> Option<InputState> {
    match notification {
        Notification::InputChanged { state, .. } => Some(*state),
        _ => None,
    }
}

/// 첫 채팅을 응답하지 않을 수 있는 실제 Codex 연결과 고정 모델로 바꾼다.
async fn use_silent_codex(flow: &mut Flow) {
    let codex = start_fake_codex(flow, slow_open())
        .await
        .with_reply_timeouts(SHORT_REPLY_TIMEOUT, OPEN_REPLY_TIMEOUT);
    use_codex(flow, codex).await;
}

/// 가짜 app-server에 실제 Codex 연결을 맺는다.
async fn start_fake_codex(
    flow: &Flow,
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
) -> CodexClient {
    let dir = flow.fixture.root.path().join("codex-bin");
    std::fs::create_dir_all(&dir).unwrap();
    CodexClient::start(fake_codex_launch(&dir, env), flow.engine.supervisor.clone())
        .await
        .unwrap()
}

/// 첫 채팅의 연결을 `codex`로 바꾸고 그 provider의 모델로 고정한다.
async fn use_codex(flow: &mut Flow, codex: CodexClient) {
    flow.engine
        .flow
        .questions_of_connection
        .insert((flow.chat, Provider::Codex), true);
    flow.engine
        .add_connection(flow.chat, ProviderConnection::Codex(codex));
    flow.pin(&ModelChoice {
        provider: Provider::Codex,
        model: "gpt-test".to_owned(),
    })
    .await;
}

/// 다른 작업 폴더의 두 번째 채팅과 그 채팅의 가짜 Claude.
async fn add_other_chat(flow: &mut Flow) -> (ChatId, PathBuf, FakeProvider) {
    let dir = flow.fixture.workdir.with_file_name("other-work");
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    let chat = flow.engine.store.create_chat(dir.clone()).await.unwrap();
    let env = vec![("PATH".to_owned(), "/nonexistent".to_owned())];
    flow.engine
        .chats
        .insert(chat, ChatEnv::new(dir.clone(), env));
    let fake = FakeProvider::new(Provider::Claude);
    flow.engine
        .flow
        .questions_of_connection
        .insert((chat, Provider::Claude), true);
    flow.engine
        .add_connection(chat, ProviderConnection::Fake(fake.clone()));
    (chat, dir, fake)
}

fn submit(chat: ChatId, text: &str) -> Request {
    Request::SubmitInput {
        chat,
        client_ref: 1,
        text: text.to_owned(),
        skip_relation: false,
    }
}

#[tokio::test]
async fn a_silent_provider_request_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    use_silent_codex(&mut flow).await;
    let (other, other_dir, other_fake) = add_other_chat(&mut flow).await;
    let chat = flow.chat;
    let mut first = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;

    let first_seen = drive(&mut flow.engine, async {
        first.attach(2, submit(chat, "stall")).await;
        // 전송을 시작하면 provider가 응답하지 않는 `turn/start`를 기다린다
        first
            .until(|n| (input_state(n) == Some(InputState::Delivering)).then_some(()))
            .await;

        let attach = Request::Attach {
            chat: Some(other),
            workdir: other_dir.display().to_string(),
            env: Vec::new(),
            overrides: Vec::new(),
            add_dirs: Vec::new(),
        };
        second.attach(1, attach).await;
        second.attach(2, submit(other, "other work")).await;
        second
            .until(|n| (input_state(n) == Some(InputState::Applied)).then_some(()))
            .await;

        let history = Request::LoadHistory {
            chat,
            before: None,
            limit: 10,
        };
        let mut seen = first.attach(3, history).await;
        seen.extend(first.attach(4, Request::Stop { chat }).await);
        seen
    })
    .await;

    assert!(
        first_seen.iter().any(|n| matches!(
            n,
            Notification::ChatNotice {
                notice: ChatNotice::Stopped { .. },
                ..
            }
        )),
        "the stop should finish without waiting for the provider: {first_seen:?}"
    );
    assert!(matches!(
        &other_fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}

/// 느린 시작 요청이 끝나기 전에 engine이 `work`에 답하는지 본다.
async fn prompt<T>(what: &str, work: impl Future<Output = T>) -> T {
    timeout(PROMPT, work)
        .await
        .unwrap_or_else(|_| panic!("{what} should be handled while a provider start is slow"))
}

fn attach_request(chat: ChatId, workdir: &std::path::Path) -> Request {
    Request::Attach {
        chat: Some(chat),
        workdir: workdir.display().to_string(),
        env: Vec::new(),
        overrides: Vec::new(),
        add_dirs: Vec::new(),
    }
}

fn is_running(notification: &Notification) -> bool {
    matches!(
        notification,
        Notification::TaskChanged {
            state: TaskState::Running,
            ..
        }
    )
}

#[tokio::test]
async fn a_slow_start_request_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let start = vec![("FAKE_SLOW_OPEN_MS".into(), SLOW_START_MS.into())];
    let codex = start_fake_codex(&flow, start).await;
    use_codex(&mut flow, codex).await;
    let (other, other_dir, other_fake) = add_other_chat(&mut flow).await;
    let chat = flow.chat;
    let mut first = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    let begun = Instant::now();

    let stopped = drive(&mut flow.engine, async {
        first.attach(2, submit(chat, "slow start")).await;
        first
            .until(|n| (input_state(n) == Some(InputState::Delivering)).then_some(()))
            .await;

        prompt("an attach of another chat", async {
            second.attach(1, attach_request(other, &other_dir)).await
        })
        .await;
        prompt("an input of another chat", async {
            second.attach(2, submit(other, "other work")).await;
            second
                .until(|n| (input_state(n) == Some(InputState::Applied)).then_some(()))
                .await;
        })
        .await;
        let history = Request::LoadHistory {
            chat,
            before: None,
            limit: 10,
        };
        prompt("a history load of the same chat", first.attach(3, history)).await;
        prompt(
            "a stop of the same chat",
            first.attach(4, Request::Stop { chat }),
        )
        .await
    })
    .await;

    assert!(
        begun.elapsed() < Duration::from_millis(4500),
        "the requests should have been handled before the slow start finished"
    );
    assert!(
        stopped.iter().any(|n| matches!(
            n,
            Notification::ChatNotice {
                notice: ChatNotice::Stopped { .. },
                ..
            }
        )),
        "the stop should finish while the start is pending: {stopped:?}"
    );
    assert!(matches!(
        &other_fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}

#[tokio::test]
async fn a_stop_during_a_slow_start_holds_the_input_instead_of_sending_it() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let codex = start_fake_codex(&flow, slow_open()).await;
    use_codex(&mut flow, codex).await;
    let chat = flow.chat;
    let mut client = flow.client().await;

    let seen = drive(&mut flow.engine, async {
        client.attach(2, submit(chat, "slow start")).await;
        client
            .until(|n| (input_state(n) == Some(InputState::Delivering)).then_some(()))
            .await;
        client.attach(3, Request::Stop { chat }).await;
        // 시작 요청이 끝난 뒤에야 오는 입력 상태까지 읽는다
        let mut seen = Vec::new();
        loop {
            let notification = client.notification().await;
            let held = input_state(&notification) == Some(InputState::Held);
            seen.push(notification);
            if held {
                break;
            }
        }
        seen.extend(client.window().await);
        seen
    })
    .await;

    assert!(
        !seen.iter().any(is_running),
        "a stopped input should not start a turn: {seen:?}"
    );
}

#[tokio::test]
async fn a_provider_that_stops_reading_input_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        idle_reply(0.95),
        idle_reply(0.95),
        idle_reply(0.95),
    ])
    .await;
    let env = vec![("FAKE_STOP_READING_SECS".into(), "10".into())];
    let codex = start_fake_codex(&flow, env).await;
    use_codex(&mut flow, codex).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    let chat = flow.chat;
    let mut first = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    // 파이프가 가득 차 쓰기가 막히도록 파이프 크기보다 훨씬 긴 입력
    let long_input = "x".repeat(200_000);

    drive(&mut flow.engine, async {
        first.attach(2, submit(chat, &long_input)).await;
        first
            .until(|n| (input_state(n) == Some(InputState::Delivering)).then_some(()))
            .await;
        // `thread/start` 응답 뒤 `turn/start` 쓰기가 막힌다
        tokio::time::sleep(PROMPT).await;

        prompt("an attach of another chat", async {
            second.attach(1, attach_request(other, &other_dir)).await
        })
        .await;
        prompt("an input of another chat", async {
            second.attach(2, submit(other, "other work")).await;
            second
                .until(|n| (input_state(n) == Some(InputState::Applied)).then_some(()))
                .await;
        })
        .await;
        prompt(
            "a stop of the same chat",
            first.attach(3, Request::Stop { chat }),
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn a_silent_turn_start_asks_the_user_to_check_while_other_chats_keep_working() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    use_silent_codex(&mut flow).await;
    let (other, other_dir, other_fake) = add_other_chat(&mut flow).await;
    let chat = flow.chat;
    let mut first = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;

    drive(&mut flow.engine, async {
        first.attach(2, submit(chat, "stall")).await;
        second.attach(1, attach_request(other, &other_dir)).await;
        second.attach(2, submit(other, "other work")).await;
        second
            .until(|n| (input_state(n) == Some(InputState::Applied)).then_some(()))
            .await;
        // 응답 없는 `turn/start`가 제한 시간(시험에서는 1초)에 끝나기를 기다린다
        first
            .until(|n| {
                matches!(
                    n,
                    Notification::TaskChanged {
                        state: TaskState::NeedsCheck,
                        ..
                    }
                )
                .then_some(())
            })
            .await;
    })
    .await;

    assert!(matches!(
        &other_fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}

fn task_state(notification: &Notification) -> Option<TaskState> {
    match notification {
        Notification::TaskChanged { state, .. } => Some(*state),
        _ => None,
    }
}

#[tokio::test]
async fn a_connection_task_that_panics_while_sending_leaves_the_task_to_check() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.panic_on_send();
    let chat = flow.chat;
    let mut client = flow.client().await;

    let state = drive(&mut flow.engine, async {
        client.attach(2, submit(chat, "fix the build")).await;
        client
            .until(|n| task_state(n).filter(|state| *state == TaskState::NeedsCheck))
            .await
    })
    .await;

    assert_eq!(state, TaskState::NeedsCheck);
    assert!(flow.engine.flow.deliveries.is_empty());
    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(chat, Provider::Claude))
    );
}

#[tokio::test]
async fn a_connection_task_that_panics_while_opening_rejects_the_input() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.panic_on_open();
    let chat = flow.chat;
    let mut client = flow.client().await;

    let state = drive(&mut flow.engine, async {
        client.attach(2, submit(chat, "fix the build")).await;
        client
            .until(|n| task_state(n).filter(|state| *state == TaskState::Failed))
            .await
    })
    .await;

    assert_eq!(state, TaskState::Failed);
    assert!(flow.engine.flow.deliveries.is_empty());
    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(chat, Provider::Claude))
    );
}
