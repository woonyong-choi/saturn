//! 스키마 버전 확인, 이관 직전 백업, 자동 이관, 오래된 백업 삭제.
//! 설계: docs/design/records.md

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{DB_FILE_MODE, Store, StoreError, schema_target, to_millis};

/// 스키마를 바꾸면 1 올리고 이관 단계를 더한다.
pub(crate) const SCHEMA_VERSION: u32 = 18;

pub(crate) const BACKUP_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// 이 형식의 파일만 백업으로 보고 지운다. 초안 값.
const BACKUP_PREFIX: &str = "saturn-v";
const BACKUP_SUFFIX: &str = ".db";

/// `MIGRATIONS[i]`는 버전 `i`를 `i + 1`로 올리고, 길이가 `SCHEMA_VERSION`과 같아야 한다.
pub(crate) const MIGRATIONS: &[&str] = &[
    V1, V2, V3, V4, V5, V6, V7, V8, V9, V10, V11, V12, V13, V14, V15, V16, V17, V18,
];

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

/// `chats.name`은 사용자가 붙인 채팅 이름, `chats.group_name`은 작업 목록의 묶음 이름이다. 비어 있으면 NULL이고 이관 전 채팅도 NULL이다.
const V8: &str = r#"
ALTER TABLE chats ADD COLUMN name TEXT;
ALTER TABLE chats ADD COLUMN group_name TEXT;
"#;

/// `provider_versions`는 provider마다 마지막으로 확인한 CLI 버전이다. 이관은 표만 비어 있게 더한다.
const V9: &str = r#"
CREATE TABLE provider_versions (
    provider TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    checked_at INTEGER NOT NULL
);
"#;

/// 제약 표 다섯 개. 이관은 표만 비어 있게 더하고 이관 전 채팅의 입력을 소급해 판단하지 않는다.
/// 행은 지우지 않고 채팅을 지울 때만 함께 지운다. `constraints.scope`는 줄바꿈으로 이은 경로이고 NULL이면 전체다.
const V10: &str = r#"
CREATE TABLE constraints (
    constraint_id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    input_id INTEGER NOT NULL,
    line INTEGER NOT NULL,
    rule TEXT NOT NULL,
    scope TEXT,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX constraints_chat ON constraints(chat_id);
CREATE INDEX constraints_input ON constraints(input_id);
CREATE TABLE constraint_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    constraint_id INTEGER NOT NULL REFERENCES constraints(constraint_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT,
    input_id INTEGER,
    judgment_id INTEGER,
    undoes INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX constraint_events_chat ON constraint_events(chat_id);
CREATE INDEX constraint_events_constraint ON constraint_events(constraint_id);
CREATE TABLE constraint_asks (
    ask_id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    constraint_id INTEGER NOT NULL REFERENCES constraints(constraint_id) ON DELETE CASCADE,
    judgment_id INTEGER,
    asked_at INTEGER NOT NULL,
    answer TEXT,
    answered_at INTEGER
);
CREATE INDEX constraint_asks_chat ON constraint_asks(chat_id);
CREATE TABLE constraint_exceptions (
    exception_id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    constraint_id INTEGER NOT NULL REFERENCES constraints(constraint_id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    event_id INTEGER NOT NULL,
    task_id INTEGER,
    condition TEXT,
    ended_event_id INTEGER
);
CREATE INDEX constraint_exceptions_constraint ON constraint_exceptions(constraint_id);
CREATE TABLE packet_constraints (
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    constraint_id INTEGER NOT NULL REFERENCES constraints(constraint_id) ON DELETE CASCADE,
    tier TEXT NOT NULL,
    PRIMARY KEY (session_id, constraint_id)
);
"#;

/// 실행별 수정 파일 목록. 이관 전 실행은 측정하지 않았으므로 `changes_state`가 비어 있다.
const V11: &str = r#"
ALTER TABLE runs ADD COLUMN changes_state TEXT;
CREATE TABLE run_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    actors TEXT NOT NULL
);
CREATE INDEX run_changes_run ON run_changes(run_id);
CREATE INDEX run_changes_chat ON run_changes(chat_id);
"#;

/// `extensions`는 설치한 확장이다. `parts`는 부분별 판정을 담은 JSON 글이다. 이관은 표만 비어 있게 더한다.
const V12: &str = r#"
CREATE TABLE extensions (
    name TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    installed_at INTEGER NOT NULL,
    parts TEXT NOT NULL
);
"#;

/// `steered_run`은 실행 중에 끼워 넣어 적용한 입력이 들어간 실행이고, `steered_after`는 그때까지 채팅에 쌓인 마지막
/// 기록 번호다. 실행을 연 입력과 아직 끼워 넣지 않았거나 거절된 입력은 NULL이고, 이관 전 입력도 NULL이다.
const V13: &str = r#"
ALTER TABLE inputs ADD COLUMN steered_run INTEGER;
ALTER TABLE inputs ADD COLUMN steered_after INTEGER;
"#;

/// `direct_installs`는 provider에 직접 설치된 항목 중 옮길지 물은 것이다. `state`는 `asked`나 `moved`다. 이관은 표만
/// 비어 있게 더한다.
const V14: &str = r#"
CREATE TABLE direct_installs (
    provider TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    state TEXT NOT NULL,
    seen_at INTEGER NOT NULL,
    PRIMARY KEY (provider, kind, name)
);
"#;

/// `raw_unattributed`는 어느 실행의 것인지 알 수 없는 provider 원시 줄이다. 이관은 표만 비어 있게 더한다.
const V15: &str = r#"
CREATE TABLE raw_unattributed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    agent_id INTEGER,
    provider_session TEXT,
    is_json INTEGER NOT NULL,
    bytes BLOB NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX raw_unattributed_chat ON raw_unattributed(chat_id);
"#;

/// `handoff_packets`는 provider에 보낸 인계 패킷의 시도마다 한 행이다. 본문은 해시와 크기만 두고, `handoff_packet_items`가
/// 패킷에 들어갔거나 빠진 재료를 기록 번호나 제약 번호로 가리킨다. 이관은 표만 비어 있게 더한다.
const V16: &str = r#"
CREATE TABLE handoff_packets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chat_id INTEGER NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    reduced_from INTEGER,
    session_id INTEGER NOT NULL,
    input_id INTEGER,
    run_id INTEGER,
    provider TEXT NOT NULL,
    provider_session TEXT,
    settings_revision INTEGER NOT NULL,
    chat_revision INTEGER NOT NULL,
    constraint_revision INTEGER NOT NULL,
    policy TEXT NOT NULL,
    body_hash TEXT NOT NULL,
    body_bytes INTEGER NOT NULL,
    estimated_tokens INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX handoff_packets_chat ON handoff_packets(chat_id);
CREATE INDEX handoff_packets_input ON handoff_packets(input_id);
CREATE TABLE handoff_packet_items (
    packet_id INTEGER NOT NULL REFERENCES handoff_packets(id) ON DELETE CASCADE,
    zone TEXT NOT NULL,
    ref_id INTEGER NOT NULL,
    selector TEXT NOT NULL,
    form TEXT,
    reason TEXT
);
CREATE INDEX handoff_packet_items_packet ON handoff_packet_items(packet_id);
"#;

/// 모델 판단 그림자. 판단 기록 한 건에 한 행이고, 후보와 확률은 JSON으로 둔다.
const V17: &str = r#"
CREATE TABLE model_shadows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    judgment_id INTEGER NOT NULL UNIQUE,
    input_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    settings_revision INTEGER NOT NULL,
    chat_revision INTEGER NOT NULL,
    policy_digest TEXT NOT NULL,
    catalog_version TEXT NOT NULL,
    question_set TEXT NOT NULL,
    candidates_hash TEXT NOT NULL,
    candidates TEXT NOT NULL,
    status TEXT NOT NULL,
    applied_model TEXT,
    request_bytes INTEGER NOT NULL
);
CREATE INDEX model_shadows_input ON model_shadows(input_id);
"#;

/// 실행마다 완료 검사 근거. 이관 전 실행과 근거를 가리지 않은 실행은 `evidence_state`가 비어 있다.
const V18: &str = r#"
ALTER TABLE runs ADD COLUMN evidence_state TEXT;
ALTER TABLE runs ADD COLUMN evidence_reason TEXT;
ALTER TABLE runs ADD COLUMN evidence_events TEXT;
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

    #[tokio::test]
    async fn v17_file_migrates_to_completion_evidence_columns_keeping_runs() {
        let (dir, store) = temp_store_at(17).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0);
             INSERT INTO runs (id, chat_id, task_id, agent_id, session_id, provider, effect_scope, started_at)
             VALUES (7, 1, 3, 4, 5, 'claude', 'unobserved', 100)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (17, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        // 이관 전 실행은 근거를 가리지 않았으므로 비어 있다
        assert_eq!(
            store
                .run_completion(saturn_protocol::ids::RunId(7))
                .await
                .unwrap(),
            None
        );
        let kept: (i64, i64) = sqlx::query_as("SELECT chat_id, started_at FROM runs WHERE id = 7")
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert_eq!(kept, (1, 100));
    }

    #[tokio::test]
    async fn v16_file_migrates_to_model_shadows_keeping_chats() {
        let (dir, store) = temp_store_at(16).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (16, SCHEMA_VERSION));
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.model_shadows_by_judgment().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v15_file_migrates_to_handoff_packets_keeping_chats() {
        let (dir, store) = temp_store_at(15).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (15, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.packets_of_chat(chat).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v14_file_migrates_to_raw_unattributed_keeping_chats() {
        let (dir, store) = temp_store_at(14).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (14, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.unattributed_raw(chat).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v13_file_migrates_to_direct_installs_keeping_chats() {
        let (dir, store) = temp_store_at(13).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (13, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.direct_rows().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v12_file_migrates_to_steered_input_columns_keeping_inputs() {
        let (dir, store) = temp_store_at(12).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0); \
             INSERT INTO inputs (id, chat_id, text, settings_revision, permission, workdir, skip_relation, state, accepted_at) \
             VALUES (1, 1, 'old input', 1, 'Write', '/work', 0, 'Applied', 0)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (12, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        assert!(
            store
                .steered_inputs(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        let (input, state) = store
            .stored_input(saturn_protocol::ids::InputId(1))
            .await
            .unwrap();
        assert_eq!(input.text, "old input");
        assert_eq!(state, saturn_protocol::state::InputState::Applied);
    }

    #[tokio::test]
    async fn v11_file_migrates_to_extensions_keeping_chats() {
        let (dir, store) = temp_store_at(11).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (11, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.extension_rows().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn v8_file_migrates_to_provider_versions_keeping_chats() {
        let (dir, store) = temp_store_at(8).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (8, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        assert_eq!(
            store
                .chat_workdir(saturn_protocol::ids::ChatId(1))
                .await
                .unwrap(),
            PathBuf::from("/work")
        );
        let provider = crate::providers::test_support::CODEX;
        assert_eq!(store.provider_cli_version(provider).await.unwrap(), None);
    }

    #[tokio::test]
    async fn v7_file_migrates_to_chat_name_and_group_keeping_chats() {
        let (dir, store) = temp_store_at(7).await;
        sqlx::raw_sql("INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0)")
            .execute(&store.pool)
            .await
            .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (7, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        assert_eq!(store.chat_labels(chat).await.unwrap(), (None, None));
    }

    #[tokio::test]
    async fn constraint_v9_file_migrates_to_empty_constraint_tables_keeping_chats() {
        let (dir, store) = temp_store_at(9).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0); \
             INSERT INTO inputs (id, chat_id, text, settings_revision, permission, workdir, skip_relation, state, accepted_at) \
             VALUES (3, 1, 'always answer in English', 1, 'Write', '/work', 0, 'Applied', 0)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (9, SCHEMA_VERSION));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        for table in [
            "constraints",
            "constraint_events",
            "constraint_asks",
            "constraint_exceptions",
            "packet_constraints",
        ] {
            let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&store.pool)
                .await
                .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        let inputs = store.history_page(chat, None, 10).await.unwrap().entries;
        assert_eq!(inputs.len(), 1);
    }

    #[tokio::test]
    async fn v10_file_migrates_to_run_changes_keeping_runs() {
        let (dir, store) = temp_store_at(10).await;
        sqlx::raw_sql(
            "INSERT INTO chats (id, workdir, created_at) VALUES (1, '/work', 0);
             INSERT INTO runs (id, chat_id, task_id, agent_id, session_id, provider, effect_scope, started_at)
             VALUES (7, 1, 3, 4, 5, 'claude', 'unobserved', 100)",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.pool.close().await;

        let (store, notice) = Store::open(dir.path()).await.unwrap();

        let notice = notice.unwrap();
        assert_eq!((notice.from, notice.to), (10, SCHEMA_VERSION));
        // 이관 전 실행은 측정하지 않았으므로 `changes_state`가 비어 있다
        let kept: (i64, i64, String, i64, Option<String>) = sqlx::query_as(
            "SELECT chat_id, task_id, provider, started_at, changes_state FROM runs WHERE id = 7",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(kept, (1, 3, "claude".to_owned(), 100, None));
        assert!(notice.backup.exists());
        assert_eq!(std::fs::read_dir(store.backup_dir()).unwrap().count(), 1);
        let chat = saturn_protocol::ids::ChatId(1);
        assert_eq!(
            store.chat_workdir(chat).await.unwrap(),
            PathBuf::from("/work")
        );
        assert!(store.run_changes(chat).await.unwrap().is_empty());
    }
}
