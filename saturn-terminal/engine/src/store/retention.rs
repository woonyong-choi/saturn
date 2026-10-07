//! 보존과 정리. 판단 기록과 `~/.claude`, `~/.codex`의 provider 기록은 지우지 않는다.
//! 설계: docs/design/records.md

use std::time::{Duration, SystemTime};

use saturn_protocol::ids::ChatId;
#[cfg(test)]
use sqlx::Row;

use super::records::FINAL_INPUT_STATES;
use super::{Store, StoreError, from_millis, from_sql_int, sha256_hex, to_millis, to_sql_int};

/// 채팅 생성, 입력 접수, 실행 시작과 끝, 이벤트 중 가장 늦은 시각(unix 밀리초).
const LAST_ACTIVE: &str = "MAX(c.created_at, \
     COALESCE((SELECT MAX(accepted_at) FROM inputs WHERE chat_id = c.id), 0), \
     COALESCE((SELECT MAX(MAX(started_at, COALESCE(ended_at, 0))) FROM runs WHERE chat_id = c.id), 0), \
     COALESCE((SELECT MAX(at) FROM events WHERE chat_id = c.id), 0))";

/// 판단 기록과 설정 스냅샷은 넣지 않는다.
const COUNTED_TABLES: [&str; 5] = ["inputs", "runs", "events", "usage", "sessions"];

/// 기본값은 무제한 보존.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RetentionPolicy {
    /// `None`이면 자동 정리하지 않는다.
    pub max_age: Option<Duration>,
    /// 거짓이면 시작 때 정리하지 않는다. `max_age`만으로 켜지지 않는다.
    pub auto_prune: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PruneScope {
    Chats(Vec<ChatId>),
    /// 마지막 활동이 이 시각보다 이른 채팅 전부.
    InactiveBefore(SystemTime),
}

/// `yes`가 거짓이면 아무것도 지우지 않고 계획만 돌려준다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PruneRequest {
    pub scope: PruneScope,
    pub yes: bool,
}

/// 어떤 명령으로 지워도 이 이유가 있는 항목은 남긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipReason {
    OpenInput,
    OpenRun,
    /// `begin_stop` 뒤 `end_stop` 전.
    PendingStop,
    /// `Open` session.
    ActiveSession,
    /// `ClosedResumable`, `Held` session.
    WaitingSession,
}

/// 미리보기와 실제 삭제 모두 같은 계획으로 만든다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PrunePlan {
    pub chats: Vec<ChatId>,
    /// 이유가 여럿이면 모두.
    pub skipped: Vec<(ChatId, Vec<SkipReason>)>,
    /// 판단 기록과 설정 스냅샷은 세지 않는다.
    pub rows: u64,
    /// 지울 채팅마다의 행 수. 합이 `rows`다.
    pub chat_rows: Vec<(ChatId, u64)>,
}

/// 같은 id가 다시 들어오면 알아보는 데 쓴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tombstone {
    pub chat: ChatId,
    /// 지우기 직전 입력 원문과 이벤트의 해시(hex).
    pub hash: String,
    pub deleted_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PruneOutcome {
    Preview(PrunePlan),
    Deleted {
        plan: PrunePlan,
        /// 지운 채팅마다 하나.
        tombstones: Vec<Tombstone>,
    },
}

impl Store {
    /// 쓰지 않는다.
    pub(crate) async fn plan_prune(&self, scope: &PruneScope) -> Result<PrunePlan, StoreError> {
        let mut conn = self.pool.acquire().await?;
        plan_prune_on(&mut conn, scope).await
    }

    /// 미리보기 이후 열린 항목이 생겼을 수 있어 삭제 거래 안에서 계획을 다시 만든다.
    ///
    /// # Errors
    /// 삭제 거래가 끝난 뒤 정리 단계만 실패하면 `Database`이고 삭제와 흔적은 남는다.
    pub(crate) async fn prune(&self, request: &PruneRequest) -> Result<PruneOutcome, StoreError> {
        if !request.yes {
            return Ok(PruneOutcome::Preview(
                self.plan_prune(&request.scope).await?,
            ));
        }
        let mut tx = self.pool.begin().await?;
        let plan = plan_prune_on(&mut tx, &request.scope).await?;
        if plan.chats.is_empty() {
            return Ok(PruneOutcome::Deleted {
                plan,
                tombstones: Vec::new(),
            });
        }
        let deleted_at = SystemTime::now();
        let mut tombstones = Vec::with_capacity(plan.chats.len());
        for chat in &plan.chats {
            let hash = chat_hash(&mut tx, chat.0).await?;
            // 나머지 행은 외래 키 `ON DELETE CASCADE`로 함께 지운다
            sqlx::query("DELETE FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT OR REPLACE INTO tombstones (chat_id, hash, deleted_at) VALUES (?, ?, ?)",
            )
            .bind(to_sql_int(chat.0))
            .bind(&hash)
            .bind(to_millis(deleted_at))
            .execute(&mut *tx)
            .await?;
            tombstones.push(Tombstone {
                chat: *chat,
                hash,
                deleted_at: from_millis(to_millis(deleted_at)),
            });
        }
        tx.commit().await?;
        self.compact_file().await?;
        Ok(PruneOutcome::Deleted { plan, tombstones })
    }

    /// 시작 때 한 번 부른다. `policy.max_age`가 `None`이면 아무것도 하지 않는다.
    pub(crate) async fn prune_on_start(
        &self,
        policy: RetentionPolicy,
        now: SystemTime,
    ) -> Result<Option<PruneOutcome>, StoreError> {
        let Some(max_age) = policy.max_age else {
            return Ok(None);
        };
        let before = now.checked_sub(max_age).unwrap_or(SystemTime::UNIX_EPOCH);
        let request = PruneRequest {
            scope: PruneScope::InactiveBefore(before),
            yes: true,
        };
        Ok(Some(self.prune(&request).await?))
    }

    #[cfg(test)]
    pub(crate) async fn tombstone(&self, chat: ChatId) -> Result<Option<Tombstone>, StoreError> {
        let row = sqlx::query("SELECT hash, deleted_at FROM tombstones WHERE chat_id = ?")
            .bind(to_sql_int(chat.0))
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| {
            Ok(Tombstone {
                chat,
                hash: row.try_get("hash")?,
                deleted_at: from_millis(row.try_get("deleted_at")?),
            })
        })
        .transpose()
    }

    /// `secure_delete=ON`은 연결 옵션에 있다.
    pub(crate) async fn compact_file(&self) -> Result<(), StoreError> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await?;
        sqlx::query("VACUUM").execute(&self.pool).await?;
        Ok(())
    }
}

async fn plan_prune_on(
    conn: &mut sqlx::SqliteConnection,
    scope: &PruneScope,
) -> Result<PrunePlan, StoreError> {
    let candidates: Vec<i64> = match scope {
        PruneScope::Chats(chats) => {
            let mut found = Vec::new();
            for chat in chats {
                let id: Option<i64> = sqlx::query_scalar("SELECT id FROM chats WHERE id = ?")
                    .bind(to_sql_int(chat.0))
                    .fetch_optional(&mut *conn)
                    .await?;
                found.extend(id.filter(|id| !found.contains(id)));
            }
            found
        }
        PruneScope::InactiveBefore(at) => {
            sqlx::query_scalar(&format!(
                "SELECT c.id FROM chats c WHERE {LAST_ACTIVE} < ? ORDER BY c.id"
            ))
            .bind(to_millis(*at))
            .fetch_all(&mut *conn)
            .await?
        }
    };
    let mut plan = PrunePlan::default();
    for id in candidates {
        let reasons = skip_reasons(conn, id).await?;
        let chat = ChatId(from_sql_int(id));
        if !reasons.is_empty() {
            plan.skipped.push((chat, reasons));
            continue;
        }
        let mut chat_rows = 0;
        for table in COUNTED_TABLES {
            let count: i64 =
                sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table} WHERE chat_id = ?"))
                    .bind(id)
                    .fetch_one(&mut *conn)
                    .await?;
            chat_rows += from_sql_int(count);
        }
        plan.rows += chat_rows;
        plan.chat_rows.push((chat, chat_rows));
        plan.chats.push(chat);
    }
    Ok(plan)
}

/// 비어 있으면 지워도 된다.
async fn skip_reasons(
    conn: &mut sqlx::SqliteConnection,
    chat: i64,
) -> Result<Vec<SkipReason>, StoreError> {
    let checks = [
        (
            SkipReason::OpenInput,
            format!("SELECT COUNT(*) FROM inputs WHERE chat_id = ? AND state NOT IN {FINAL_INPUT_STATES}"),
        ),
        (
            SkipReason::OpenRun,
            "SELECT COUNT(*) FROM runs WHERE chat_id = ? AND end_kind IS NULL".to_owned(),
        ),
        (
            SkipReason::PendingStop,
            "SELECT COUNT(*) FROM stops WHERE chat_id = ?".to_owned(),
        ),
        (
            SkipReason::ActiveSession,
            "SELECT COUNT(*) FROM sessions WHERE chat_id = ? AND state = 'Open'".to_owned(),
        ),
        (
            SkipReason::WaitingSession,
            "SELECT COUNT(*) FROM sessions WHERE chat_id = ? AND state IN ('ClosedResumable', 'Held')"
                .to_owned(),
        ),
    ];
    let mut reasons = Vec::new();
    for (reason, sql) in checks {
        let count: i64 = sqlx::query_scalar(&sql)
            .bind(chat)
            .fetch_one(&mut *conn)
            .await?;
        if count > 0 {
            reasons.push(reason);
        }
    }
    Ok(reasons)
}

/// 항목마다 종류 글자와 길이를 앞에 붙여 경계가 섞이지 않게 한다. 초안 형식.
async fn chat_hash(conn: &mut sqlx::SqliteConnection, chat: u64) -> Result<String, StoreError> {
    let inputs: Vec<String> =
        sqlx::query_scalar("SELECT text FROM inputs WHERE chat_id = ? ORDER BY id")
            .bind(to_sql_int(chat))
            .fetch_all(&mut *conn)
            .await?;
    let events: Vec<String> =
        sqlx::query_scalar("SELECT body FROM events WHERE chat_id = ? ORDER BY seq")
            .bind(to_sql_int(chat))
            .fetch_all(&mut *conn)
            .await?;
    let mut content = Vec::new();
    let tagged = inputs
        .iter()
        .map(|text| (b'i', text))
        .chain(events.iter().map(|body| (b'e', body)));
    for (tag, text) in tagged {
        content.push(tag);
        content.extend_from_slice(&(text.len() as u64).to_be_bytes());
        content.extend_from_slice(text.as_bytes());
    }
    Ok(sha256_hex(&content))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_protocol::ids::SessionId;
    use saturn_protocol::state::{InputState, SessionState};

    use super::*;
    use crate::store::RunEnd;
    use crate::store::records::tests::{chat_with_run, session_record};
    use crate::store::tests::temp_store;

    /// 입력, 실행, session을 모두 끝내 지울 수 있는 채팅.
    async fn closed_chat(store: &Store) -> ChatId {
        let (chat, input, session, run) = chat_with_run(store).await;
        store.finish_run(run, RunEnd::Completed).await.unwrap();
        store
            .set_input_state(input, InputState::Applied, None)
            .await
            .unwrap();
        store
            .upsert_session(&session_record(session, chat, SessionState::Ended))
            .await
            .unwrap();
        chat
    }

    async fn count(store: &Store, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&store.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn open_items_are_never_deleted() {
        let (_dir, store) = temp_store().await;
        let (open_chat, _, _, _) = chat_with_run(&store).await;
        let stopping = closed_chat(&store).await;
        store.begin_stop(stopping).await.unwrap();
        let waiting = closed_chat(&store).await;
        store
            .upsert_session(&session_record(
                SessionId(waiting.0 * 100),
                waiting,
                SessionState::Held,
            ))
            .await
            .unwrap();
        let request = PruneRequest {
            scope: PruneScope::Chats(vec![open_chat, stopping, waiting]),
            yes: true,
        };

        let outcome = store.prune(&request).await.unwrap();

        let PruneOutcome::Deleted { plan, tombstones } = outcome else {
            panic!("yes should delete");
        };
        assert!(plan.chats.is_empty());
        assert!(tombstones.is_empty());
        assert_eq!(
            plan.skipped,
            vec![
                (
                    open_chat,
                    vec![
                        SkipReason::OpenInput,
                        SkipReason::OpenRun,
                        SkipReason::ActiveSession
                    ]
                ),
                (stopping, vec![SkipReason::PendingStop]),
                (waiting, vec![SkipReason::WaitingSession]),
            ]
        );
        assert_eq!(count(&store, "chats").await, 3);
        assert_eq!(count(&store, "inputs").await, 3);
    }

    #[tokio::test]
    async fn prune_without_yes_only_previews() {
        let (_dir, store) = temp_store().await;
        let chat = closed_chat(&store).await;
        let request = PruneRequest {
            scope: PruneScope::Chats(vec![chat]),
            yes: false,
        };

        let outcome = store.prune(&request).await.unwrap();

        let PruneOutcome::Preview(plan) = outcome else {
            panic!("no yes should preview");
        };
        assert_eq!(plan.chats, vec![chat]);
        assert_eq!(plan.rows, 3);
        assert_eq!(count(&store, "chats").await, 1);
        assert_eq!(count(&store, "runs").await, 1);
    }

    #[tokio::test]
    async fn prune_deletes_rows_and_leaves_tombstone() {
        let (_dir, store) = temp_store().await;
        let chat = closed_chat(&store).await;
        let kept = closed_chat(&store).await;
        let request = PruneRequest {
            scope: PruneScope::Chats(vec![chat]),
            yes: true,
        };

        let outcome = store.prune(&request).await.unwrap();

        let PruneOutcome::Deleted { plan, tombstones } = outcome else {
            panic!("yes should delete");
        };
        assert_eq!(plan.chats, vec![chat]);
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].hash.len(), 64);
        assert_eq!(
            store.tombstone(chat).await.unwrap(),
            Some(tombstones[0].clone())
        );
        assert_eq!(store.tombstone(kept).await.unwrap(), None);
        for table in ["chats", "inputs", "runs", "sessions"] {
            assert_eq!(count(&store, table).await, 1, "{table}");
        }
        let next = store.create_chat(PathBuf::from("/work")).await.unwrap();
        assert!(next.0 > kept.0);
    }

    #[tokio::test]
    async fn ended_tasks_come_from_runs_and_go_with_their_chat() {
        let (_dir, store) = temp_store().await;
        let chat = closed_chat(&store).await;
        chat_with_run(&store).await; // 열린 실행만 있는 채팅은 끝난 작업이 아니다

        let before = store.ended_tasks().await.unwrap();
        let request = PruneRequest {
            scope: PruneScope::Chats(vec![chat]),
            yes: true,
        };
        store.prune(&request).await.unwrap();
        let after = store.ended_tasks().await.unwrap();

        assert_eq!(
            before
                .iter()
                .map(|ended| (ended.chat, ended.end))
                .collect::<Vec<_>>(),
            [(chat, RunEnd::Completed)]
        );
        assert!(before[0].ended_at_ms > 0);
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn prune_on_start_respects_policy() {
        let (_dir, store) = temp_store().await;
        let chat = closed_chat(&store).await;
        let now = SystemTime::now();

        let off = store
            .prune_on_start(RetentionPolicy::default(), now)
            .await
            .unwrap();
        assert_eq!(off, None);
        let policy = RetentionPolicy {
            max_age: Some(Duration::from_secs(3600)),
            auto_prune: true,
        };
        let recent = store.prune_on_start(policy, now).await.unwrap().unwrap();
        assert!(matches!(recent, PruneOutcome::Deleted { ref plan, .. } if plan.chats.is_empty()));

        let later = now + Duration::from_secs(7200);
        let old = store.prune_on_start(policy, later).await.unwrap().unwrap();
        assert!(matches!(old, PruneOutcome::Deleted { ref plan, .. } if plan.chats == vec![chat]));
    }
}
