//! 모델 고르기 테스트: 목록, 고정 모델로 보내기, 모델이 바뀌면 새 메인 session.

use saturn_protocol::ids::{Provider, SessionId};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, Notification, QueryResult, Request};
use saturn_protocol::state::SessionState;

use super::support::{Flow, idle_reply, turn_completed};
use super::{Client, drive};
use crate::providers::test_support::{Call, FakeProvider};

fn pinned(provider: Provider, model: &str) -> ModelChoice {
    ModelChoice {
        provider,
        model: model.to_owned(),
    }
}

fn judged(count: usize) -> Vec<super::FakeReply> {
    (0..count).map(|_| idle_reply(0.95)).collect()
}

fn opens(fake: &FakeProvider) -> Vec<(Option<String>, Option<String>)> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { model, packet, .. } => Some((model, packet)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn pinned_model_opens_the_session_with_that_model_and_records_it() {
    let mut flow = Flow::new(judged(2)).await;

    flow.submit_with(
        "hello",
        Some(pinned(crate::providers::test_support::CLAUDE, "sonnet")),
        false,
    )
    .await;

    assert_eq!(opens(&flow.fake), vec![(Some("sonnet".to_owned()), None)]);
    let session = flow.engine.sessions.get(SessionId(1)).unwrap();
    assert_eq!(session.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn pinned_model_decides_the_provider() {
    let mut flow = Flow::new(judged(2)).await;
    let codex = flow.add_provider(crate::providers::test_support::CODEX);

    flow.submit_with(
        "hello",
        Some(pinned(crate::providers::test_support::CODEX, "gpt-x")),
        false,
    )
    .await;

    assert_eq!(opens(&codex), vec![(Some("gpt-x".to_owned()), None)]);
    assert!(opens(&flow.fake).is_empty());
}

#[tokio::test]
async fn same_model_keeps_using_the_open_session() {
    let mut flow = Flow::new(judged(2)).await;
    let opus = pinned(crate::providers::test_support::CLAUDE, "opus");
    flow.submit_with("one", Some(opus.clone()), false).await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.submit_with("two", Some(opus.clone()), false).await;

    assert_eq!(opens(&flow.fake).len(), 1);
}

#[tokio::test]
async fn changing_the_model_opens_a_new_main_session_with_a_packet() {
    let mut flow = Flow::new(judged(2)).await;
    flow.submit_with(
        "one",
        Some(pinned(crate::providers::test_support::CLAUDE, "opus")),
        false,
    )
    .await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.submit_with(
        "two",
        Some(pinned(crate::providers::test_support::CLAUDE, "haiku")),
        false,
    )
    .await;

    let opened = opens(&flow.fake);
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[1].0.as_deref(), Some("haiku"));
    assert!(
        opened[1]
            .1
            .as_deref()
            .is_some_and(|packet| packet.contains("one"))
    );
    let old = flow.engine.sessions.get(SessionId(1)).unwrap();
    let new = flow.engine.sessions.get(SessionId(2)).unwrap();
    assert_eq!(old.state, SessionState::ClosedResumable);
    assert_eq!(new.state, SessionState::Open);
    assert_eq!(new.model.as_deref(), Some("haiku"));
}

async fn list_models(flow: &mut Flow, provider: Option<Provider>) -> Vec<ModelInfo> {
    let mut client = flow.client().await;
    let chat = flow.chat;
    drive(&mut flow.engine, async {
        let request = Request::ListModels { chat, provider };
        let QueryResult::Models { models } = client.query(2, request).await else {
            panic!("expected Models");
        };
        models
    })
    .await
}

#[tokio::test]
async fn model_list_comes_in_provider_order() {
    let mut flow = Flow::new(judged(2)).await;
    flow.add_provider(crate::providers::test_support::CODEX);

    let models = list_models(&mut flow, None).await;

    let providers: Vec<Provider> = models.iter().map(|info| info.choice.provider).collect();
    assert_eq!(
        providers,
        vec![
            crate::providers::test_support::CLAUDE,
            crate::providers::test_support::CODEX
        ]
    );
}

#[tokio::test]
async fn model_list_can_be_limited_to_one_provider() {
    let mut flow = Flow::new(judged(2)).await;
    flow.add_provider(crate::providers::test_support::CODEX);

    let models = list_models(&mut flow, Some(crate::providers::test_support::CODEX)).await;

    assert_eq!(models.len(), 1);
    assert_eq!(
        models[0].choice.provider,
        crate::providers::test_support::CODEX
    );
}

#[tokio::test]
async fn pinned_model_is_saved_and_told_to_every_tui_that_attaches() {
    let mut flow = Flow::new(Vec::new()).await;
    let model = pinned(crate::providers::test_support::CODEX, "gpt-x");
    flow.pin(&model).await;
    let mut client = Client::connect(&flow.fixture.socket()).await;
    let (chat, workdir) = (flow.chat, flow.fixture.workdir.display().to_string());

    let notifications = drive(&mut flow.engine, async {
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
            .await
    })
    .await;

    assert!(notifications.contains(&Notification::ModelPinned { chat, model }));
}

#[tokio::test]
async fn pinned_input_gets_the_relation_judgment_and_the_pinned_model() {
    let mut flow = Flow::new(judged(1)).await;

    flow.submit_with(
        "hello",
        Some(pinned(crate::providers::test_support::CLAUDE, "haiku")),
        false,
    )
    .await;

    assert_eq!(flow.router_calls(), 1);
    assert_eq!(opens(&flow.fake), vec![(Some("haiku".to_owned()), None)]);
}
