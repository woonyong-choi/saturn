use std::ffi::OsString;

use super::*;
use crate::RouterGate;
use crate::rpc::{EngineLock, LOCK_FILE, RpcError};
use crate::settings::SettingsError;
use crate::store::DB_FILE;

#[tokio::test]
async fn start_runs_every_step_and_opens_socket() {
    let fixture = Fixture::new();

    let engine = fixture.ready().await;

    assert!(fixture.options.home.join(LOCK_FILE).exists());
    assert!(fixture.options.home.join(DB_FILE).exists());
    assert!(engine.settings.current().is_some());
    assert_eq!(engine.router_gate, RouterGate::Open);
    assert!(fixture.socket().exists());
}

#[tokio::test]
async fn default_manual_router_starts_without_a_key_or_external_check() {
    let fixture = Fixture::new();
    let env = StartEnv {
        nested_marker: None,
        router: None,
        secrets: Some(fixture.secrets(false).await),
        key_inputs: Some(Vec::new()),
    };

    let engine = fixture.start(env).await.unwrap();

    assert_eq!(
        engine.routers.method(),
        saturn_core::routers::Method::Manual
    );
    assert_eq!(engine.router_gate, RouterGate::Open);
    assert!(fixture.socket().exists());
    assert!(!fixture.key_file().exists());
}

#[tokio::test]
async fn start_nested_is_refused_before_lock() {
    let fixture = Fixture::new();
    let transport = FakeTransport::new(Vec::new());
    let mut env = fixture.env(true, Arc::clone(&transport)).await;
    env.nested_marker = Some(OsString::from("1"));

    let error = fixture.start(env).await.unwrap_err();

    assert!(matches!(error, EngineError::Nested));
    assert!(!fixture.options.home.exists());
}

#[tokio::test]
async fn start_with_running_engine_stops_before_store() {
    let fixture = Fixture::new();
    let _running = EngineLock::acquire(&fixture.options.home).unwrap();
    let transport = FakeTransport::new(Vec::new());

    let error = fixture
        .start(fixture.env(true, Arc::clone(&transport)).await)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EngineError::Rpc(RpcError::AlreadyRunning { .. })
    ));
    assert!(!fixture.options.home.join(DB_FILE).exists());
}

#[tokio::test]
async fn start_invalid_settings_without_revision_stops_before_router_and_socket() {
    let fixture = Fixture::new();
    fixture.write_user_config("broken = = 1\n");
    let transport = FakeTransport::new(check_passes());

    let error = fixture
        .start(fixture.env(true, Arc::clone(&transport)).await)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EngineError::Settings(SettingsError::NoPreviousRevision)
    ));
    assert!(transport.calls().is_empty());
    assert!(!fixture.socket().exists());
}

#[tokio::test]
async fn start_disallowed_router_stops_before_socket() {
    let fixture = Fixture::new();
    fixture.write_user_config("[router]\nmode = \"jev\"\nendpoint = \"https://evil.example\"\n");
    let mut env = fixture.env(true, FakeTransport::new(Vec::new())).await;
    env.router = None;

    let error = fixture.start(env).await.unwrap_err();

    assert!(
        matches!(error, EngineError::RouterUnavailable { ref reason } if reason.contains("evil.example"))
    );
    assert!(!fixture.socket().exists());
}

#[tokio::test]
async fn start_without_key_opens_socket_and_waits_for_key() {
    let fixture = Fixture::new();

    let engine = fixture.waiting_for_key(Vec::new()).await;

    assert!(matches!(
        engine.router_gate,
        RouterGate::KeyRequired { ref reason } if !reason.is_empty()
    ));
    assert!(fixture.socket().exists());
}

#[tokio::test]
async fn start_tries_automatic_key_inputs_before_waiting() {
    let fixture = Fixture::new();
    let transport = FakeTransport::new(check_passes());
    let mut env = fixture.env(false, Arc::clone(&transport)).await;
    // 자식 프로세스는 병렬 테스트의 잠금 fd를 잠시 물려받을 수 있어 관리자 명령 대신 쓴다.
    env.key_inputs = Some(vec![
        KeyInput::Hidden("  \n".to_owned()),
        KeyInput::Hidden(KEY.to_owned()),
    ]);

    let engine = fixture.start(env).await.unwrap();

    assert_eq!(engine.router_gate, RouterGate::Open);
    assert_eq!(transport.calls().len(), 2);
    assert_eq!(std::fs::read_to_string(fixture.key_file()).unwrap(), KEY);
    assert_eq!(engine.masker.mask(KEY).as_str(), crate::secrets::REDACTED);
}

#[tokio::test]
async fn shutdown_removes_socket_and_releases_lock() {
    let fixture = Fixture::new();
    let engine = fixture.ready().await;

    engine.shutdown().await.unwrap();

    assert!(!fixture.socket().exists());
    // 병렬 테스트가 띄운 자식이 exec 전까지 잠금 fd를 잠시 물려받을 수 있어 조금 기다린다.
    let mut acquired = EngineLock::acquire(&fixture.options.home);
    for _ in 0..50 {
        if acquired.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        acquired = EngineLock::acquire(&fixture.options.home);
    }
    assert!(acquired.is_ok());
}

// #525: 종료 신호를 받으면 요청 처리를 끝내고, 정리 단계가 멈춤 유예 없이 provider 묶음의 자식까지 끝낸다
#[tokio::test]
async fn terminate_signal_ends_serve_and_leaves_no_provider_child() {
    use crate::processes::{ProcessGroupId, ProcessSpec};
    use tokio::io::{AsyncBufReadExt, BufReader};

    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut spawned = engine
        .supervisor
        .spawn(ProcessSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".to_owned(),
                "sleep 60 & echo $!; trap '' TERM; wait".to_owned(),
            ],
            workdir: fixture.workdir.clone(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        })
        .unwrap();
    let mut line = String::new();
    BufReader::new(&mut spawned.io.stdout)
        .read_line(&mut line)
        .await
        .unwrap();
    let grandchild: i32 = line.trim().parse().unwrap();
    let group: ProcessGroupId = spawned.group;

    engine.terminate.notify_one();
    let served = timeout(WAIT, engine.serve()).await;
    assert!(matches!(served, Ok(Ok(()))));
    let started = std::time::Instant::now();
    engine.shutdown().await.unwrap();

    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(
        !is_alive(i32::try_from(group.0).unwrap()),
        "the leader should be gone"
    );
    assert!(!is_alive(grandchild), "the child should be gone");
}

/// 신호 0으로 프로세스가 남았는지 본다.
#[expect(unsafe_code, reason = "libc kill 호출")]
fn is_alive(pid: i32) -> bool {
    // SAFETY: 신호 0은 존재 확인만 하는 `kill` 호출이고 인자는 정수다
    unsafe { libc::kill(pid, 0) == 0 }
}

// #491
#[tokio::test]
async fn restart_with_broken_user_settings_does_not_adopt_a_connection_layer() {
    let fixture = Fixture::new();
    fixture.write_user_config("[permission]\nmode = \"read-only\"\n");
    let mut engine = fixture.ready().await;
    let user_revision = engine.settings.current().unwrap();
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let (full, _) = engine
        .settings
        .apply_trusted(
            &engine.store,
            Some(chat),
            &fixture.workdir,
            &["permission.mode=full".to_owned()],
        )
        .await
        .unwrap();
    assert_ne!(full.revision, user_revision);
    drop(engine);
    fixture.write_user_config("broken = = 1\n");

    let transport = FakeTransport::new(check_passes());
    let engine = fixture
        .start(fixture.env(true, Arc::clone(&transport)).await)
        .await
        .unwrap();

    let current = engine.settings.current().unwrap();
    let mode = engine
        .settings
        .at(&engine.store, current)
        .await
        .unwrap()
        .permission()
        .mode;
    assert_eq!(current, user_revision);
    assert_eq!(mode, saturn_core::permission::Mode::ReadOnly);
}
