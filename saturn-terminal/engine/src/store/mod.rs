//! 기록 저장소: SQLite 파일 하나(`~/.saturn/saturn.db`)에 입력, 실행, session, 이벤트, 사용량, 판단 기록, 설정 스냅샷을 쓴다.
//!
//! 설계: docs/design/records.md, docs/architecture.md(불변 조건). 쓰는 쪽은 engine 하나다(사용자당 잠금은 `rpc`가 먼저 잡는다).
//! 규칙:
//! - 한 번의 변경은 한 거래(transaction)로 처리한다. 여러 표를 고쳐도 중간 상태를 남기지 않는다.
//! - provider가 보고하지 않은 값은 0이 아니라 NULL로 둔다(`Option::None` ↔ NULL).
//! - judge 키는 어떤 표에도 쓰지 않는다. 판단 원문은 `secrets::Masked`로만 받는다.
//! - `~/.claude`, `~/.codex`의 provider 기록은 읽지도 지우지도 않는다.
//!
//! 시작 흐름: `rpc` 잠금 → `Store::open`(스키마 확인, 백업, 이관, 14일 지난 백업 삭제) → `settings` 병합과 스냅샷
//! → 자동 정리가 켜져 있으면 `Store::prune_on_start` 한 번 → 크래시 복구(`unfinished_runs`).
//! TODO(#29): 첫 스키마를 전체 정의로 쓸지, 옛 스키마 위 변경분으로 쓸지
//! TODO(#30): 옛 구현 v8 기록을 가져오지 않을지, 명령으로 가져올지
//! TODO(#31): 표 이름을 용어 표(채팅, 입력, 턴)에 맞출지, 초안 이름을 유지할지

mod judgments;
mod raw;
mod records;
mod retention;
mod schema;
mod snapshots;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

pub use judgments::{JudgmentOutcome, JudgmentPruneRequest, NewJudgment};
pub use raw::RawDigest;
pub use records::{NewInput, NewRun, RunEnd, RunRecord, UsageRow};
pub use retention::{
    PruneOutcome, PrunePlan, PruneRequest, PruneScope, RetentionPolicy, SkipReason, Tombstone,
};
pub use schema::{BACKUP_RETENTION, MigrationNotice, SCHEMA_VERSION};

/// 기록 저장소 파일 이름. `~/.saturn/` 아래에 둔다.
pub const DB_FILE: &str = "saturn.db";

/// 이관 직전 백업 폴더 이름. `~/.saturn/` 아래에 둔다.
pub const BACKUP_DIR: &str = "backup";

/// 다른 연결이 쓰는 중일 때 기다리는 시간. 쓰는 쪽은 engine 하나라 길게 기다릴 일이 없다. 값은 초안이다(설계에 없음).
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// 기록 저장소 파일 권한. 입력 원문과 판단 기록이 들어 있어 소유자만 읽고 쓴다. 값은 초안이다(설계에 없음).
const DB_FILE_MODE: u32 = 0o600;

/// 기록 저장소 오류. 호출자는 variant로 재시도 여부를 고르지 않는다(보내기 전 확정된 실패만 재전송하는 규칙은 호출자 몫).
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 파일을 열거나 만들지 못했다.
    #[error("failed to open record store: {path}")]
    Open {
        /// 기록 저장소 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// SQLite 질의나 거래 실패. 거래는 전부 되돌려졌다.
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    /// 이미 끝나 압축한 실행의 원시 기록에 이어 쓰려 했다. 끝난 실행은 원시 기록을 바꾸지 않는다.
    #[error("raw log of run {run} is already sealed")]
    RawSealed {
        /// 실행 id 숫자.
        run: u64,
    },
    /// 파일의 스키마 버전이 이 실행 파일이 아는 버전보다 높다. 옛 실행 파일로 새 기록을 열었으니 쓰지 않고 끝낸다.
    #[error("schema version {found} is newer than supported {supported}")]
    NewerSchema {
        /// 파일에 적힌 버전.
        found: u32,
        /// 이 실행 파일의 `SCHEMA_VERSION`.
        supported: u32,
    },
    /// 이관 실패. 거래를 되돌렸고 이관 직전 백업은 남아 있다.
    #[error("schema migration from {from} to {to} failed")]
    Migration {
        /// 이관 전 버전.
        from: u32,
        /// 목표 버전.
        to: u32,
        /// 원인.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// 이관 직전 백업을 만들거나 옛 백업을 지우지 못했다. 백업 없이는 이관하지 않는다.
    #[error("failed to manage backup: {path}")]
    Backup {
        /// 백업 파일 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// 원시 기록 gzip 압축이나 해제 실패.
    #[error("failed to compress or decompress raw log")]
    Compression(#[source] std::io::Error),
    /// 원시 기록을 풀었더니 압축 전에 기록한 해시나 크기와 다르다.
    #[error("raw log digest mismatch")]
    DigestMismatch,
    /// JSONL 내보내기 파일을 쓰지 못했다.
    #[error("failed to export judgments: {path}")]
    Export {
        /// 내보낼 파일 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// 저장한 JSON(설정 스냅샷, 이벤트)을 직렬화하거나 읽지 못했다.
    #[error("failed to encode or decode stored json")]
    Json(#[from] serde_json::Error),
    /// 없는 행. `what`은 `input 12`처럼 종류와 id다.
    #[error("record not found: {what}")]
    NotFound {
        /// 찾던 대상.
        what: String,
    },
}

/// 기록 저장소 연결. engine에 하나만 만들고 모든 쓰기를 이것으로만 한다.
/// 메서드는 파일 영역별로 `records`, `judgments`, `snapshots`, `retention`, `raw`, `schema` 하위 파일에 나뉘어 있다.
#[derive(Debug)]
pub struct Store {
    /// `~/.saturn`. 백업 폴더와 DB 경로의 기준.
    home: PathBuf,
    /// 연결 하나짜리 pool. WAL 저널, `foreign_keys=ON`, `secure_delete=ON`, `busy_timeout`.
    pool: SqlitePool,
}

impl Store {
    /// `home/saturn.db`를 열고(없으면 만든다) 스키마를 맞춘다. 호출 전에 `rpc`가 사용자당 engine 잠금을 잡았어야 한다.
    /// 순서: 스키마 버전 읽기 → 올라갔으면 `schema::backup_before_migration` → `schema::migrate` → `schema::sweep_backups`.
    /// 이관했으면 한 줄 안내를 돌려주고 호출자가 TUI와 stderr에 보인다.
    ///
    /// # Errors
    /// 파일을 못 열면 `Open`, 파일 버전이 더 높으면 `NewerSchema`, 백업 실패면 `Backup`, 이관 실패면 `Migration`.
    pub async fn open(home: &Path) -> Result<(Self, Option<MigrationNotice>), StoreError> {
        Self::open_with(home, schema::MIGRATIONS, SystemTime::now()).await
    }

    /// `open`의 본문. 테스트가 이관 단계와 시각을 바꿔 넣는다.
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

    /// DB 파일 경로 `home/saturn.db`.
    pub fn db_path(&self) -> PathBuf {
        self.home.join(DB_FILE)
    }

    /// 백업 폴더 경로 `home/backup`.
    pub fn backup_dir(&self) -> PathBuf {
        self.home.join(BACKUP_DIR)
    }

    /// `home`을 만들고 DB 파일을 연다. 스키마는 건드리지 않는다.
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

/// 이관 단계 목록이 만드는 스키마 버전. 단계 하나가 버전 하나다.
fn schema_target(migrations: &[&str]) -> u32 {
    u32::try_from(migrations.len()).expect("migration steps should fit in u32")
}

/// 시각을 unix 밀리초로. 기록 저장소의 모든 시각 칸은 이 값이다.
fn to_millis(at: SystemTime) -> i64 {
    let millis = at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// unix 밀리초를 시각으로.
fn from_millis(millis: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(u64::try_from(millis).unwrap_or_default())
}

/// id와 개수 같은 `u64`를 SQLite 정수로. 범위를 넘는 값은 만들지 않는다.
fn to_sql_int(value: u64) -> i64 {
    i64::try_from(value).expect("stored integers should fit in i64")
}

/// SQLite 정수를 `u64`로. 음수는 쓰지 않는다.
fn from_sql_int(value: i64) -> u64 {
    u64::try_from(value).expect("stored integers should be non-negative")
}

/// 값 없는 enum을 variant 이름 문자열로. SQL 조건에서 비교하려고 JSON 따옴표 없이 쓴다.
fn enum_text<T: Serialize>(value: &T) -> Result<String, StoreError> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(text) => Ok(text),
        other => Ok(other.to_string()),
    }
}

/// `enum_text`로 쓴 문자열을 다시 enum으로.
fn parse_enum<T: DeserializeOwned>(text: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(
        text.to_owned(),
    ))?)
}

/// 바이트의 SHA-256 hex.
fn sha256_hex(bytes: &[u8]) -> String {
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
mod tests {
    use super::*;

    /// 자동 삭제 폴더에 새 기록 저장소를 연다. 폴더가 살아 있는 동안만 쓴다.
    pub(crate) async fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let (store, notice) = Store::open(dir.path()).await.unwrap();
        assert!(notice.is_none());
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
        let steps = [schema::MIGRATIONS[0], "CREATE TABLE extra (id INTEGER)"];

        let (store, notice) = Store::open_with(dir.path(), &steps, SystemTime::now())
            .await
            .unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (1, 2));
        assert!(notice.backup.starts_with(store.backup_dir()));
        assert!(notice.backup.exists());
        assert!(notice.line().contains("1 → 2"));
        assert_eq!(store.schema_version().await.unwrap(), 2);
        let backups = std::fs::read_dir(store.backup_dir()).unwrap().count();
        assert_eq!(backups, 1);
    }
}
