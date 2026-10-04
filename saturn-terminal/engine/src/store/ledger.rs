//! 채팅 기록을 session과 입력 원문까지 이어 읽는다. 패킷과 변경분 재료다.
//! 설계: docs/design/records.md, docs/design/context-management.md

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, LedgerSeq, RunId, SessionId, TaskId};
use sqlx::Row;

use super::records::parse_run_end;
use super::{RunEnd, Store, StoreError, from_sql_int, to_sql_int};

/// 이벤트 한 건과 그 이벤트를 낸 실행의 session, 실행을 연 입력 원문.
#[derive(Debug, Clone)]
pub(crate) struct LedgerRow {
    pub(crate) seq: LedgerSeq,
    pub(crate) run: RunId,
    pub(crate) session: SessionId,
    /// 실행이 속한 작업.
    pub(crate) task: TaskId,
    /// 입력 없이 provider가 시작한 실행이면 `None`.
    pub(crate) input: Option<String>,
    /// 실행이 끝난 방식. 아직 끝나지 않았으면 `None`.
    pub(crate) end: Option<RunEnd>,
    /// unix 밀리초, UTC.
    pub(crate) at_ms: i64,
    pub(crate) event: ProviderEvent,
}

impl Store {
    /// `after`보다 큰 기록 번호의 이벤트를 번호 순서로 돌려준다.
    ///
    /// # Errors
    /// 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn ledger_since(
        &self,
        chat: ChatId,
        after: LedgerSeq,
    ) -> Result<Vec<LedgerRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT e.seq, e.run_id, r.session_id, r.task_id, r.end_kind, i.text, e.at, e.body FROM events e \
             JOIN runs r ON r.id = e.run_id \
             LEFT JOIN inputs i ON i.id = r.input_id \
             WHERE e.chat_id = ? AND e.seq > ? ORDER BY e.seq",
        )
        .bind(to_sql_int(chat.0))
        .bind(to_sql_int(after.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(LedgerRow {
                    seq: LedgerSeq(from_sql_int(row.try_get("seq")?)),
                    run: RunId(from_sql_int(row.try_get("run_id")?)),
                    session: SessionId(from_sql_int(row.try_get("session_id")?)),
                    task: TaskId(from_sql_int(row.try_get("task_id")?)),
                    input: row.try_get("text")?,
                    end: row
                        .try_get::<Option<String>, _>("end_kind")?
                        .map(|text| parse_run_end(&text))
                        .transpose()?,
                    at_ms: row.try_get("at")?,
                    event: serde_json::from_str(row.try_get("body")?)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::ids::AgentId;

    use super::*;
    use crate::store::records::tests::chat_with_run;
    use crate::store::tests::temp_store;

    fn text(agent: u64, text: &str) -> ProviderEvent {
        ProviderEvent::Text {
            agent: AgentId(agent),
            subagent: None,
            text: text.to_owned(),
        }
    }

    #[tokio::test]
    async fn ledger_since_joins_session_and_input_text_in_seq_order() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, session, run) = chat_with_run(&store).await;
        let first = store.append_event(run, chat, &text(1, "a")).await.unwrap();
        let second = store.append_event(run, chat, &text(1, "b")).await.unwrap();

        let rows = store.ledger_since(chat, first).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].seq, second);
        assert_eq!(rows[0].session, session);
        assert_eq!(rows[0].run, run);
        assert!(rows[0].input.is_some());
        assert_eq!(rows[0].event, text(1, "b"));
    }

    #[tokio::test]
    async fn ledger_since_carries_how_the_run_ended() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, _session, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text(1, "a")).await.unwrap();

        let running = store.ledger_since(chat, LedgerSeq(0)).await.unwrap();
        store.finish_run(run, RunEnd::Stopped).await.unwrap();
        let ended = store.ledger_since(chat, LedgerSeq(0)).await.unwrap();

        assert_eq!(running[0].end, None);
        assert_eq!(ended[0].end, Some(RunEnd::Stopped));
    }

    #[tokio::test]
    async fn ledger_since_other_chat_is_empty() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, _session, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text(1, "a")).await.unwrap();

        let rows = store
            .ledger_since(ChatId(chat.0 + 1), LedgerSeq(0))
            .await
            .unwrap();

        assert!(rows.is_empty());
    }
}
