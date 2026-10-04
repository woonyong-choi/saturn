//! provider 요청 하나가 응답하지 않거나 느려도 engine의 다른 요청 처리가 멈추지 않는다(#325, #352).
//! 실제 Codex 연결이 가짜 app-server와 말하고, 시험은 소켓 요청과 응답만 본다.

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ChatNotice, ModelChoice, Notification, PermissionAnswer};
use saturn_protocol::state::{InputState, TaskState};

use super::support::{
    CLIENT, Flow, context_size, idle_reply, input_request, permission, text, turn_completed,
};
use super::*;
use crate::calls::Responder;
use crate::chat_env::ChatEnv;
use crate::providers::OPEN_REPLY_TIMEOUT;
use crate::providers::ProviderConnection;
use crate::providers::test_support::{Call, FakeAdapter, FakeProvider, Stall, fake_descriptor};
use crate::providers::test_support::{CodexClient, fake_codex_launch};
use std::os::unix::fs::PermissionsExt;
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
        .insert((flow.chat, crate::providers::test_support::CODEX), true);
    flow.engine.add_connection(
        flow.chat,
        ProviderConnection::new(crate::providers::test_support::CODEX, codex),
    );
    flow.pin(&ModelChoice {
        provider: crate::providers::test_support::CODEX,
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
    let fake = FakeProvider::new(crate::providers::test_support::CLAUDE);
    flow.engine
        .flow
        .questions_of_connection
        .insert((chat, crate::providers::test_support::CLAUDE), true);
    flow.engine.add_connection(chat, fake.connection());
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
            .contains_key(&(chat, crate::providers::test_support::CLAUDE))
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
            .contains_key(&(chat, crate::providers::test_support::CLAUDE))
    );
}

// 아래는 #364: 허가·입력 답, session 닫기, `/model` 목록, 맥락 정리의 새 session 열기도 연결 작업이 실행해
// 느린 provider가 루프를 막지 못한다. 느린 호출은 `FakeProvider::stall`로 풀어 줄 때까지 멈춰 세운다.

/// 가짜 provider가 `found`인 호출을 받을 때까지 기다린다. 호출이 provider까지 갔다는 뜻이다.
async fn reached(fake: &FakeProvider, what: &str, found: impl Fn(&Call) -> bool) {
    let wait = async {
        while !fake.calls().iter().any(&found) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(Duration::from_secs(3), wait)
        .await
        .unwrap_or_else(|_| panic!("{what} should reach the provider"));
}

/// `id` 요청의 응답까지 읽는다.
async fn response_to(client: &mut Client, id: u64) -> Response {
    loop {
        if let ServerMessage::Response(response) = client.recv().await {
            assert_eq!(response.id, Some(RequestId(id)));
            return response;
        }
    }
}

/// 멈춰 선 provider 요청이 있어도 다른 채팅의 붙기와 입력, 같은 채팅의 멈춤과 조회를 바로 처리한다.
async fn others_keep_working(
    (other, other_dir): (ChatId, &std::path::Path),
    (chat, stopper): (ChatId, &mut Client),
    second: &mut Client,
) {
    prompt("an attach of another chat", async {
        second.attach(1, attach_request(other, other_dir)).await
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
    prompt(
        "a history load of the same chat",
        stopper.attach(3, history),
    )
    .await;
    prompt(
        "a stop of the same chat",
        stopper.attach(4, Request::Stop { chat }),
    )
    .await;
}

fn is_answer(call: &Call) -> bool {
    matches!(
        call,
        Call::AnswerPermission { .. } | Call::AnswerInput { .. }
    )
}

#[tokio::test]
async fn a_slow_permission_answer_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, other_fake) = add_other_chat(&mut flow).await;
    flow.submit("fix the build").await;
    let (agent, chat, fake) = (flow.agent(), flow.chat, flow.fake.clone());
    let mut asker = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    fake.stall(Stall::Answer);

    let answered = drive(&mut flow.engine, async {
        fake.emit(permission(agent, "req-1"));
        let request_id = asker
            .until(|n| match n {
                Notification::PermissionRequested { request_id, .. } => Some(request_id.clone()),
                _ => None,
            })
            .await;
        let answer = Request::AnswerPermission {
            request_id,
            answer: PermissionAnswer::AllowOnce,
        };
        asker.send(10, answer).await;
        reached(&fake, "the permission answer", is_answer).await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;

        fake.release(Stall::Answer);
        response_to(&mut asker, 10).await
    })
    .await;

    assert_eq!(answered, Response::ok(RequestId(10)));
    assert!(matches!(
        &other_fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}

#[tokio::test]
async fn a_slow_input_answer_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    flow.submit("fix the build").await;
    let (agent, chat, fake) = (flow.agent(), flow.chat, flow.fake.clone());
    let mut asker = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    fake.stall(Stall::Answer);

    let answered = drive(&mut flow.engine, async {
        fake.emit(input_request(agent, "ask-1"));
        let request_id = asker
            .until(|n| match n {
                Notification::InputRequested { request_id, .. } => Some(request_id.clone()),
                _ => None,
            })
            .await;
        let answer = Request::AnswerInput {
            request_id,
            answer: InputAnswer::Cancel,
        };
        asker.send(10, answer).await;
        reached(&fake, "the input answer", is_answer).await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;

        fake.release(Stall::Answer);
        response_to(&mut asker, 10).await
    })
    .await;

    assert_eq!(answered, Response::ok(RequestId(10)));
}

#[tokio::test]
async fn a_slow_session_close_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    let claude = crate::providers::test_support::CLAUDE;
    let opus = ModelChoice {
        provider: claude,
        model: "opus".to_owned(),
    };
    flow.submit_with("one", Some(opus), false).await;
    let (agent, chat, fake) = (flow.agent(), flow.chat, flow.fake.clone());
    flow.claude_event(turn_completed(agent)).await;
    // 모델이 바뀌면 떠나는 session을 닫는다
    flow.pin(&ModelChoice {
        provider: claude,
        model: "haiku".to_owned(),
    })
    .await;
    let mut first = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    fake.stall(Stall::Close);

    drive(&mut flow.engine, async {
        first.attach(2, submit(chat, "two")).await;
        reached(&fake, "the close of the leaving session", |call| {
            matches!(call, Call::Close { .. })
        })
        .await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;
        fake.release(Stall::Close);
    })
    .await;
}

#[tokio::test]
async fn a_slow_model_list_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    let (chat, fake) = (flow.chat, flow.fake.clone());
    let mut asker = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    fake.stall(Stall::Models);

    let models = drive(&mut flow.engine, async {
        let list = Request::ListModels {
            chat,
            provider: None,
        };
        asker.send(10, list).await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;

        fake.release(Stall::Models);
        let models = asker
            .until(|n| match n {
                Notification::Models { models } => Some(models.clone()),
                _ => None,
            })
            .await;
        (models, response_to(&mut asker, 10).await)
    })
    .await;

    assert_eq!(models.0.len(), 1);
    assert_eq!(models.1, Response::ok(RequestId(10)));
}

#[tokio::test]
async fn a_slow_connection_for_the_model_list_does_not_stall_other_chats_or_stop() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    let (chat, workdir) = (flow.chat, flow.fixture.workdir.clone());
    let fake_agent = Provider::from_static("fake-agent");
    let fake = FakeProvider::new(fake_agent);
    flow.engine
        .registry
        .register(std::sync::Arc::new(FakeAdapter {
            descriptor: fake_descriptor(fake_agent),
            provider: fake.clone(),
        }))
        .unwrap();
    // 어댑터 실행 파일이 PATH에 있어야 설치된 provider로 본다
    let bin = flow.fixture.root.path().join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = bin.join("fake-agent");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut asker = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    // 붙을 때 채팅 환경이 덮이므로 붙은 뒤에 바꾼다
    flow.engine.chats.insert(
        chat,
        ChatEnv::new(
            workdir,
            vec![("PATH".to_owned(), bin.display().to_string())],
        ),
    );
    fake.stall(Stall::Connect);

    let models = drive(&mut flow.engine, async {
        let list = Request::ListModels {
            chat,
            provider: Some(fake_agent),
        };
        asker.send(10, list).await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;

        fake.release(Stall::Connect);
        asker
            .until(|n| match n {
                Notification::Models { models } => Some(models.clone()),
                _ => None,
            })
            .await
    })
    .await;

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].choice.provider, fake_agent);
}

#[tokio::test]
async fn a_slow_context_restart_does_not_stall_other_chats_and_holds_the_next_input() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95), idle_reply(0.95)]).await;
    let (other, other_dir, _) = add_other_chat(&mut flow).await;
    flow.submit("fix the build").await;
    let (agent, chat, fake) = (flow.agent(), flow.chat, flow.fake.clone());
    let mut first = flow.client().await;
    let mut stopper = flow.client().await;
    let mut second = Client::connect(&flow.fixture.socket()).await;
    fake.stall(Stall::Open);

    drive(&mut flow.engine, async {
        fake.emit(text(agent, "the cache is fixed"));
        fake.emit(context_size(agent, 10_000_000));
        fake.emit(turn_completed(agent));
        // 턴이 끝나면 맥락 정리가 새 session 열기를 맡기고 멈춘다
        reached(&fake, "the restart open", |call| {
            matches!(
                call,
                Call::Open {
                    packet: Some(_),
                    ..
                }
            )
        })
        .await;

        others_keep_working((other, &other_dir), (chat, &mut stopper), &mut second).await;
        // 멈춤은 대기 입력을 보류하므로 멈춘 뒤에 접수한다
        first.attach(2, submit(chat, "next")).await;
        let turns = || {
            fake.calls()
                .into_iter()
                .filter(|call| matches!(call, Call::SendTurn { .. }))
                .count()
        };
        assert_eq!(turns(), 1, "the next input should wait for the new session");

        fake.release(Stall::Open);
        first
            .until(|n| (input_state(n) == Some(InputState::Applied)).then_some(()))
            .await;
    })
    .await;

    let sent: Vec<Call> = fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::SendTurn { .. }))
        .collect();
    assert!(
        matches!(&sent[1], Call::SendTurn { session, text } if session.0 == "fake-session-2" && text == "next"),
        "the next input should go to the new session: {sent:?}"
    );
}

#[tokio::test]
async fn a_second_answer_to_a_request_already_being_answered_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(permission(agent, "req-1")).await;
    let id = flow.permission_id("req-1").unwrap();
    flow.fake.stall(Stall::Answer);
    let (responder, first) = Responder::local();
    flow.engine
        .answer_permission(CLIENT, id.clone(), PermissionAnswer::AllowOnce, responder)
        .await;

    let second = flow
        .answer_permission_as(CLIENT, id, PermissionAnswer::Deny { note: None })
        .await;

    assert!(matches!(second, Err(EngineError::UnexpectedAnswer { .. })));
    flow.fake.release(Stall::Answer);
    flow.answered(first).await.unwrap();
    assert!(flow.engine.flow.permissions.is_empty());
    let answers = flow.fake.calls().into_iter().filter(is_answer).count();
    assert_eq!(answers, 1);
}
