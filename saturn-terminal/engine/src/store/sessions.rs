//! session의 마지막 턴 값 저장과, 시작 때 살아 있는 메인 session 읽기.
//! 설계: docs/design/records.md

use saturn_core::sessions::{LastTurn, SessionRecord};
use saturn_protocol::ids::SessionId;
use sqlx::Row;

use super::records::{ensure_found, session_from_row};
use super::{Store, StoreError, from_millis, from_sql_int, to_millis, to_sql_int};

/// 번호를 받는 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdKind {
    Session,
    Agent,
}

impl IdKind {
    fn meta_key(self) -> &'static str {
        match self {
            Self::Session => "last_session_id",
            Self::Agent => "last_agent_id",
        }
    }

    fn max_recorded(self) -> &'static str {
        match self {
            Self::Session => "SELECT COALESCE(MAX(id), 0) FROM sessions",
            Self::Agent => {
                "SELECT COALESCE(MAX(id), 0) FROM \
                 (SELECT agent_id AS id FROM sessions UNION ALL SELECT agent_id AS id FROM runs)"
            }
        }
    }
}

impl Store {
    /// 같은 session의 이전 값은 덮어쓴다.
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`.
    pub(crate) async fn record_last_turn(
        &self,
        session: SessionId,
        last_turn: LastTurn,
    ) -> Result<(), StoreError> {
        let done =
            sqlx::query("UPDATE sessions SET last_active = ?, last_turn_ended_at = ? WHERE id = ?")
                .bind(to_sql_int(last_turn.active))
                .bind(to_millis(last_turn.ended_at))
                .bind(to_sql_int(session.0))
                .execute(&self.pool)
                .await?;
        ensure_found(done.rows_affected(), || format!("session {}", session.0))
    }

    /// 지금까지 받은 번호와 기록에 있는 번호보다 큰 새 번호를 한 거래로 받는다. 번호를 다시 쓰지 않는다.
    pub(crate) async fn allocate_id(&self, kind: IdKind) -> Result<u64, StoreError> {
        let mut tx = self.pool.begin().await?;
        let issued: Option<i64> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
            .bind(kind.meta_key())
            .fetch_optional(&mut *tx)
            .await?;
        let recorded: i64 = sqlx::query_scalar(kind.max_recorded())
            .fetch_one(&mut *tx)
            .await?;
        let next = issued.unwrap_or(0).max(recorded) + 1;
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(kind.meta_key())
        .bind(next)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(from_sql_int(next))
    }

    /// `Ended`가 아닌 메인을 id 순서로 돌려준다. 마지막 턴 값이 없으면 `None`.
    pub(crate) async fn live_mains(
        &self,
    ) -> Result<Vec<(SessionRecord, Option<LastTurn>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, chat_id, agent_id, role, provider, provider_session, model, state, delivered, \
             last_active, last_turn_ended_at FROM sessions \
             WHERE role = 'Main' AND state != 'Ended' ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let active: Option<i64> = row.try_get("last_active")?;
                let ended_at: Option<i64> = row.try_get("last_turn_ended_at")?;
                let last_turn = active.zip(ended_at).map(|(active, ended_at)| LastTurn {
                    active: from_sql_int(active),
                    ended_at: from_millis(ended_at),
                });
                Ok((session_from_row(row)?, last_turn))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use saturn_protocol::state::SessionState;

    use super::*;
    use crate::store::records::tests::{chat_with_run, session_record};
    use crate::store::tests::temp_store;

    fn last_turn(active: u64) -> LastTurn {
        LastTurn {
            active,
            ended_at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_123),
        }
    }

    #[tokio::test]
    async fn record_last_turn_round_trips_through_live_mains() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        let mut record = session_record(session, chat, SessionState::ClosedResumable);
        record.delivered = saturn_protocol::ids::LedgerSeq(4);
        store.upsert_session(&record).await.unwrap();

        store
            .record_last_turn(session, last_turn(150_000))
            .await
            .unwrap();

        let live = store.live_mains().await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0.id, session);
        assert_eq!(live[0].0.delivered, record.delivered);
        assert_eq!(live[0].1, Some(last_turn(150_000)));
    }

    #[tokio::test]
    async fn record_last_turn_overwrites_and_survives_session_upsert() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        store
            .record_last_turn(session, last_turn(10))
            .await
            .unwrap();
        store
            .record_last_turn(session, last_turn(20))
            .await
            .unwrap();

        store
            .upsert_session(&session_record(session, chat, SessionState::Held))
            .await
            .unwrap();

        let live = store.live_mains().await.unwrap();
        assert_eq!(live[0].1, Some(last_turn(20)));
    }

    #[tokio::test]
    async fn record_last_turn_for_missing_session_returns_not_found() {
        let (_dir, store) = temp_store().await;

        let error = store
            .record_last_turn(SessionId(9), last_turn(1))
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn live_mains_skip_ended_and_sub_sessions() {
        let (_dir, store) = temp_store().await;
        let (chat, _, open, _) = chat_with_run(&store).await;
        store
            .upsert_session(&session_record(open, chat, SessionState::Ended))
            .await
            .unwrap();
        let ended = SessionId(open.0 + 1);
        store
            .upsert_session(&session_record(ended, chat, SessionState::Ended))
            .await
            .unwrap();
        let mut sub = session_record(SessionId(open.0 + 2), chat, SessionState::ClosedResumable);
        sub.role = saturn_core::sessions::AgentRole::Sub;
        store.upsert_session(&sub).await.unwrap();

        let live = store.live_mains().await.unwrap();

        assert!(live.is_empty());
    }

    #[tokio::test]
    async fn session_model_round_trips_through_live_mains() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        let mut record = session_record(session, chat, SessionState::ClosedResumable);
        record.model = Some("opus".to_owned());
        store.upsert_session(&record).await.unwrap();

        let live = store.live_mains().await.unwrap();

        assert_eq!(live[0].0.model.as_deref(), Some("opus"));
    }

    #[tokio::test]
    async fn allocate_id_counts_up_and_skips_recorded_ids() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        store
            .upsert_session(&session_record(session, chat, SessionState::Open))
            .await
            .unwrap();

        let first = store.allocate_id(IdKind::Session).await.unwrap();
        let second = store.allocate_id(IdKind::Session).await.unwrap();
        let agent = store.allocate_id(IdKind::Agent).await.unwrap();

        assert_eq!(first, session.0 + 1);
        assert_eq!(second, first + 1);
        assert!(agent >= 1);
        assert_eq!(store.allocate_id(IdKind::Agent).await.unwrap(), agent + 1);
    }

    #[tokio::test]
    async fn live_main_without_last_turn_has_none() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        store
            .upsert_session(&session_record(
                session,
                chat,
                SessionState::ClosedResumable,
            ))
            .await
            .unwrap();

        let live = store.live_mains().await.unwrap();

        assert_eq!(live[0].1, None);
    }
}
