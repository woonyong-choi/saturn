use saturn_protocol::envelope::INVALID_PARAMS;
use std::path::PathBuf;

use saturn_protocol::ids::{ChatId, SettingsRevision};
use saturn_protocol::rpc::{Alert, PruneSkipReason, PruneSkipped};
use saturn_protocol::state::InputState;

use super::chats::chat_with_input;
use super::*;
use crate::store::NewInput;

/// 사용자 설정에 정리 기준을 둔 engine.
async fn ready_with_retention(fixture: &Fixture) -> Engine {
    fixture.write_user_config("retention.max_age_days = 1\n");
    fixture.ready().await
}

/// 3일 전에 쓰고 끝낸 채팅.
async fn old_chat(engine: &Engine, folder: &str, text: &str) -> ChatId {
    let chat = chat_with_input(engine, folder, text).await;
    finish_inputs(engine, chat).await;
    engine.store.age_chat(chat, 3).await;
    chat
}

async fn finish_inputs(engine: &Engine, chat: ChatId) {
    for (input, new, _) in engine.store.open_inputs().await.unwrap() {
        if new.chat == chat {
            engine
                .store
                .set_input_state(input, InputState::Applied, None)
                .await
                .unwrap();
        }
    }
}

struct Report {
    chats: Vec<ChatId>,
    /// 채팅 줄마다의 행 수.
    chat_rows: Vec<Option<u64>>,
    skipped: Vec<PruneSkipped>,
    rows: u64,
    is_preview: bool,
}

async fn prune(engine: &mut Engine, client: &mut Client, id: u64, yes: bool) -> Report {
    drive(engine, async {
        let (chats, skipped, rows, is_preview) =
            match client.query(id, Request::Prune { yes }).await {
                QueryResult::PrunePreview {
                    chats,
                    skipped,
                    rows,
                } => (chats, skipped, rows, true),
                QueryResult::Pruned {
                    chats,
                    skipped,
                    rows,
                } => (chats, skipped, rows, false),
                other => panic!("expected a prune report, got {other:?}"),
            };
        Report {
            chat_rows: chats.iter().map(|item| item.rows).collect(),
            chats: chats.into_iter().map(|item| item.chat).collect(),
            skipped,
            rows,
            is_preview,
        }
    })
    .await
}

#[tokio::test]
async fn prune_without_yes_previews_old_chats_and_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let recent = chat_with_input(&engine, "/work/recent", "new work").await;
    let open = chat_with_input(&engine, "/work/open", "still going").await;
    engine.store.age_chat(open, 3).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let report = prune(&mut engine, &mut client, 1, false).await;

    assert!(report.is_preview);
    assert_eq!(report.chats, [old]);
    assert_eq!(
        report.skipped,
        [PruneSkipped {
            chat: open,
            reasons: vec![PruneSkipReason::OpenInput]
        }]
    );
    assert!(report.rows > 0);
    for chat in [old, recent, open] {
        assert!(engine.store.chat_workdir(chat).await.is_ok(), "{chat:?}");
    }
    assert!(engine.store.tombstone(old).await.unwrap().is_none());
}

#[tokio::test]
async fn prune_with_yes_deletes_only_the_old_finished_chats_and_reports_them() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let recent = chat_with_input(&engine, "/work/recent", "new work").await;
    let open = chat_with_input(&engine, "/work/open", "still going").await;
    engine.store.age_chat(open, 3).await;
    let mut client = Client::connect(&fixture.socket()).await;
    let preview = prune(&mut engine, &mut client, 1, false).await;

    let report = prune(&mut engine, &mut client, 2, true).await;

    assert!(!report.is_preview);
    assert_eq!(report.chats, [old]);
    assert_eq!(report.rows, preview.rows);
    assert_eq!(
        report
            .skipped
            .iter()
            .map(|item| item.chat)
            .collect::<Vec<_>>(),
        [open]
    );
    assert!(engine.store.chat_workdir(old).await.is_err());
    assert!(engine.store.tombstone(old).await.unwrap().is_some());
    assert!(engine.store.chat_workdir(recent).await.is_ok());
    assert!(engine.store.chat_workdir(open).await.is_ok());
}

#[tokio::test]
async fn prune_lines_carry_the_row_count_of_each_chat_and_add_up_to_the_total() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    old_chat(&engine, "/work/old", "old work").await;
    let second = chat_with_input(&engine, "/work/older", "older work").await;
    engine
        .store
        .accept_input(&NewInput {
            chat: second,
            text: "and more".to_owned(),
            settings: SettingsRevision(1),
            permission: saturn_core::queue::Permission::Write,
            workdir: PathBuf::from("/work/older"),
            pinned_model: None,
            skip_relation: false,
        })
        .await
        .unwrap();
    finish_inputs(&engine, second).await;
    engine.store.age_chat(second, 3).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let preview = prune(&mut engine, &mut client, 1, false).await;
    let done = prune(&mut engine, &mut client, 2, true).await;

    for report in [&preview, &done] {
        let counts: Vec<u64> = report.chat_rows.iter().map(|rows| rows.unwrap()).collect();
        assert_eq!(counts.len(), 2);
        assert!(counts[1] > counts[0]);
        assert_eq!(counts.iter().sum::<u64>(), report.rows);
    }
}

#[tokio::test]
async fn prune_keeps_a_chat_a_tui_is_attached_to() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut viewer = Client::connect(&fixture.socket()).await;
    let mut client = Client::connect(&fixture.socket()).await;
    drive(&mut engine, async {
        viewer
            .attach(
                1,
                Request::Attach {
                    chat: Some(old),
                    workdir: fixture.workdir.display().to_string(),
                    env: Vec::new(),
                    overrides: Vec::new(),
                    add_dirs: Vec::new(),
                },
            )
            .await;
    })
    .await;
    engine.store.age_chat(old, 3).await;

    let preview = prune(&mut engine, &mut client, 1, false).await;
    let deleted = prune(&mut engine, &mut client, 2, true).await;

    assert!(preview.chats.is_empty() && deleted.chats.is_empty());
    for report in [&preview, &deleted] {
        assert_eq!(
            report.skipped,
            [PruneSkipped {
                chat: old,
                reasons: vec![PruneSkipReason::Attached]
            }]
        );
    }
    assert!(engine.store.chat_workdir(old).await.is_ok());
}

#[tokio::test]
async fn prune_without_a_retention_setting_is_refused_and_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut client = Client::connect(&fixture.socket()).await;

    let (told, refused) = drive(&mut engine, async {
        client.send(1, Request::Prune { yes: true }).await;
        (client.notification().await, client.response().await)
    })
    .await;

    assert_eq!(
        told,
        Notification::Alert {
            alert: Alert::PruneNeedsRetention
        }
    );
    assert_eq!(error_code(&refused), INVALID_PARAMS);
    assert!(engine.store.chat_workdir(old).await.is_ok());
}

/// 오래 쓰지 않아 정리 대상인 채팅, 최근 채팅, 오래됐지만 열린 입력이 있는 채팅을 둔 engine.
struct AutoPruneSetup {
    engine: Engine,
    old: ChatId,
    recent: ChatId,
    open: ChatId,
}

async fn auto_prune_setup(fixture: &Fixture, config: &str) -> AutoPruneSetup {
    fixture.write_user_config(config);
    let engine = fixture.ready().await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let recent = chat_with_input(&engine, "/work/recent", "new work").await;
    let open = chat_with_input(&engine, "/work/open", "still going").await;
    engine.store.age_chat(open, 3).await;
    AutoPruneSetup {
        engine,
        old,
        recent,
        open,
    }
}

async fn survivors(setup: &AutoPruneSetup) -> Vec<ChatId> {
    let mut found = Vec::new();
    for chat in [setup.old, setup.recent, setup.open] {
        if setup.engine.store.chat_workdir(chat).await.is_ok() {
            found.push(chat);
        }
    }
    found
}

/// 처음 붙는 TUI와 그다음 TUI가 받은 알림 가운데 자동 정리 알림.
async fn auto_prune_alerts(fixture: &Fixture, engine: &mut Engine) -> (Vec<Alert>, Vec<Alert>) {
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;
    let (first_seen, second_seen) = drive(engine, async {
        let first_seen = first.attach(1, new_chat(&fixture.workdir)).await;
        let second_seen = second.attach(1, new_chat(&fixture.workdir)).await;
        (first_seen, second_seen)
    })
    .await;
    let alerts = |seen: Vec<Notification>| {
        seen.into_iter()
            .filter_map(|notification| match notification {
                Notification::Alert { alert } => Some(alert),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    (alerts(first_seen), alerts(second_seen))
}

#[tokio::test]
async fn auto_prune_on_start_deletes_old_finished_chats_and_tells_the_first_tui_only() {
    let fixture = Fixture::new();
    let mut setup = auto_prune_setup(
        &fixture,
        "retention.max_age_days = 1\nretention.auto_prune = true\n",
    )
    .await;

    setup.engine.finish_start().await.unwrap();
    let (first, second) = auto_prune_alerts(&fixture, &mut setup.engine).await;

    assert_eq!(survivors(&setup).await, [setup.recent, setup.open]);
    assert!(
        setup
            .engine
            .store
            .tombstone(setup.old)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(first.len(), 1);
    assert!(matches!(first[0], Alert::AutoPruned { chats: 1, rows } if rows > 0));
    assert!(second.is_empty());
}

#[tokio::test]
async fn auto_prune_on_start_does_nothing_unless_the_switch_is_on_with_a_max_age() {
    for config in [
        "retention.max_age_days = 1\n",
        "retention.auto_prune = true\n",
        "retention.auto_prune = false\nretention.max_age_days = 1\n",
    ] {
        let fixture = Fixture::new();
        let mut setup = auto_prune_setup(&fixture, config).await;

        setup.engine.finish_start().await.unwrap();
        let (first, second) = auto_prune_alerts(&fixture, &mut setup.engine).await;

        assert_eq!(
            survivors(&setup).await,
            [setup.old, setup.recent, setup.open],
            "{config}"
        );
        assert!(first.is_empty() && second.is_empty(), "{config}");
    }
}

#[tokio::test]
async fn auto_prune_failure_keeps_every_chat_and_still_tells_the_first_tui() {
    let fixture = Fixture::new();
    let mut setup = auto_prune_setup(
        &fixture,
        "retention.max_age_days = 1\nretention.auto_prune = true\n",
    )
    .await;
    setup.engine.store.break_tombstones().await;

    setup.engine.finish_start().await.unwrap();
    let (first, second) = auto_prune_alerts(&fixture, &mut setup.engine).await;

    assert_eq!(
        survivors(&setup).await,
        [setup.old, setup.recent, setup.open]
    );
    assert_eq!(first, [Alert::AutoPruneFailed]);
    assert!(second.is_empty());
}
