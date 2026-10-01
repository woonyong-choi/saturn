//! judge 학습용 채점과 라벨 나누기.
//! 설계: docs/design/judge-training.md
//! TODO(#45): 채점 모델 출력을 학습에도 쓸지 미정, 그때까지 라벨에 출처를 남긴다
//! TODO(#64): subagent 출력이 섞인 턴의 처리 미정

use saturn_core::judges::Answer;
use saturn_core::judges::calibration::{Label, Signal};
use saturn_protocol::ids::JudgmentId;

use super::TrainingError;
use crate::judges::RemoteJudge;

#[derive(Debug, Clone)]
pub struct GradingCandidate {
    pub judgment: JudgmentId,
    /// 질문 id.
    pub question: String,
    pub answer: Answer,
    /// 관찰 시간 전이면 `None`.
    pub signal: Option<Signal>,
    pub confidence: f64,
    /// judge끼리 답이 갈렸는지.
    pub disagreement: bool,
    /// 결정 이후 대화의 가린 원문.
    pub later_context: String,
    /// 사용자가 피드백으로 직접 답했으면 그 답.
    pub human: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct LabeledSets {
    pub train: Vec<(JudgmentId, Label)>,
    pub eval: Vec<(JudgmentId, Label)>,
    /// 게이트에서 버린 라벨 수.
    pub dropped: u32,
}

/// TODO(#91): 낮은 확신도 기준과 한 번에 채점할 최대 건수 미정
pub fn select_candidates(pool: Vec<GradingCandidate>) -> Vec<GradingCandidate> {
    todo!("#91")
}

/// 후보마다 채점 모델 수 × 2(선택지 순서) × (질문 + 결정 이후 대화)로 센다.
pub fn estimate_tokens(candidates: &[GradingCandidate], graders: usize) -> u64 {
    todo!("#91")
}

/// # Errors
/// 채점 모델 호출이 실패하면 `Grader`.
pub async fn grade(
    grader: &RemoteJudge,
    candidate: &GradingCandidate,
) -> Result<Label, TrainingError> {
    todo!("#91")
}

/// 판정은 `calibration::gate_label`이 맡는다.
pub fn split_labels(labels: Vec<(JudgmentId, Label)>) -> LabeledSets {
    todo!("#91")
}
