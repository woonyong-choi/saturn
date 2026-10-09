//! 수동 입력 경로: 판단 모델 없이 현재 provider에 보내고 전환 패킷은 원본 기록으로 만든다.
//! 설계: docs/design/router.md#코드와-router의-경계

use saturn_core::routers::Method;
use saturn_protocol::state::InputState;

use super::constraint_handoff::{packet_of, turn};
use super::support::{Flow, text, turn_completed};
use crate::providers::test_support::{CLAUDE, CODEX, Call};

#[tokio::test]
async fn manual_inputs_switch_both_ways_without_judgments_or_router_calls() {
    let mut flow = Flow::with_config(
        "[router]\nmode = \"manual\"\n[constraint]\nauto_apply = true\n",
        Vec::new(),
    )
    .await;
    assert_eq!(flow.engine.routers.method(), Method::Manual);
    let codex = flow.add_provider(CODEX);
    let claude = flow.fake.clone();
    let startup_calls = flow.transport.calls().len();

    flow.engine.switch_provider(flow.chat, CODEX);
    let first = turn(&mut flow, CODEX, "use X-Route-Key", "c1").await;
    assert_eq!(flow.state(first), InputState::Applied);
    flow.engine.switch_provider(flow.chat, CLAUDE);
    let second = flow.submit("review the header").await;
    assert_eq!(flow.state(second), InputState::Applied);
    assert!(packet_of(&claude).contains("User: use X-Route-Key"));
    let agent = flow.agent();
    flow.event(CLAUDE, turn_completed(agent)).await;
    flow.event(CLAUDE, text(agent, "header reviewed")).await;
    flow.event(CLAUDE, turn_completed(agent)).await;

    flow.engine.switch_provider(flow.chat, CODEX);
    let third = flow.submit("apply the review").await;
    assert_eq!(flow.state(third), InputState::Applied);
    let catch_up = codex
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { packet, .. } => packet,
            _ => None,
        })
        .next_back()
        .expect("Codex should receive the Claude update");
    assert!(catch_up.contains("header reviewed"));
    assert!(!catch_up.contains("User: use X-Route-Key"));
    assert_eq!(flow.transport.calls().len(), startup_calls);
    assert_eq!(flow.router_calls(), 0);

    let path = flow.fixture.options.home.join("manual-judgments.jsonl");
    assert_eq!(flow.engine.store.export_judgments(&path).await.unwrap(), 0);
}
