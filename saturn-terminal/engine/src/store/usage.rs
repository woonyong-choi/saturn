//! `/usage` 조회: 범위 안의 사용량 보고 원값, router 호출 합계, session의 실행 목록.
//! 설계: docs/design/records.md

use std::collections::HashSet;

use saturn_protocol::ids::{ChatId, RunId, SessionId};
use saturn_protocol::rpc::{ChatListItem, UsageRange};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::records::not_found;
use super::{Store, StoreError, UsageRow, from_millis, from_sql_int, parse_enum, to_sql_int};

/// 채팅의 마지막 활동 시각: 마지막 입력 접수, 입력이 없으면 만든 시각. `latest_chat_in`과 `list_chats`가 같은 기준을 쓴다.
const LAST_ACTIVE: &str =
    "COALESCE((SELECT MAX(accepted_at) FROM inputs WHERE chat_id = chats.id), created_at)";

/// router 하나의 범위 안 호출 합계. 토큰을 보고한 호출이 없으면 `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouterUsage {
    /// 중립 이름. 예: `jev`.
    pub router: String,
    pub calls: u32,
    pub input: Option<u64>,
    pub output: Option<u64>,
}

impl Store {
    /// 기록 순서로 돌려준다.
    ///
    /// # Errors
    /// `Chat`인데 `chat`이 없으면 `NotFound`.
    pub(crate) async fn usage_rows(
        &self,
        range: UsageRange,
        chat: Option<ChatId>,
    ) -> Result<Vec<UsageRow>, StoreError> {
        let condition = range_condition(range, "usage.at", "usage.chat_id");
        let sql = usage_sql(&condition);
        let rows = bind_chat(sqlx::query(&sql), range, chat)?
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(usage_row).collect()
    }

    /// 범위와 상관없이 모든 보고. 턴 값은 같은 session의 직전 누적을 뺀 값이라 범위 밖 보고가 필요하다.
    pub(crate) async fn all_usage_rows(&self) -> Result<Vec<UsageRow>, StoreError> {
        let rows = sqlx::query(&usage_sql("1 = 1"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(usage_row).collect()
    }

    /// 마지막 입력을 접수한 시각이 가장 늦은 채팅. 입력이 없는 채팅은 만든 시각으로 견준다.
    pub(crate) async fn latest_chat_in(&self, workdir: &str) -> Result<Option<ChatId>, StoreError> {
        let id: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT id FROM chats WHERE workdir = ? ORDER BY {LAST_ACTIVE} DESC, id DESC LIMIT 1"
        ))
        .bind(workdir)
        .fetch_optional(&self.pool)
        .await?;
        Ok(id.map(|id| ChatId(from_sql_int(id))))
    }

    // cost: time O(c log c), heap O(c), stack O(1), io 1
    // vars: c = 조회 범위의 채팅 수
    // basis: estimate
    /// `workdir`의 채팅을 `latest_chat_in`과 같은 기준으로 최근 것부터 돌려준다. `None`이면 모든 폴더.
    pub(crate) async fn list_chats(
        &self,
        workdir: Option<&str>,
    ) -> Result<Vec<ChatListItem>, StoreError> {
        let rows = sqlx::query(&format!(
            "SELECT id, workdir, name, {LAST_ACTIVE} AS last_active, \
             (SELECT text FROM inputs WHERE chat_id = chats.id ORDER BY id LIMIT 1) AS preview \
             FROM chats WHERE (?1 IS NULL OR workdir = ?1) ORDER BY last_active DESC, id DESC"
        ))
        .bind(workdir)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(ChatListItem {
                    chat: ChatId(from_sql_int(row.try_get("id")?)),
                    folder: row.try_get("workdir")?,
                    name: row.try_get("name")?,
                    last_active_ms: from_sql_int(row.try_get("last_active")?),
                    preview: row.try_get("preview")?,
                    rows: None,
                })
            })
            .collect()
    }

    /// router마다 한 행, 이름 순서.
    ///
    /// # Errors
    /// `Chat`인데 `chat`이 없으면 `NotFound`.
    pub(crate) async fn router_usage(
        &self,
        range: UsageRange,
        chat: Option<ChatId>,
    ) -> Result<Vec<RouterUsage>, StoreError> {
        let condition = range_condition(range, "started_at", "chat_id");
        let sql = format!(
            "SELECT router, COUNT(*) AS calls, SUM(input_tokens) AS input, \
             SUM(output_tokens) AS output FROM judgments WHERE {condition} \
             GROUP BY router ORDER BY router"
        );
        let rows = bind_chat(sqlx::query(&sql), range, chat)?
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                let calls: i64 = row.try_get("calls")?;
                Ok(RouterUsage {
                    router: row.try_get("router")?,
                    calls: u32::try_from(calls).unwrap_or(u32::MAX),
                    input: row.try_get::<Option<i64>, _>("input")?.map(from_sql_int),
                    output: row.try_get::<Option<i64>, _>("output")?.map(from_sql_int),
                })
            })
            .collect()
    }

    /// 채팅에서 `since`(unix 밀리초) 이후에 시작한 실행. 한 요청에 속한 실행을 고른다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn runs_started_since(
        &self,
        chat: ChatId,
        since: i64,
    ) -> Result<HashSet<RunId>, StoreError> {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM runs WHERE chat_id = ? AND started_at >= ?")
                .bind(to_sql_int(chat.0))
                .bind(since)
                .fetch_all(&self.pool)
                .await?;
        Ok(ids.into_iter().map(|id| RunId(from_sql_int(id))).collect())
    }

    /// 채팅에서 `since`(unix 밀리초) 이후에 시작한 router 호출 수와 보고한 토큰 합. 토큰을 하나도 보고하지 않았으면 `None`.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn router_usage_since(
        &self,
        chat: ChatId,
        since: i64,
    ) -> Result<(u32, Option<u64>), StoreError> {
        let row = sqlx::query(
            "SELECT COUNT(*) AS calls, SUM(input_tokens) AS input, SUM(output_tokens) AS output \
             FROM judgments WHERE chat_id = ? AND started_at >= ?",
        )
        .bind(to_sql_int(chat.0))
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        let calls: i64 = row.try_get("calls")?;
        let input = row.try_get::<Option<i64>, _>("input")?.map(from_sql_int);
        let output = row.try_get::<Option<i64>, _>("output")?.map(from_sql_int);
        let tokens = match (input, output) {
            (None, None) => None,
            (input, output) => Some(input.unwrap_or(0) + output.unwrap_or(0)),
        };
        Ok((u32::try_from(calls).unwrap_or(u32::MAX), tokens))
    }

    /// 누적 보고가 몇 턴에 걸쳤는지 세는 데 쓴다. 실행 순서.
    pub(crate) async fn session_runs(&self, session: SessionId) -> Result<Vec<RunId>, StoreError> {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM runs WHERE session_id = ? ORDER BY id")
                .bind(to_sql_int(session.0))
                .fetch_all(&self.pool)
                .await?;
        Ok(ids.into_iter().map(|id| RunId(from_sql_int(id))).collect())
    }
}

fn usage_sql(condition: &str) -> String {
    format!(
        "SELECT usage.id, usage.run_id, usage.session_id, usage.body, usage.at, \
         usage.spans_turns, runs.provider FROM usage JOIN runs ON runs.id = usage.run_id \
         WHERE {condition} ORDER BY usage.id"
    )
}

/// `Day`는 지금부터 24시간 전, `Week`는 7일 전부터다(Claude Code와 같은 방식).
fn range_condition(range: UsageRange, at: &str, chat: &str) -> String {
    match range {
        UsageRange::Chat => format!("{chat} = ?"),
        UsageRange::Day => {
            format!("{at} >= (CAST(strftime('%s', 'now') AS INTEGER) - 86400) * 1000")
        }
        UsageRange::Week => {
            format!("{at} >= (CAST(strftime('%s', 'now') AS INTEGER) - 604800) * 1000")
        }
    }
}

type Query<'q> = sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>;

fn bind_chat(
    query: Query<'_>,
    range: UsageRange,
    chat: Option<ChatId>,
) -> Result<Query<'_>, StoreError> {
    if range != UsageRange::Chat {
        return Ok(query);
    }
    let chat = chat.ok_or_else(|| not_found("chat for usage range".to_owned()))?;
    Ok(query.bind(to_sql_int(chat.0)))
}

fn usage_row(row: &SqliteRow) -> Result<UsageRow, StoreError> {
    Ok(UsageRow {
        id: from_sql_int(row.try_get("id")?),
        run: RunId(from_sql_int(row.try_get("run_id")?)),
        session: SessionId(from_sql_int(row.try_get("session_id")?)),
        provider: parse_enum(row.try_get("provider")?)?,
        report: serde_json::from_str(row.try_get("body")?)?,
        at: from_millis(row.try_get("at")?),
        spans_turns: row.try_get("spans_turns")?,
    })
}
