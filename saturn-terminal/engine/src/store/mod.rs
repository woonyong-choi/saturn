//! 기록 저장소: SQLite 파일 하나에 입력, 실행, 판단 기록, 설정 스냅샷을 쓴다.
//! 설계: docs/design/records.md

mod chat_dirs;
mod chat_labels;
mod history;
mod judgments;
mod ledger;
mod outcomes;
mod permissions;
mod raw;
mod records;
mod recovery;
mod retention;
mod schema;
mod sessions;
mod snapshots;
mod usage;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub(crate) use history::HistoryEntry;
pub(crate) use judgments::{JudgmentOutcome, NewJudgment};
pub(crate) use ledger::LedgerRow;
pub(crate) use records::{NewInput, NewRun, RunEnd, RunRecord, UsageRow};
pub(crate) use recovery::StoredHold;
pub(crate) use retention::RetentionPolicy;
pub(crate) use schema::MigrationNotice;
pub(crate) use sessions::IdKind;

#[cfg(test)]
pub(crate) use judgments::tests::judgment as test_judgment;

pub(crate) const DB_FILE: &str = "saturn.db";

pub(crate) const BACKUP_DIR: &str = "backup";

/// 쓰는 쪽이 engine 하나라 길게 기다릴 일이 없다. 초안 값.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// 입력 원문과 판단 기록이 들어 있어 소유자만 읽고 쓴다.
const DB_FILE_MODE: u32 = 0o600;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("failed to open record store: {path}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 거래는 전부 되돌려진 상태다.
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("raw log of run {run} is already sealed")]
    RawSealed {
        /// `RunId`의 숫자.
        run: u64,
    },
    /// 옛 실행 파일로 새 기록을 연 경우라 쓰지 않고 끝낸다.
    #[error("schema version {found} is newer than supported {supported}")]
    NewerSchema {
        found: u32,
        /// 이 실행 파일의 `SCHEMA_VERSION`.
        supported: u32,
    },
    /// 거래를 되돌렸고 이관 직전 백업은 남아 있다.
    #[error("schema migration from {from} to {to} failed")]
    Migration {
        from: u32,
        to: u32,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// 백업 없이는 이관하지 않는다.
    #[error("failed to manage backup: {path}")]
    Backup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to compress or decompress raw log")]
    Compression(#[source] std::io::Error),
    /// 풀어 본 원시 기록의 해시나 크기가 압축 전 기록과 다르다.
    #[error("raw log digest mismatch")]
    DigestMismatch,
    #[error("failed to export judgments: {path}")]
    Export {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to encode or decode stored json")]
    Json(#[from] serde_json::Error),
    #[error("record not found: {what}")]
    NotFound {
        /// 종류와 id. 예: `input 12`.
        what: String,
    },
}

/// engine에 하나만 만들고 모든 쓰기를 이것으로만 한다.
#[derive(Debug)]
pub(crate) struct Store {
    home: PathBuf,
    pool: SqlitePool,
}

impl Store {
    /// 호출 전에 `rpc`가 사용자당 engine 잠금을 잡았어야 한다.
    ///
    /// # Errors
    /// 파일을 못 열면 `Open`, 파일 버전이 더 높으면 `NewerSchema`, 백업 실패면 `Backup`, 이관 실패면 `Migration`.
    pub(crate) async fn open(home: &Path) -> Result<(Self, Option<MigrationNotice>), StoreError> {
        Self::open_with(home, schema::MIGRATIONS, SystemTime::now()).await
    }

    /// 테스트가 이관 단계와 시각을 바꿔 넣는다.
    pub(crate) async fn open_with(
        home: &Path,
        migrations: &[&str],
        now: SystemTime,
    ) -> Result<(Self, Option<MigrationNotice>), StoreError> {
        let store = Self::connect(home).await?;
        let target = schema_target(migrations);
        let found = store.schema_version().await?;
        if found > target {
            return Err(StoreError::NewerSchema {
                found,
                supported: target,
            });
        }
        let mut notice = None;
        if found < target {
            if found > 0 {
                let backup = store.backup_before_migration(found).await?;
                notice = Some(MigrationNotice {
                    from: found,
                    to: target,
                    backup,
                });
            }
            store.migrate_with(found, migrations).await?;
        }
        store.sweep_backups(now).await?;
        Ok((store, notice))
    }

    pub(crate) fn db_path(&self) -> PathBuf {
        self.home.join(DB_FILE)
    }

    pub(crate) fn backup_dir(&self) -> PathBuf {
        self.home.join(BACKUP_DIR)
    }

    /// 이 뒤의 쓰기가 모두 실패한다. 접수 기록 실패 시험이 쓴다.
    #[cfg(test)]
    pub(crate) async fn deny_writes(&self) {
        sqlx::query("PRAGMA query_only = ON")
            .execute(&self.pool)
            .await
            .expect("pragma should apply");
    }

    /// `deny_writes`로 막은 쓰기를 다시 연다.
    #[cfg(test)]
    pub(crate) async fn allow_writes(&self) {
        sqlx::query("PRAGMA query_only = OFF")
            .execute(&self.pool)
            .await
            .expect("pragma should apply");
    }

    /// 스키마는 건드리지 않는다.
    async fn connect(home: &Path) -> Result<Self, StoreError> {
        let path = home.join(DB_FILE);
        let open_error = |source| StoreError::Open {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(home).map_err(open_error)?;
        let options = SqliteConnectOptions::from_str("sqlite:")?
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT)
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(DB_FILE_MODE))
            .map_err(open_error)?;
        Ok(Self {
            home: home.to_path_buf(),
            pool,
        })
    }
}

/// 단계 하나가 버전 하나다.
fn schema_target(migrations: &[&str]) -> u32 {
    u32::try_from(migrations.len()).expect("migration steps should fit in u32")
}

/// 기록 저장소의 모든 시각 칸은 unix 밀리초다.
fn to_millis(at: SystemTime) -> i64 {
    let millis = at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn from_millis(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(millis).unwrap_or_default())
}

/// 범위를 넘는 값은 만들지 않는다.
fn to_sql_int(value: u64) -> i64 {
    i64::try_from(value).expect("stored integers should fit in i64")
}

/// 음수는 쓰지 않는다.
fn from_sql_int(value: i64) -> u64 {
    u64::try_from(value).expect("stored integers should be non-negative")
}

/// SQL 조건에서 비교하려고 JSON 따옴표 없이 쓴다.
fn enum_text<T: Serialize>(value: &T) -> Result<String, StoreError> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}

fn parse_enum<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_owned(),
    ))?)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;

    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}"); // String에 쓰기는 실패하지 않는다
    }
    hex
}

#[cfg(test)]
pub(crate) mod tests {
    use super::schema::SCHEMA_VERSION;
    use super::*;

    /// 폴더가 살아 있는 동안만 쓴다.
    pub(crate) async fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let (store, notice) = Store::open(dir.path()).await.unwrap();
        assert!(notice.is_none());
        (dir, store)
    }

    /// 스키마 `version`까지만 만든 옛 파일. 이관 시험의 출발점이다.
    pub(crate) async fn temp_store_at(version: usize) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let (store, notice) = Store::open_with(
            dir.path(),
            &schema::MIGRATIONS[..version],
            SystemTime::now(),
        )
        .await
        .unwrap();
        assert!(notice.is_none());
        assert_eq!(store.schema_version().await.unwrap() as usize, version);
        (dir, store)
    }

    #[tokio::test]
    async fn open_creates_schema_without_backup_or_notice() {
        let (dir, store) = temp_store().await;

        assert_eq!(store.schema_version().await.unwrap(), SCHEMA_VERSION);
        assert_eq!(store.db_path(), dir.path().join(DB_FILE));
        assert!(!store.backup_dir().exists());
        let mode = std::fs::metadata(store.db_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, DB_FILE_MODE);
    }

    #[tokio::test]
    async fn reopen_same_version_does_not_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(dir.path()).await.unwrap();
        drop(store);

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        assert!(notice.is_none());
        assert_eq!(store.schema_version().await.unwrap(), SCHEMA_VERSION);
        assert!(!store.backup_dir().exists());
    }

    #[tokio::test]
    async fn open_newer_schema_is_rejected() {
        let (dir, store) = temp_store().await;
        sqlx::raw_sql("PRAGMA user_version = 99")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let error = Store::open(dir.path()).await.unwrap_err();

        assert!(matches!(
            error,
            StoreError::NewerSchema {
                found: 99,
                supported: SCHEMA_VERSION
            }
        ));
    }

    #[tokio::test]
    async fn open_older_schema_backs_up_then_migrates() {
        let (dir, store) = temp_store().await;
        store.pool.close().await;
        let mut steps = schema::MIGRATIONS.to_vec();
        steps.push("CREATE TABLE extra (id INTEGER)");

        let (store, notice) = Store::open_with(dir.path(), &steps, SystemTime::now())
            .await
            .unwrap();

        let notice = notice.unwrap();
        assert_eq!(
            (notice.from, notice.to),
            (SCHEMA_VERSION, SCHEMA_VERSION + 1)
        );
        assert!(notice.backup.starts_with(store.backup_dir()));
        assert!(notice.backup.exists());
        assert!(
            notice
                .line()
                .contains(&format!("{SCHEMA_VERSION} → {}", SCHEMA_VERSION + 1))
        );
        assert_eq!(store.schema_version().await.unwrap(), SCHEMA_VERSION + 1);
        let backups = std::fs::read_dir(store.backup_dir()).unwrap().count();
        assert_eq!(backups, 1);
    }
}
