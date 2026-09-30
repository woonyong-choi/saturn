//! 스키마 버전 확인, 이관 직전 백업, 자동 이관, 오래된 백업 삭제.
//!
//! 설계: docs/design/records.md(스키마 이관). `Store::open`이 사용자당 잠금을 얻은 직후 이 순서로 부른다.
//! TODO(#29): 첫 스키마를 전체 정의로 쓸지, 옛 스키마 위 변경분으로 쓸지
//! TODO(#30): 옛 구현 v8 기록 파일을 만나면 가져오지 않을지, 명령으로 가져올지

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::{Store, StoreError};

/// 이 실행 파일이 아는 스키마 버전. SQLite `PRAGMA user_version`에 적는다. 스키마를 바꾸면 1 올리고 이관 단계를 더한다.
pub const SCHEMA_VERSION: u32 = 1;

/// 이관 직전 백업 보관 기간. 만든 뒤 14일이 지나면 다음 시작 때 지운다.
pub const BACKUP_RETENTION: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// 이관 사실 한 줄 안내. TUI 대화 기록과 stderr에 한 번 보인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationNotice {
    /// 이관 전 버전.
    pub from: u32,
    /// 이관 뒤 버전(`SCHEMA_VERSION`).
    pub to: u32,
    /// 만든 백업 파일. `~/.saturn/backup/saturn-v<from>-<unix초>.db`. 14일 뒤 지운다.
    pub backup: PathBuf,
}

impl MigrationNotice {
    /// 한 줄 문구. 예: `기록 저장소 스키마 1 → 2 이관 · 백업 ~/.saturn/backup/... (14일 보관)`.
    pub fn line(&self) -> String {
        todo!("#82")
    }
}

impl Store {
    /// 파일의 스키마 버전(`PRAGMA user_version`). 새 파일은 0.
    ///
    /// # Errors
    /// 질의 실패면 `Database`.
    pub(crate) async fn schema_version(&self) -> Result<u32, StoreError> {
        todo!("#82")
    }

    /// 이관 직전 백업. SQLite 온라인 백업(`VACUUM INTO`)으로 `backup/`에 새 파일을 만든 뒤, 그 전 백업 파일을 모두 지운다.
    /// 백업은 항상 가장 최근 1개만 남는다. 새 파일(버전 0)은 백업하지 않는다.
    ///
    /// # Errors
    /// 파일 생성이나 옛 백업 삭제 실패면 `Backup`. 이 경우 이관하지 않는다.
    pub(crate) async fn backup_before_migration(&self, from: u32) -> Result<PathBuf, StoreError> {
        todo!("#82")
    }

    /// `from`에서 `SCHEMA_VERSION`까지 단계별 이관을 한 거래로 실행하고 `user_version`을 올린다.
    /// `from > SCHEMA_VERSION`이면 아무것도 바꾸지 않는다(호출 전에 `NewerSchema`로 거른다).
    ///
    /// # Errors
    /// 어느 단계든 실패하면 거래를 되돌리고 `Migration`.
    pub(crate) async fn migrate(&self, from: u32) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// `backup/`에서 수정 시각이 `now - BACKUP_RETENTION`보다 이른 백업 파일을 지운다. 시작마다 한 번 부른다.
    /// 지운 파일 수를 돌려준다. `backup/` 밖 파일은 건드리지 않는다.
    ///
    /// # Errors
    /// 목록 조회나 삭제 실패면 `Backup`.
    pub(crate) async fn sweep_backups(&self, now: SystemTime) -> Result<usize, StoreError> {
        todo!("#82")
    }
}
