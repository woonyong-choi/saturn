use std::ffi::OsString;

use saturn_core::queue::Permission;
use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{
    AgentId, ChatId, LedgerSeq, Provider, SessionId, SettingsRevision, TaskId, TaskLabel,
};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::{EffectScope, InputState, SessionState, TaskState};

use super::*;
use crate::secrets::ROUTER_KEY_ENV;
use crate::store::{MigrationNotice, NewInput, NewRun, RunEnd};

fn attach_to(chat: ChatId, workdir: &Path) -> Request {
    Request::Attach {
        chat: Some(chat),
        workdir: workdir.display().to_string(),
        env: tui_env(),
        overrides: vec![("model".to_owned(), "fast".to_owned())],
        add_dirs: Vec::new(),
    }
}

fn tui_env() -> Vec<(String, String)> {
    vec![
        ("PATH".to_owned(), "/opt/tui/bin:/usr/bin".to_owned()),
        (ROUTER_KEY_ENV.to_owned(), "sk-from-tui".to_owned()),
    ]
}

fn permission() -> Notification {
    Notification::PermissionRequested {
        task: TaskId(1),
        label: TaskLabel('A'),
        provider: Provider::Codex,
        request_id: "p1".to_owned(),
        summary: "rm -rf target".to_owned(),
        reason: "cleanup".to_owned(),
        waiting: 0,
    }
}

fn text(text: &str) -> ProviderEvent {
    ProviderEvent::Text {
        agent: AgentId(1),
        subagent: None,
        text: text.to_owned(),
    }
}

/// 입력 하나와 그 실행의 이벤트 하나가 있는 채팅.
async fn chat_with_history(engine: &Engine, workdir: &Path) -> ChatId {
    let store = &engine.store;
    let chat = store.create_chat(workdir.to_path_buf()).await.unwrap();
    let input = store
        .accept_input(&NewInput {
            chat,
            text: "fix the build".to_owned(),
            settings: SettingsRevision(1),
            permission: Permission::Write,
            workdir: workdir.to_path_buf(),
            pinned_model: None,
            skip_relation: false,
        })
        .await
        .unwrap();
    let session = SessionId(7);
    store
        .upsert_session(&SessionRecord {
            id: session,
            chat,
            agent: AgentId(1),
            role: AgentRole::Main,
            provider: Provider::Codex,
            provider_session: None,
            model: None,
            state: SessionState::Open,
            delivered: LedgerSeq(0),
            idle_since: None,
        })
        .await
        .unwrap();
    let run = store
        .start_run(&NewRun {
            input: Some(input),
            task: TaskId(1),
            agent: AgentId(1),
            session,
            provider: Provider::Codex,
            effect_scope: EffectScope::NetworkPossible,
        })
        .await
        .unwrap();
    store.append_event(run, chat, &text("done")).await.unwrap();
    chat
}

#[tokio::test]
async fn attach_sends_start_info_then_history_then_permissions() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = chat_with_history(&engine, &fixture.workdir).await;
    engine.rpc.offer_permission(chat, permission()).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, attach_to(chat, &fixture.workdir)).await
    })
    .await;

    assert_eq!(received.len(), 3);
    let Notification::StartInfo {
        router,
        router_version,
        folder,
        ..
    } = &received[0]
    else {
        panic!("expected StartInfo first, got {:?}", received[0]);
    };
    assert_eq!(router, "jev");
    assert_eq!(router_version, "jev-1.13.0");
    assert_eq!(folder, &fixture.workdir.display().to_string());
    let Notification::HistoryChunk {
        chat: history_chat,
        entries,
        has_more,
    } = &received[1]
    else {
        panic!("expected HistoryChunk second, got {:?}", received[1]);
    };
    assert_eq!(*history_chat, chat);
    assert!(!has_more);
    assert!(matches!(
        &entries[0],
        Notification::InputChanged { text, state: InputState::Judging, .. } if text == "fix the build"
    ));
    assert!(matches!(
        &entries[1],
        Notification::TaskChanged {
            task: TaskId(1),
            state: TaskState::Running,
            ..
        }
    ));
    assert_eq!(
        entries[2],
        Notification::TaskEvent {
            task: TaskId(1),
            event: text("done"),
        }
    );
    assert_eq!(received[2], permission());
    let attachment = &engine.attachments.values().next().unwrap();
    assert_eq!(attachment.chat, chat);
    assert_eq!(
        attachment.overrides,
        vec![("model".to_owned(), "fast".to_owned())]
    );
    let chat_env = engine.chat_env(chat).unwrap();
    assert_eq!(chat_env.workdir(), fixture.workdir);
    assert_eq!(
        chat_env.provider_env(),
        vec![(
            OsString::from("PATH"),
            OsString::from("/opt/tui/bin:/usr/bin")
        )]
    );
}

/// 입력과 끝난 실행 하나. 답은 글자 조각 `pieces`개로 쌓인다.
async fn add_finished_exchange(
    engine: &Engine,
    chat: ChatId,
    workdir: &Path,
    task: u64,
    pieces: usize,
) {
    let store = &engine.store;
    let input = store
        .accept_input(&NewInput {
            chat,
            text: format!("question {task}"),
            settings: SettingsRevision(1),
            permission: Permission::Write,
            workdir: workdir.to_path_buf(),
            pinned_model: None,
            skip_relation: false,
        })
        .await
        .unwrap();
    let run = store
        .start_run(&NewRun {
            input: Some(input),
            task: TaskId(task),
            agent: AgentId(1),
            session: SessionId(7),
            provider: Provider::Codex,
            effect_scope: EffectScope::NetworkPossible,
        })
        .await
        .unwrap();
    for piece in 0..pieces {
        let event = text(&format!("answer {task} piece {piece}\n"));
        store.append_event(run, chat, &event).await.unwrap();
    }
    store.finish_run(run, RunEnd::Completed).await.unwrap();
}

/// 답이 글자 조각 수십 개여도 끝 몇 개 조각이 앞 입력과 답을 밀어내지 않는다(#384).
#[tokio::test]
async fn attach_history_keeps_earlier_inputs_and_replies_when_the_last_reply_is_long() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let chat = chat_with_history(&engine, &fixture.workdir).await;
    add_finished_exchange(&engine, chat, &fixture.workdir, 2, 80).await;
    add_finished_exchange(&engine, chat, &fixture.workdir, 3, 80).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, attach_to(chat, &fixture.workdir)).await
    })
    .await;

    let Notification::HistoryChunk {
        entries, has_more, ..
    } = &received[1]
    else {
        panic!("expected HistoryChunk, got {:?}", received[1]);
    };
    assert!(!has_more);
    let inputs: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            Notification::InputChanged { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(inputs, vec!["fix the build", "question 2", "question 3"]);
    let pieces = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                Notification::TaskEvent {
                    task: TaskId(2),
                    ..
                }
            )
        })
        .count();
    assert_eq!(pieces, 80);
}

#[tokio::test]
async fn attach_keeps_first_workdir_and_latest_tui_env() {
    let fixture = Fixture::new();
    let other = fixture.root.path().join("other");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    let mut engine = fixture.ready().await;
    let first_chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let second_chat = engine.store.create_chat(other.clone()).await.unwrap();
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;
    let mut third = Client::connect(&fixture.socket()).await;
    let moved = Request::Attach {
        chat: Some(first_chat),
        workdir: other.display().to_string(),
        env: vec![("PATH".to_owned(), "/second/bin".to_owned())],
        overrides: Vec::new(),
        add_dirs: Vec::new(),
    };

    drive(&mut engine, async {
        first
            .attach(1, attach_to(first_chat, &fixture.workdir))
            .await;
        second.attach(1, attach_to(second_chat, &other)).await;
        third.attach(1, moved).await;
    })
    .await;

    assert_eq!(
        engine.chat_env(first_chat).unwrap().workdir(),
        fixture.workdir
    );
    assert_eq!(
        engine.store.chat_workdir(first_chat).await.unwrap(),
        fixture.workdir
    );
    assert_eq!(
        engine.chat_env(first_chat).unwrap().provider_env(),
        vec![(OsString::from("PATH"), OsString::from("/second/bin"))]
    );
    assert_eq!(engine.chat_env(second_chat).unwrap().workdir(), other);
    assert!(engine.chat_env(ChatId(99)).is_none());
}

#[tokio::test]
async fn existing_chat_uses_first_folder_for_trust_even_from_another_folder() {
    let fixture = Fixture::new();
    fixture.write_folder_config("[router.thresholds]\ninjection = 0.9\n");
    let other = fixture.root.path().join("other");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    let mut engine = fixture.ready().await;
    let chat = engine
        .store
        .create_chat(fixture.workdir.clone())
        .await
        .unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, attach_to(chat, &other)).await
    })
    .await;

    assert!(matches!(
        received.last(),
        Some(Notification::FolderTrustRequested { .. })
    ));
    assert!(matches!(
        &received[0],
        Notification::StartInfo { folder, .. } if folder == &fixture.workdir.display().to_string()
    ));
}

#[tokio::test]
async fn folder_trust_is_asked_per_chat_workdir() {
    let fixture = Fixture::new();
    fixture.write_folder_config("[router.thresholds]\ninjection = 0.9\n");
    let other = fixture.root.path().join("other");
    std::fs::create_dir_all(other.join(".git")).unwrap();
    let mut engine = fixture.ready().await;
    let mut with_config = Client::connect(&fixture.socket()).await;
    let mut without_config = Client::connect(&fixture.socket()).await;

    let (asked, not_asked) = drive(&mut engine, async {
        let asked = with_config.attach(1, new_chat(&fixture.workdir)).await;
        let not_asked = without_config.attach(1, new_chat(&other)).await;
        (asked, not_asked)
    })
    .await;

    assert!(matches!(
        asked.last(),
        Some(Notification::FolderTrustRequested { .. })
    ));
    assert!(
        !not_asked
            .iter()
            .any(|notification| matches!(notification, Notification::FolderTrustRequested { .. }))
    );
}

#[tokio::test]
async fn attach_without_chat_creates_new_chat() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, new_chat(&fixture.workdir)).await
    })
    .await;

    assert!(matches!(received[0], Notification::StartInfo { .. }));
    let Notification::HistoryChunk { chat, entries, .. } = &received[1] else {
        panic!("expected HistoryChunk, got {:?}", received[1]);
    };
    assert!(entries.is_empty());
    assert!(engine.store.chat_layer(*chat).await.is_ok());
}

#[tokio::test]
async fn attach_missing_chat_returns_error() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let response = drive(&mut engine, async {
        client
            .send(1, attach_to(ChatId(42), &fixture.workdir))
            .await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
    assert!(engine.attachments.is_empty());
}

#[tokio::test]
async fn attach_while_waiting_for_key_asks_for_key_after_greeting() {
    let fixture = Fixture::new();
    let mut engine = fixture.waiting_for_key(Vec::new()).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, new_chat(&fixture.workdir)).await
    })
    .await;

    assert!(matches!(
        &received[0],
        Notification::StartInfo { router_version, .. } if router_version.is_empty()
    ));
    assert!(matches!(received[1], Notification::HistoryChunk { .. }));
    assert!(matches!(
        &received[2],
        Notification::RouterKeyRequired { reason } if !reason.is_empty()
    ));
    assert_eq!(received.len(), 3);
}

#[tokio::test]
async fn attach_asks_folder_trust_and_answer_applies_folder_settings() {
    let fixture = Fixture::new();
    fixture.write_folder_config("[router.thresholds]\ninjection = 0.9\n");
    let mut engine = fixture.ready().await;
    let before = engine.settings.current().unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let (prompt, applied) = drive(&mut engine, async {
        let received = client.attach(1, new_chat(&fixture.workdir)).await;
        let prompt = received.last().unwrap().clone();
        let Notification::FolderTrustRequested {
            path, fingerprint, ..
        } = prompt.clone()
        else {
            panic!("expected FolderTrustRequested last, got {prompt:?}");
        };
        client
            .send(
                2,
                Request::AnswerFolderTrust {
                    path,
                    fingerprint,
                    apply: true,
                },
            )
            .await;
        let applied = client.notification().await;
        assert_eq!(client.response().await, Response::ok(RequestId(2)));
        (prompt, applied)
    })
    .await;

    assert!(matches!(prompt, Notification::FolderTrustRequested { .. }));
    let Notification::SettingsApplied { revision, .. } = applied else {
        panic!("expected SettingsApplied, got {applied:?}");
    };
    assert_ne!(revision, before);
    let settings = engine.settings.at(&engine.store, revision).await.unwrap();
    assert_eq!(settings.thresholds().injection, 0.9);
    assert!(
        engine
            .attachments
            .values()
            .all(|attachment| attachment.folder_trust.is_none())
    );
}

#[tokio::test]
async fn folder_trust_answer_for_unasked_path_is_refused() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let response = drive(&mut engine, async {
        client
            .send(
                1,
                Request::AnswerFolderTrust {
                    path: "/elsewhere/.saturn/config.toml".to_owned(),
                    fingerprint: "00".to_owned(),
                    apply: true,
                },
            )
            .await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
}

#[tokio::test]
async fn attach_sends_schema_migration_alert_to_first_tui_only() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    engine.notices.migration = Some(MigrationNotice {
        from: 1,
        to: 2,
        backup: fixture.root.path().join("backup.db"),
    });
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;

    let (first_seen, second_seen) = drive(&mut engine, async {
        let first_seen = first.attach(1, new_chat(&fixture.workdir)).await;
        let second_seen = second.attach(1, new_chat(&fixture.workdir)).await;
        (first_seen, second_seen)
    })
    .await;

    let alert = Notification::Alert {
        alert: Alert::SchemaMigrated { from: 1, to: 2 },
    };
    assert_eq!(first_seen.last(), Some(&alert));
    assert!(!second_seen.contains(&alert));
}
