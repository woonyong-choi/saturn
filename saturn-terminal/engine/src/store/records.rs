//! 채팅, 입력, 실행, session, provider 이벤트, 사용량 원값 기록.
//! 설계: docs/design/records.md

use std::path::PathBuf;
use std::time::SystemTime;

use saturn_core::queue::Permission;
use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::event::{ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, Provider, ProviderSessionId, RunId, SessionId,
    SettingsRevision, TaskId,
};
use saturn_protocol::state::{EffectScope, InputState, QueueReason};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{
    Store, StoreError, enum_text, from_millis, from_sql_int, parse_enum, to_millis, to_sql_int,
};

/// 이 밖의 입력은 열린 입력이다.
pub(crate) const FINAL_INPUT_STATES: &str = "('Applied', 'Rejected', 'Cancelled')";

/// 접수 때 설정 번호와 권한을 고정하고 끝까지 바꾸지 않는다.
#[derive(Debug, Clone)]
pub struct NewInput {
    pub chat: ChatId,
    /// 사용자 입력이고 router 키가 아니라 마스킹하지 않는다.
    pub text: String,
    pub settings: SettingsRevision,
    pub permission: Permission,
    pub workdir: PathBuf,
    pub pinned_model: Option<String>,
    /// `Tab`으로 관계 판단 없이 대기.
    pub skip_relation: bool,
}

/// provider 턴 하나.
#[derive(Debug, Clone)]
pub struct NewRun {
    /// `provider-wake` 턴이면 `None`.
    pub input: Option<InputId>,
    pub task: TaskId,
    pub agent: AgentId,
    pub session: SessionId,
    pub provider: Provider,
    /// 적용된 provider 설정으로 증명되면 `ProvenByConfig`, 아니면 `NetworkPossible`에서 시작한다.
    pub effect_scope: EffectScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Completed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct RunRecord {
    pub id: RunId,
    pub input: Option<InputId>,
    pub task: TaskId,
    pub session: SessionId,
    /// `allows_auto_resume()`이 참일 때만 자동 재개한다.
    pub effect_scope: EffectScope,
    pub started_at: SystemTime,
    /// 끝나지 않았으면 `None`(열린 실행).
    pub end: Option<RunEnd>,
}

/// 보고하지 않은 칸은 `None`이고 합계에서 0으로 세지 않는다.
#[derive(Debug, Clone)]
pub struct UsageRow {
    /// 기록 순서.
    pub id: u64,
    pub run: RunId,
    /// `ThreadCumulative`는 같은 session의 직전 누적을 빼서 턴 값을 구한다.
    pub session: SessionId,
    /// 그 실행의 provider.
    pub provider: Provider,
    pub report: UsageReport,
    pub at: SystemTime,
    /// 중간 보고가 빠져 이 값의 차이가 여러 턴에 걸친다. `/usage` 응답은 protocol `UsageRow::turns`로 턴 수를 싣는다.
    pub spans_turns: bool,
}

impl Store {
    /// 판단 기록 저장은 켜진 상태로 시작한다.
    pub async fn create_chat(&self, workdir: PathBuf) -> Result<ChatId, StoreError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO chats (workdir, recording, created_at) VALUES (?, 1, ?) RETURNING id",
        )
        .bind(workdir.to_string_lossy().into_owned())
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&self.pool)
        .await?;
        Ok(ChatId(from_sql_int(id)))
    }

    /// 채팅을 만들 때 고정한 작업 폴더.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn chat_workdir(&self, chat: ChatId) -> Result<PathBuf, StoreError> {
        let row: Option<String> = sqlx::query_scalar("SELECT workdir FROM chats WHERE id = ?")
            .bind(to_sql_int(chat.0))
            .fetch_optional(&self.pool)
            .await?;
        row.map(PathBuf::from)
            .ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 끄면 새 판단 기록만 저장하지 않고 이미 저장한 것은 둔다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_recording(&self, chat: ChatId, on: bool) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET recording = ? WHERE id = ?")
            .bind(on)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 채팅의 고정 모델 글. 다시 바꿀 때까지 이어지고 다음 입력부터 쓴다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_chat_model(&self, chat: ChatId, model: &str) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET pinned_model = ? WHERE id = ?")
            .bind(model)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 고정하지 않았으면 `None`.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn chat_model(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT pinned_model FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 보조 에이전트도 같은 채팅이라 부모 채팅의 값을 그대로 읽는다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn chat_layer(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        let row: Option<Option<String>> =
            sqlx::query_scalar("SELECT chat_layer FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }

    /// 원본 파일이 없는 층이라 여기가 정본이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_chat_layer(&self, chat: ChatId, toml: &str) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET chat_layer = ? WHERE id = ?")
            .bind(toml)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// 이 함수가 성공한 뒤에만 `Queue::accept`를 부르고 TUI에 에코를 보낸다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅 포함)이고 입력은 접수되지 않았다.
    pub async fn accept_input(&self, input: &NewInput) -> Result<InputId, StoreError> {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO inputs (chat_id, text, settings_revision, permission, workdir, pinned_model, \
             skip_relation, state, reason, accepted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?) RETURNING id",
        )
        .bind(to_sql_int(input.chat.0))
        .bind(&input.text)
        .bind(to_sql_int(input.settings.0))
        .bind(permission_text(input.permission))
        .bind(input.workdir.to_string_lossy().into_owned())
        .bind(&input.pinned_model)
        .bind(input.skip_relation)
        .bind(enum_text(&InputState::Judging)?)
        .bind(to_millis(SystemTime::now()))
        .fetch_one(&self.pool)
        .await?;
        Ok(InputId(from_sql_int(id)))
    }

    /// 전이 규칙 검사는 `core::queue`가 먼저 한다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub async fn set_input_state(
        &self,
        input: InputId,
        state: InputState,
        reason: Option<QueueReason>,
    ) -> Result<(), StoreError> {
        let reason = reason
            .map(|reason| serde_json::to_string(&reason))
            .transpose()?;
        let done = sqlx::query("UPDATE inputs SET state = ?, reason = ? WHERE id = ?")
            .bind(enum_text(&state)?)
            .bind(reason)
            .bind(to_sql_int(input.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("input {}", input.0))
    }

    /// 시작 때 대기열을 되살리는 데 쓴다. 접수 순서.
    pub async fn open_inputs(&self) -> Result<Vec<(InputId, NewInput, InputState)>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT id, chat_id, text, settings_revision, permission, workdir, pinned_model, \
             skip_relation, state FROM inputs WHERE state NOT IN {FINAL_INPUT_STATES} ORDER BY id"
        ))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let input = NewInput {
                    chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                    text: row.try_get("text")?,
                    settings: SettingsRevision(from_sql_int(row.try_get("settings_revision")?)),
                    permission: parse_permission(row.try_get("permission")?)?,
                    workdir: PathBuf::from(row.try_get::<String, _>("workdir")?),
                    pinned_model: row.try_get("pinned_model")?,
                    skip_relation: row.try_get("skip_relation")?,
                };
                let state = parse_enum(row.try_get("state")?)?;
                Ok((InputId(from_sql_int(row.try_get("id")?)), input, state))
            })
            .collect()
    }

    /// 처리 중인 동안 그 채팅은 정리 대상에서 빠지고, 이미 처리 중이면 그대로 둔다.
    pub async fn begin_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        sqlx::query("INSERT OR IGNORE INTO stops (chat_id, started_at) VALUES (?, ?)")
            .bind(to_sql_int(chat.0))
            .bind(to_millis(SystemTime::now()))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// # Errors
    /// 처리 중인 요청이 없으면 `NotFound`.
    pub async fn end_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        let done = sqlx::query("DELETE FROM stops WHERE chat_id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("stop of chat {}", chat.0))
    }

    /// provider에 보내기 전에 부른다.
    ///
    /// # Errors
    /// 입력도 session 행도 없어 채팅을 정하지 못하면 `NotFound`.
    pub async fn start_run(&self, run: &NewRun) -> Result<RunId, StoreError> {
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
    pub async fn set_effect_scope(&self, run: RunId, scope: EffectScope) -> Result<(), StoreError> {
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
    pub async fn finish_run(&self, run: RunId, end: RunEnd) -> Result<(), StoreError> {
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

    pub async fn unfinished_runs(&self) -> Result<Vec<RunRecord>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, input_id, task_id, session_id, effect_scope, started_at, end_kind FROM runs \
             WHERE end_kind IS NULL ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(run_record).collect()
    }

    /// `idle_since`(`Instant`)는 저장하지 않는다.
    pub async fn upsert_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        self.upsert_sessions(std::slice::from_ref(session)).await
    }

    /// 모두 한 거래로 쓴다. 한 변경이 여러 session의 상태를 함께 바꾸기 때문이다.
    pub async fn upsert_sessions(&self, sessions: &[SessionRecord]) -> Result<(), StoreError> {
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
    pub async fn sessions(&self, chat: ChatId) -> Result<Vec<SessionRecord>, StoreError> {
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
    pub async fn append_event(
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
    pub async fn events_since(
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
    pub async fn record_usage(
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

/// 직전 누적보다 작은 칸이 있거나, 직전 누적 보고 뒤에 보고 없는 실행이 있으면 참.
async fn spans_turns(
    tx: &mut sqlx::SqliteConnection,
    run: RunId,
    session: SessionId,
    report: &UsageReport,
) -> Result<bool, StoreError> {
    let previous = sqlx::query(
        "SELECT run_id, body FROM usage WHERE session_id = ? AND scope = ? AND run_id < ? \
         ORDER BY id DESC LIMIT 1",
    )
    .bind(to_sql_int(session.0))
    .bind(enum_text(&UsageScope::ThreadCumulative)?)
    .bind(to_sql_int(run.0))
    .fetch_optional(&mut *tx)
    .await?;
    let after_run = match &previous {
        Some(row) => row.try_get::<i64, _>("run_id")?,
        None => 0,
    };
    let silent_runs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM runs WHERE session_id = ? AND id > ? AND id < ?")
            .bind(to_sql_int(session.0))
            .bind(after_run)
            .bind(to_sql_int(run.0))
            .fetch_one(&mut *tx)
            .await?;
    if silent_runs > 0 {
        return Ok(true);
    }
    let Some(row) = previous else {
        return Ok(false);
    };
    let before: UsageReport = serde_json::from_str(row.try_get("body")?)?;
    let pairs = [
        (before.input, report.input),
        (before.cache_read, report.cache_read),
        (before.cache_write, report.cache_write),
        (before.output, report.output),
        (before.reasoning, report.reasoning),
    ];
    Ok(pairs
        .iter()
        .any(|pair| matches!(pair, (Some(old), Some(new)) if new < old)))
}

pub(super) fn session_from_row(row: &SqliteRow) -> Result<SessionRecord, StoreError> {
    Ok(SessionRecord {
        id: SessionId(from_sql_int(row.try_get("id")?)),
        chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
        agent: AgentId(from_sql_int(row.try_get("agent_id")?)),
        role: parse_role(row.try_get("role")?)?,
        provider: parse_enum(row.try_get("provider")?)?,
        provider_session: row
            .try_get::<Option<String>, _>("provider_session")?
            .map(ProviderSessionId),
        model: row.try_get("model")?,
        state: parse_enum(row.try_get("state")?)?,
        delivered: LedgerSeq(from_sql_int(row.try_get("delivered")?)),
        idle_since: None,
    })
}

fn run_record(row: &SqliteRow) -> Result<RunRecord, StoreError> {
    let end = row
        .try_get::<Option<String>, _>("end_kind")?
        .map(|text| parse_run_end(&text))
        .transpose()?;
    Ok(RunRecord {
        id: RunId(from_sql_int(row.try_get("id")?)),
        input: row
            .try_get::<Option<i64>, _>("input_id")?
            .map(|id| InputId(from_sql_int(id))),
        task: TaskId(from_sql_int(row.try_get("task_id")?)),
        session: SessionId(from_sql_int(row.try_get("session_id")?)),
        effect_scope: parse_enum(row.try_get("effect_scope")?)?,
        started_at: from_millis(row.try_get("started_at")?),
        end,
    })
}

pub(crate) fn ensure_found(rows: u64, what: impl FnOnce() -> String) -> Result<(), StoreError> {
    if rows == 0 {
        return Err(not_found(what()));
    }
    Ok(())
}

pub(crate) fn not_found(what: String) -> StoreError {
    StoreError::NotFound { what }
}

/// 이 실행 파일이 쓰지 않은 값이다.
pub(super) fn unknown_value(column: &str, text: &str) -> StoreError {
    StoreError::Database(sqlx::Error::Decode(
        format!("unknown {column} value: {text}").into(),
    ))
}

fn permission_text(permission: Permission) -> &'static str {
    match permission {
        Permission::ReadOnly => "ReadOnly",
        Permission::Write => "Write",
    }
}

fn parse_permission(text: &str) -> Result<Permission, StoreError> {
    match text {
        "ReadOnly" => Ok(Permission::ReadOnly),
        "Write" => Ok(Permission::Write),
        other => Err(unknown_value("permission", other)),
    }
}

fn role_text(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Main => "Main",
        AgentRole::Sub => "Sub",
    }
}

fn parse_role(text: &str) -> Result<AgentRole, StoreError> {
    match text {
        "Main" => Ok(AgentRole::Main),
        "Sub" => Ok(AgentRole::Sub),
        other => Err(unknown_value("role", other)),
    }
}

fn run_end_text(end: RunEnd) -> &'static str {
    match end {
        RunEnd::Completed => "Completed",
        RunEnd::Failed => "Failed",
        RunEnd::Stopped => "Stopped",
    }
}

fn parse_run_end(text: &str) -> Result<RunEnd, StoreError> {
    match text {
        "Completed" => Ok(RunEnd::Completed),
        "Failed" => Ok(RunEnd::Failed),
        "Stopped" => Ok(RunEnd::Stopped),
        other => Err(unknown_value("run end", other)),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use saturn_protocol::event::TurnOrigin;
    use saturn_protocol::rpc::UsageRange;
    use saturn_protocol::state::SessionState;

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
            provider: Provider::Codex,
            effect_scope: EffectScope::NetworkPossible,
        }
    }

    pub(crate) fn session_record(
        id: SessionId,
        chat: ChatId,
        state: SessionState,
    ) -> SessionRecord {
        SessionRecord {
            id,
            chat,
            agent: AgentId(1),
            role: AgentRole::Main,
            provider: Provider::Claude,
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
        let now_ms: i64 =
            sqlx::query_scalar("SELECT CAST(strftime('%s', 'now') AS INTEGER) * 1000")
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
}
