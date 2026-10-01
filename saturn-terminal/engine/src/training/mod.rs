//! judge 학습 실행. 계산 규칙은 `saturn_core::judges::calibration`에 있고 여기는 실행만 맡는다.
//! 설계: docs/design/judge-training.md
//! TODO(#91): 채점 안 된 판단 수, 채점 후보, 라벨, judge 버전을 읽고 쓰는 `store` 메서드 없음
//! TODO(#55): 멈춤 명령이 진행 중인 학습도 멈출지 미정

mod grading;
mod trainer;

use std::path::PathBuf;

use saturn_core::judges::calibration::{EvalReport, ThresholdState};
use saturn_protocol::rpc::Notification;
use tokio::sync::mpsc;

use crate::judges::{JudgesError, RemoteJudge};
use crate::processes::{ProcessError, Supervisor};
use crate::settings::Settings;
use crate::store::{Store, StoreError};

pub use grading::{
    GradingCandidate, LabeledSets, estimate_tokens, grade, select_candidates, split_labels,
};
pub use trainer::{CandidateModel, TrainerSpec, evaluate, run_trainer};

/// 느린 조정의 질문별 최소 채점 건수로도 쓴다.
pub const MIN_UNLABELED: u32 = 200;

#[derive(Debug, thiserror::Error)]
pub enum TrainingError {
    #[error("not enough unlabeled judgments: {have} of {need}")]
    NotEnough { have: u32, need: u32 },
    #[error("grading model is not configured")]
    NoGrader,
    #[error("grading model call failed")]
    Grader(#[source] JudgesError),
    #[error("local trainer failed: {detail}")]
    Trainer {
        /// 종료 코드와 가린 stderr 첫 줄.
        detail: String,
    },
    #[error("trainer process failed")]
    Process(#[from] ProcessError),
    #[error("training record access failed")]
    Store(#[from] StoreError),
    #[error("judge version not found: {version}")]
    UnknownVersion { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainOptions {
    /// 기준값을 1차 영점으로 되돌린다.
    pub reset_thresholds: bool,
    /// 이 judge 버전에서 다시 학습한다.
    pub from: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TrainPlan {
    /// 우선순위 순서.
    pub candidates: Vec<GradingCandidate>,
    pub grader: String,
    pub estimated_tokens: u64,
    /// 느린 조정 대상 질문 id.
    pub threshold_targets: Vec<String>,
    pub retrain_model: bool,
    pub options: TrainOptions,
}

impl TrainPlan {
    pub fn to_notification(&self) -> Notification {
        todo!("#91")
    }
}

#[derive(Debug, Clone)]
pub enum TrainOutcome {
    Promoted {
        version: String,
        report: EvalReport,
    },
    /// 현재 버전을 유지하되 기준값 조정은 반영됐다.
    Kept {
        report: EvalReport,
    },
    /// 베이스 모델 교체 후보. 사용자 승인 전에는 바꾸지 않는다.
    AwaitingApproval {
        version: String,
        report: EvalReport,
    },
    ThresholdsReset,
}

/// # Errors
/// `MIN_UNLABELED` 미만이면 `NotEnough`, 채점 모델이 없으면 `NoGrader`, 조회 실패면 `Store`.
pub async fn preview(
    store: &Store,
    settings: &Settings,
    options: TrainOptions,
) -> Result<TrainPlan, TrainingError> {
    todo!("#91")
}

/// # Errors
/// 채점 실패면 `Grader`, 학습기 실패면 `Trainer`나 `Process`, 기록 실패면 `Store`.
pub async fn run(
    store: &Store,
    grader: &RemoteJudge,
    supervisor: &Supervisor,
    plan: TrainPlan,
    progress: mpsc::Sender<Notification>,
) -> Result<TrainOutcome, TrainingError> {
    todo!("#91")
}

/// 느린 조정. 쓰인 결과가 `MIN_RECENTER_RESULTS` 미만인 질문은 그대로 둔다.
pub async fn recenter_thresholds(
    store: &Store,
    targets: &[String],
) -> Result<Vec<ThresholdState>, TrainingError> {
    todo!("#91")
}

/// `reset`이면 1차 영점으로 되돌린다.
///
/// # Errors
/// 없는 버전이면 `UnknownVersion`, 기록 실패면 `Store`.
pub async fn recompute_thresholds(
    store: &Store,
    version: &str,
    reset: bool,
) -> Result<Vec<ThresholdState>, TrainingError> {
    todo!("#91")
}

/// 게이트를 통과해도 `AwaitingApproval`을 돌려주고 `approve_swap` 전에는 바꾸지 않는다.
///
/// # Errors
/// 학습기 실패면 `Trainer`나 `Process`, 기록 실패면 `Store`.
pub async fn on_base_model_change(
    store: &Store,
    supervisor: &Supervisor,
    base_model: &str,
    progress: mpsc::Sender<Notification>,
) -> Result<TrainOutcome, TrainingError> {
    todo!("#91")
}

/// 기준값도 다시 계산한다.
///
/// # Errors
/// 없는 버전이면 `UnknownVersion`, 기록 실패면 `Store`.
pub async fn approve_swap(store: &Store, version: &str) -> Result<(), TrainingError> {
    todo!("#91")
}

/// 승격된 모델도 버전별로 둔다. TODO(#91): 경로 미정
pub fn models_dir(home: &std::path::Path) -> PathBuf {
    todo!("#91")
}
