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
    /// 미리보기가 알린 확인 번호.
    plan: Option<String>,
}

/// 확인 번호 없이 요청한다. 확인(`yes`)은 요청 순간의 기준으로 대상을 정해 지운다.
async fn prune(engine: &mut Engine, client: &mut Client, id: u64, yes: bool) -> Report {
    prune_with(engine, client, id, yes, None).await
}

/// 미리보기가 알린 번호를 실어 확인한다.
async fn confirm(engine: &mut Engine, client: &mut Client, id: u64, preview: &Report) -> Report {
    prune_with(engine, client, id, true, preview.plan.clone()).await
}

async fn prune_with(
    engine: &mut Engine,
    client: &mut Client,
    id: u64,
    yes: bool,
    plan: Option<String>,
) -> Report {
    drive(engine, async {
        let (chats, skipped, rows, is_preview, plan) = match client
            .query(
                id,
                Request::Prune {
                    all: yes && plan.is_none(),
                    yes,
                    plan,
                },
            )
            .await
        {
            QueryResult::PrunePreview {
                chats,
                skipped,
                rows,
                plan,
            } => (chats, skipped, rows, true, Some(plan)),
            QueryResult::Pruned {
                chats,
                skipped,
                rows,
            } => (chats, skipped, rows, false, None),
            other => panic!("expected a prune report, got {other:?}"),
        };
        Report {
            chat_rows: chats.iter().map(|item| item.rows).collect(),
            chats: chats.into_iter().map(|item| item.chat).collect(),
            skipped,
            rows,
            is_preview,
            plan,
        }
    })
    .await
}

/// 확인 번호를 실은 `Prune`이 거절됐는지.
async fn refused_with_plan(engine: &mut Engine, client: &mut Client, id: u64, plan: &str) -> bool {
    let plan = Some(plan.to_owned());
    let response = drive(engine, async {
        client
            .send(
                id,
                Request::Prune {
                    yes: true,
                    plan,
                    all: false,
                },
            )
            .await;
        client.response().await
    })
    .await;
    error_code(&response) == INVALID_PARAMS
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
        client
            .send(
                1,
                Request::Prune {
                    yes: true,
                    plan: None,
                    all: true,
                },
            )
            .await;
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

// #457
#[tokio::test]
async fn prune_confirmation_keeps_a_previewed_chat_that_changed_since() {
    #[derive(Clone, Copy)]
    enum Change {
        UsedAgain,
        OpenInput,
        TuiAttached,
    }
    let cases = [
        ("used again", Change::UsedAgain),
        ("got an open input", Change::OpenInput),
        ("a tui attached afterwards", Change::TuiAttached),
    ];

    for (name, change) in cases {
        let fixture = Fixture::new();
        let mut engine = ready_with_retention(&fixture).await;
        let stays = old_chat(&engine, "/work/stays", "changed work").await;
        let goes = match change {
            Change::UsedAgain => Some(old_chat(&engine, "/work/goes", "left alone").await),
            _ => None,
        };
        let mut viewer = Client::connect(&fixture.socket()).await;
        let mut client = Client::connect(&fixture.socket()).await;
        let preview = prune(&mut engine, &mut client, 1, false).await;
        let mut previewed = vec![stays];
        previewed.extend(goes);
        assert_eq!(preview.chats, previewed, "{name}");
        match change {
            Change::UsedAgain | Change::OpenInput => {
                engine
                    .store
                    .accept_input(&NewInput {
                        chat: stays,
                        text: "back again".to_owned(),
                        settings: SettingsRevision(1),
                        permission: saturn_core::queue::Permission::Write,
                        workdir: PathBuf::from("/work/stays"),
                        pinned_model: None,
                        skip_relation: false,
                    })
                    .await
                    .unwrap();
            }
            Change::TuiAttached => {
                drive(&mut engine, async {
                    viewer
                        .attach(
                            1,
                            Request::Attach {
                                chat: Some(stays),
                                workdir: fixture.workdir.display().to_string(),
                                env: Vec::new(),
                                overrides: Vec::new(),
                                add_dirs: Vec::new(),
                            },
                        )
                        .await;
                })
                .await;
            }
        }
        match change {
            Change::UsedAgain => finish_inputs(&engine, stays).await,
            Change::OpenInput | Change::TuiAttached => engine.store.age_chat(stays, 3).await,
        }

        let deleted = confirm(&mut engine, &mut client, 2, &preview).await;

        let reason = match change {
            Change::UsedAgain => PruneSkipReason::UsedSincePreview,
            Change::OpenInput => PruneSkipReason::OpenInput,
            Change::TuiAttached => PruneSkipReason::Attached,
        };
        assert_eq!(
            deleted.chats,
            goes.into_iter().collect::<Vec<_>>(),
            "{name}"
        );
        assert_eq!(
            deleted.skipped,
            [PruneSkipped {
                chat: stays,
                reasons: vec![reason]
            }],
            "{name}"
        );
        assert!(engine.store.chat_workdir(stays).await.is_ok(), "{name}");
    }
}

// #457
#[tokio::test]
async fn prune_confirmation_deletes_only_the_previewed_chats() {
    // (사례, 미리보기 전에 만들어 두고 뒤에 오래된 채팅으로 만드는가, 다른 접속이 확인하는가)
    let cases = [
        ("a chat that aged after the preview", true, false),
        (
            "a chat created after the preview, confirmed elsewhere",
            false,
            true,
        ),
    ];

    for (name, aged_after_preview, other_connection) in cases {
        let fixture = Fixture::new();
        let mut engine = ready_with_retention(&fixture).await;
        let old = old_chat(&engine, "/work/previewed", "previewed work").await;
        let early = if aged_after_preview {
            let chat = chat_with_input(&engine, "/work/later", "later work").await;
            finish_inputs(&engine, chat).await;
            Some(chat)
        } else {
            None
        };
        let mut previewer = Client::connect(&fixture.socket()).await;
        let mut deleter = Client::connect(&fixture.socket()).await;
        let preview = prune(&mut engine, &mut previewer, 1, false).await;
        assert_eq!(preview.chats, [old], "{name}");
        let later = match early {
            Some(chat) => {
                engine.store.age_chat(chat, 3).await;
                chat
            }
            None => old_chat(&engine, "/work/later", "later work").await,
        };

        let deleted = if other_connection {
            confirm(&mut engine, &mut deleter, 1, &preview).await
        } else {
            confirm(&mut engine, &mut previewer, 2, &preview).await
        };

        assert_eq!(
            deleted.chats, preview.chats,
            "{name}: confirmation must not expand the previewed deletion set"
        );
        assert!(engine.store.chat_workdir(later).await.is_ok(), "{name}");
    }
}

// #457
#[tokio::test]
async fn a_prune_plan_works_once_and_a_made_up_one_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut client = Client::connect(&fixture.socket()).await;
    let preview = prune(&mut engine, &mut client, 1, false).await;
    let id = preview.plan.clone().unwrap();

    assert!(refused_with_plan(&mut engine, &mut client, 2, "not-a-plan").await);
    assert!(engine.store.chat_workdir(old).await.is_ok());
    let deleted = confirm(&mut engine, &mut client, 3, &preview).await;
    assert_eq!(deleted.chats, [old]);

    assert!(refused_with_plan(&mut engine, &mut client, 4, &id).await);
}

// #457
#[tokio::test]
async fn confirming_without_a_plan_decides_the_targets_at_that_moment() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut client = Client::connect(&fixture.socket()).await;
    let _ = prune(&mut engine, &mut client, 1, false).await;
    let later = old_chat(&engine, "/work/later", "later work").await;

    let deleted = prune(&mut engine, &mut client, 2, true).await;

    assert_eq!(deleted.chats, [old, later]);
}

// #457: 확인 번호를 모르는 옛 클라이언트가 미리보기 뒤에 보내는 `Prune { yes: true }`.
#[tokio::test]
async fn a_confirmation_from_a_client_without_plan_support_deletes_nothing() {
    let fixture = Fixture::new();
    let mut engine = ready_with_retention(&fixture).await;
    let old = old_chat(&engine, "/work/old", "old work").await;
    let mut client = Client::connect(&fixture.socket()).await;
    let _ = prune(&mut engine, &mut client, 1, false).await;
    let later = old_chat(&engine, "/work/later", "later work").await;
    let legacy: Request =
        serde_json::from_value(serde_json::json!({ "method": "Prune", "params": { "yes": true } }))
            .unwrap();

    let response = drive(&mut engine, async {
        client.send(2, legacy).await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
    for chat in [old, later] {
        assert!(engine.store.chat_workdir(chat).await.is_ok(), "{chat:?}");
    }
}
