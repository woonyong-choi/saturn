//! 모델 판단 그림자: 켜면 입력 처리 요청에 후보별 충분성 질문을 묶어 모델 선택을 미리 재 보고 기록한다. 실제 선택에는 쓰지 않는다.
//! 설계: docs/design/router.md#모델-판단-그림자

use saturn_core::models::Quality;
use saturn_core::queue::QueuedInput;
use saturn_core::routers::shadow::{
    SET_MODEL_SHADOW, ShadowRead, Split, shadow_fits, shadow_questions,
};
use saturn_core::routers::{QuestionSetId, RouterRequest};
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision};

use crate::Engine;
use crate::store::{NewModelShadow, ShadowCandidate, ShadowStatus, sha256_hex};

/// 판단 요청에 그림자 질문을 묻고 돌려받은 값. 판단 기록을 쓸 때 함께 쓴다.
#[derive(Debug, Clone)]
pub(crate) struct ShadowPlan {
    pub(crate) candidates: Vec<String>,
    pub(crate) read: ShadowRead,
    pub(crate) question_set: QuestionSetId,
    /// 그림자 질문이 요청에 더한 바이트.
    pub(crate) bytes: usize,
    pub(crate) chat_revision: ChatRevision,
    pub(crate) policy: String,
    /// 실제로 정한 모델. 사용자 고정이나 기본 모델이고, 정하지 않았으면 `None`(현재 모델).
    pub(crate) decided: Option<String>,
}

impl Engine {
    /// 켜져 있고, 모델을 고정하지 않았고, 후보가 있고, 더해도 요청이 나뉘지 않을 때만 그림자 질문을 더한다.
    pub(crate) fn add_shadow(
        &self,
        record: &QueuedInput,
        enabled: bool,
        request: &mut RouterRequest,
    ) {
        if !enabled || record.pinned_model.is_some() {
            return;
        }
        let candidates = self.model_candidates(record.chat);
        if candidates.is_empty() {
            return;
        }
        let (id, questions) = shadow_questions(&candidates);
        if shadow_fits(request, &questions) {
            request.sets.push((id, questions));
        }
    }

    /// 묻지 않았으면 `None`.
    pub(crate) fn shadow_plan(
        &self,
        request: &RouterRequest,
        split: &Split,
        (chat_revision, policy): (ChatRevision, String),
    ) -> Option<ShadowPlan> {
        if split.shadow == ShadowRead::NotAsked {
            return None;
        }
        let (question_set, questions) = request
            .sets
            .iter()
            .find(|(id, _)| id.name == SET_MODEL_SHADOW)?;
        let candidates: Vec<String> = questions
            .iter()
            .map(|question| {
                question
                    .id
                    .split_once(':')
                    .map_or(question.id.as_str(), |(_, candidate)| candidate)
                    .to_owned()
            })
            .collect();
        let bytes = questions
            .iter()
            .map(|question| question.id.len() + question.text.len())
            .sum();
        Some(ShadowPlan {
            candidates,
            read: split.shadow.clone(),
            question_set: question_set.clone(),
            bytes,
            chat_revision,
            policy,
            decided: None,
        })
    }

    /// 판단 기록을 쓴 뒤 그림자 기록을 쓴다. 어긋난 판단은 미적용으로 쓴다. 쓰기 실패는 로그만 남긴다.
    pub(crate) async fn record_shadow(
        &self,
        (judgment, input, chat, settings): (JudgmentId, InputId, ChatId, SettingsRevision),
        plan: ShadowPlan,
        superseded: bool,
    ) {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let answered = match &plan.read {
            ShadowRead::Answered(answers) => answers.clone(),
            _ => Vec::new(),
        };
        let candidates: Vec<ShadowCandidate> = plan
            .candidates
            .iter()
            .map(|text| {
                let (provider, model) = text.split_once('/').unwrap_or(("", text));
                let quality = self.catalog.quality(provider, model, None, &today);
                ShadowCandidate {
                    model: text.clone(),
                    quality: quality_label(quality),
                    probability: answered
                        .iter()
                        .find(|(candidate, _)| candidate == text)
                        .map(|(_, yes)| *yes),
                }
            })
            .collect();
        let status = if superseded {
            ShadowStatus::Superseded
        } else {
            match plan.read {
                ShadowRead::Answered(_) => ShadowStatus::Answered,
                ShadowRead::Invalid => ShadowStatus::Invalid,
                ShadowRead::NoAnswer | ShadowRead::NotAsked => ShadowStatus::NoAnswer,
            }
        };
        let shadow = NewModelShadow {
            judgment,
            input,
            chat,
            settings,
            chat_revision: plan.chat_revision,
            policy: plan.policy,
            catalog_version: self.catalog.version.clone(),
            question_set: plan.question_set,
            candidates_hash: candidates_hash(&self.catalog.version, &plan.candidates),
            candidates,
            status,
            applied_model: if superseded { None } else { plan.decided },
            request_bytes: plan.bytes,
        };
        if let Err(error) = self.store.record_model_shadow(&shadow).await {
            tracing::warn!(error = %crate::masked_chain(&self.masker, &error), "failed to record model shadow");
        }
    }
}

/// 후보 목록과 모델 목록 버전의 지문. 같은 후보를 같은 순서로 물었는지 비교한다.
fn candidates_hash(catalog_version: &str, candidates: &[String]) -> String {
    sha256_hex(format!("{catalog_version}\n{}", candidates.join("\n")).as_bytes())
}

fn quality_label(quality: Quality) -> String {
    use saturn_core::models::Unverified;
    match quality {
        Quality::Verified => "verified".to_owned(),
        Quality::Unverified(reason) => format!(
            "unverified:{}",
            match reason {
                Unverified::NotInCatalog => "not-in-catalog",
                Unverified::CatalogExpired => "catalog-expired",
                Unverified::NoDatedEvidence => "no-dated-evidence",
                Unverified::RevisionUnknown => "revision-unknown",
                Unverified::RevisionChanged => "revision-changed",
            }
        ),
    }
}
