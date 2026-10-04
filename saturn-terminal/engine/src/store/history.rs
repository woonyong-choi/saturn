//! TUI가 붙을 때 화면을 되살리는 채팅 기록 조회. 접수한 입력과 provider 실행(답)을 시각 순서로 합친다.
//! 설계: docs/design/engine-lifecycle.md

use saturn_protocol::event::{ProviderEvent, UsageReport};
use saturn_protocol::ids::{ChatId, InputId, LedgerSeq, Provider, TaskId};
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
        /// 이 실행이 보고한 사용량. `usage` 표의 원값을 보고 순서로 담는다. 이벤트 표에는 없다.
        usage: Vec<UsageReport>,
        /// 같은 에이전트의 앞 실행이 다른 provider였으면 그 provider. 메인 전환을 이 차이로 되살린다.
        switched_from: Option<Provider>,
    },
}

/// 한 번에 읽은 기록 묶음. `oldest`는 담긴 단위 중 가장 오래된 것의 기록 시각(unix 밀리초)이다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoryPage {
    pub(crate) entries: Vec<HistoryEntry>,
    /// 묶음이 비면 `None`.
    pub(crate) oldest: Option<LedgerSeq>,
    pub(crate) has_more: bool,
}

const UNITS: &str = "SELECT 0 AS kind, id, id AS owner, NULL AS task, text, state, reason, \
     NULL AS provider, NULL AS ended_at, NULL AS end_kind, accepted_at AS at \
     FROM inputs WHERE chat_id = ?1 \
     UNION ALL \
     SELECT 1 AS kind, id, COALESCE(input_id, 0) AS owner, task_id AS task, NULL AS text, \
     NULL AS state, NULL AS reason, provider, ended_at, end_kind, started_at AS at \
     FROM runs WHERE chat_id = ?1";

impl Store {
    /// `before`보다 앞 기록에서 끝 `limit`단위를 오래된 것부터 돌려준다. `before`가 없으면 가장 최근부터다.
    /// 단위는 입력 하나 또는 실행 하나이고, 실행의 이벤트는 개수와 상관없이 모두 담는다.
    /// 같은 밀리초에 쌓인 단위는 입력, 그 입력의 실행 순서로 두고, 묶음 경계가 그 사이를 가르지 않도록
    /// 가장 오래된 밀리초의 단위는 `limit`을 넘더라도 모두 담는다. 그래야 `oldest`보다 앞을 이어 받을 때
    /// 겹치거나 빠지는 단위가 없다.
    ///
    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn history_page(
        &self,
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    ) -> Result<HistoryPage, StoreError> {
        let before = before.map_or(i64::MAX, |seq| i64::try_from(seq.0).unwrap_or(i64::MAX));
        let skip = i64::from(limit.max(1)) - 1;
        let boundary: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT at FROM ({UNITS}) WHERE at < ?2 ORDER BY at DESC LIMIT 1 OFFSET ?3"
        ))
        .bind(to_sql_int(chat.0))
        .bind(before)
        .bind(skip)
        .fetch_optional(&self.pool)
        .await?;
        let from = boundary.unwrap_or(i64::MIN);
        let rows = sqlx::query(&format!(
            "SELECT * FROM ({UNITS}) WHERE at >= ?3 AND at < ?2 \
             ORDER BY at ASC, owner ASC, kind ASC, id ASC"
        ))
        .bind(to_sql_int(chat.0))
        .bind(before)
        .bind(from)
        .fetch_all(&self.pool)
        .await?;
        let has_more: bool = boundary.is_some()
            && sqlx::query_scalar(&format!(
                "SELECT EXISTS(SELECT 1 FROM ({UNITS}) WHERE at < ?2)"
            ))
            .bind(to_sql_int(chat.0))
            .bind(from)
            .fetch_one(&self.pool)
            .await?;
        let oldest = match rows.first() {
            Some(row) => Some(LedgerSeq(from_sql_int(row.try_get("at")?))),
            None => None,
        };
        let mut entries = Vec::with_capacity(rows.len());
        for row in &rows {
            entries.push(self.history_entry(row).await?);
        }
        Ok(HistoryPage {
            entries,
            oldest,
            has_more,
        })
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
        let usage =
            sqlx::query_scalar::<_, String>("SELECT body FROM usage WHERE run_id = ? ORDER BY id")
                .bind(id)
                .fetch_all(&self.pool)
                .await?
                .iter()
                .map(|body| serde_json::from_str(body))
                .collect::<Result<Vec<UsageReport>, _>>()?;
        let provider: Provider = parse_enum(row.try_get("provider")?)?;
        let previous: Option<String> = sqlx::query_scalar(
            "SELECT previous.provider FROM runs AS previous JOIN runs AS current \
             ON previous.chat_id = current.chat_id AND previous.agent_id = current.agent_id \
             AND previous.id < current.id WHERE current.id = ? ORDER BY previous.id DESC LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let switched_from = previous
            .map(|text| parse_enum::<Provider>(&text))
            .transpose()?
            .filter(|before| *before != provider);
        let started: i64 = row.try_get("at")?;
        let ended: Option<i64> = row.try_get("ended_at")?;
        Ok(HistoryEntry::Run {
            task: TaskId(from_sql_int(row.try_get("task")?)),
            provider,
            end: row
                .try_get::<Option<String>, _>("end_kind")?
                .map(|text| parse_run_end(&text))
                .transpose()?,
            elapsed_ms: ended.map_or(0, |ended| u64::try_from(ended - started).unwrap_or(0)),
            events,
            usage,
            switched_from,
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

        let HistoryPage {
            entries, has_more, ..
        } = store.history_page(chat, None, 10).await.unwrap();

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

        let HistoryPage {
            entries, has_more, ..
        } = store.history_page(chat, None, 4).await.unwrap();

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
        // 기록 시각이 밀리초라 단위마다 시각이 다르도록 쉰다
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second_input = store.accept_input(&new_input(chat, "two")).await.unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        store
            .start_run(&new_run(Some(second_input), session))
            .await
            .unwrap();

        let HistoryPage {
            entries, has_more, ..
        } = store.history_page(chat, None, 2).await.unwrap();

        assert!(has_more);
        assert!(matches!(&entries[0], HistoryEntry::Input { text, .. } if text == "two"));
        assert_eq!(run_of(&entries[1]), (&None, &Vec::new()));
        let empty = store.history_page(ChatId(99), None, 5).await.unwrap();
        assert!(empty.entries.is_empty());
        assert_eq!(empty.oldest, None);
        assert!(!empty.has_more);
    }

    #[tokio::test]
    async fn history_page_keeps_units_of_the_same_millisecond_together() {
        let (_dir, store) = temp_store().await;
        let (chat, _, session, _) = chat_with_run(&store).await;
        let second_input = store.accept_input(&new_input(chat, "two")).await.unwrap();
        store
            .start_run(&new_run(Some(second_input), session))
            .await
            .unwrap();
        sqlx::query("UPDATE inputs SET accepted_at = 5000")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE runs SET started_at = 5000")
            .execute(&store.pool)
            .await
            .unwrap();

        let page = store.history_page(chat, None, 1).await.unwrap();

        assert_eq!(page.entries.len(), 4);
        assert_eq!(page.oldest, Some(LedgerSeq(5000)));
        assert!(!page.has_more);
        let before = store.history_page(chat, page.oldest, 1).await.unwrap();
        assert!(before.entries.is_empty());
    }
}
