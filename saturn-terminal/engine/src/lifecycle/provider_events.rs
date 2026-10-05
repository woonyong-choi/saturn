//! provider 원시 메시지 관측 기록 설정 테스트: 기본은 꺼짐이고, 사용자 설정으로 켜며, 설정이 바뀌면 열린 연결의 손잡이도 바로 따른다.

use super::support::Flow;
use crate::providers::{Frame, ProviderTrace};

const ON: &str = "[debug]\nprovider_events = true\n";

async fn handle_of(flow: &Flow) -> ProviderTrace {
    let revision = flow.engine.settings.current().unwrap();
    flow.engine
        .launch_spec(crate::providers::test_support::CLAUDE, flow.chat, revision)
        .await
        .unwrap()
        .events
}

fn trace_files(flow: &Flow) -> Vec<std::path::PathBuf> {
    let logs = flow.fixture.options.home.join("logs");
    std::fs::read_dir(logs)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("provider-events-"))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn provider_events_are_off_by_default_and_write_nothing() {
    let flow = Flow::new(Vec::new()).await;

    let events = handle_of(&flow).await;
    events.record(Frame::Notification, "thread/closed", &serde_json::json!({}));

    assert!(!events.is_enabled());
    assert!(trace_files(&flow).is_empty());
}

#[tokio::test]
async fn user_setting_turns_provider_events_on() {
    let flow = Flow::with_config(ON, Vec::new()).await;

    let events = handle_of(&flow).await;
    events.record(Frame::Notification, "thread/closed", &serde_json::json!({}));

    assert!(events.is_enabled());
    assert_eq!(trace_files(&flow).len(), 1);
}

#[tokio::test]
async fn changing_the_setting_reaches_an_open_connection_without_restarting_it() {
    let mut flow = Flow::new(Vec::new()).await;
    let events = handle_of(&flow).await;
    assert!(!events.is_enabled());
    flow.fixture.write_user_config(ON);

    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;

    assert!(events.is_enabled());
    flow.fixture
        .write_user_config("[debug]\nprovider_events = false\n");
    flow.engine.watch_settings().await;
    flow.engine.watch_settings().await;
    assert!(!events.is_enabled());
}
