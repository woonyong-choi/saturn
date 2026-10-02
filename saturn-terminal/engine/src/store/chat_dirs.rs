//! 채팅에 더한 폴더: 이어 열어도 그대로 되살린다.
//! 설계: docs/design/engine-lifecycle.md#채팅-폴더와-이어-열기

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use saturn_protocol::ids::ChatId;

use super::{Store, StoreError, to_millis, to_sql_int};

impl Store {
    // cost: time O(log n), heap O(1), stack O(1), io 1
    // vars: n = 저장한 더한 폴더 수
    // basis: estimate
    /// 이미 더한 폴더면 `false`.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`(없는 채팅 포함).
    pub async fn add_chat_dir(&self, chat: ChatId, path: &Path) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO chat_dirs (chat_id, path, added_at) VALUES (?, ?, ?)",
        )
        .bind(to_sql_int(chat.0))
        .bind(path.to_string_lossy().into_owned())
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    // cost: time O(n), heap O(n), stack O(1), alloc n, io 1
    // vars: n = 그 채팅의 더한 폴더 수
    // basis: estimate
    /// 더한 순서대로 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub async fn chat_dirs(&self, chat: ChatId) -> Result<Vec<PathBuf>, StoreError> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT path FROM chat_dirs WHERE chat_id = ? ORDER BY id")
                .bind(to_sql_int(chat.0))
                .fetch_all(&self.pool)
                .await?;
        Ok(rows.into_iter().map(PathBuf::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn add_dir_is_kept_per_chat_in_added_order_without_duplicates() {
        let (_dir, store) = temp_store().await;
        let first = store.create_chat(PathBuf::from("/work")).await.unwrap();
        let second = store.create_chat(PathBuf::from("/work")).await.unwrap();

        let added = store.add_chat_dir(first, Path::new("/b")).await.unwrap();
        store.add_chat_dir(first, Path::new("/a")).await.unwrap();
        let again = store.add_chat_dir(first, Path::new("/b")).await.unwrap();
        store.add_chat_dir(second, Path::new("/c")).await.unwrap();

        assert!(added);
        assert!(!again);
        assert_eq!(
            store.chat_dirs(first).await.unwrap(),
            vec![PathBuf::from("/b"), PathBuf::from("/a")]
        );
        assert_eq!(
            store.chat_dirs(second).await.unwrap(),
            vec![PathBuf::from("/c")]
        );
    }

    #[tokio::test]
    async fn add_dir_for_a_missing_chat_is_refused() {
        let (_dir, store) = temp_store().await;

        let refused = store.add_chat_dir(ChatId(99), Path::new("/a")).await;

        assert!(matches!(refused, Err(StoreError::Database(_))));
    }

    #[tokio::test]
    async fn add_dir_rows_follow_the_chat_when_it_is_deleted() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        store.add_chat_dir(chat, Path::new("/a")).await.unwrap();

        sqlx::query("DELETE FROM chats WHERE id = ?")
            .bind(to_sql_int(chat.0))
            .execute(&store.pool)
            .await
            .unwrap();

        assert!(store.chat_dirs(chat).await.unwrap().is_empty());
    }
}
