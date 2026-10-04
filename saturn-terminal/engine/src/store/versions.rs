//! provider CLI 버전: 마지막으로 확인한 버전을 기록해 다음 시작 때 바뀐 것을 알아챈다.

use std::time::SystemTime;

use saturn_protocol::ids::Provider;

use super::{Store, StoreError, to_millis};

impl Store {
    /// 확인한 적이 없으면 `None`.
    ///
    /// # Errors
    /// 읽기 실패면 `Sqlx`.
    pub(crate) async fn provider_cli_version(
        &self,
        provider: Provider,
    ) -> Result<Option<String>, StoreError> {
        let version =
            sqlx::query_scalar("SELECT version FROM provider_versions WHERE provider = ?")
                .bind(provider.as_str())
                .fetch_optional(&self.pool)
                .await?;
        Ok(version)
    }

    /// 같은 provider의 이전 값은 덮어쓴다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Sqlx`.
    pub(crate) async fn record_provider_cli_version(
        &self,
        provider: Provider,
        version: &str,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO provider_versions (provider, version, checked_at) VALUES (?, ?, ?) \
             ON CONFLICT(provider) DO UPDATE SET version = excluded.version, checked_at = excluded.checked_at",
        )
        .bind(provider.as_str())
        .bind(version)
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::providers::test_support::{CLAUDE, CODEX};
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn recorded_version_is_read_back_and_overwritten_per_provider() {
        let (_dir, store) = temp_store().await;

        assert_eq!(store.provider_cli_version(CODEX).await.unwrap(), None);
        store
            .record_provider_cli_version(CODEX, "0.158.0")
            .await
            .unwrap();
        store
            .record_provider_cli_version(CLAUDE, "2.1.285")
            .await
            .unwrap();
        store
            .record_provider_cli_version(CODEX, "0.159.0")
            .await
            .unwrap();

        assert_eq!(
            store.provider_cli_version(CODEX).await.unwrap().as_deref(),
            Some("0.159.0")
        );
        assert_eq!(
            store.provider_cli_version(CLAUDE).await.unwrap().as_deref(),
            Some("2.1.285")
        );
    }
}
