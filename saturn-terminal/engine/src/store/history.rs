//! TUI가 붙을 때 화면을 되살리는 채팅 기록 조회. 접수한 입력과 provider 이벤트를 시각 순서로 합친다.
//! 설계: docs/design/engine-lifecycle.md

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, LedgerSeq, TaskId};
use saturn_protocol::state::{InputState, QueueReason};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::{Store, StoreError, from_sql_int, parse_enum, to_sql_int};

const INPUT_KIND: i64 = 0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HistoryEntry {
    Input {
        input: InputId,
        text: String,
        state: InputState,
        reason: Option<QueueReason>,
    },
    Event {
        seq: LedgerSeq,
        task: TaskId,
        event: ProviderEvent,
    },
}

impl Store {
    /// 끝에서 `limit`개를 오래된 것부터 돌려주고, 그보다 앞 기록이 있으면 참을 함께 준다.
    ///
    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn recent_history(
        &self,
        chat: ChatId,
        limit: u32,
    ) -> Result<(Vec<HistoryEntry>, bool), StoreError> {
        let rows = sqlx::query(
            "SELECT kind, id, task, text, body, state, reason, at FROM ( \
             SELECT 0 AS kind, id, NULL AS task, text, NULL AS body, state, reason, \
             accepted_at AS at FROM inputs WHERE chat_id = ?1 \
             UNION ALL \
             SELECT 1 AS kind, events.seq AS id, runs.task_id AS task, NULL AS text, events.body, \
             NULL AS state, NULL AS reason, events.at FROM events \
             JOIN runs ON runs.id = events.run_id WHERE events.chat_id = ?1) \
             ORDER BY at DESC, kind DESC, id DESC LIMIT ?2",
        )
        .bind(to_sql_int(chat.0))
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await?;
        let limit = usize::try_from(limit).expect("u32 limit should fit in usize");
        let has_more = rows.len() > limit;
        let mut entries = rows
            .iter()
            .take(limit)
            .map(history_entry)
            .collect::<Result<Vec<_>, _>>()?;
        entries.reverse();
        Ok((entries, has_more))
    }
}

fn history_entry(row: &SqliteRow) -> Result<HistoryEntry, StoreError> {
    let id = from_sql_int(row.try_get("id")?);
    if row.try_get::<i64, _>("kind")? == INPUT_KIND {
        let reason = row
            .try_get::<Option<String>, _>("reason")?
            .map(|text| serde_json::from_str(&text))
            .transpose()?;
        return Ok(HistoryEntry::Input {
            input: InputId(id),
            text: row.try_get("text")?,
            state: parse_enum(row.try_get("state")?)?,
            reason,
        });
    }
    Ok(HistoryEntry::Event {
        seq: LedgerSeq(id),
        task: TaskId(from_sql_int(row.try_get("task")?)),
        event: serde_json::from_str(row.try_get("body")?)?,
    })
}

#[cfg(test)]
mod tests {
    use saturn_protocol::ids::AgentId;

    use super::*;
    use crate::store::records::tests::{chat_with_run, new_input};
    use crate::store::tests::temp_store;

    fn text(text: &str) -> ProviderEvent {
        ProviderEvent::Text {
            agent: AgentId(1),
            subagent: None,
            text: text.to_owned(),
        }
    }

    #[tokio::test]
    async fn recent_history_merges_inputs_and_events_in_order() {
        let (_dir, store) = temp_store().await;
        let (chat, input, _, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text("one")).await.unwrap();
        store.append_event(run, chat, &text("two")).await.unwrap();
        let other = store.create_chat("/other".into()).await.unwrap();
        store
            .accept_input(&new_input(other, "elsewhere"))
            .await
            .unwrap();

        let (entries, has_more) = store.recent_history(chat, 10).await.unwrap();

        assert!(!has_more);
        assert_eq!(entries.len(), 3);
        assert!(matches!(
            &entries[0],
            HistoryEntry::Input { input: id, text, state: InputState::Judging, reason: None }
                if *id == input && text == "fix the build"
        ));
        assert_eq!(
            entries[1],
            HistoryEntry::Event {
                seq: LedgerSeq(1),
                task: TaskId(1),
                event: text("one"),
            }
        );
        assert!(matches!(
            &entries[2],
            HistoryEntry::Event {
                seq: LedgerSeq(2),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn recent_history_keeps_latest_and_reports_more() {
        let (_dir, store) = temp_store().await;
        let (chat, _, _, run) = chat_with_run(&store).await;
        for word in ["a", "b", "c"] {
            store.append_event(run, chat, &text(word)).await.unwrap();
        }

        let (entries, has_more) = store.recent_history(chat, 2).await.unwrap();

        assert!(has_more);
        assert_eq!(
            entries,
            vec![
                HistoryEntry::Event {
                    seq: LedgerSeq(2),
                    task: TaskId(1),
                    event: text("b"),
                },
                HistoryEntry::Event {
                    seq: LedgerSeq(3),
                    task: TaskId(1),
                    event: text("c"),
                },
            ]
        );
        let (empty, more) = store.recent_history(ChatId(99), 5).await.unwrap();
        assert!(empty.is_empty());
        assert!(!more);
    }
}
