use std::time::SystemTime;

use saturn_core::sessions::SessionRecord;
use saturn_protocol::event::{ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{ChatId, LedgerSeq, RunId, SessionId};
use saturn_protocol::state::EffectScope;
use sqlx::Row;

use super::rows::{
    ensure_found, not_found, role_text, run_end_text, run_record, session_from_row, spans_turns,
};
use super::{NewRun, RunEnd, RunRecord};
use crate::store::{Store, StoreError, enum_text, from_sql_int, to_millis, to_sql_int};

impl Store {
    /// provider에 보내기 전에 부른다.
    ///
    /// # Errors
    /// 입력도 session 행도 없어 채팅을 정하지 못하면 `NotFound`.
    pub(crate) async fn start_run(&self, run: &NewRun) -> Result<RunId, StoreError> {
        let mut tx = self.pool.begin().await?;
        let chat: Option<i64> = match run.input {
            Some(input) => {
                sqlx::query_scalar("SELECT chat_id FROM inputs WHERE id = ?")
                    .bind(to_sql_int(input.0))
                    .fetch_optional(&mut *tx)
                    .await?
            }
            None => {
                sqlx::query_scalar("SELECT chat_id FROM sessions WHERE id = ?")
                    .bind(to_sql_int(run.session.0))
                    .fetch_optional(&mut *tx)
                    .await?
            }
        };
        let chat = chat.ok_or_else(|| match run.input {
            Some(input) => not_found(format!("input {}", input.0)),
            None => not_found(format!("session {}", run.session.0)),
        })?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO runs (chat_id, input_id, task_id, agent_id, session_id, provider, effect_scope, \
             started_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(chat)
        .bind(run.input.map(|input| to_sql_int(input.0)))
        .bind(to_sql_int(run.task.0))
        .bind(to_sql_int(run.agent.0))
        .bind(to_sql_int(run.session.0))
        .bind(enum_text(&run.provider)?)
        .bind(enum_text(&run.effect_scope)?)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(RunId(from_sql_int(id)))
    }

    /// 이미 `Unobserved`인 실행은 다른 값으로 바꾸지 않는다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub(crate) async fn set_effect_scope(
        &self,
        run: RunId,
        scope: EffectScope,
    ) -> Result<(), StoreError> {
        let unobserved = enum_text(&EffectScope::Unobserved)?;
        let done = sqlx::query(
            "UPDATE runs SET effect_scope = CASE WHEN effect_scope = ? THEN effect_scope ELSE ? END \
             WHERE id = ?",
        )
        .bind(unobserved)
        .bind(enum_text(&scope)?)
        .bind(to_sql_int(run.0))
        .execute(&self.pool)
        .await?;
        ensure_found(done.rows_affected(), || format!("run {}", run.0))
    }

    /// 이미 끝난 실행이면 처음 기록한 끝을 그대로 둔다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 압축 실패면 `Compression`(끝 기록은 남는다).
    pub(crate) async fn finish_run(&self, run: RunId, end: RunEnd) -> Result<(), StoreError> {
        let done = sqlx::query(
            "UPDATE runs SET end_kind = COALESCE(end_kind, ?), ended_at = COALESCE(ended_at, ?) \
             WHERE id = ?",
        )
        .bind(run_end_text(end))
        .bind(to_millis(SystemTime::now()))
        .bind(to_sql_int(run.0))
        .execute(&self.pool)
        .await?;
        ensure_found(done.rows_affected(), || format!("run {}", run.0))?;
        self.compress_run(run).await?;
        Ok(())
    }

    pub(crate) async fn unfinished_runs(&self) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, input_id, task_id, session_id, effect_scope, started_at, end_kind FROM runs \
             WHERE end_kind IS NULL ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_record).collect()
    }

    /// `idle_since`(`Instant`)는 저장하지 않는다.
    pub(crate) async fn upsert_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        self.upsert_sessions(std::slice::from_ref(session)).await
    }

    /// 모두 한 거래로 쓴다. 한 변경이 여러 session의 상태를 함께 바꾸기 때문이다.
    pub(crate) async fn upsert_sessions(
        &self,
        sessions: &[SessionRecord],
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        for session in sessions {
            sqlx::query(
                "INSERT INTO sessions (id, chat_id, agent_id, role, provider, provider_session, model, state, delivered) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET \
                 provider_session = excluded.provider_session, model = excluded.model, state = excluded.state, \
                 delivered = excluded.delivered",
            )
            .bind(to_sql_int(session.id.0))
            .bind(to_sql_int(session.chat.0))
            .bind(to_sql_int(session.agent.0))
            .bind(role_text(session.role))
            .bind(enum_text(&session.provider)?)
            .bind(session.provider_session.as_ref().map(|id| id.0.clone()))
            .bind(&session.model)
            .bind(enum_text(&session.state)?)
            .bind(to_sql_int(session.delivered.0))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// id 순서.
    pub(crate) async fn sessions(&self, chat: ChatId) -> Result<Vec<SessionRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, chat_id, agent_id, role, provider, provider_session, model, state, delivered \
             FROM sessions WHERE chat_id = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(session_from_row).collect()
    }

    /// 번호는 채팅 트리 전체에서 1부터 1씩 늘어난다. `Usage` 이벤트는 `record_usage`로 쓴다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 직렬화 실패면 `Json`.
    pub(crate) async fn append_event(
        &self,
        run: RunId,
        chat: ChatId,
        event: &ProviderEvent,
    ) -> Result<LedgerSeq, StoreError> {
        let body = serde_json::to_string(event)?;
        let mut tx = self.pool.begin().await?;
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM runs WHERE id = ?")
            .bind(to_sql_int(run.0))
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Err(not_found(format!("run {}", run.0)));
        }
        let seq: i64 = sqlx::query_scalar(
            "INSERT INTO events (chat_id, seq, run_id, body, at) \
             VALUES (?1, (SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE chat_id = ?1), ?2, ?3, ?4) \
             RETURNING seq",
        )
        .bind(to_sql_int(chat.0))
        .bind(to_sql_int(run.0))
        .bind(body)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(LedgerSeq(from_sql_int(seq)))
    }

    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn events_since(
        &self,
        chat: ChatId,
        after: LedgerSeq,
    ) -> Result<Vec<(LedgerSeq, ProviderEvent)>, StoreError> {
        let rows =
            sqlx::query("SELECT seq, body FROM events WHERE chat_id = ? AND seq > ? ORDER BY seq")
                .bind(to_sql_int(chat.0))
                .bind(to_sql_int(after.0))
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| {
                let seq = LedgerSeq(from_sql_int(row.try_get("seq")?));
                let event = serde_json::from_str(row.try_get("body")?)?;
                Ok((seq, event))
            })
            .collect()
    }

    /// `None`인 칸은 NULL로 쓴다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub(crate) async fn record_usage(
        &self,
        run: RunId,
        session: SessionId,
        report: &UsageReport,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let chat: Option<i64> = sqlx::query_scalar("SELECT chat_id FROM runs WHERE id = ?")
            .bind(to_sql_int(run.0))
            .fetch_optional(&mut *tx)
            .await?;
        let chat = chat.ok_or_else(|| not_found(format!("run {}", run.0)))?;
        let spans_turns = match report.scope {
            UsageScope::ThreadCumulative => spans_turns(&mut tx, run, session, report).await?,
            UsageScope::MainTurn | UsageScope::TreeTotal => false,
        };
        sqlx::query(
            "INSERT INTO usage (chat_id, run_id, session_id, body, scope, agent_id, subagent, model, \
             input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, reasoning_tokens, \
             spans_turns, at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(chat)
        .bind(to_sql_int(run.0))
        .bind(to_sql_int(session.0))
        .bind(serde_json::to_string(report)?)
        .bind(enum_text(&report.scope)?)
        .bind(to_sql_int(report.agent.0))
        .bind(report.subagent.as_ref().map(|id| id.0.clone()))
        .bind(&report.model)
        .bind(report.input.map(to_sql_int))
        .bind(report.cache_read.map(to_sql_int))
        .bind(report.cache_write.map(to_sql_int))
        .bind(report.output.map(to_sql_int))
        .bind(report.reasoning.map(to_sql_int))
        .bind(spans_turns)
        .bind(to_millis(SystemTime::now()))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
