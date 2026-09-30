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

use std::path::{Path, PathBuf};

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
    /// SQLite 질의나 거래 실패. 거래는 전부 되돌려졌다. TODO(#82): 원인을 `sqlx::Error`로 바꾼다
    #[error("database operation failed")]
    Database(#[source] Box<dyn std::error::Error + Send + Sync>),
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
    // TODO(#82): sqlx `SqlitePool` 필드. 연결 옵션: WAL 저널, `foreign_keys=ON`, `busy_timeout`, 쓰기 연결 하나
}

impl Store {
    /// `home/saturn.db`를 열고(없으면 만든다) 스키마를 맞춘다. 호출 전에 `rpc`가 사용자당 engine 잠금을 잡았어야 한다.
    /// 순서: 스키마 버전 읽기 → 올라갔으면 `schema::backup_before_migration` → `schema::migrate` → `schema::sweep_backups`.
    /// 이관했으면 한 줄 안내를 돌려주고 호출자가 TUI와 stderr에 보인다.
    ///
    /// # Errors
    /// 파일을 못 열면 `Open`, 파일 버전이 더 높으면 `NewerSchema`, 백업 실패면 `Backup`, 이관 실패면 `Migration`.
    pub async fn open(home: &Path) -> Result<(Self, Option<MigrationNotice>), StoreError> {
        todo!("#82")
    }

    /// DB 파일 경로 `home/saturn.db`.
    pub fn db_path(&self) -> PathBuf {
        todo!("#82")
    }

    /// 백업 폴더 경로 `home/backup`.
    pub fn backup_dir(&self) -> PathBuf {
        todo!("#82")
    }
}
