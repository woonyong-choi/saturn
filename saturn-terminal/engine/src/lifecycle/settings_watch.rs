//! 설정 파일 감시 테스트: 입력 없이도 파일이 바뀌면 적용하고, 쓰는 도중의 파일은 적용하지 않는다.

use saturn_protocol::rpc::{ChatNotice, Notification, SettingsWarning};

use super::agent_questions::restarted;
use super::support::{Flow, idle_reply, turn_completed};

const FULL: &str = "[permission]\nmode = \"full\"\n";

fn connected(flow: &Flow) -> bool {
    flow.engine
        .providers
        .contains_key(&(flow.chat, crate::providers::CLAUDE))
}

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

#[tokio::test]
async fn settings_watch_restarts_an_idle_chat_without_any_input() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    flow.fixture.write_user_config(FULL);

    flow.engine.watch_settings().await;
    assert!(connected(&flow));

    flow.engine.watch_settings().await;

    assert!(!connected(&flow));
    assert!(restarted(&client.window().await, crate::providers::CLAUDE));
}

#[tokio::test]
async fn settings_watch_waits_for_the_turn_end_when_the_chat_is_running() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.fixture.write_user_config(FULL);

    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;

    assert!(connected(&flow));
    let before = client.window().await;
    assert!(permissions_changed(&before));
    assert!(!restarted(&before, crate::providers::CLAUDE));

    flow.event(crate::providers::CLAUDE, turn_completed(agent))
        .await;

    assert!(!connected(&flow));
    assert!(restarted(&client.window().await, crate::providers::CLAUDE));
}

#[tokio::test]
async fn settings_watch_ignores_a_file_that_is_still_being_written() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    flow.fixture.write_user_config(FULL);
    flow.engine.watch_settings().await;
    flow.fixture
        .write_user_config("[permission]\nmode = \"ask\"\n");

    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;

    assert!(connected(&flow));
    assert!(client.window().await.is_empty());
}

#[tokio::test]
async fn settings_watch_keeps_the_connection_and_warns_when_the_file_is_invalid() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    flow.fixture.write_user_config("[permission\nmode = ");

    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;

    assert!(connected(&flow));
    let notices = client.window().await;
    let warnings = notices
        .iter()
        .filter(|notification| {
            matches!(
                notification,
                Notification::SettingsApplied {
                    warning: Some(SettingsWarning::Fallback { .. }),
                    ..
                }
            )
        })
        .count();
    assert_eq!(warnings, 1);
    assert!(!restarted(&notices, crate::providers::CLAUDE));
}

#[tokio::test]
async fn settings_watch_runs_in_the_serve_loop() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    flow.fixture.write_user_config(FULL);

    let provider = super::drive(&mut flow.engine, async {
        client
            .until(|notification| match notification {
                Notification::ChatNotice {
                    notice: ChatNotice::ProviderRestarted { provider },
                    ..
                } => Some(*provider),
                _ => None,
            })
            .await
    })
    .await;

    assert_eq!(provider, crate::providers::CLAUDE);
}
