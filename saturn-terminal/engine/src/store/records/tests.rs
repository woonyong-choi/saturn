use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::event::{ProviderEvent, TurnOrigin, UsageScope};
use saturn_protocol::ids::{LedgerSeq, ProviderSessionId};
use saturn_protocol::rpc::UsageRange;
use saturn_protocol::state::SessionState;

use crate::store::HistoryEntry;

use super::*;
use crate::store::tests::temp_store;

pub(crate) async fn chat_with_run(store: &Store) -> (ChatId, InputId, SessionId, RunId) {
    let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
    let input = store
        .accept_input(&new_input(chat, "fix the build"))
        .await
        .unwrap();
    let session = SessionId(chat.0 * 100);
    store
        .upsert_session(&session_record(session, chat, SessionState::Open))
        .await
        .unwrap();
    let run = store
        .start_run(&new_run(Some(input), session))
        .await
        .unwrap();
    (chat, input, session, run)
}

pub(crate) fn new_input(chat: ChatId, text: &str) -> NewInput {
    NewInput {
        chat,
        text: text.to_owned(),
        settings: SettingsRevision(1),
        permission: Permission::Write,
        workdir: PathBuf::from("/work"),
        pinned_model: None,
        skip_relation: false,
    }
}

pub(crate) fn new_run(input: Option<InputId>, session: SessionId) -> NewRun {
    NewRun {
        input,
        task: TaskId(1),
        agent: AgentId(1),
        session,
        provider: crate::providers::CODEX,
        effect_scope: EffectScope::NetworkPossible,
    }
}

pub(crate) fn session_record(id: SessionId, chat: ChatId, state: SessionState) -> SessionRecord {
    SessionRecord {
        id,
        chat,
        agent: AgentId(1),
        role: AgentRole::Main,
        provider: crate::providers::CLAUDE,
        provider_session: Some(ProviderSessionId("thread-1".to_owned())),
        model: None,
        state,
        delivered: LedgerSeq(0),
        idle_since: None,
    }
}

fn cumulative(input: Option<u64>, output: Option<u64>) -> UsageReport {
    UsageReport {
        agent: AgentId(1),
        subagent: None,
        model: None,
        scope: UsageScope::ThreadCumulative,
        input,
        cache_read: None,
        cache_write: None,
        output,
        reasoning: None,
    }
}

#[tokio::test]
async fn accepted_input_is_open_until_final_state() {
    let (_dir, store) = temp_store().await;
    let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
    let input = store.accept_input(&new_input(chat, "hello")).await.unwrap();

    let open = store.open_inputs().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].0, input);
    assert_eq!(open[0].1.text, "hello");
    assert_eq!(open[0].1.permission, Permission::Write);
    assert_eq!(open[0].2, InputState::Judging);

    store
        .set_input_state(input, InputState::Queued, Some(QueueReason::RouterOrder))
        .await
        .unwrap();
    assert_eq!(store.open_inputs().await.unwrap()[0].2, InputState::Queued);
    store
        .set_input_state(input, InputState::Cancelled, None)
        .await
        .unwrap();
    assert!(store.open_inputs().await.unwrap().is_empty());
}

#[tokio::test]
async fn accept_input_for_missing_chat_writes_nothing() {
    let (_dir, store) = temp_store().await;

    let error = store
        .accept_input(&new_input(ChatId(42), "lost"))
        .await
        .unwrap_err();

    assert!(matches!(error, StoreError::Database(_)));
    assert!(store.open_inputs().await.unwrap().is_empty());
}

#[tokio::test]
async fn missing_rows_return_not_found() {
    let (_dir, store) = temp_store().await;

    let errors = [
        store.set_recording(ChatId(9), false).await.unwrap_err(),
        store.chat_layer(ChatId(9)).await.unwrap_err(),
        store
            .set_input_state(InputId(9), InputState::Queued, None)
            .await
            .unwrap_err(),
        store.end_stop(ChatId(9)).await.unwrap_err(),
        store
            .finish_run(RunId(9), RunEnd::Completed)
            .await
            .unwrap_err(),
        store
            .start_run(&new_run(None, SessionId(9)))
            .await
            .unwrap_err(),
    ];

    for error in errors {
        assert!(matches!(error, StoreError::NotFound { .. }), "{error:?}");
    }
}

#[tokio::test]
async fn chat_layer_round_trips() {
    let (_dir, store) = temp_store().await;
    let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();

    assert_eq!(store.chat_layer(chat).await.unwrap(), None);
    store.set_chat_layer(chat, "model = \"x\"").await.unwrap();

    assert_eq!(
        store.chat_layer(chat).await.unwrap().as_deref(),
        Some("model = \"x\"")
    );
}

#[tokio::test]
async fn run_stays_unfinished_until_finish() {
    let (_dir, store) = temp_store().await;
    let (_, input, _, run) = chat_with_run(&store).await;

    let open = store.unfinished_runs().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, run);
    assert_eq!(open[0].input, Some(input));
    assert_eq!(open[0].effect_scope, EffectScope::NetworkPossible);
    assert_eq!(open[0].end, None);

    store.finish_run(run, RunEnd::Completed).await.unwrap();
    assert!(store.unfinished_runs().await.unwrap().is_empty());
}

#[tokio::test]
async fn provider_wake_run_takes_chat_from_session() {
    let (_dir, store) = temp_store().await;
    let (_, _, session, _) = chat_with_run(&store).await;

    let run = store.start_run(&new_run(None, session)).await.unwrap();

    assert_eq!(store.unfinished_runs().await.unwrap()[1].id, run);
}

#[tokio::test]
async fn unobserved_scope_is_never_reverted() {
    let (_dir, store) = temp_store().await;
    let (_, _, _, run) = chat_with_run(&store).await;

    store
        .set_effect_scope(run, EffectScope::Unobserved)
        .await
        .unwrap();
    store
        .set_effect_scope(run, EffectScope::ProvenByObservation)
        .await
        .unwrap();

    let scope = store.unfinished_runs().await.unwrap()[0].effect_scope;
    assert_eq!(scope, EffectScope::Unobserved);
}

#[tokio::test]
async fn session_upsert_updates_state_and_delivered() {
    let (_dir, store) = temp_store().await;
    let (chat, _, session, _) = chat_with_run(&store).await;
    let mut record = session_record(session, chat, SessionState::ClosedResumable);
    record.delivered = LedgerSeq(7);

    store.upsert_session(&record).await.unwrap();

    let sessions = store.sessions(chat).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].state, SessionState::ClosedResumable);
    assert_eq!(sessions[0].delivered, LedgerSeq(7));
    assert_eq!(sessions[0].role, AgentRole::Main);
    assert_eq!(
        sessions[0].provider_session,
        Some(ProviderSessionId("thread-1".to_owned()))
    );
}

#[tokio::test]
async fn events_get_consecutive_numbers_per_chat() {
    let (_dir, store) = temp_store().await;
    let (chat, _, _, run) = chat_with_run(&store).await;
    let text = ProviderEvent::Text {
        agent: AgentId(1),
        subagent: None,
        text: "hi".to_owned(),
    };
    let done = ProviderEvent::TurnCompleted {
        agent: AgentId(1),
        origin: TurnOrigin::User,
    };

    let first = store.append_event(run, chat, &text).await.unwrap();
    let second = store.append_event(run, chat, &done).await.unwrap();

    assert_eq!((first, second), (LedgerSeq(1), LedgerSeq(2)));
    let since = store.events_since(chat, LedgerSeq(1)).await.unwrap();
    assert_eq!(since, vec![(LedgerSeq(2), done)]);
    let missing = store
        .append_event(RunId(99), chat, &text)
        .await
        .unwrap_err();
    assert!(matches!(missing, StoreError::NotFound { .. }));
}

#[tokio::test]
async fn unreported_usage_stays_null() {
    let (_dir, store) = temp_store().await;
    let (chat, _, session, run) = chat_with_run(&store).await;
    let report = UsageReport {
        scope: UsageScope::MainTurn,
        ..cumulative(Some(10), None)
    };

    store.record_usage(run, session, &report).await.unwrap();

    let output: Option<i64> = sqlx::query_scalar("SELECT output_tokens FROM usage")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(output, None);
    let rows = store
        .usage_rows(UsageRange::Chat, Some(chat))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].report, report);
    assert!(!rows[0].spans_turns);
    for range in [UsageRange::Day, UsageRange::Week] {
        assert_eq!(store.usage_rows(range, None).await.unwrap().len(), 1);
    }
    assert_eq!(store.all_usage_rows().await.unwrap().len(), 1);
    let no_chat = store.usage_rows(UsageRange::Chat, None).await.unwrap_err();
    assert!(matches!(no_chat, StoreError::NotFound { .. }));
}

#[tokio::test]
async fn day_and_week_ranges_are_rolling_windows_over_all_chats() {
    let (_dir, store) = temp_store().await;
    let (_, _, session, run) = chat_with_run(&store).await;
    for _ in 0..3 {
        store
            .record_usage(run, session, &cumulative(Some(10), None))
            .await
            .unwrap();
    }
    let now_ms: i64 = sqlx::query_scalar("SELECT CAST(strftime('%s', 'now') AS INTEGER) * 1000")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let hour = 3_600_000;
    for (id, age) in [(1, 23 * hour), (2, 25 * hour), (3, 8 * 24 * hour)] {
        sqlx::query("UPDATE usage SET at = ? WHERE id = ?")
            .bind(now_ms - age)
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    let day = store.usage_rows(UsageRange::Day, None).await.unwrap();
    let week = store.usage_rows(UsageRange::Week, None).await.unwrap();

    assert_eq!(day.iter().map(|row| row.id).collect::<Vec<_>>(), [1]);
    assert_eq!(week.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(store.all_usage_rows().await.unwrap().len(), 3);
}

#[tokio::test]
async fn latest_chat_in_prefers_the_chat_with_the_latest_input() {
    let (_dir, store) = temp_store().await;
    let (older, _, _, _) = chat_with_run(&store).await;
    let (newer, _, _, _) = chat_with_run(&store).await;
    sqlx::query("UPDATE inputs SET accepted_at = 1 WHERE chat_id = ?")
        .bind(to_sql_int(newer.0))
        .execute(&store.pool)
        .await
        .unwrap();

    let latest = store.latest_chat_in("/work").await.unwrap();

    assert_eq!(latest, Some(older));
    assert_eq!(store.latest_chat_in("/other").await.unwrap(), None);
}

#[tokio::test]
async fn list_chats_orders_like_latest_chat_and_filters_by_folder() {
    let (_dir, store) = temp_store().await;
    let (older, _, _, _) = chat_with_run(&store).await;
    let (newer, _, _, _) = chat_with_run(&store).await;
    let other = store.create_chat(PathBuf::from("/other")).await.unwrap();
    sqlx::query("UPDATE inputs SET accepted_at = 1 WHERE chat_id = ?")
        .bind(to_sql_int(newer.0))
        .execute(&store.pool)
        .await
        .unwrap();

    let in_folder = store.list_chats(Some("/work")).await.unwrap();
    let everywhere = store.list_chats(None).await.unwrap();

    let ids = |list: &[saturn_protocol::rpc::ChatListItem]| {
        list.iter().map(|item| item.chat).collect::<Vec<_>>()
    };
    assert_eq!(ids(&in_folder), [older, newer]);
    assert_eq!(
        Some(in_folder[0].chat),
        store.latest_chat_in("/work").await.unwrap()
    );
    assert_eq!(in_folder[0].preview.as_deref(), Some("fix the build"));
    assert_eq!(in_folder[1].last_active_ms, 1);
    assert_eq!(ids(&everywhere), [other, older, newer]);
    assert_eq!(everywhere[0].preview, None);
    assert_eq!(everywhere[0].folder, "/other");
    assert!(store.list_chats(Some("/nowhere")).await.unwrap().is_empty());
}

#[tokio::test]
async fn cumulative_usage_marks_turn_spans() {
    let (_dir, store) = temp_store().await;
    let (chat, input, session, first) = chat_with_run(&store).await;
    store
        .record_usage(first, session, &cumulative(Some(100), Some(10)))
        .await
        .unwrap();
    let second = store
        .start_run(&new_run(Some(input), session))
        .await
        .unwrap();
    store
        .record_usage(second, session, &cumulative(Some(150), Some(20)))
        .await
        .unwrap();
    let _silent = store
        .start_run(&new_run(Some(input), session))
        .await
        .unwrap();
    let fourth = store
        .start_run(&new_run(Some(input), session))
        .await
        .unwrap();
    store
        .record_usage(fourth, session, &cumulative(Some(300), Some(30)))
        .await
        .unwrap();
    let fifth = store
        .start_run(&new_run(Some(input), session))
        .await
        .unwrap();
    store
        .record_usage(fifth, session, &cumulative(Some(50), Some(40)))
        .await
        .unwrap();

    let spans: Vec<bool> = store
        .usage_rows(UsageRange::Chat, Some(chat))
        .await
        .unwrap()
        .iter()
        .map(|row| row.spans_turns)
        .collect();
    assert_eq!(spans, vec![false, false, true, true]);
}

#[tokio::test]
async fn stop_request_begins_and_ends_once() {
    let (_dir, store) = temp_store().await;
    let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();

    store.begin_stop(chat).await.unwrap();
    store.begin_stop(chat).await.unwrap();
    store.end_stop(chat).await.unwrap();

    assert!(matches!(
        store.end_stop(chat).await.unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

/// 옛 버전이 `Codex`, `Claude`로 쓴 provider 열을 같은 id로 읽는다. 옛 행은 고치지 않는다.
#[tokio::test]
async fn old_provider_values_read_as_open_ids() {
    let (_dir, store) = temp_store().await;
    let (chat, _, session, _) = chat_with_run(&store).await;
    sqlx::query("UPDATE sessions SET provider = 'Claude'")
        .execute(&store.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE runs SET provider = 'Codex'")
        .execute(&store.pool)
        .await
        .unwrap();

    let mains = store.live_mains().await.unwrap();
    let (history, _) = store.recent_history(chat, 10).await.unwrap();

    assert_eq!(mains[0].0.id, session);
    assert_eq!(mains[0].0.provider, crate::providers::CLAUDE);
    let provider = history.iter().find_map(|entry| match entry {
        HistoryEntry::Run { provider, .. } => Some(*provider),
        HistoryEntry::Input { .. } => None,
    });
    assert_eq!(provider, Some(crate::providers::CODEX));
    let stored: String = sqlx::query_scalar("SELECT provider FROM runs")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(stored, "Codex");
}

/// 새로 쓰는 값은 id 글자다.
#[tokio::test]
async fn new_provider_values_are_written_as_ids() {
    let (_dir, store) = temp_store().await;
    chat_with_run(&store).await;

    let stored: String = sqlx::query_scalar("SELECT provider FROM runs")
        .fetch_one(&store.pool)
        .await
        .unwrap();

    assert_eq!(stored, "codex");
}
