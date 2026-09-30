//! 설정 스냅샷: 검사를 통과한 병합 결과와 층 목록을 설정 번호로 저장한다. 같은 내용이면 기존 번호를 다시 쓴다.
//!
//! 설계: docs/design/settings.md(병합과 설정 번호). 설정 원본은 파일이고 여기는 적용된 결과만 둔다.

use saturn_protocol::ids::SettingsRevision;

use super::{Store, StoreError};
use crate::settings::SettingsSnapshot;

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
        todo!("#82")
    }

    /// 번호로 스냅샷을 읽는다. 입력은 접수 때 고정한 번호로 이것을 불러 끝까지 같은 값을 쓴다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`, 저장된 JSON이 깨졌으면 `Json`.
    pub async fn settings_snapshot(
        &self,
        revision: SettingsRevision,
    ) -> Result<SettingsSnapshot, StoreError> {
        todo!("#82")
    }

    /// 가장 최근에 적용한 설정 번호. 한 번도 적용하지 않았으면 `None`(시작 때 검사 실패와 겹치면 실행하지 않는다).
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn latest_settings_revision(&self) -> Result<Option<SettingsRevision>, StoreError> {
        todo!("#82")
    }

    /// 가장 최근 적용 번호를 기록한다. 재사용한 번호도 다시 최근으로 올린다.
    ///
    /// # Errors
    /// 없는 번호면 `NotFound`.
    pub async fn mark_settings_applied(
        &self,
        revision: SettingsRevision,
    ) -> Result<(), StoreError> {
        todo!("#82")
    }
}
