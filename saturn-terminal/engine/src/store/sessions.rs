//! session의 마지막 턴 값 저장과, 시작 때 살아 있는 메인 session 읽기.
//! 설계: docs/design/records.md

use saturn_core::sessions::{LastTurn, SessionRecord};
use saturn_protocol::ids::SessionId;
use sqlx::Row;

use super::records::{ensure_found, session_from_row};
use super::{Store, StoreError, from_millis, from_sql_int, to_millis, to_sql_int};

impl Store {
    /// 같은 session의 이전 값은 덮어쓴다.
    ///
    /// # Errors
    /// 없는 session이면 `NotFound`.
    pub async fn record_last_turn(
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

    /// `Ended`가 아닌 메인을 id 순서로 돌려준다. 마지막 턴 값이 없으면 `None`.
    pub async fn live_mains(&self) -> Result<Vec<(SessionRecord, Option<LastTurn>)>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, chat_id, agent_id, role, provider, provider_session, state, delivered, \
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
