//! TUI가 붙을 때 화면을 되살리는 채팅 기록 조회. 접수한 입력과 provider 실행(답)을 시각 순서로 합친다.
//! 설계: docs/design/engine-lifecycle.md

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, Provider, TaskId};
use saturn_protocol::state::{InputState, QueueReason};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use super::records::parse_run_end;
use super::{RunEnd, Store, StoreError, from_sql_int, parse_enum, to_sql_int};

const INPUT_KIND: i64 = 0;

/// 기록 한 단위. 입력 하나, 또는 실행 하나와 그 답이다. 글자 조각과 도구 호출은 실행 안에 묶인다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HistoryEntry {
    Input {
        input: InputId,
        text: String,
        state: InputState,
        reason: Option<QueueReason>,
    },
    Run {
        task: TaskId,
        provider: Provider,
        /// 끝나지 않은 실행이면 `None`.
        end: Option<RunEnd>,
        /// 끝난 실행의 걸린 시간. 끝나지 않았으면 0.
        elapsed_ms: u64,
        events: Vec<ProviderEvent>,
    },
}

impl Store {
    /// 끝에서 `limit`단위를 오래된 것부터 돌려주고, 그보다 앞 기록이 있으면 참을 함께 준다.
    /// 단위는 입력 하나 또는 실행 하나이고, 실행의 이벤트는 개수와 상관없이 모두 담는다.
    /// 같은 밀리초에 쌓인 것은 입력, 그 입력의 실행 순서로 둔다.
    ///
    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn recent_history(
        &self,
        chat: ChatId,
        limit: u32,
    ) -> Result<(Vec<HistoryEntry>, bool), StoreError> {
        let rows = sqlx::query(
            "SELECT kind, id, task, text, state, reason, provider, ended_at, end_kind, at FROM ( \
             SELECT 0 AS kind, id, id AS owner, NULL AS task, text, state, reason, \
             NULL AS provider, NULL AS ended_at, NULL AS end_kind, accepted_at AS at \
             FROM inputs WHERE chat_id = ?1 \
             UNION ALL \
             SELECT 1 AS kind, id, COALESCE(input_id, 0) AS owner, task_id AS task, NULL AS text, \
             NULL AS state, NULL AS reason, provider, ended_at, end_kind, started_at AS at \
             FROM runs WHERE chat_id = ?1) \
             ORDER BY at DESC, owner DESC, kind DESC, id DESC LIMIT ?2",
        )
        .bind(to_sql_int(chat.0))
        .bind(i64::from(limit) + 1)
        .fetch_all(&self.pool)
        .await?;
        let limit = usize::try_from(limit).expect("u32 limit should fit in usize");
        let has_more = rows.len() > limit;
        let mut entries = Vec::with_capacity(rows.len().min(limit));
        for row in rows.iter().take(limit) {
            entries.push(self.history_entry(row).await?);
        }
        entries.reverse();
        Ok((entries, has_more))
    }

    async fn history_entry(&self, row: &SqliteRow) -> Result<HistoryEntry, StoreError> {
        let id: i64 = row.try_get("id")?;
        if row.try_get::<i64, _>("kind")? == INPUT_KIND {
            let reason = row
                .try_get::<Option<String>, _>("reason")?
                .map(|text| serde_json::from_str(&text))
                .transpose()?;
            return Ok(HistoryEntry::Input {
                input: InputId(from_sql_int(id)),
                text: row.try_get("text")?,
                state: parse_enum(row.try_get("state")?)?,
                reason,
            });
        }
        let bodies: Vec<String> =
            sqlx::query_scalar("SELECT body FROM events WHERE run_id = ? ORDER BY seq")
                .bind(id)
                .fetch_all(&self.pool)
                .await?;
        let events = bodies
            .iter()
            .map(|body| serde_json::from_str(body))
            .collect::<Result<Vec<_>, _>>()?;
        let started: i64 = row.try_get("at")?;
        let ended: Option<i64> = row.try_get("ended_at")?;
        Ok(HistoryEntry::Run {
            task: TaskId(from_sql_int(row.try_get("task")?)),
            provider: parse_enum(row.try_get("provider")?)?,
            end: row
                .try_get::<Option<String>, _>("end_kind")?
                .map(|text| parse_run_end(&text))
                .transpose()?,
            elapsed_ms: ended.map_or(0, |ended| u64::try_from(ended - started).unwrap_or(0)),
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::ids::AgentId;

    use super::*;
    use crate::store::records::tests::{chat_with_run, new_input, new_run};
    use crate::store::tests::temp_store;

    fn text(text: &str) -> ProviderEvent {
        ProviderEvent::Text {
            agent: AgentId(1),
            subagent: None,
            text: text.to_owned(),
        }
    }

    fn run_of(entry: &HistoryEntry) -> (&Option<RunEnd>, &Vec<ProviderEvent>) {
        let HistoryEntry::Run { end, events, .. } = entry else {
            panic!("expected a run, got {entry:?}");
        };
        (end, events)
    }

    #[tokio::test]
    async fn recent_history_merges_inputs_and_runs_in_order() {
        let (_dir, store) = temp_store().await;
        let (chat, input, _, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text("one")).await.unwrap();
        store.append_event(run, chat, &text("two")).await.unwrap();
        store.finish_run(run, RunEnd::Completed).await.unwrap();
        let other = store.create_chat("/other".into()).await.unwrap();
        store
            .accept_input(&new_input(other, "elsewhere"))
            .await
            .unwrap();

        let (entries, has_more) = store.recent_history(chat, 10).await.unwrap();

        assert!(!has_more);
        assert_eq!(entries.len(), 2);
        assert!(matches!(
            &entries[0],
            HistoryEntry::Input { input: id, text, state: InputState::Judging, reason: None }
                if *id == input && text == "fix the build"
        ));
        let (end, events) = run_of(&entries[1]);
        assert_eq!(*end, Some(RunEnd::Completed));
        assert_eq!(events, &vec![text("one"), text("two")]);
    }

    #[tokio::test]
    async fn recent_history_counts_inputs_and_runs_not_events() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, first) = chat_with_run(&store).await;
        for index in 0..60 {
            store
                .append_event(first, chat, &text(&format!("a{index}")))
                .await
                .unwrap();
        }
        let second_input = store
            .accept_input(&new_input(chat, "second"))
            .await
            .unwrap();
        let second = store
            .start_run(&new_run(Some(second_input), session))
            .await
            .unwrap();
        for index in 0..60 {
            store
                .append_event(second, chat, &text(&format!("b{index}")))
                .await
                .unwrap();
        }

        let (entries, has_more) = store.recent_history(chat, 4).await.unwrap();

        assert!(!has_more);
        assert_eq!(entries.len(), 4);
        assert!(matches!(&entries[0], HistoryEntry::Input { text, .. } if text == "fix the build"));
        assert_eq!(run_of(&entries[1]).1.len(), 60);
        assert!(matches!(&entries[2], HistoryEntry::Input { text, .. } if text == "second"));
        assert_eq!(run_of(&entries[3]).1.len(), 60);
    }

    #[tokio::test]
    async fn recent_history_keeps_latest_and_reports_more() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        let second_input = store.accept_input(&new_input(chat, "two")).await.unwrap();
        store
            .start_run(&new_run(Some(second_input), session))
            .await
            .unwrap();

        let (entries, has_more) = store.recent_history(chat, 2).await.unwrap();

        assert!(has_more);
        assert!(matches!(&entries[0], HistoryEntry::Input { text, .. } if text == "two"));
        assert_eq!(run_of(&entries[1]), (&None, &Vec::new()));
        let (empty, more) = store.recent_history(ChatId(99), 5).await.unwrap();
        assert!(empty.is_empty());
        assert!(!more);
    }
}
