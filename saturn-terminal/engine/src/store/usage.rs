//! `/usage` 조회: 범위 안의 사용량 보고 원값, router 호출 합계, session의 실행 목록.
//! 설계: docs/design/records.md

use saturn_protocol::ids::{ChatId, RunId, SessionId};
use saturn_protocol::rpc::UsageRange;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::records::not_found;
use super::{Store, StoreError, UsageRow, from_millis, from_sql_int, parse_enum, to_sql_int};

/// router 하나의 범위 안 호출 합계. 토큰을 보고한 호출이 없으면 `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterUsage {
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
    pub async fn usage_rows(
        &self,
        range: UsageRange,
        chat: Option<ChatId>,
    ) -> Result<Vec<UsageRow>, StoreError> {
        let condition = range_condition(range, "usage.at", "usage.chat_id");
        let sql = format!(
            "SELECT usage.id, usage.run_id, usage.session_id, usage.body, usage.at, \
             usage.spans_turns, runs.provider FROM usage JOIN runs ON runs.id = usage.run_id \
             WHERE {condition} ORDER BY usage.id"
        );
        let rows = bind_chat(sqlx::query(&sql), range, chat)?
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(usage_row).collect()
    }

    /// router마다 한 행, 이름 순서.
    ///
    /// # Errors
    /// `Chat`인데 `chat`이 없으면 `NotFound`.
    pub async fn router_usage(
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

    /// 누적 보고가 몇 턴에 걸쳤는지 세는 데 쓴다. 실행 순서.
    pub async fn session_runs(&self, session: SessionId) -> Result<Vec<RunId>, StoreError> {
        let ids: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM runs WHERE session_id = ? ORDER BY id")
                .bind(to_sql_int(session.0))
                .fetch_all(&self.pool)
                .await?;
        Ok(ids.into_iter().map(|id| RunId(from_sql_int(id))).collect())
    }
}

/// `Today`는 로컬 오늘 0시부터, `Week`는 로컬 이번 주 월요일 0시부터다. 초안 값.
fn range_condition(range: UsageRange, at: &str, chat: &str) -> String {
    match range {
        UsageRange::Chat => format!("{chat} = ?"),
        UsageRange::Today => format!(
            "{at} >= CAST(strftime('%s', 'now', 'localtime', 'start of day', 'utc') AS INTEGER) * 1000"
        ),
        UsageRange::Week => format!(
            "{at} >= CAST(strftime('%s', 'now', 'localtime', 'start of day', 'weekday 0', \
             '-6 days', 'utc') AS INTEGER) * 1000"
        ),
        UsageRange::All => "1 = 1".to_owned(),
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
