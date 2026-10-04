//! 설치한 확장: 이름, 원천, 설치 시각, 부분별 판정. 원본 파일은 확장 저장소 폴더에 있고 여기에 넣지 않는다.
//! 설계: docs/design/records.md#기록-저장소

use std::time::SystemTime;

use sqlx::Row;

use super::{Store, StoreError, to_millis};

/// `extensions` 표의 한 행. `parts`는 JSON 글이고 뜻은 `extensions` 모듈이 안다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtensionRow {
    pub(crate) name: String,
    pub(crate) source: String,
    /// unix 밀리초.
    pub(crate) installed_at: i64,
    pub(crate) parts: String,
}

impl Store {
    /// 설치한 순서대로.
    ///
    /// # Errors
    /// 읽기 실패면 `Database`.
    pub(crate) async fn extension_rows(&self) -> Result<Vec<ExtensionRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT name, source, installed_at, parts FROM extensions ORDER BY installed_at, rowid",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| ExtensionRow {
                name: row.get("name"),
                source: row.get("source"),
                installed_at: row.get("installed_at"),
                parts: row.get("parts"),
            })
            .collect())
    }

    /// 같은 이름이 이미 있으면 바꾸지 않고 `false`.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn insert_extension(
        &self,
        name: &str,
        source: &str,
        parts: &str,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "INSERT OR IGNORE INTO extensions (name, source, installed_at, parts) VALUES (?, ?, ?, ?)",
        )
        .bind(name)
        .bind(source)
        .bind(to_millis(SystemTime::now()))
        .bind(parts)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() == 1)
    }

    /// 부분별 판정만 바꾼다. 없는 이름이면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn update_extension_parts(
        &self,
        name: &str,
        parts: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE extensions SET parts = ? WHERE name = ?")
            .bind(parts)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 지웠으면 `true`.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub(crate) async fn delete_extension(&self, name: &str) -> Result<bool, StoreError> {
        let done = sqlx::query("DELETE FROM extensions WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn extension_rows_keep_install_order_and_refuse_a_second_row_with_the_same_name() {
        let (_dir, store) = temp_store().await;

        assert!(
            store
                .insert_extension("b-kit", "/src/b", "[]")
                .await
                .unwrap()
        );
        assert!(
            store
                .insert_extension("a-kit", "/src/a", "[1]")
                .await
                .unwrap()
        );
        assert!(
            !store
                .insert_extension("a-kit", "/other", "[2]")
                .await
                .unwrap()
        );

        let rows = store.extension_rows().await.unwrap();
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            ["b-kit", "a-kit"]
        );
        assert_eq!(
            (rows[1].source.as_str(), rows[1].parts.as_str()),
            ("/src/a", "[1]")
        );

        store.update_extension_parts("a-kit", "[3]").await.unwrap();
        assert!(store.delete_extension("b-kit").await.unwrap());
        assert!(!store.delete_extension("b-kit").await.unwrap());
        let rows = store.extension_rows().await.unwrap();
        assert_eq!((rows.len(), rows[0].parts.as_str()), (1, "[3]"));
    }
}
