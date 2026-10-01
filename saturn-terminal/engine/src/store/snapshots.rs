//! 설정 스냅샷: 검사를 통과한 병합 결과와 층 목록을 설정 번호로 저장한다. 같은 내용이면 기존 번호를 다시 쓴다.
//!
//! 설계: docs/design/settings.md(병합과 설정 번호). 설정 원본은 파일이고 여기는 적용된 결과만 둔다.

use saturn_protocol::ids::SettingsRevision;

use std::time::SystemTime;

use super::records::{ensure_found, not_found};
use super::{Store, StoreError, from_sql_int, to_millis, to_sql_int};
use crate::settings::SettingsSnapshot;

/// `meta` 표에서 가장 최근 적용 번호를 담는 키.
const LATEST_REVISION_KEY: &str = "latest_settings_revision";

impl Store {
    /// 스냅샷을 저장하고 번호를 돌려준다. `snapshot.digest()`가 같은 행이 있으면 새로 쓰지 않고 그 번호를 돌려준다.
    /// 조회와 삽입을 한 거래로 해 같은 내용에 번호가 둘 생기지 않게 한다. 번호는 1부터 1씩 늘어난다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`, 직렬화 실패면 `Json`.
    pub async fn save_settings_snapshot(
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

    /// 번호로 스냅샷을 읽는다. 입력은 접수 때 고정한 번호로 이것을 불러 끝까지 같은 값을 쓴다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`, 저장된 JSON이 깨졌으면 `Json`.
    pub async fn settings_snapshot(
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

    /// 가장 최근에 적용한 설정 번호. 한 번도 적용하지 않았으면 `None`(시작 때 검사 실패와 겹치면 실행하지 않는다).
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn latest_settings_revision(&self) -> Result<Option<SettingsRevision>, StoreError> {
        let value: Option<i64> = sqlx::query_scalar("SELECT value FROM meta WHERE key = ?")
            .bind(LATEST_REVISION_KEY)
            .fetch_optional(&self.pool)
            .await?;
        Ok(value.map(|value| SettingsRevision(from_sql_int(value))))
    }

    /// 가장 최근 적용 번호를 기록한다. 재사용한 번호도 다시 최근으로 올린다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`.
    pub async fn mark_settings_applied(
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
        .bind(LATEST_REVISION_KEY)
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

        assert_eq!(store.latest_settings_revision().await.unwrap(), None);
        let read = store
            .settings_snapshot(SettingsRevision(1))
            .await
            .unwrap_err();
        let mark = store
            .mark_settings_applied(SettingsRevision(1))
            .await
            .unwrap_err();

        assert!(matches!(read, StoreError::NotFound { .. }));
        assert!(matches!(mark, StoreError::NotFound { .. }));
    }
}
