//! 보류한 작업과 끊긴 하위 에이전트: engine이 다시 떠도 재개 제안과 막기를 이어 가기 위해 남긴다.
//! 설계: docs/design/records.md, docs/design/engine-lifecycle.md#크래시-뒤-복구

use saturn_protocol::ids::{AgentId, ChatId, InputId, SubagentId, TaskId};
use sqlx::Row;

use super::{Store, StoreError, from_sql_int, to_sql_int};

/// 멈출 때 실행 중이던 작업 하나. 입력과 에이전트가 없는 보류는 남기지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StoredHold {
    pub task: TaskId,
    pub chat: ChatId,
    pub agent: AgentId,
    pub input: InputId,
}

/// 끊긴 하위 에이전트 하나. `cleaned`는 provider에 정리를 넘겼다는 뜻이고 감시는 계속한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredInterrupted {
    pub agent: AgentId,
    pub subagent: SubagentId,
    pub cleaned: bool,
}

impl Store {
    /// 같은 작업이면 덮어쓴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅이나 입력 포함).
    pub(crate) async fn save_held_task(&self, hold: &StoredHold) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO held_tasks (task_id, chat_id, agent_id, input_id) VALUES (?, ?, ?, ?) \
             ON CONFLICT(task_id) DO UPDATE SET chat_id = excluded.chat_id, \
             agent_id = excluded.agent_id, input_id = excluded.input_id",
        )
        .bind(to_sql_int(hold.task.0))
        .bind(to_sql_int(hold.chat.0))
        .bind(to_sql_int(hold.agent.0))
        .bind(to_sql_int(hold.input.0))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 없는 작업이면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn delete_held_task(&self, task: TaskId) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM held_tasks WHERE task_id = ?")
            .bind(to_sql_int(task.0))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 보류한 순서(작업 번호)대로 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn held_tasks(&self) -> Result<Vec<StoredHold>, StoreError> {
        let rows = sqlx::query(
            "SELECT task_id, chat_id, agent_id, input_id FROM held_tasks ORDER BY task_id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(StoredHold {
                    task: TaskId(from_sql_int(row.try_get("task_id")?)),
                    chat: ChatId(from_sql_int(row.try_get("chat_id")?)),
                    agent: AgentId(from_sql_int(row.try_get("agent_id")?)),
                    input: InputId(from_sql_int(row.try_get("input_id")?)),
                })
            })
            .collect()
    }

    /// 이미 있으면 그대로 둔다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅 포함).
    pub(crate) async fn save_interrupted_subagent(
        &self,
        chat: ChatId,
        agent: AgentId,
        subagent: &SubagentId,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT OR IGNORE INTO interrupted_subagents (chat_id, agent_id, subagent) VALUES (?, ?, ?)",
        )
        .bind(to_sql_int(chat.0))
        .bind(to_sql_int(agent.0))
        .bind(&subagent.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 에이전트의 끊긴 하위 에이전트를 provider에 정리하게 넘겼다고 표시한다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn mark_interrupted_cleaned(&self, agent: AgentId) -> Result<(), StoreError> {
        sqlx::query("UPDATE interrupted_subagents SET cleaned = 1 WHERE agent_id = ?")
            .bind(to_sql_int(agent.0))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 에이전트가 끝났을 때 그 에이전트의 끊긴 하위 에이전트 정보를 지운다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn delete_interrupted_subagents(
        &self,
        agent: AgentId,
    ) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM interrupted_subagents WHERE agent_id = ?")
            .bind(to_sql_int(agent.0))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 기록한 순서대로 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn interrupted_subagents(&self) -> Result<Vec<StoredInterrupted>, StoreError> {
        let rows = sqlx::query(
            "SELECT agent_id, subagent, cleaned FROM interrupted_subagents ORDER BY rowid",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(StoredInterrupted {
                    agent: AgentId(from_sql_int(row.try_get("agent_id")?)),
                    subagent: SubagentId(row.try_get("subagent")?),
                    cleaned: row.try_get("cleaned")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::store::NewInput;
    use crate::store::tests::temp_store;

    async fn chat_with_input(store: &Store) -> (ChatId, InputId) {
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let input = store
            .accept_input(&NewInput {
                chat,
                text: "fix".to_owned(),
                settings: saturn_protocol::ids::SettingsRevision(1),
                permission: saturn_core::queue::Permission::Write,
                workdir: PathBuf::from("/work"),
                pinned_model: None,
                skip_relation: false,
            })
            .await
            .unwrap();
        (chat, input)
    }

    #[tokio::test]
    async fn held_task_is_saved_replaced_and_deleted() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_with_input(&store).await;
        let hold = StoredHold {
            task: TaskId(input.0),
            chat,
            agent: AgentId(3),
            input,
        };

        store.save_held_task(&hold).await.unwrap();
        let moved = StoredHold {
            agent: AgentId(4),
            ..hold
        };
        store.save_held_task(&moved).await.unwrap();
        assert_eq!(store.held_tasks().await.unwrap(), vec![moved]);

        store.delete_held_task(hold.task).await.unwrap();
        store.delete_held_task(hold.task).await.unwrap();
        assert!(store.held_tasks().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn interrupted_subagents_keep_cleaned_flag_until_the_agent_ends() {
        let (_dir, store) = temp_store().await;
        let (chat, _) = chat_with_input(&store).await;
        let (agent, other) = (AgentId(3), AgentId(4));
        let sub = |id: &str| SubagentId(id.to_owned());

        store
            .save_interrupted_subagent(chat, agent, &sub("a"))
            .await
            .unwrap();
        store
            .save_interrupted_subagent(chat, agent, &sub("a"))
            .await
            .unwrap();
        store
            .save_interrupted_subagent(chat, other, &sub("b"))
            .await
            .unwrap();
        store.mark_interrupted_cleaned(agent).await.unwrap();
        store
            .save_interrupted_subagent(chat, agent, &sub("a"))
            .await
            .unwrap();

        let all = store.interrupted_subagents().await.unwrap();
        assert_eq!(
            all,
            vec![
                StoredInterrupted {
                    agent,
                    subagent: sub("a"),
                    cleaned: true
                },
                StoredInterrupted {
                    agent: other,
                    subagent: sub("b"),
                    cleaned: false
                },
            ]
        );

        store.delete_interrupted_subagents(agent).await.unwrap();
        assert_eq!(store.interrupted_subagents().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn recovery_rows_follow_the_chat_when_it_is_deleted() {
        let (_dir, store) = temp_store().await;
        let (chat, input) = chat_with_input(&store).await;
        store
            .save_held_task(&StoredHold {
                task: TaskId(input.0),
                chat,
                agent: AgentId(3),
                input,
            })
            .await
            .unwrap();
        store
            .save_interrupted_subagent(chat, AgentId(3), &SubagentId("a".to_owned()))
            .await
            .unwrap();

        sqlx::query("DELETE FROM chats WHERE id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&store.pool)
            .await
            .unwrap();

        assert!(store.held_tasks().await.unwrap().is_empty());
        assert!(store.interrupted_subagents().await.unwrap().is_empty());
    }
}
