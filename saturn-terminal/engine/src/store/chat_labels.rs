//! 채팅 이름과 묶음: 작업 목록과 채팅 목록이 보인다.
//! 설계: docs/design/records.md

use saturn_protocol::ids::ChatId;

use super::records::{ensure_found, not_found};
use super::{Store, StoreError, to_sql_int};

impl Store {
    /// `None`이면 이름을 지운다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn set_chat_name(
        &self,
        chat: ChatId,
        name: Option<&str>,
    ) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET name = ? WHERE id = ?")
            .bind(name)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// `None`이면 묶음에서 뺀다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn set_chat_group(
        &self,
        chat: ChatId,
        group: Option<&str>,
    ) -> Result<(), StoreError> {
        let done = sqlx::query("UPDATE chats SET group_name = ? WHERE id = ?")
            .bind(group)
            .bind(to_sql_int(chat.0))
            .execute(&self.pool)
            .await?;
        ensure_found(done.rows_affected(), || format!("chat {}", chat.0))
    }

    /// (이름, 묶음). 붙이지 않았으면 `None`.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub(crate) async fn chat_labels(
        &self,
        chat: ChatId,
    ) -> Result<(Option<String>, Option<String>), StoreError> {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT name, group_name FROM chats WHERE id = ?")
                .bind(to_sql_int(chat.0))
                .fetch_optional(&self.pool)
                .await?;
        row.ok_or_else(|| not_found(format!("chat {}", chat.0)))
    }
}
