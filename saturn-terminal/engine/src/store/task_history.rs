//! 끝난 작업: `runs`에서 작업별 마지막 실행을 읽는다. 새 저장 칸은 없다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::{ChatId, TaskId};
use sqlx::Row;

use super::records::parse_run_end;
use super::{RunEnd, Store, StoreError, from_sql_int};

/// 채팅마다 이 수까지만 돌려준다(초안).
pub(crate) const ENDED_TASKS_PER_CHAT: u32 = 20;

/// 작업의 마지막 실행이 `Completed`나 `Failed`로 끝났다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EndedTask {
    pub chat: ChatId,
    pub task: TaskId,
    pub end: RunEnd,
    /// unix 밀리초.
    pub ended_at_ms: u64,
}

impl Store {
    /// 작업별 마지막 실행이 `Completed`나 `Failed`인 작업을 채팅 번호, 작업 번호 순서로 돌려준다.
    /// 마지막 실행이 `Stopped`이거나 열려 있으면 뺀다. 채팅마다 끝난 시각이 늦은 `ENDED_TASKS_PER_CHAT`개까지.
    ///
    /// # Errors
    /// 읽지 못하면 `Database`.
    pub(crate) async fn ended_tasks(&self) -> Result<Vec<EndedTask>, StoreError> {
        let rows = sqlx::query(
            "SELECT chat_id, task_id, end_kind, ended_at FROM ( \
                 SELECT chat_id, task_id, end_kind, ended_at, \
                        ROW_NUMBER() OVER (PARTITION BY chat_id ORDER BY ended_at DESC, id DESC) AS rank \
                 FROM runs \
                 WHERE id IN (SELECT MAX(id) FROM runs GROUP BY chat_id, task_id) \
                   AND end_kind IN ('Completed', 'Failed') AND ended_at IS NOT NULL \
             ) WHERE rank <= ? ORDER BY chat_id, task_id",
        )
        .bind(i64::from(ENDED_TASKS_PER_CHAT))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                let end: String = row.try_get("end_kind")?;
                Ok(EndedTask {
                    chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                    task: TaskId(from_sql_int(row.try_get("task_id")?)),
                    end: parse_run_end(&end)?,
                    ended_at_ms: from_sql_int(row.try_get("ended_at")?),
                })
            })
            .collect()
    }
}
