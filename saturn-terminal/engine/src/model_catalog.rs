//! 배포에 묶인 모델 평가 근거 목록. engine이 시작할 때 한 번 읽고 끝까지 쓴다.
//! 설계: docs/design/model-evidence.md

use saturn_core::models::ModelCatalog;

const BUILTIN: &str = include_str!("../data/model-catalog.json");

/// 배포에 담은 목록. 읽지 못하거나 검사에 실패하면 빈 목록이라 모든 모델이 미검증이다.
// cost: time O(n), heap O(n), stack O(1)
// vars: n = 목록 글자 수
// basis: estimate
pub(crate) fn builtin() -> ModelCatalog {
    parse(BUILTIN).unwrap_or_else(|error| {
        tracing::error!(%error, "model catalog is not usable, every model stays unverified");
        ModelCatalog {
            version: "none".to_owned(),
            collected: "1970-01-01".to_owned(),
            expires: "1970-01-01".to_owned(),
            entries: Vec::new(),
        }
    })
}

fn parse(text: &str) -> Result<ModelCatalog, String> {
    let catalog: ModelCatalog = serde_json::from_str(text).map_err(|error| error.to_string())?;
    catalog.validate().map_err(|error| error.to_string())?;
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use saturn_core::models::{EvidenceKind, Quality, Unverified};

    use super::*;

    // 배포한 목록은 검사를 통과하고, 출처 없는 점수와 품질 확정 근거가 없는 모델은 미검증이다
    #[test]
    fn shipped_catalog_is_valid_and_confirms_no_quality_without_measurements() {
        let catalog = parse(BUILTIN).unwrap();
        assert!(!catalog.entries.is_empty());
        for entry in &catalog.entries {
            assert!(
                entry
                    .evidence
                    .iter()
                    .all(|evidence| evidence.kind == EvidenceKind::OfficialSpec),
                "{}",
                entry.model_id
            );
            assert!(matches!(
                catalog.quality(
                    &entry.provider,
                    &entry.model_id,
                    entry.revision.as_deref(),
                    "2026-10-06"
                ),
                Quality::Unverified(Unverified::NoDatedEvidence | Unverified::RevisionUnknown)
            ));
        }
        assert!(parse("{").is_err());
    }
}
