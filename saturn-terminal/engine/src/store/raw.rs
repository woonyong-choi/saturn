//! 실행별 원시 기록: 실행 중 이어 쓰기, 끝나면 gzip 압축, 읽을 때 자동 해제.
//! 설계: docs/design/records.md

#[cfg(test)]
use std::io::Read;
use std::io::Write;

use flate2::Compression;
#[cfg(test)]
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::time::SystemTime;

use saturn_protocol::ids::{AgentId, ChatId, Provider, RunId};
use sqlx::Row;

#[cfg(test)]
use super::parse_enum;
use super::records::not_found;
use super::{Store, StoreError, enum_text, from_sql_int, sha256_hex, to_millis, to_sql_int};

/// 압축이 대조 값을 바꾸지 않게 항상 압축 전 바이트로 계산한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawDigest {
    /// SHA-256 hex. 초안 값.
    pub hash: String,
    pub size: u64,
}

/// 실행에 붙이지 못한 provider 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnattributedRaw {
    pub provider: Provider,
    /// 에이전트를 찾았지만 그 에이전트의 열린 실행이 없을 때만 있다.
    pub agent: Option<AgentId>,
    pub provider_session: Option<String>,
    pub is_json: bool,
    pub bytes: Vec<u8>,
}

impl Store {
    /// 받는 쪽(`RawTap`)이 router 키를 가린 줄을 받아 그대로 쓴다. 여기서는 마스킹하지 않는다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 이미 압축한(끝난) 실행이면 `Database`.
    pub(crate) async fn append_raw(&self, run: RunId, chunk: &[u8]) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await?;
        let sealed: Option<bool> =
            sqlx::query_scalar("SELECT raw_gzip IS NOT NULL FROM runs WHERE id = ?")
                .bind(to_sql_int(run.0))
                .fetch_optional(&mut *tx)
                .await?;
        match sealed {
            None => return Err(not_found(format!("run {}", run.0))),
            Some(true) => return Err(StoreError::RawSealed { run: run.0 }),
            Some(false) => {}
        }
        sqlx::query("INSERT INTO raw_chunks (run_id, bytes) VALUES (?, ?)")
            .bind(to_sql_int(run.0))
            .bind(chunk)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// 어느 실행의 것인지 정하지 못한 줄을 따로 쌓는다. 어떤 실행에도 붙이지 않는다.
    pub(crate) async fn append_unattributed_raw(
        &self,
        chat: ChatId,
        line: &UnattributedRaw,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO raw_unattributed \
             (chat_id, provider, agent_id, provider_session, is_json, bytes, received_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(to_sql_int(chat.0))
        .bind(enum_text(&line.provider)?)
        .bind(line.agent.map(|agent| to_sql_int(agent.0)))
        .bind(&line.provider_session)
        .bind(line.is_json)
        .bind(&line.bytes)
        .bind(to_millis(SystemTime::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 채팅의 미귀속 줄을 받은 순서대로 읽는다.
    #[cfg(test)]
    pub(crate) async fn unattributed_raw(
        &self,
        chat: ChatId,
    ) -> Result<Vec<UnattributedRaw>, StoreError> {
        let rows = sqlx::query(
            "SELECT provider, agent_id, provider_session, is_json, bytes FROM raw_unattributed \
             WHERE chat_id = ? ORDER BY id",
        )
        .bind(to_sql_int(chat.0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| {
                Ok(UnattributedRaw {
                    provider: parse_enum(&row.try_get::<String, _>("provider")?)?,
                    agent: row
                        .try_get::<Option<i64>, _>("agent_id")?
                        .map(|id| AgentId(from_sql_int(id))),
                    provider_session: row.try_get("provider_session")?,
                    is_json: row.try_get("is_json")?,
                    bytes: row.try_get("bytes")?,
                })
            })
            .collect()
    }

    /// 이미 압축됐으면 그대로 둔다.
    pub(crate) async fn compress_run(&self, run: RunId) -> Result<RawDigest, StoreError> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT raw_gzip IS NOT NULL AS sealed, raw_hash, raw_size FROM runs WHERE id = ?",
        )
        .bind(to_sql_int(run.0))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| not_found(format!("run {}", run.0)))?;
        if row.try_get::<bool, _>("sealed")? {
            return Ok(RawDigest {
                hash: row.try_get("raw_hash")?,
                size: from_sql_int(row.try_get("raw_size")?),
            });
        }
        let raw = chunks(&mut tx, run).await?;
        let digest = digest_of(&raw);
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&raw).map_err(StoreError::Compression)?;
        let packed = encoder.finish().map_err(StoreError::Compression)?;
        sqlx::query("UPDATE runs SET raw_gzip = ?, raw_hash = ?, raw_size = ? WHERE id = ?")
            .bind(packed)
            .bind(&digest.hash)
            .bind(to_sql_int(digest.size))
            .bind(to_sql_int(run.0))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM raw_chunks WHERE run_id = ?")
            .bind(to_sql_int(run.0))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(digest)
    }

    /// 푼 결과를 기록된 `RawDigest`와 대조한다.
    ///
    /// # Errors
    /// 해제 실패면 `Compression`, 대조가 다르면 `DigestMismatch`, 없는 실행이면 `NotFound`.
    #[cfg(test)]
    pub(crate) async fn read_raw(&self, run: RunId) -> Result<Vec<u8>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let row = sqlx::query("SELECT raw_gzip, raw_hash, raw_size FROM runs WHERE id = ?")
            .bind(to_sql_int(run.0))
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| not_found(format!("run {}", run.0)))?;
        let Some(packed) = row.try_get::<Option<Vec<u8>>, _>("raw_gzip")? else {
            return chunks(&mut conn, run).await;
        };
        let mut raw = Vec::new();
        GzDecoder::new(packed.as_slice())
            .read_to_end(&mut raw)
            .map_err(StoreError::Compression)?;
        let recorded = RawDigest {
            hash: row.try_get("raw_hash")?,
            size: from_sql_int(row.try_get("raw_size")?),
        };
        if digest_of(&raw) != recorded {
            return Err(StoreError::DigestMismatch);
        }
        Ok(raw)
    }

    /// 실행 중이면 지금까지 쓴 바이트로 계산한다.
    #[cfg(test)]
    pub(crate) async fn raw_digest(&self, run: RunId) -> Result<RawDigest, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let row = sqlx::query(
            "SELECT raw_gzip IS NOT NULL AS sealed, raw_hash, raw_size FROM runs WHERE id = ?",
        )
        .bind(to_sql_int(run.0))
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| not_found(format!("run {}", run.0)))?;
        if row.try_get::<bool, _>("sealed")? {
            return Ok(RawDigest {
                hash: row.try_get("raw_hash")?,
                size: from_sql_int(row.try_get("raw_size")?),
            });
        }
        Ok(digest_of(&chunks(&mut conn, run).await?))
    }
}

async fn chunks(conn: &mut sqlx::SqliteConnection, run: RunId) -> Result<Vec<u8>, StoreError> {
    let parts: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT bytes FROM raw_chunks WHERE run_id = ? ORDER BY id")
            .bind(to_sql_int(run.0))
            .fetch_all(conn)
            .await?;
    Ok(parts.concat())
}

fn digest_of(raw: &[u8]) -> RawDigest {
    RawDigest {
        hash: sha256_hex(raw),
        size: raw.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::RunEnd;
    use crate::store::records::tests::chat_with_run;
    use crate::store::tests::temp_store;

    #[tokio::test]
    async fn digest_is_taken_before_compression() {
        let (_dir, store) = temp_store().await;
        let (_, _, _, run) = chat_with_run(&store).await;
        let lines = b"{\"type\":\"a\"}\n{\"type\":\"b\"}\n";
        store.append_raw(run, &lines[..14]).await.unwrap();
        store.append_raw(run, &lines[14..]).await.unwrap();
        let running = store.raw_digest(run).await.unwrap();

        store.finish_run(run, RunEnd::Completed).await.unwrap();

        let expected = RawDigest {
            hash: sha256_hex(lines),
            size: lines.len() as u64,
        };
        assert_eq!(running, expected);
        assert_eq!(store.raw_digest(run).await.unwrap(), expected);
        assert_eq!(store.read_raw(run).await.unwrap(), lines.to_vec());
        let chunks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM raw_chunks")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(chunks, 0);
    }

    #[tokio::test]
    async fn sealed_raw_log_rejects_append() {
        let (_dir, store) = temp_store().await;
        let (_, _, _, run) = chat_with_run(&store).await;
        store.finish_run(run, RunEnd::Failed).await.unwrap();

        let error = store.append_raw(run, b"late").await.unwrap_err();

        assert!(matches!(error, StoreError::RawSealed { .. }));
        assert!(store.read_raw(run).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn tampered_raw_log_fails_digest_check() {
        let (_dir, store) = temp_store().await;
        let (_, _, _, run) = chat_with_run(&store).await;
        store.append_raw(run, b"original").await.unwrap();
        store.finish_run(run, RunEnd::Completed).await.unwrap();
        sqlx::query("UPDATE runs SET raw_size = 1")
            .execute(&store.pool)
            .await
            .unwrap();

        let error = store.read_raw(run).await.unwrap_err();

        assert!(matches!(error, StoreError::DigestMismatch));
    }

    #[tokio::test]
    async fn missing_run_is_not_found() {
        let (_dir, store) = temp_store().await;

        let error = store.append_raw(RunId(5), b"x").await.unwrap_err();

        assert!(matches!(error, StoreError::NotFound { .. }));
    }
}
