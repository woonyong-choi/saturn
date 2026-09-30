//! 채점: 후보 고르기, 예상 토큰, 채점 모델 호출(선택지 순서 두 번 바꿔 풀기), 라벨 게이트로 학습용·평가용 나누기.
//!
//! 설계: docs/design/judge-training.md(채점, 결과 신호, 사용자에게 묻기).
//! 채점 모델은 설정(`Settings::grading_model`)으로 정하고 외부 judge 전송 규칙(`RemoteJudge`)을 그대로 쓴다.
//! TODO(#45): 기준 judge와 채점 모델 출력을 비교·평가에만 쓸지, 허용 범위에서 학습에도 쓸지 미정. 정해지기 전에는 라벨에 출처를 남긴다
//! TODO(#64): subagent 출력이 섞인 턴을 출처로 구분할지, 빼거나 구분 없이 둘지 미정

use saturn_core::judges::Answer;
use saturn_core::judges::calibration::{Label, Signal};
use saturn_protocol::ids::JudgmentId;

use super::TrainingError;
use crate::judges::RemoteJudge;

/// 채점 후보 하나. 판단 기록에서 만든다.
#[derive(Debug, Clone)]
pub struct GradingCandidate {
    /// 판단 기록 id.
    pub judgment: JudgmentId,
    /// 질문 id.
    pub question: String,
    /// 판단 때의 답.
    pub answer: Answer,
    /// 확정된 결과 신호. 관찰 시간(다음 입력 3개 또는 10분) 전이면 `None`.
    pub signal: Option<Signal>,
    /// 판단 때 확신도.
    pub confidence: f64,
    /// judge끼리 답이 갈렸는지.
    pub disagreement: bool,
    /// 사후 판정에 쓸 결정 이후 대화(가린 원문).
    pub later_context: String,
    /// 사용자가 피드백으로 직접 답했으면 그 답. 사람 라벨로 AI 채점 치우침을 보정한다.
    pub human: Option<bool>,
}

/// 게이트를 통과한 라벨.
#[derive(Debug, Clone, Default)]
pub struct LabeledSets {
    /// 학습용: 합의, 선택지 순서 일관, 반대 결과 없음.
    pub train: Vec<(JudgmentId, Label)>,
    /// 평가용: 만장일치, 결과 신호와 일치.
    pub eval: Vec<(JudgmentId, Label)>,
    /// 버린 수.
    pub dropped: u32,
}

/// 후보를 우선순위대로 고른다. 결과 신호가 있거나, 확신도가 낮거나, judge끼리 답이 갈린 판단을 먼저 둔다.
/// TODO(#91): 낮은 확신도 기준과 한 번에 채점할 최대 건수 미정
pub fn select_candidates(pool: Vec<GradingCandidate>) -> Vec<GradingCandidate> {
    todo!("#91")
}

/// 예상 토큰. 후보마다 채점 모델 수 × 순서 두 번 × (질문 + 결정 이후 대화)로 센다.
pub fn estimate_tokens(candidates: &[GradingCandidate], graders: usize) -> u64 {
    todo!("#91")
}

/// 후보 하나를 채점한다.
/// 1. 채점 모델마다 선택지 순서를 두 번 바꿔 푼다. 두 답이 같을 때만 그 모델의 답으로 채택한다(`order_consistent`).
/// 2. 사후 판정은 결정 이후 대화를 보되 결정 시점에 알 수 있던 정보를 기준으로 정답을 묻는다.
/// 3. 결과 신호와 사람 답을 붙여 `Label`을 만든다. 결합과 게이트는 `split_labels`가 한다.
///
/// # Errors
/// 채점 모델 호출이 실패하면 `Grader`.
pub async fn grade(
    grader: &RemoteJudge,
    candidate: &GradingCandidate,
) -> Result<Label, TrainingError> {
    todo!("#91")
}

/// 라벨 게이트. 라벨마다 `calibration::gate_label`로 학습용, 평가용, 버림을 정한다.
/// 뒤집기와 취소는 약한 라벨로 가중하는 규칙도 `gate_label`이 맡는다.
pub fn split_labels(labels: Vec<(JudgmentId, Label)>) -> LabeledSets {
    todo!("#91")
}
