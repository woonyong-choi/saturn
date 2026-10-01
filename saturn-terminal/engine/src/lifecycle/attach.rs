use saturn_core::queue::Permission;
use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{
    AgentId, ChatId, LedgerSeq, Provider, SessionId, SettingsRevision, TaskId, TaskLabel,
};
use saturn_protocol::state::{EffectScope, InputState, SessionState};

use super::*;
use crate::store::{NewInput, NewRun};

fn attach_to(chat: ChatId, workdir: &Path) -> Request {
    Request::Attach {
        chat: Some(chat),
        workdir: workdir.display().to_string(),
        overrides: vec![("model".to_owned(), "fast".to_owned())],
    }
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
    let chat = chat_with_history(&engine, &fixture.options.workdir).await;
    engine.rpc.offer_permission(chat, permission()).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client
            .attach(1, attach_to(chat, &fixture.options.workdir))
            .await
    })
    .await;

    assert_eq!(received.len(), 3);
    let Notification::StartInfo {
        judge,
        judge_version,
        folder,
        ..
    } = &received[0]
    else {
        panic!("expected StartInfo first, got {:?}", received[0]);
    };
    assert_eq!(judge, "jev");
    assert_eq!(judge_version, "jev-1.13.0");
    assert_eq!(folder, &fixture.options.workdir.display().to_string());
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
    assert_eq!(
        entries[1],
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
}

#[tokio::test]
async fn attach_without_chat_creates_new_chat() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let received = drive(&mut engine, async {
        client.attach(1, new_chat(&fixture.options.workdir)).await
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
            .send(1, attach_to(ChatId(42), &fixture.options.workdir))
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
        client.attach(1, new_chat(&fixture.options.workdir)).await
    })
    .await;

    assert!(matches!(
        &received[0],
        Notification::StartInfo { judge_version, .. } if judge_version.is_empty()
    ));
    assert!(matches!(received[1], Notification::HistoryChunk { .. }));
    assert!(matches!(
        &received[2],
        Notification::JudgeKeyRequired { reason } if !reason.is_empty()
    ));
    assert_eq!(received.len(), 3);
}

#[tokio::test]
async fn attach_asks_folder_trust_and_answer_applies_folder_settings() {
    let fixture = Fixture::new();
    fixture.write_folder_config("[judge.thresholds]\ninjection = 0.9\n");
    let mut engine = fixture.ready().await;
    let before = engine.settings.current().unwrap();
    let mut client = Client::connect(&fixture.socket()).await;

    let (prompt, applied) = drive(&mut engine, async {
        let received = client.attach(1, new_chat(&fixture.options.workdir)).await;
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
    assert!(engine.notices.folder_trust.is_none());
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
