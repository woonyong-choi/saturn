//! 공통 코드를 고치지 않고 어댑터 하나와 등록 한 줄로 provider를 붙인다: 가짜 어댑터가 session 열기부터 턴 끝까지 쓰인다.
//! 설계: docs/design/providers-and-sessions.md#어댑터-등록

use std::sync::Arc;

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::ModelChoice;
use saturn_protocol::state::InputState;

use super::support::{Flow, idle_reply, running_reply, turn_completed};
use crate::providers::test_support::{Call, FakeAdapter, FakeProvider, fake_descriptor};
use crate::providers::{ContextDefaults, Feature};

const FAKE: Provider = Provider::from_static("fake-agent");

/// 가짜 어댑터를 레지스트리에 한 줄로 등록한다.
fn register(flow: &mut Flow, descriptor: crate::providers::Descriptor) -> FakeProvider {
    let provider = FakeProvider::new(descriptor.id);
    flow.engine
        .registry
        .register(Arc::new(FakeAdapter {
            descriptor,
            provider: provider.clone(),
        }))
        .unwrap();
    provider
}

fn pinned(model: &str) -> ModelChoice {
    ModelChoice {
        provider: FAKE,
        model: model.to_owned(),
    }
}

fn calls(provider: &FakeProvider) -> (Vec<Option<String>>, Vec<String>, Vec<String>) {
    let (mut opens, mut turns, mut steers) = (Vec::new(), Vec::new(), Vec::new());
    for call in provider.calls() {
        match call {
            Call::Open { model, .. } => opens.push(model),
            Call::SendTurn { text, .. } => turns.push(text),
            Call::Steer { text, .. } => steers.push(text),
            _ => {}
        }
    }
    (opens, turns, steers)
}

#[tokio::test]
async fn a_registered_adapter_runs_an_input_from_open_to_turn_end() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let fake = register(&mut flow, fake_descriptor(FAKE));

    let input = flow
        .submit_with("hello fake", Some(pinned("model-a")), false)
        .await;

    let (opens, turns, _) = calls(&fake);
    assert_eq!(opens, vec![Some("model-a".to_owned())]);
    assert_eq!(turns, vec!["hello fake"]);
    assert_eq!(flow.state(input), InputState::Applied);
    let agent = flow.agent();
    flow.event(FAKE, turn_completed(agent)).await;
    assert_eq!(
        flow.engine
            .flow
            .live
            .values()
            .next()
            .map(|live| live.provider),
        Some(FAKE)
    );
}

#[tokio::test]
async fn descriptor_values_reach_the_common_code() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut descriptor = fake_descriptor(FAKE);
    descriptor.display_name = "Fake Agent";
    register(&mut flow, descriptor);

    let registry = &flow.engine.registry;

    assert_eq!(registry.display_name(FAKE), "Fake Agent");
    assert!(registry.instruction_docs().contains(&"FAKE.md".to_owned()));
    assert_eq!(registry.ids().last(), Some(&FAKE));
    let budget = flow
        .engine
        .context_budget(flow.chat, saturn_protocol::ids::AgentId(1), FAKE)
        .await
        .unwrap();
    assert_eq!(budget.window, 50_000);
    assert_eq!(budget.cache_write, 2.0);
    assert_eq!(
        flow.engine.registry.context_defaults(FAKE),
        ContextDefaults {
            window: 50_000,
            cache_write: 2.0
        }
    );
}

#[tokio::test]
async fn an_adapter_without_the_steer_feature_never_gets_a_steer() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    let mut descriptor = fake_descriptor(FAKE);
    descriptor.features = &[Feature::Compact];
    let fake = register(&mut flow, descriptor);
    fake.verify_steer();
    flow.submit_with("first", Some(pinned("m")), false).await;

    flow.submit("also this").await;

    let (_, turns, steers) = calls(&fake);
    assert_eq!(steers, Vec::<String>::new());
    assert_eq!(turns, vec!["first"]);
}

#[tokio::test]
async fn an_adapter_with_the_steer_feature_gets_the_steer() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    let fake = register(&mut flow, fake_descriptor(FAKE));
    fake.verify_steer();
    flow.submit_with("first", Some(pinned("m")), false).await;

    flow.submit("also this").await;

    let (_, _, steers) = calls(&fake);
    assert_eq!(steers, vec!["also this"]);
}
