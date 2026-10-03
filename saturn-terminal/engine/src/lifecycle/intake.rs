//! 입력 접수 테스트: 기록 저장소에 접수된 뒤에만 provider로 보낸다.

use saturn_core::queue::QueueError;
use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::ids::{ChatId, InputId, Provider, ProviderSessionId};
use saturn_protocol::state::InputState;

use super::support::{CLIENT, Flow, idle_reply, running_reply};
use super::*;
use crate::chat_env::ChatEnv;
use crate::providers::ProviderConnection;
use crate::providers::test_support::{Call, FakeProvider};
use crate::rpc::ClientId;
use crate::secrets::ROUTER_KEY_ENV;
use crate::store::HistoryEntry;

#[tokio::test]
async fn record_write_failure_sends_nothing_anywhere() {
    let mut flow = Flow::new(Vec::new()).await;
    let workdir = flow.fixture.workdir.clone();
    flow.engine
        .settings
        .apply_trusted(&flow.engine.store, Some(flow.chat), &workdir)
        .await
        .unwrap();
    flow.engine.store.deny_writes().await;

    let error = flow
        .engine
        .submit_input(CLIENT, flow.chat, 1, "fix the build".to_owned(), false)
        .await
        .unwrap_err();

    assert!(matches!(error, EngineError::Store(_)));
    assert!(flow.fake.calls().is_empty());
    assert_eq!(flow.router_calls(), 0);
    assert_eq!(flow.engine.queue.next_to_route(flow.chat), None);
    flow.engine.store.allow_writes().await;
    let (entries, _) = flow
        .engine
        .store
        .recent_history(flow.chat, 10)
        .await
        .unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn accepted_input_reaches_first_provider_after_it_is_recorded() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;

    let input = flow.submit("fix the build").await;

    let session = ProviderSessionId("fake-session-1".to_owned());
    assert_eq!(
        flow.fake.calls(),
        vec![
            Call::Open {
                agent: flow.agent(),
                model: None,
                resume: None,
                packet: None,
                add_dirs: Vec::new(),
            },
            Call::SendTurn {
                session,
                text: "fix the build".to_owned()
            },
        ]
    );
    assert_eq!(flow.state(input), InputState::Applied);
    let (entries, _) = flow
        .engine
        .store
        .recent_history(flow.chat, 10)
        .await
        .unwrap();
    assert!(matches!(
        entries.as_slice(),
        [HistoryEntry::Input { state: InputState::Applied, text, .. }] if text == "fix the build"
    ));
}

#[tokio::test]
async fn unattached_client_cannot_submit() {
    let mut flow = Flow::new(Vec::new()).await;

    let wrong_client = flow
        .engine
        .submit_input(ClientId(2), flow.chat, 1, "hi".to_owned(), false)
        .await
        .unwrap_err();
    let wrong_chat = flow
        .engine
        .submit_input(CLIENT, ChatId(99), 1, "hi".to_owned(), false)
        .await
        .unwrap_err();

    assert!(matches!(wrong_client, EngineError::ChatNotAttached { .. }));
    assert!(matches!(wrong_chat, EngineError::ChatNotAttached { .. }));
    assert_eq!(wrong_chat.code(), INVALID_PARAMS);
    assert!(flow.fake.calls().is_empty());
}

#[tokio::test]
async fn inputs_are_routed_one_at_a_time_in_accept_order() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;

    flow.submit("first request").await;
    let second = flow.submit("second request").await;

    let bodies: Vec<String> = flow
        .transport
        .calls()
        .into_iter()
        .filter_map(|call| call.2)
        .collect();
    assert!(bodies[1].contains("first request"));
    assert!(bodies[2].contains("second request"));
    assert!(bodies[2].contains("chat: running"));
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(flow.router_calls(), 2);
}

#[tokio::test]
async fn first_input_prefers_claude_then_codex() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let chat = flow.chat;
    flow.engine.providers.remove(&(chat, Provider::Claude));
    let codex = FakeProvider::new(Provider::Codex);
    flow.engine
        .add_connection(chat, ProviderConnection::Fake(codex.clone()));

    let input = flow.submit("hello").await;

    assert_eq!(flow.state(input), InputState::Applied);
    assert_eq!(codex.calls().len(), 2);
    let main = flow.engine.sessions.live_main(chat).unwrap();
    assert_eq!(main.provider, Provider::Codex);
}

#[tokio::test]
async fn first_input_without_any_provider_is_rejected_and_sent_nowhere() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let chat = flow.chat;
    flow.engine.providers.remove(&(chat, Provider::Claude));

    let input = flow.submit("hello").await;

    assert_eq!(flow.state(input), InputState::Rejected);
    assert!(flow.fake.calls().is_empty());
    let (entries, _) = flow.engine.store.recent_history(chat, 10).await.unwrap();
    assert!(matches!(
        entries.as_slice(),
        [HistoryEntry::Input {
            state: InputState::Rejected,
            ..
        }]
    ));
}

#[tokio::test]
async fn pinned_model_input_is_judged_and_goes_to_the_session_with_that_model() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let model = saturn_protocol::rpc::ModelChoice {
        provider: Provider::Claude,
        model: "the-pinned-model".to_owned(),
    };

    let input = flow.submit_with("hello", Some(model), false).await;

    assert_eq!(flow.router_calls(), 1);
    assert_eq!(flow.state(input), InputState::Applied);
    assert!(matches!(
        &flow.fake.calls()[0],
        Call::Open { model: Some(model), .. } if model == "the-pinned-model"
    ));
}

#[tokio::test]
async fn launch_spec_takes_workdir_and_env_from_the_chat() {
    let flow = Flow::new(Vec::new()).await;
    let mut engine = flow.engine;
    let chat = flow.chat;
    let other = flow.fixture.root.path().join("other");
    engine.chats.insert(
        chat,
        ChatEnv::new(
            other.clone(),
            vec![
                ("PATH".to_owned(), "/opt/tui/bin".to_owned()),
                (ROUTER_KEY_ENV.to_owned(), "sk-secret".to_owned()),
            ],
        ),
    );
    let revision = engine.settings.current().unwrap();

    let launch = engine
        .launch_spec(Provider::Claude, chat, revision)
        .await
        .unwrap();

    assert_eq!(launch.workdir, other);
    assert_eq!(launch.program, PathBuf::from("claude"));
    assert_eq!(launch.settings, revision);
    assert!(
        launch
            .env
            .iter()
            .all(|(name, _)| name != ROUTER_KEY_ENV && name != "SATURN_KEY")
    );
    assert!(launch.env.iter().any(|(name, _)| name == "PATH"));
    assert!(launch.hook_settings.is_some());
}

#[tokio::test]
async fn unknown_input_is_invalid_params() {
    let mut flow = Flow::new(Vec::new()).await;

    let error = flow
        .engine
        .cancel_input(CLIENT, InputId(77))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EngineError::Queue(QueueError::NotFound(InputId(77)))
    ));
    assert_eq!(error.code(), INVALID_PARAMS);
}
