//! 새 작업의 모델 정하기: 사용자 명시 고정, router 선택, 사용자 선호, 기본 모델, 현재 모델의 순서와 그 이유를 기록한다.
//! 설계: docs/design/providers-and-sessions.md#기본-모델과-선택-방식

use std::collections::HashMap;

use saturn_core::models::{AvailableModel, CandidateInput, Quality, SkippedReason, candidate_set};
use saturn_core::queue::QueuedInput;
use saturn_protocol::rpc::{ModelChoice, ModelMode};

use crate::Engine;
use crate::models::ModelPlan;
use crate::providers::Registry;
use crate::store::{NewModelSelection, SelectionSource, sha256_hex};

/// 판단 기록을 쓸 때 함께 쓰는 모델 정하기 결과.
#[derive(Debug, Clone)]
pub(crate) struct SelectionPlan {
    pub(crate) source: SelectionSource,
    /// 정한 모델. 현재 모델이나 provider 기본값이면 `None`.
    pub(crate) model: Option<String>,
    /// router 선택을 쓰지 못한 이유. 고정이거나 router 선택을 썼으면 `None`.
    pub(crate) reason: Option<&'static str>,
    /// 선호했지만 쓰지 못한 모델과 이유.
    pub(crate) skipped_preferences: Vec<(String, &'static str)>,
    pub(crate) candidates_hash: String,
    pub(crate) policy: String,
}

/// router 선택을 쓰지 못한 이유를 가르는 판단 결과.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouterOutcome {
    pub(crate) failed: bool,
    pub(crate) invalid: bool,
}

impl Engine {
    /// 순서는 사용자 명시 고정, router 선택(오토 모드, 지원하는 모델), 사용자 선호(오토 모드, 품질 확정 후보), 기본 모델,
    /// 현재 모델이다. 선호는 강제 고정이 아니라 품질을 확정한 후보 안의 우선순위다.
    pub(crate) fn select_model(
        &self,
        (record, plan): (&QueuedInput, &ModelPlan),
        routed: Option<String>,
        (outcome, policy): (RouterOutcome, String),
    ) -> SelectionPlan {
        let candidates = self.model_candidates(record.chat);
        let base = |source, model, reason, skipped| SelectionPlan {
            source,
            model,
            reason,
            skipped_preferences: skipped,
            candidates_hash: sha256_hex(
                format!("{}\n{}", self.catalog.version, candidates.join("\n")).as_bytes(),
            ),
            policy: policy.clone(),
        };
        if record.pinned_model.is_some() {
            return base(
                SelectionSource::Pinned,
                record.pinned_model.clone(),
                None,
                Vec::new(),
            );
        }
        let mut reason = None;
        if let Some(text) = routed.filter(|_| plan.mode == ModelMode::Auto) {
            if self.is_supported(record, &text) {
                return base(SelectionSource::Router, Some(text), None, Vec::new());
            }
            reason = Some("unsupported");
        }
        let reason = reason.or_else(|| match () {
            () if plan.mode == ModelMode::Manual => Some("manual"),
            () if candidates.is_empty() => Some("no-candidates"),
            () if outcome.failed => Some("router-failed"),
            () if outcome.invalid => Some("invalid"),
            () => Some("fallback"),
        });
        let (preferred, skipped) = if plan.mode == ModelMode::Auto {
            self.preferred_model(record, plan)
        } else {
            (None, Vec::new())
        };
        if let Some(text) = preferred {
            return base(SelectionSource::Preference, Some(text), reason, skipped);
        }
        match &plan.default {
            Some(default) => base(
                SelectionSource::Default,
                Some(default.clone()),
                reason,
                skipped,
            ),
            None => base(SelectionSource::Current, None, reason, skipped),
        }
    }

    /// provider가 지금 알려 준 모델 목록에 있는 모델인지. 목록에 없으면 어댑터가 지원한다고 알 수 없다.
    fn is_supported(&self, record: &QueuedInput, text: &str) -> bool {
        let Some(choice) = self.registry.parse_pinned(text) else {
            return false;
        };
        self.flow
            .models
            .get(&(record.chat, choice.provider))
            .is_some_and(|list| list.iter().any(|info| info.choice == choice))
    }

    /// 선호 순서에서 품질을 확정한 첫 후보. 확정하지 못했거나 없는 선호는 이유와 함께 돌려준다. 지금은 provider가 모델의
    /// revision을 알려 주지 않아 선호는 쓰이지 않고 건너뛴 이유만 남는다.
    fn preferred_model(
        &self,
        record: &QueuedInput,
        plan: &ModelPlan,
    ) -> (Option<String>, Vec<(String, &'static str)>) {
        let preferences: Vec<ModelChoice> = plan
            .prefer
            .iter()
            .filter_map(|text| self.registry.parse_pinned(text))
            .collect();
        if preferences.is_empty() {
            return (None, Vec::new());
        }
        let available: Vec<AvailableModel> = self
            .registry
            .ids()
            .into_iter()
            .filter_map(|provider| self.flow.models.get(&(record.chat, provider)))
            .flatten()
            .map(|info| AvailableModel {
                choice: info.choice.clone(),
                revision: None,
            })
            .collect();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let set = candidate_set(&CandidateInput {
            catalog: &self.catalog,
            available: &available,
            limits: &HashMap::new(),
            pinned: None,
            preferences: &preferences,
            today: &today,
        });
        let chosen = set
            .candidates
            .iter()
            .find(|candidate| {
                candidate.quality == Quality::Verified && preferences.contains(&candidate.choice)
            })
            .map(|candidate| Registry::pinned_text(&candidate.choice));
        let skipped = set
            .skipped_preferences
            .iter()
            .map(|skipped| {
                let reason = match skipped.reason {
                    SkippedReason::NotAvailable => "not-available",
                    SkippedReason::LimitExhausted => "limit-exhausted",
                    SkippedReason::Unverified => "unverified",
                };
                (Registry::pinned_text(&skipped.choice), reason)
            })
            .collect();
        (chosen, skipped)
    }

    /// 판단 기록을 쓴 뒤 모델 정하기 기록을 쓴다. 어긋난 판단은 적용하지 않은 것으로 쓴다. 쓰기 실패는 로그만 남긴다.
    pub(crate) async fn record_selection(
        &self,
        ids: crate::store::SelectionIds,
        plan: SelectionPlan,
        applied: bool,
    ) {
        let selection = NewModelSelection {
            ids,
            source: plan.source,
            model: plan.model,
            reason: plan.reason,
            skipped_preferences: plan.skipped_preferences,
            candidates_hash: plan.candidates_hash,
            policy: plan.policy,
            catalog_version: self.catalog.version.clone(),
            applied,
        };
        if let Err(error) = self.store.record_model_selection(&selection).await {
            tracing::warn!(error = %crate::masked_chain(&self.masker, &error), "failed to record model selection");
        }
    }
}
