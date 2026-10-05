use std::collections::HashMap;

use saturn_protocol::rpc::ModelChoice;

use super::catalog::{ModelCatalog, Quality};

/// provider가 지금 쓸 수 있다고 알린 모델.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableModel {
    pub choice: ModelChoice,
    /// provider가 알려 준 모델의 고정 revision. 알려 주지 않으면 `None`.
    pub revision: Option<String>,
}

/// provider 한도 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitState {
    Available,
    /// 한도를 다 썼다. 후보에서 뺀다.
    Exhausted,
    /// 한도를 알 수 없다. 후보에는 두고 표시한다.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct CandidateInput<'a> {
    pub catalog: &'a ModelCatalog,
    /// provider 순서와 provider가 알려 준 순서를 지킨 가용 모델.
    pub available: &'a [AvailableModel],
    /// provider id 글자별 한도. 없는 provider는 `Unknown`이다.
    pub limits: &'a HashMap<String, LimitState>,
    /// 사용자가 명시로 고정한 모델(`/model`).
    pub pinned: Option<&'a ModelChoice>,
    /// 사용자 선호. 앞선 것이 우선이다. 고정이 아니다.
    pub preferences: &'a [ModelChoice],
    /// `YYYY-MM-DD`.
    pub today: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub choice: ModelChoice,
    pub quality: Quality,
    pub limit: LimitState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkippedReason {
    /// provider가 지금 알리지 않은 모델.
    NotAvailable,
    /// 한도를 다 썼다.
    LimitExhausted,
    /// 품질을 확정하지 못했다. 선호가 품질 하한을 덮지 않는다.
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPreference {
    pub choice: ModelChoice,
    pub reason: SkippedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSet {
    /// 사용자가 명시로 고정한 모델. 있으면 후보를 묻지 않고 이 모델을 쓴다. 선호와 품질 자료는 고정을 덮지 못한다.
    pub pinned: Option<ModelChoice>,
    /// router에 물을 후보. 고정이 있으면 비어 있다.
    pub candidates: Vec<Candidate>,
    pub skipped_preferences: Vec<SkippedPreference>,
}

/// 가용 모델에서 후보 집합을 만든다. 명시 고정은 후보를 대신하고, 선호는 한도가 남은 품질 확정 후보의 순서만 앞당긴다.
/// 미검증 후보도 후보에는 남고 표시만 다르다. 고르는 쪽이 미검증을 어떻게 다룰지 정한다.
// cost: time O(a·(c + p)), heap O(a + p), stack O(1)
// vars: a = 가용 모델 수, c = 목록 항목 수, p = 선호 수
// basis: estimate
#[must_use]
pub fn candidate_set(input: &CandidateInput<'_>) -> CandidateSet {
    if let Some(pinned) = input.pinned {
        return CandidateSet {
            pinned: Some(pinned.clone()),
            candidates: Vec::new(),
            skipped_preferences: Vec::new(),
        };
    }
    let limit_of = |model: &AvailableModel| {
        input
            .limits
            .get(model.choice.provider.as_str())
            .copied()
            .unwrap_or(LimitState::Unknown)
    };
    let mut candidates: Vec<Candidate> = input
        .available
        .iter()
        .filter(|model| limit_of(model) != LimitState::Exhausted)
        .map(|model| Candidate {
            choice: model.choice.clone(),
            quality: input.catalog.quality(
                model.choice.provider.as_str(),
                &model.choice.model,
                model.revision.as_deref(),
                input.today,
            ),
            limit: limit_of(model),
        })
        .collect();
    let mut skipped_preferences = Vec::new();
    let mut front = Vec::new();
    for preferred in input.preferences {
        if front
            .iter()
            .any(|candidate: &Candidate| candidate.choice == *preferred)
        {
            continue;
        }
        let reason = match candidates
            .iter()
            .position(|candidate| candidate.choice == *preferred)
        {
            Some(index) if candidates[index].quality == Quality::Verified => {
                front.push(candidates.remove(index));
                continue;
            }
            Some(_) => SkippedReason::Unverified,
            None if input
                .available
                .iter()
                .any(|model| model.choice == *preferred) =>
            {
                SkippedReason::LimitExhausted
            }
            None => SkippedReason::NotAvailable,
        };
        skipped_preferences.push(SkippedPreference {
            choice: preferred.clone(),
            reason,
        });
    }
    front.append(&mut candidates);
    CandidateSet {
        pinned: None,
        candidates: front,
        skipped_preferences,
    }
}
