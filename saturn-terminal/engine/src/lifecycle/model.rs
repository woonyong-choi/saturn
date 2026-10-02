//! 모델 고르기 테스트: 목록, 고정 모델로 보내기, 모델이 바뀌면 새 메인 session.

use saturn_protocol::ids::{Provider, SessionId};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, Notification, Request};
use saturn_protocol::state::SessionState;

use super::drive;
use super::support::{Flow, turn_completed};
use crate::providers::pinned_text;
use crate::providers::test_support::{Call, FakeProvider};

fn pinned(provider: Provider, model: &str) -> String {
    pinned_text(&ModelChoice {
        provider,
        model: model.to_owned(),
    })
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
    let mut flow = Flow::new(Vec::new()).await;

    flow.submit_with("hello", Some(&pinned(Provider::Claude, "sonnet")), false)
        .await;

    assert_eq!(opens(&flow.fake), vec![(Some("sonnet".to_owned()), None)]);
    let session = flow.engine.sessions.get(SessionId(1)).unwrap();
    assert_eq!(session.model.as_deref(), Some("sonnet"));
}

#[tokio::test]
async fn pinned_model_decides_the_provider() {
    let mut flow = Flow::new(Vec::new()).await;
    let codex = flow.add_provider(Provider::Codex);

    flow.submit_with("hello", Some(&pinned(Provider::Codex, "gpt-x")), false)
        .await;

    assert_eq!(opens(&codex), vec![(Some("gpt-x".to_owned()), None)]);
    assert!(opens(&flow.fake).is_empty());
}

#[tokio::test]
async fn same_model_keeps_using_the_open_session() {
    let mut flow = Flow::new(Vec::new()).await;
    let opus = pinned(Provider::Claude, "opus");
    flow.submit_with("one", Some(&opus), false).await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.submit_with("two", Some(&opus), false).await;

    assert_eq!(opens(&flow.fake).len(), 1);
}

#[tokio::test]
async fn changing_the_model_opens_a_new_main_session_with_a_packet() {
    let mut flow = Flow::new(Vec::new()).await;
    flow.submit_with("one", Some(&pinned(Provider::Claude, "opus")), false)
        .await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.submit_with("two", Some(&pinned(Provider::Claude, "haiku")), false)
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
        client.send(2, Request::ListModels { chat, provider }).await;
        client
            .until(|notification| match notification {
                Notification::Models { models } => Some(models.clone()),
                _ => None,
            })
            .await
    })
    .await
}

#[tokio::test]
async fn model_list_comes_in_provider_order() {
    let mut flow = Flow::new(Vec::new()).await;
    flow.add_provider(Provider::Codex);

    let models = list_models(&mut flow, None).await;

    let providers: Vec<Provider> = models.iter().map(|info| info.choice.provider).collect();
    assert_eq!(providers, vec![Provider::Claude, Provider::Codex]);
}

#[tokio::test]
async fn model_list_can_be_limited_to_one_provider() {
    let mut flow = Flow::new(Vec::new()).await;
    flow.add_provider(Provider::Codex);

    let models = list_models(&mut flow, Some(Provider::Codex)).await;

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].choice.provider, Provider::Codex);
}
