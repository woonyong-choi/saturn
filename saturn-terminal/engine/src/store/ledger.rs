//! 채팅 기록을 session과 입력 원문까지 이어 읽는다. 패킷과 변경분 재료다.
//! 설계: docs/design/records.md, docs/design/context-management.md

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, LedgerSeq, RunId, SessionId, TaskId};
use saturn_protocol::state::InputState;
use sqlx::Row;

use super::records::parse_run_end;
use super::{RunEnd, Store, StoreError, enum_text, from_sql_int, to_sql_int};

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

/// 실행 중에 끼워 넣어 적용한 입력. 패킷이 그 실행의 사용자 입력으로 함께 읽는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SteeredInput {
    pub(crate) input: InputId,
    /// 입력이 들어간 실행.
    pub(crate) run: RunId,
    /// 입력을 적용할 때까지 채팅에 쌓인 마지막 기록 번호. 입력이 이벤트 사이 어디에 끼었는지를 나타낸다.
    pub(crate) after: LedgerSeq,
    pub(crate) text: String,
}

impl Store {
    /// 끼워 넣기가 적용된 입력을 그 실행에 묶고 지금까지 쌓인 기록 번호를 남긴다. 같은 입력을 다시 부르면 덮어쓴다.
    ///
    /// # Errors
    /// 저장 실패면 `Database`.
    pub(crate) async fn mark_steered(&self, input: InputId, run: RunId) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE inputs SET steered_run = ?, steered_after = \
             (SELECT COALESCE(MAX(seq), 0) FROM events WHERE chat_id = inputs.chat_id) WHERE id = ?",
        )
        .bind(to_sql_int(run.0))
        .bind(to_sql_int(input.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 채팅에서 끼워 넣어 적용한 입력을 접수 순서로 돌려준다.
    ///
    /// # Errors
    /// 저장된 값이 깨졌으면 `Database`.
    pub(crate) async fn steered_inputs(
        &self,
        chat: ChatId,
    ) -> Result<Vec<SteeredInput>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, text, steered_run, steered_after FROM inputs \
             WHERE chat_id = ? AND steered_run IS NOT NULL AND state = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .bind(enum_text(&InputState::Applied)?)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(SteeredInput {
                    input: InputId(from_sql_int(row.try_get("id")?)),
                    run: RunId(from_sql_int(row.try_get("steered_run")?)),
                    after: LedgerSeq(from_sql_int(row.try_get("steered_after")?)),
                    text: row.try_get("text")?,
                })
            })
            .collect()
    }

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

    #[tokio::test]
    async fn steered_inputs_keep_acceptance_order_and_the_sequence_they_arrived_after() {
        let (_dir, store) = temp_store().await;
        let (chat, _input, _session, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text(1, "a")).await.unwrap();
        let first = store
            .accept_input(&crate::store::records::tests::new_input(
                chat,
                "use a branch",
            ))
            .await
            .unwrap();
        let second = store
            .accept_input(&crate::store::records::tests::new_input(
                chat,
                "and skip docs",
            ))
            .await
            .unwrap();
        store.mark_steered(first, run).await.unwrap();
        store.append_event(run, chat, &text(1, "b")).await.unwrap();
        store.mark_steered(second, run).await.unwrap();
        store
            .set_input_state(first, InputState::Applied, None)
            .await
            .unwrap();
        store
            .set_input_state(second, InputState::Applied, None)
            .await
            .unwrap();

        let steered = store.steered_inputs(chat).await.unwrap();

        assert_eq!(
            steered,
            vec![
                SteeredInput {
                    input: first,
                    run,
                    after: LedgerSeq(1),
                    text: "use a branch".to_owned()
                },
                SteeredInput {
                    input: second,
                    run,
                    after: LedgerSeq(2),
                    text: "and skip docs".to_owned()
                },
            ]
        );
    }

    #[tokio::test]
    async fn steered_inputs_leave_out_inputs_that_are_not_applied_and_run_openers() {
        let (_dir, store) = temp_store().await;
        let (chat, opener, _session, run) = chat_with_run(&store).await;
        store.append_event(run, chat, &text(1, "a")).await.unwrap();
        let returned = store
            .accept_input(&crate::store::records::tests::new_input(
                chat,
                "refused later",
            ))
            .await
            .unwrap();
        let queued = store
            .accept_input(&crate::store::records::tests::new_input(chat, "waits"))
            .await
            .unwrap();
        store.mark_steered(returned, run).await.unwrap();
        store
            .set_input_state(returned, InputState::Queued, None)
            .await
            .unwrap();
        store
            .set_input_state(opener, InputState::Applied, None)
            .await
            .unwrap();

        let steered = store.steered_inputs(chat).await.unwrap();

        assert!(steered.is_empty(), "{steered:?} {queued:?}");
    }
}
