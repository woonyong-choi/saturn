//! 에이전트 질문 테스트: 권한 모드가 두 provider의 질문 기능을 정하고, 모드가 바뀌면 다시 시작으로 적용하며, 적용 전에
//! 온 질문은 사용자에게 보인다.

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ChatNotice, Notification};
use toml_edit::DocumentMut;

use super::support::{Flow, idle_reply, input_request, turn_completed};
use crate::providers::test_support::{Call, FakeProvider};

pub(super) fn restarted(notifications: &[Notification], expected: Provider) -> bool {
    notifications.iter().any(|notification| {
        matches!(
            notification,
            Notification::ChatNotice {
                notice: ChatNotice::ProviderRestarted { provider },
                ..
            } if *provider == expected
        )
    })
}

fn answered(fake: &FakeProvider) -> bool {
    fake.calls()
        .iter()
        .any(|call| matches!(call, Call::AnswerInput { .. }))
}

/// 지금 모드로 `provider`를 새로 열 때의 실행 설정에서 질문 기능을 뺐는지와 Codex 생성 설정의 값.
pub(super) async fn launched(flow: &Flow, provider: Provider) -> (bool, Option<bool>) {
    let revision = flow.engine.settings.current().unwrap();
    let launch = flow
        .engine
        .launch_spec(provider, flow.chat, revision)
        .await
        .unwrap();
    let feature = launch.permission.codex_home.as_ref().map(|home| {
        let config = std::fs::read_to_string(home.join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        config["features"]["default_mode_request_user_input"]
            .as_bool()
            .unwrap()
    });
    (launch.permission.questions_disabled, feature)
}

#[tokio::test]
async fn agent_questions_are_on_in_the_default_mode_for_both_providers() {
    let flow = Flow::new(Vec::new()).await;

    assert_eq!(
        launched(&flow, crate::providers::CLAUDE).await,
        (false, None)
    );
    assert_eq!(
        launched(&flow, crate::providers::CODEX).await,
        (false, Some(true))
    );
}

#[tokio::test]
async fn agent_questions_are_off_for_new_connections_in_full_mode() {
    let mut flow = Flow::with_config("[permission]\nmode = \"full\"\n", Vec::new()).await;

    assert_eq!(
        launched(&flow, crate::providers::CLAUDE).await,
        (true, None)
    );
    assert_eq!(
        launched(&flow, crate::providers::CODEX).await,
        (true, Some(false))
    );

    flow.engine
        .set_permission_mode(flow.chat, "edit")
        .await
        .unwrap();

    assert_eq!(
        launched(&flow, crate::providers::CLAUDE).await,
        (false, None)
    );
    assert_eq!(
        launched(&flow, crate::providers::CODEX).await,
        (false, Some(true))
    );
}

#[tokio::test]
async fn agent_questions_keep_asking_while_the_mode_stays_below_full() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.add_provider(crate::providers::CODEX);
    flow.submit("fix the build").await;

    for mode in ["ask", "read-only", "edit"] {
        flow.engine
            .set_permission_mode(flow.chat, mode)
            .await
            .unwrap();
    }

    assert!(flow.engine.flow.stale_connections.is_empty());
}

#[tokio::test]
async fn agent_questions_restart_an_idle_claude_connection_at_once() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::CLAUDE))
    );
    assert!(flow.engine.flow.live.is_empty());
    assert!(flow.engine.flow.stale_connections.is_empty());
    assert!(restarted(&client.window().await, crate::providers::CLAUDE));
    assert_eq!(
        launched(&flow, crate::providers::CLAUDE).await,
        (true, None)
    );
}

#[tokio::test]
async fn agent_questions_restart_a_running_claude_connection_after_the_turn() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    assert!(
        flow.engine
            .providers
            .contains_key(&(flow.chat, crate::providers::CLAUDE))
    );
    assert!(
        flow.engine
            .flow
            .stale_connections
            .contains(&(flow.chat, crate::providers::CLAUDE))
    );
    assert!(!restarted(&client.window().await, crate::providers::CLAUDE));

    flow.claude_event(turn_completed(agent)).await;

    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::CLAUDE))
    );
    assert!(flow.engine.flow.stale_connections.is_empty());
    assert!(restarted(&client.window().await, crate::providers::CLAUDE));
}

#[tokio::test]
async fn agent_questions_restart_is_dropped_when_the_mode_returns_before_the_turn_ends() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();
    flow.engine
        .set_permission_mode(flow.chat, "edit")
        .await
        .unwrap();
    flow.claude_event(turn_completed(agent)).await;

    assert!(flow.engine.flow.stale_connections.is_empty());
    assert!(
        flow.engine
            .providers
            .contains_key(&(flow.chat, crate::providers::CLAUDE))
    );
}

#[tokio::test]
async fn agent_questions_asked_before_the_claude_restart_reach_the_tui() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    flow.claude_event(input_request(agent, "ask-1")).await;

    let asked = client
        .until(|notification| match notification {
            Notification::InputRequested {
                request_id,
                provider,
                ..
            } => Some((request_id.clone(), *provider)),
            _ => None,
        })
        .await;
    assert_eq!(
        (Some(asked.0), asked.1),
        (flow.input_id("ask-1"), crate::providers::CLAUDE)
    );
    assert!(flow.input_id("ask-1").is_some());
    assert!(!answered(&flow.fake));
}

#[tokio::test]
async fn agent_questions_asked_before_codex_is_switched_off_reach_the_tui() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.add_provider(crate::providers::CODEX);
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    flow.event(crate::providers::CODEX, input_request(agent, "ask-2"))
        .await;

    let asked = client
        .until(|notification| match notification {
            Notification::InputRequested { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .await;
    assert_eq!(Some(asked), flow.input_id("ask-2"));
    assert!(
        flow.engine
            .flow
            .stale_connections
            .contains(&(flow.chat, crate::providers::CODEX))
    );
    assert!(!answered(&flow.fake));
}
