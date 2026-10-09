//! 모델 평가 근거 목록과 후보 집합 규칙. 파일은 읽지 않고, 목록 자료는 호출하는 쪽이 넘긴다.
//! 설계: docs/design/model-evidence.md

mod candidates;
mod catalog;

#[cfg(test)]
mod tests;

pub use candidates::{
    AvailableModel, Candidate, CandidateInput, CandidateSet, LimitState, SkippedPreference,
    SkippedReason, candidate_set,
};
pub use catalog::{
    CatalogEntry, CatalogError, Evidence, EvidenceKind, ModelCatalog, Price, Quality, Unverified,
};
