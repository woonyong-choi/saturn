//! 설정 스냅샷: 적용된 병합 결과만 설정 번호로 저장한다. 원본은 설정 파일이다.
//! 설계: docs/design/settings.md

use saturn_protocol::ids::SettingsRevision;

use std::time::SystemTime;

use super::records::{ensure_found, not_found};
use super::{Store, StoreError, from_sql_int, to_millis, to_sql_int};
use crate::settings::SettingsSnapshot;

const COMMON_REVISION_KEY: &str = "common_settings_revision";

impl Store {
    /// 조회와 삽입을 한 거래로 해 같은 내용에 번호가 둘 생기지 않게 한다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`, 직렬화 실패면 `Json`.
    pub(crate) async fn save_settings_snapshot(
        &self,
        snapshot: &SettingsSnapshot,
    ) -> Result<SettingsRevision, StoreError> {
        let digest = snapshot.digest();
        let body = serde_json::to_string(snapshot)?;
        let mut tx = self.pool.begin().await?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM settings_snapshots WHERE digest = ?")
                .bind(&digest)
                .fetch_optional(&mut *tx)
                .await?;
        let revision =
            match existing {
                Some(revision) => revision,
                None => sqlx::query_scalar(
                    "INSERT INTO settings_snapshots (digest, body, created_at) VALUES (?, ?, ?) \
                     RETURNING revision",
                )
                .bind(&digest)
                .bind(body)
                .bind(to_millis(SystemTime::now()))
                .fetch_one(&mut *tx)
                .await?,
            };
        tx.commit().await?;
        Ok(SettingsRevision(from_sql_int(revision)))
    }

    /// 입력은 접수 때 고정한 번호로 이것을 불러 끝까지 같은 값을 쓴다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`, 저장된 JSON이 깨졌으면 `Json`.
    pub(crate) async fn settings_snapshot(
        &self,
        revision: SettingsRevision,
    ) -> Result<SettingsSnapshot, StoreError> {
        let body: Option<String> =
            sqlx::query_scalar("SELECT body FROM settings_snapshots WHERE revision = ?")
                .bind(to_sql_int(revision.0))
                .fetch_optional(&self.pool)
                .await?;
        let body = body.ok_or_else(|| not_found(format!("settings revision {}", revision.0)))?;
        Ok(serde_json::from_str(&body)?)
    }

    /// 채팅과 접속 설정 없이 사용자 층과 시작 `-c`만 병합한 마지막 정상 번호. 한 번도 없으면 `None`.
    pub(crate) async fn common_settings_revision(
        &self,
    ) -> Result<Option<SettingsRevision>, StoreError> {
        let value: Option<i64> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
            .bind(COMMON_REVISION_KEY)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value.map(|value| SettingsRevision(from_sql_int(value))))
    }

    /// 채팅과 접속 설정이 없는 병합이 성공한 번호를 기록한다. 재사용한 번호도 다시 최근으로 올린다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`.
    pub(crate) async fn mark_common_settings_applied(
        &self,
        revision: SettingsRevision,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM settings_snapshots WHERE revision = ?")
                .bind(to_sql_int(revision.0))
                .fetch_optional(&mut *tx)
                .await?;
        ensure_found(u64::from(exists.is_some()), || {
            format!("settings revision {}", revision.0)
        })?;
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(COMMON_REVISION_KEY)
        .bind(to_sql_int(revision.0))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn unknown_revision_is_not_found() {
        let (_dir, store) = temp_store().await;

        assert_eq!(store.common_settings_revision().await.unwrap(), None);
        let read = store
            .settings_snapshot(SettingsRevision(1))
            .await
            .unwrap_err();
        let mark = store
            .mark_common_settings_applied(SettingsRevision(1))
            .await
            .unwrap_err();

        assert!(matches!(read, StoreError::NotFound { .. }));
        assert!(matches!(mark, StoreError::NotFound { .. }));
    }
}
