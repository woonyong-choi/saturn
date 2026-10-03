//! provider 요청 하나가 응답하지 않아도 engine의 다른 요청 처리가 멈추지 않는다(#325).
//! 실제 Codex 연결이 가짜 app-server와 말하고, 시험은 소켓 요청과 응답만 본다.

use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{ModelChoice, Notification};
use saturn_protocol::state::{InputState, TaskState};

use super::support::{Flow, idle_reply};
use super::*;
use crate::chat_env::ChatEnv;
use crate::providers::ProviderConnection;
use crate::providers::test_support::{Call, FakeProvider};
use crate::providers::{CodexClient, fake_codex_launch};
use std::path::PathBuf;

/// 시험이 기다리는 시간을 줄인다. 실제 값은 `providers::REPLY_TIMEOUT`.
const SHORT_REPLY_TIMEOUT: Duration = Duration::from_secs(1);

fn input_state(notification: &Notification) -> Option<InputState> {
    match notification {
        Notification::InputChanged { state, .. } => Some(*state),
        _ => None,
    }
}

/// 첫 채팅을 응답하지 않을 수 있는 실제 Codex 연결과 고정 모델로 바꾼다.
async fn use_silent_codex(flow: &mut Flow) {
    let dir = flow.fixture.root.path().join("codex-bin");
    std::fs::create_dir_all(&dir).unwrap();
    let codex = CodexClient::start(
        fake_codex_launch(&dir, Vec::new()),
        flow.engine.supervisor.clone(),
    )
    .await
    .unwrap()
    .with_reply_timeout(SHORT_REPLY_TIMEOUT);
    flow.engine
        .flow
        .questions_of_connection
        .insert((flow.chat, Provider::Codex), true);
    flow.engine.providers.insert(
        (flow.chat, Provider::Codex),
        ProviderConnection::Codex(codex),
    );
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
    flow.engine.providers.insert(
        (chat, Provider::Claude),
        ProviderConnection::Fake(fake.clone()),
    );
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

    let needs_check = |n: &Notification| {
        matches!(
            n,
            Notification::TaskChanged {
                state: TaskState::NeedsCheck,
                ..
            }
        )
    };
    assert!(
        first_seen.iter().any(needs_check),
        "a turn whose send got no reply should ask the user to check: {first_seen:?}"
    );
    assert!(matches!(
        &other_fake.calls()[..],
        [Call::Open { .. }, Call::SendTurn { .. }]
    ));
}
