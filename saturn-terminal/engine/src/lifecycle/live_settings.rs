//! 설정 즉시 적용 테스트: 설정이 바뀌는 순간(`/permissions`, 입력 접수 때 다시 읽기) 다시 시작이 필요한 연결을
//! 작업 중이 아니면 바로, 작업 중이면 턴 끝에 다시 시작한다.

use saturn_protocol::rpc::{ChatNotice, Notification};

use super::agent_questions::{launched, restarted};
use super::support::{CLIENT, Flow, idle_reply, turn_completed};
use crate::providers::test_support::Call;

fn permissions_changed(notifications: &[Notification]) -> bool {
    notifications.iter().any(|notification| {
        matches!(
            notification,
            Notification::ChatNotice {
                notice: ChatNotice::PermissionsChanged,
                ..
            }
        )
    })
}

fn sent(calls: Vec<Call>) -> usize {
    calls
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. } | Call::SendTurn { .. }))
        .count()
}

#[tokio::test]
async fn live_settings_idle_codex_restarts_at_once_and_the_next_input_uses_the_new_settings() {
    let mut flow = Flow::new(Vec::new()).await;
    flow.add_provider(crate::providers::test_support::CODEX);
    let mut client = flow.client().await;
    assert_eq!(
        launched(&flow, crate::providers::test_support::CODEX).await,
        (false, Some(true))
    );

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::test_support::CODEX))
    );
    assert!(flow.engine.flow.stale_connections.is_empty());
    let notices = client.window().await;
    assert!(restarted(&notices, crate::providers::test_support::CODEX));
    assert!(!permissions_changed(&notices));
    assert_eq!(
        launched(&flow, crate::providers::test_support::CODEX).await,
        (true, Some(false))
    );
}

#[tokio::test]
async fn live_settings_changed_codex_rules_restart_an_idle_chat_when_the_input_is_accepted() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let codex = flow.add_provider(crate::providers::test_support::CODEX);
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);
    flow.engine.flow.rules_of_connection.insert(
        (flow.chat, crate::providers::test_support::CODEX),
        "older-rules".to_owned(),
    );
    let mut client = flow.client().await;

    let _ = flow
        .engine
        .submit_input(CLIENT, flow.chat, 1, "add the tests".to_owned(), false)
        .await;

    assert_eq!(sent(codex.calls()), 0);
    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::test_support::CODEX))
    );
    let notices = client.window().await;
    assert!(restarted(&notices, crate::providers::test_support::CODEX));
    assert!(!permissions_changed(&notices));
}

#[tokio::test]
async fn live_settings_changed_settings_file_restarts_before_the_input_is_sent() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.fixture
        .write_user_config("[permission]\nmode = \"full\"\n");

    let _ = flow
        .engine
        .submit_input(CLIENT, flow.chat, 1, "fix the build".to_owned(), false)
        .await;

    assert_eq!(sent(flow.fake.calls()), 0);
    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::test_support::CLAUDE))
    );
    assert!(restarted(
        &client.window().await,
        crate::providers::test_support::CLAUDE
    ));
}

#[tokio::test]
async fn live_settings_running_codex_restarts_after_the_turn_and_tells_both_notices() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.add_provider(crate::providers::test_support::CODEX);
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    assert!(
        flow.engine
            .providers
            .contains_key(&(flow.chat, crate::providers::test_support::CODEX))
    );
    assert!(
        flow.engine
            .flow
            .stale_connections
            .contains(&(flow.chat, crate::providers::test_support::CODEX))
    );
    let before = client.window().await;
    assert!(permissions_changed(&before));
    assert!(!restarted(&before, crate::providers::test_support::CODEX));

    flow.event(crate::providers::test_support::CODEX, turn_completed(agent))
        .await;

    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, crate::providers::test_support::CODEX))
    );
    assert!(flow.engine.flow.stale_connections.is_empty());
    assert!(restarted(
        &client.window().await,
        crate::providers::test_support::CODEX
    ));
    assert_eq!(
        launched(&flow, crate::providers::test_support::CODEX).await,
        (true, Some(false))
    );
}

#[tokio::test]
async fn live_settings_notice_is_not_repeated_while_the_restart_waits() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let mut client = flow.client().await;

    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();
    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();

    let notices = client.window().await;
    let count = notices
        .iter()
        .filter(|notification| {
            matches!(
                notification,
                Notification::ChatNotice {
                    notice: ChatNotice::PermissionsChanged,
                    ..
                }
            )
        })
        .count();
    assert_eq!(count, 1);
}
