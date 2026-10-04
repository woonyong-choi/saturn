//! provider CLI 버전 감지: 시작 때 읽은 버전이 마지막으로 확인한 버전과 다르면 첫 TUI에 한 줄 알린다.
//! 설계: docs/design/providers-and-sessions.md#직접-연결과-acp-어댑터

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{Alert, Notification};

use super::support::Flow;
use crate::providers::test_support::{FakeAdapter, FakeProvider, fake_descriptor};

const FAKE: Provider = Provider::from_static("fake-agent");

/// 버전 명령에 `output`을 내는 가짜 CLI가 든 `PATH`.
fn path_with_cli(output: &str) -> (tempfile::TempDir, Vec<(OsString, OsString)>) {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("fake-agent");
    std::fs::write(&program, format!("#!/bin/sh\necho '{output}'\n")).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = vec![("PATH".into(), dir.path().as_os_str().to_owned())];
    (dir, env)
}

async fn flow_with_fake_adapter() -> Flow {
    let mut flow = Flow::new(Vec::new()).await;
    flow.engine
        .registry
        .register(Arc::new(FakeAdapter {
            descriptor: fake_descriptor(FAKE),
            provider: FakeProvider::new(FAKE),
        }))
        .unwrap();
    flow
}

fn updates(notifications: &[Notification]) -> Vec<(String, String, String)> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::Alert {
                alert: Alert::ProviderUpdated { provider, from, to },
            } => Some((provider.to_string(), from.clone(), to.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn the_first_check_records_the_version_without_a_notice() {
    let mut flow = flow_with_fake_adapter().await;
    let (_dir, env) = path_with_cli("fake-cli 1.2.3");

    flow.engine.detect_provider_versions(&env).await;

    assert!(flow.engine.notices.provider_updates.is_empty());
    let recorded = flow.engine.store.provider_cli_version(FAKE).await.unwrap();
    assert_eq!(recorded.as_deref(), Some("1.2.3"));
}

#[tokio::test]
async fn a_changed_version_is_told_once_to_the_first_tui_and_recorded() {
    let mut flow = flow_with_fake_adapter().await;
    flow.engine
        .store
        .record_provider_cli_version(FAKE, "1.0.0")
        .await
        .unwrap();
    let (_dir, env) = path_with_cli("fake-cli 1.2.3");
    flow.engine.detect_provider_versions(&env).await;

    let (_first, first_seen) = flow.attach().await;
    let (_second, second_seen) = flow.attach().await;

    assert_eq!(
        updates(&first_seen),
        vec![(
            "fake-agent".to_owned(),
            "1.0.0".to_owned(),
            "1.2.3".to_owned()
        )]
    );
    assert!(updates(&second_seen).is_empty());
    let recorded = flow.engine.store.provider_cli_version(FAKE).await.unwrap();
    assert_eq!(recorded.as_deref(), Some("1.2.3"));
}

#[tokio::test]
async fn the_same_version_is_not_told() {
    let mut flow = flow_with_fake_adapter().await;
    flow.engine
        .store
        .record_provider_cli_version(FAKE, "1.2.3")
        .await
        .unwrap();
    let (_dir, env) = path_with_cli("fake-cli 1.2.3");

    flow.engine.detect_provider_versions(&env).await;

    assert!(flow.engine.notices.provider_updates.is_empty());
}

#[tokio::test]
async fn a_cli_that_cannot_be_read_keeps_the_last_checked_version() {
    let mut flow = flow_with_fake_adapter().await;
    flow.engine
        .store
        .record_provider_cli_version(FAKE, "1.0.0")
        .await
        .unwrap();
    let env = vec![("PATH".into(), "/nonexistent".into())];

    flow.engine.detect_provider_versions(&env).await;

    assert!(flow.engine.notices.provider_updates.is_empty());
    let recorded = flow.engine.store.provider_cli_version(FAKE).await.unwrap();
    assert_eq!(recorded.as_deref(), Some("1.0.0"));
}

#[tokio::test]
async fn start_info_carries_the_detected_versions() {
    let mut flow = flow_with_fake_adapter().await;
    let (_dir, env) = path_with_cli("fake-cli 1.2.3");
    flow.engine.detect_provider_versions(&env).await;

    let versions: Vec<(String, String)> = flow
        .engine
        .provider_infos()
        .into_iter()
        .map(|info| (info.provider.to_string(), info.version))
        .collect();

    assert!(versions.contains(&("fake-agent".to_owned(), "1.2.3".to_owned())));
    assert!(versions.contains(&("codex".to_owned(), String::new())));
}
