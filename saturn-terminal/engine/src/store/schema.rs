//! 스키마 버전 확인, 이관 직전 백업, 자동 이관, 오래된 백업 삭제.
//! 설계: docs/design/records.md

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{DB_FILE_MODE, Store, StoreError, schema_target, to_millis};

/// 스키마를 바꾸면 1 올리고 이관 단계를 더한다.
pub(crate) const SCHEMA_VERSION: u32 = 7;

pub(crate) const BACKUP_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// 이 형식의 파일만 백업으로 보고 지운다. 초안 값.
const BACKUP_PREFIX: &str = "saturn-v";
const BACKUP_SUFFIX: &str = ".db";

/// `MIGRATIONS[i]`는 버전 `i`를 `i + 1`로 올리고, 길이가 `SCHEMA_VERSION`과 같아야 한다.
pub(crate) const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7];

const _: () = assert!(MIGRATIONS.len() == SCHEMA_VERSION as usize);

/// 첫 스키마 전체 정의.
const V1: &str = r#"
CREATE TABLE chats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workdir TEXT NOT NULL,
    recording INTEGER NOT NULL DEFAULT 1,
    chat_layer TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE inputs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    settings_revision INTEGER NOT NULL,
    permission TEXT NOT NULL,
    workdir TEXT NOT NULL,
    pinned_model TEXT,
    skip_relation INTEGER NOT NULL,
    state TEXT NOT NULL,
    reason TEXT,
    accepted_at INTEGER NOT NULL
);
CREATE INDEX inputs_chat ON inputs(chat_id);
CREATE TABLE stops (
    chat_id INTEGER PRIMARY KEY REFERENCES chats(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL
);
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL,
    role TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_session TEXT,
    state TEXT NOT NULL,
    delivered INTEGER NOT NULL
);
CREATE INDEX sessions_chat ON sessions(chat_id);
CREATE TABLE runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    input_id INTEGER REFERENCES inputs(id) ON DELETE CASCADE,
    task_id INTEGER NOT NULL,
    agent_id INTEGER NOT NULL,
    session_id INTEGER NOT NULL,
    provider TEXT NOT NULL,
    effect_scope TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER,
    end_kind TEXT,
    raw_gzip BLOB,
    raw_hash TEXT,
    raw_size INTEGER
);
CREATE INDEX runs_chat ON runs(chat_id);
CREATE INDEX runs_session ON runs(session_id);
CREATE TABLE raw_chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    bytes BLOB NOT NULL
);
CREATE INDEX raw_chunks_run ON raw_chunks(run_id);
CREATE TABLE events (
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (chat_id, seq)
);
CREATE TABLE usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    session_id INTEGER NOT NULL,
    body TEXT NOT NULL,
    scope TEXT NOT NULL,
    agent_id INTEGER NOT NULL,
    subagent TEXT,
    model TEXT,
    input_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_write_tokens INTEGER,
    output_tokens INTEGER,
    reasoning_tokens INTEGER,
    spans_turns INTEGER NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX usage_chat ON usage(chat_id);
CREATE INDEX usage_session ON usage(session_id);
CREATE TABLE judgments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL,
    input_id INTEGER,
    method TEXT NOT NULL,
    router TEXT NOT NULL,
    model TEXT NOT NULL,
    reported_model TEXT,
    question_sets TEXT NOT NULL,
    settings_revision INTEGER NOT NULL,
    sent TEXT NOT NULL,
    received TEXT,
    answers TEXT NOT NULL,
    fallbacks TEXT NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    started_at INTEGER NOT NULL,
    elapsed_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    router_version TEXT NOT NULL,
    thresholds TEXT NOT NULL,
    asked_with REAL
);
CREATE INDEX judgments_chat ON judgments(chat_id);
CREATE TABLE settings_snapshots (
    revision INTEGER PRIMARY KEY AUTOINCREMENT,
    digest TEXT NOT NULL UNIQUE,
    body TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value INTEGER NOT NULL
);
CREATE TABLE tombstones (
    chat_id INTEGER PRIMARY KEY,
    hash TEXT NOT NULL,
    deleted_at INTEGER NOT NULL
);
"#;

/// 보관 session의 마지막 턴 값. 값이 없는 옛 행은 NULL로 두고 재개로 판정한다.
const V2: &str = r#"
ALTER TABLE sessions ADD COLUMN last_active INTEGER;
ALTER TABLE sessions ADD COLUMN last_turn_ended_at INTEGER;
"#;

/// 판단 기록의 결과 신호와 물은 답. NULL은 관찰 중이거나 묻지 않았다는 뜻이다. 물은 확률 q는 V1의 `asked_with`다.
const V3: &str = r#"
ALTER TABLE judgments ADD COLUMN signal TEXT;
ALTER TABLE judgments ADD COLUMN asked_answer TEXT;
"#;

/// 항상 허용. 작업 폴더, 도구, 패턴이 같은 행은 하나다.
const V4: &str = r#"
CREATE TABLE permission_allows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workdir TEXT NOT NULL,
    tool TEXT NOT NULL,
    pattern TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (workdir, tool, pattern)
);
"#;

/// 채팅에 더한 폴더. 채팅마다 같은 경로는 한 행이고, id 순서가 더한 순서다.
const V5: &str = r#"
CREATE TABLE chat_dirs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    UNIQUE (chat_id, path)
);
"#;

/// `sessions.model`은 session을 열 때 고른 모델이다. 없으면 provider 기본값이고 이관 전 session도 비어 있다.
/// `chats.pinned_model`은 채팅이 다시 바꿀 때까지 쓰는 고정 모델이다.
const V6: &str = r#"
ALTER TABLE sessions ADD COLUMN model TEXT;
ALTER TABLE chats ADD COLUMN pinned_model TEXT;
"#;

/// `held_tasks`는 멈출 때 실행 중이던 보류 작업이다. 작업 번호는 첫 입력 번호라 engine을 다시 켜도 같다.
/// `interrupted_subagents`는 크래시로 끊긴 하위 에이전트이고, `cleaned`는 provider에 정리를 넘겼다는 뜻이다.
const V7: &str = r#"
CREATE TABLE held_tasks (
    task_id INTEGER PRIMARY KEY,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL,
    input_id INTEGER NOT NULL REFERENCES inputs(id) ON DELETE CASCADE
);
CREATE INDEX held_tasks_chat ON held_tasks(chat_id);
CREATE TABLE interrupted_subagents (
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL,
    subagent TEXT NOT NULL,
    cleaned INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, subagent)
);
CREATE INDEX interrupted_subagents_chat ON interrupted_subagents(chat_id);
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationNotice {
    pub from: u32,
    pub to: u32,
    /// 파일 이름 `saturn-v<from>-<unix밀리초>.db`는 초안이다.
    pub backup: PathBuf,
}

impl MigrationNotice {
    /// 예: `기록 저장소 스키마 1 → 2 이관 · 백업 ~/.saturn/backup/... (14일 보관)`.
    pub(crate) fn line(&self) -> String {
        format!(
            "기록 저장소 스키마 {} → {} 이관 · 백업 {} (14일 보관)",
            self.from,
            self.to,
            self.backup.display()
        )
    }
}

impl Store {
    /// 새 파일은 0.
    pub(crate) async fn schema_version(&self) -> Result<u32, StoreError> {
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&self.pool)
            .await?;
        Ok(u32::try_from(version).unwrap_or_default())
    }

    /// 백업은 항상 가장 최근 1개만 남고, 새 파일(버전 0)은 백업하지 않는다.
    ///
    /// # Errors
    /// 파일 생성이나 옛 백업 삭제 실패면 `Backup`이고 이관하지 않는다.
    pub(crate) async fn backup_before_migration(&self, from: u32) -> Result<PathBuf, StoreError> {
        let dir = self.backup_dir();
        let path = dir.join(format!(
            "{BACKUP_PREFIX}{from}-{}{BACKUP_SUFFIX}",
            to_millis(SystemTime::now())
        ));
        let backup_error = |path: &Path| {
            let path = path.to_path_buf();
            move |source| StoreError::Backup { path, source }
        };
        std::fs::create_dir_all(&dir).map_err(backup_error(&dir))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(backup_error(&dir))?;
        sqlx::query("VACUUM INTO ?")
            .bind(path.to_string_lossy().into_owned())
            .execute(&self.pool)
            .await?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(DB_FILE_MODE))
            .map_err(backup_error(&path))?;
        for old in backup_files(&dir).map_err(backup_error(&dir))? {
            if old != path {
                std::fs::remove_file(&old).map_err(backup_error(&old))?;
            }
        }
        Ok(path)
    }

    /// 호출 전에 `from > SCHEMA_VERSION`은 `NewerSchema`로 거른다. 테스트가 이관 단계를 바꿔 넣을 수 있다.
    ///
    /// # Errors
    /// 어느 단계든 실패하면 거래를 되돌리고 `Migration`.
    pub(crate) async fn migrate_with(&self, from: u32, steps: &[&str]) -> Result<(), StoreError> {
        let to = schema_target(steps);
        if from >= to {
            return Ok(());
        }
        let failed = |source: sqlx::Error| StoreError::Migration {
            from,
            to,
            source: Box::new(source),
        };
        let mut tx = self.pool.begin().await.map_err(failed)?;
        for step in &steps[from as usize..] {
            sqlx::raw_sql(step)
                .execute(&mut *tx)
                .await
                .map_err(failed)?;
        }
        // PRAGMA는 값을 바인딩할 수 없다. `to`는 정수라 그대로 넣어도 안전하다
        sqlx::raw_sql(&format!("PRAGMA user_version = {to}"))
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        Ok(())
    }

    /// `backup/` 밖 파일은 건드리지 않는다.
    pub(crate) async fn sweep_backups(&self, now: SystemTime) -> Result<usize, StoreError> {
        let dir = self.backup_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let backup_error = |path: &Path| {
            let path = path.to_path_buf();
            move |source| StoreError::Backup { path, source }
        };
        let cutoff = now
            .checked_sub(BACKUP_RETENTION)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let mut removed = 0;
        for path in backup_files(&dir).map_err(backup_error(&dir))? {
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .map_err(backup_error(&path))?;
            if modified < cutoff {
                std::fs::remove_file(&path).map_err(backup_error(&path))?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

/// 다른 이름의 파일과 폴더는 빼서 건드리지 않는다.
fn backup_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_backup = name.starts_with(BACKUP_PREFIX) && name.ends_with(BACKUP_SUFFIX);
        if is_backup && entry.file_type()?.is_file() {
            files.push(entry.path());
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    use crate::store::tests::{temp_store, temp_store_at};

    #[tokio::test]
    async fn backup_keeps_only_latest_file() {
        let (_dir, store) = temp_store().await;

        let first = store.backup_before_migration(1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let second = store.backup_before_migration(1).await.unwrap();

        assert!(!first.exists());
        assert!(second.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        assert_eq!(
            std::fs::metadata(&second).unwrap().permissions().mode() & 0o777,
            DB_FILE_MODE
        );
        assert_eq!(
            std::fs::metadata(store.backup_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[tokio::test]
    async fn sweep_removes_backups_older_than_fourteen_days() {
        let (_dir, store) = temp_store().await;
        let backup = store.backup_before_migration(1).await.unwrap();
        let other = store.backup_dir().join("notes.txt");
        std::fs::write(&other, "not a backup").unwrap();
        let now = SystemTime::now();

        assert_eq!(store.sweep_backups(now).await.unwrap(), 0);
        let old = now - BACKUP_RETENTION - Duration::from_secs(60);
        for path in [&backup, &other] {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }

        assert_eq!(store.sweep_backups(now).await.unwrap(), 1);
        assert!(!backup.exists());
        assert!(other.exists());
    }

    #[tokio::test]
    async fn sweep_without_backup_dir_is_noop() {
        let (_dir, store) = temp_store().await;

        assert_eq!(store.sweep_backups(SystemTime::now()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn failed_migration_rolls_back_whole_transaction() {
        let (_dir, store) = temp_store_at(1).await;
        let steps = [
            MIGRATIONS[0],
            "CREATE TABLE half_done (id INTEGER); INSERT INTO missing_table VALUES (1);",
        ];

        let error = store.migrate_with(1, &steps).await.unwrap_err();

        assert!(matches!(
            error,
            StoreError::Migration { from: 1, to: 2, .. }
        ));
        assert_eq!(store.schema_version().await.unwrap(), 1);
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'half_done'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(tables, 0);
    }

    #[tokio::test]
    async fn v1_file_migrates_to_last_turn_columns_keeping_sessions() {
        let (dir, store) = temp_store_at(1).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0); \
             INSERT INTO sessions (id, chat_id, agent_id, role, provider, provider_session, state, delivered) \
             VALUES (7, 1, 1, 'Main', 'Codex', 'thread-7', 'ClosedResumable', 3)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (1, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        let live = store.live_mains().await.unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0.delivered.0, 3);
        assert_eq!(live[0].1, None);
    }

    #[tokio::test]
    async fn migration_keeps_only_latest_backup_and_removes_old_ones() {
        let (dir, store) = temp_store_at(1).await;
        let stale = store.backup_before_migration(1).await.unwrap();
        let older = stale.with_file_name("saturn-v0-1.db");
        std::fs::rename(&stale, &older).unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let backup = notice.unwrap().backup;
        assert!(backup.exists());
        assert!(!older.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn v2_file_migrates_to_outcome_columns_keeping_judgments() {
        let (dir, store) = temp_store_at(2).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0); \
             INSERT INTO judgments (id, chat_id, method, router, model, question_sets, settings_revision, \
             sent, answers, fallbacks, started_at, elapsed_ms, outcome, router_version, thresholds, asked_with) \
             VALUES (5, 1, 'jev', 'jev', 'm', '[]', 1, '{}', '[]', '[]', 0, 1, 'Ok', 'v1', '[]', 0.2)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (2, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        let (asked_with, signal, answer): (f64, Option<String>, Option<String>) =
            sqlx::query_as("SELECT asked_with, signal, asked_answer FROM judgments WHERE id = 5")
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!((asked_with, signal, answer), (0.2, None, None));
    }

    #[tokio::test]
    async fn v4_file_migrates_to_chat_dirs_keeping_chats() {
        let (dir, store) = temp_store_at(4).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (4, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.chat_dirs(chat).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v3_file_migrates_to_permission_allows_keeping_chats() {
        let (dir, store) = temp_store_at(3).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (3, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        assert!(
            store
                .permission_allows(Path::new("/work"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn v6_file_migrates_to_recovery_tables_keeping_chats_and_backing_up() {
        let (dir, store) = temp_store_at(6).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (6, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.held_tasks().await.unwrap().is_empty());
        assert!(store.interrupted_subagents().await.unwrap().is_empty());
    }
}
