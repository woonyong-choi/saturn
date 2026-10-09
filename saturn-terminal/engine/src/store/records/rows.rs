use saturn_core::queue::Permission;
use saturn_core::sessions::{AgentRole, SessionRecord};
use saturn_protocol::event::{UsageReport, UsageScope};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, ProviderSessionId, RunId, SessionId, TaskId,
};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{RunEnd, RunRecord};
use crate::store::{StoreError, enum_text, from_sql_int, parse_enum, to_sql_int};

/// 직전 누적보다 작은 칸이 있거나, 직전 누적 보고 뒤에 보고 없는 실행이 있으면 참.
pub(super) async fn spans_turns(
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

pub(in crate::store) fn session_from_row(row: &SqliteRow) -> Result<SessionRecord, StoreError> {
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

pub(super) fn run_record(row: &SqliteRow) -> Result<RunRecord, StoreError> {
    Ok(RunRecord {
        id: RunId(from_sql_int(row.try_get("id")?)),
        chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
        agent: AgentId(from_sql_int(row.try_get("agent_id")?)),
        input: row
            .try_get::<Option<i64>, _>("input_id")?
            .map(|id| InputId(from_sql_int(id))),
        task: TaskId(from_sql_int(row.try_get("task_id")?)),
        session: SessionId(from_sql_int(row.try_get("session_id")?)),
        effect_scope: parse_enum(row.try_get("effect_scope")?)?,
        #[cfg(test)]
        end: row
            .try_get::<Option<String>, _>("end_kind")?
            .map(|text| parse_run_end(&text))
            .transpose()?,
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
pub(in crate::store) fn unknown_value(column: &str, text: &str) -> StoreError {
    StoreError::Database(sqlx::Error::Decode(
        format!("unknown {column} value: {text}").into(),
    ))
}

pub(super) fn permission_text(permission: Permission) -> &'static str {
    match permission {
        Permission::ReadOnly => "ReadOnly",
        Permission::Write => "Write",
    }
}

pub(super) fn parse_permission(text: &str) -> Result<Permission, StoreError> {
    match text {
        "ReadOnly" => Ok(Permission::ReadOnly),
        "Write" => Ok(Permission::Write),
        other => Err(unknown_value("permission", other)),
    }
}

pub(super) fn role_text(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Main => "Main",
        AgentRole::Sub => "Sub",
    }
}

pub(super) fn parse_role(text: &str) -> Result<AgentRole, StoreError> {
    match text {
        "Main" => Ok(AgentRole::Main),
        "Sub" => Ok(AgentRole::Sub),
        other => Err(unknown_value("role", other)),
    }
}

pub(super) fn run_end_text(end: RunEnd) -> &'static str {
    match end {
        RunEnd::Completed => "Completed",
        RunEnd::Failed => "Failed",
        RunEnd::Stopped => "Stopped",
    }
}

pub(in crate::store) fn parse_run_end(text: &str) -> Result<RunEnd, StoreError> {
    match text {
        "Completed" => Ok(RunEnd::Completed),
        "Failed" => Ok(RunEnd::Failed),
        "Stopped" => Ok(RunEnd::Stopped),
        other => Err(unknown_value("run end", other)),
    }
}
