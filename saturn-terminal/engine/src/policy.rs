//! 판단 정책 고정: 판단에 쓰는 정책은 입력을 접수할 때 정한 설정 번호의 기준값과 engine 시작 때 정한 router 모델이다.
//! 판단 기록, 피드백, 취소, 실패는 정책을 바꾸지 않는다. 새 정책은 설정 변경이나 engine 재시작으로만 들어온다.
//! 설계: docs/design/router.md#정책-고정

use saturn_protocol::ids::SettingsRevision;

use crate::settings::Settings;
use crate::store::sha256_hex;
use crate::{Engine, EngineError};

/// 정책 지문은 기준값 표, router 종류·모델, 모델 목록 버전의 SHA-256이다. 같은 기준값과 같은 router면 같다.
pub(crate) fn policy_digest(
    settings: &Settings,
    router_id: &str,
    router_model: &str,
    catalog_version: &str,
) -> String {
    let thresholds = settings
        .get("router.thresholds")
        .map(ToString::to_string)
        .unwrap_or_default();
    sha256_hex(format!("{router_id}\n{router_model}\n{thresholds}\n{catalog_version}").as_bytes())
}

impl Engine {
    /// 설정 번호 하나에 대한 정책 지문. router는 engine이 시작 때 고른 것이다.
    ///
    /// # Errors
    /// 없는 설정 번호면 `Store`.
    pub(crate) async fn policy_digest_at(
        &self,
        revision: SettingsRevision,
    ) -> Result<String, EngineError> {
        let settings = self.settings.at(&self.store, revision).await?;
        let active = self.routers.active();
        Ok(policy_digest(
            &settings,
            active.router_id(),
            active.model(),
            &self.catalog.version,
        ))
    }
}
