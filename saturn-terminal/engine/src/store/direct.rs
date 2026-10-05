//! provider에 직접 설치된 항목 중 옮길지 물은 것. 한 번 물은 항목은 다시 묻지 않는다.
//! 설계: docs/design/records.md#기록-저장소

use std::time::SystemTime;

use sqlx::Row;

use super::{Store, StoreError, to_millis};

/// `direct_installs` 표의 한 행. `kind`와 `state`의 뜻은 `extensions` 모듈이 안다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectRow {
    pub(crate) provider: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) state: String,
}

impl Store {
    /// 물은 항목 전부.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn direct_rows(&self) -> Result<Vec<DirectRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT provider, kind, name, state FROM direct_installs ORDER BY seen_at, rowid",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| DirectRow {
                provider: row.get("provider"),
                kind: row.get("kind"),
                name: row.get("name"),
                state: row.get("state"),
            })
            .collect())
    }

    /// 이미 있는 행은 바꾸지 않고 `false`. 물었다는 기록은 한 번만 남는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_direct_asked(
        &self,
        provider: &str,
        kind: &str,
        name: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO direct_installs (provider, kind, name, state, seen_at) VALUES (?, ?, ?, 'asked', ?)",
        )
        .bind(provider)
        .bind(kind)
        .bind(name)
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// 옮겼다고 기록한다. 물은 적 없는 항목이어도 행을 만든다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn record_direct_moved(
        &self,
        provider: &str,
        kind: &str,
        name: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO direct_installs (provider, kind, name, state, seen_at) VALUES (?, ?, ?, 'moved', ?) \
             ON CONFLICT(provider, kind, name) DO UPDATE SET state = 'moved'",
        )
        .bind(provider)
        .bind(kind)
        .bind(name)
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn an_item_is_asked_once_and_moving_it_keeps_the_row() {
        let (_dir, store) = temp_store().await;

        assert!(
            store
                .record_direct_asked("claude", "Skill", "a")
                .await
                .unwrap()
        );
        assert!(
            !store
                .record_direct_asked("claude", "Skill", "a")
                .await
                .unwrap()
        );
        store
            .record_direct_moved("claude", "Skill", "a")
            .await
            .unwrap();
        assert!(
            !store
                .record_direct_asked("claude", "Skill", "a")
                .await
                .unwrap()
        );

        let rows = store.direct_rows().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, "moved");
    }
}
