//! judge 학습 실행(`/train`, `saturn train`): 실행 조건, 채점, 라벨 게이트, 느린 조정, 로컬 학습, 승격, 베이스 모델 교체.
//!
//! 설계: docs/design/judge-training.md(`/train` 실행, 채점, 느린 조정, 승격 게이트, 베이스 모델 교체, judge 버전).
//! 계산 규칙(라벨 게이트, 기준값 조정, 승격 게이트)은 `saturn_core::judges::calibration`에 있고 여기는 실행만 맡는다.
//! 채점은 `grading`, 로컬 학습기 실행과 평가는 `trainer`.
//!
//! `/train` 흐름:
//! 1. `preview`: 지난 실행 뒤 채점 안 된 판단 수를 센다. 200건 미만이면 `NotEnough`로 거절하고 부족한 건수를 보인다.
//! 2. 200건 이상이면 채점 후보와 예상 토큰으로 `TrainPreview`를 만들어 TUI 확인 창에 보인다.
//! 3. 사용자가 확인하면 `run`: 채점 → 라벨 게이트 → 라벨 저장.
//! 4. 느린 조정으로 질문별 중심값을 다시 계산한다(질문별 채점된 판단 200건 이상일 때만).
//! 5. 로컬 학습기로 Saturn 모델을 학습하고 승격 게이트로 현재 모델과 비교한다.
//!
//! TODO(#91): 채점 안 된 판단 수, 채점 후보, 라벨, judge 버전을 읽고 쓰는 `store` 메서드가 아직 없다
//! TODO(#55): 멈춤 명령이 진행 중인 학습도 멈출지, 학습 전용 중지를 둘지 미정

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

/// `/train`을 시작할 수 있는 채점 안 된 판단의 최소 건수. 느린 조정도 질문별로 이 건수 이상일 때만 한다.
pub const MIN_UNLABELED: u32 = 200;

/// 학습 실행 오류.
#[derive(Debug, thiserror::Error)]
pub enum TrainingError {
    /// 채점 안 된 판단이 200건 미만이다. 실행하지 않고 부족한 건수를 보인다.
    #[error("not enough unlabeled judgments: {have} of {need}")]
    NotEnough {
        /// 지금 건수.
        have: u32,
        /// 필요한 건수(`MIN_UNLABELED`).
        need: u32,
    },
    /// 채점 모델이 설정에 없다.
    #[error("grading model is not configured")]
    NoGrader,
    /// 채점 모델 호출 실패.
    #[error("grading model call failed")]
    Grader(#[source] JudgesError),
    /// 로컬 학습기(Python, MLX) 실행 실패나 0이 아닌 종료.
    #[error("local trainer failed: {detail}")]
    Trainer {
        /// 종료 코드와 가린 stderr 첫 줄.
        detail: String,
    },
    /// 학습기 프로세스 실행·중지 실패.
    #[error("trainer process failed")]
    Process(#[from] ProcessError),
    /// 판단 기록, 라벨, judge 버전 읽기·쓰기 실패.
    #[error("training record access failed")]
    Store(#[from] StoreError),
    /// 없는 judge 버전(`--from`, `saturn judge version`).
    #[error("judge version not found: {version}")]
    UnknownVersion {
        /// 버전 이름.
        version: String,
    },
}

/// `/train` 선택.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainOptions {
    /// 1차 영점으로 기준값을 되돌린다(`--reset-thresholds`, 버전 화면 `r`).
    pub reset_thresholds: bool,
    /// 이 judge 버전에서 다시 학습한다(`--from`, 버전 화면 `t`).
    pub from: Option<String>,
}

/// 학습 확인 창 값. `Notification::TrainPreview`로 보낸다.
#[derive(Debug, Clone)]
pub struct TrainPlan {
    /// 채점할 후보(우선순위 순서).
    pub candidates: Vec<GradingCandidate>,
    /// 채점 모델.
    pub grader: String,
    /// 예상 토큰(채점 모델이 순서를 두 번 바꿔 풀기 포함).
    pub estimated_tokens: u64,
    /// 느린 조정 대상 질문 id(채점된 판단 200건 이상).
    pub threshold_targets: Vec<String>,
    /// 모델도 다시 학습하는지.
    pub retrain_model: bool,
    /// 사용자 선택.
    pub options: TrainOptions,
}

impl TrainPlan {
    /// 확인 창 알림으로 바꾼다.
    pub fn to_notification(&self) -> Notification {
        todo!("#91")
    }
}

/// 학습 결과.
#[derive(Debug, Clone)]
pub enum TrainOutcome {
    /// 승격 게이트를 통과해 새 judge 버전이 됐다.
    Promoted {
        /// 새 버전 이름.
        version: String,
        /// 평가 결과.
        report: EvalReport,
    },
    /// 게이트를 통과하지 못해 현재 버전을 유지한다. 기준값 조정은 반영됐다.
    Kept {
        /// 평가 결과.
        report: EvalReport,
    },
    /// 베이스 모델 교체 뒤 게이트를 통과했고 사용자 승인을 기다린다. 승인 전에는 바꾸지 않는다.
    AwaitingApproval {
        /// 후보 버전 이름.
        version: String,
        /// 평가 결과.
        report: EvalReport,
    },
    /// `--reset-thresholds`만 했다.
    ThresholdsReset,
}

/// 1~2단계. 실행 조건을 보고 확인 창 값을 만든다.
/// 1. `store`에서 지난 실행 뒤 채점 안 된 판단 수를 센다. `MIN_UNLABELED` 미만이면 `NotEnough`.
/// 2. `select_candidates`로 후보를 고르고 `estimate_tokens`로 예상 토큰을 센다.
/// 3. 질문별 채점된 판단 수로 느린 조정 대상을 고른다.
///
/// # Errors
/// 200건 미만이면 `NotEnough`, 채점 모델이 없으면 `NoGrader`, 조회 실패면 `Store`.
pub async fn preview(
    store: &Store,
    settings: &Settings,
    options: TrainOptions,
) -> Result<TrainPlan, TrainingError> {
    todo!("#91")
}

/// 3~5단계. 사용자가 확인한 계획을 실행한다. 진행은 `Notification::TrainProgress`로 `progress`에 보낸다.
/// 1. `grade`로 후보마다 채점한다(채점 모델마다 선택지 순서를 두 번 바꿔 풀고 같은 답만 채택).
/// 2. `split_labels`로 라벨 게이트(`calibration::gate_label`)를 거쳐 학습용과 평가용으로 나누고 저장한다.
/// 3. `recenter_thresholds`로 느린 조정을 한다.
/// 4. `run_trainer`로 학습하고 `evaluate`와 `calibration::should_promote`로 승격 여부를 정한다.
///
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

/// 느린 조정. 질문별로 채점된 판단 전부로 중심값을 다시 계산하고(목표 틀림 비율을 지키는 가장 낮은 값, 기본 5%)
/// 빠른 조정 차이를 0으로 되돌린다. 채점된 판단이 200건 미만인 질문은 그대로 둔다. 계산은 `ThresholdState::recenter`.
///
/// # Errors
/// 라벨이나 기준값 기록 실패면 `Store`.
pub async fn recenter_thresholds(
    store: &Store,
    targets: &[String],
) -> Result<Vec<ThresholdState>, TrainingError> {
    todo!("#91")
}

/// 기준값 재계산. judge 버전이 바뀌면 그 버전의 질문별 목표 틀림 비율에서 기준값을 다시 계산한다(버전에는 숫자 대신 목표를 남긴다).
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

/// 베이스 모델 교체.
/// 1. 쌓인 판단 기록 전부로 새 베이스 모델에서 다시 학습한다.
/// 2. 승격 게이트로 현재 모델과 비교한다.
/// 3. 게이트를 통과해도 `AwaitingApproval`로 돌려주고, 사용자가 `approve_swap`으로 승인할 때만 바꾼다.
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

/// 사용자가 승인한 버전을 현재 judge 버전으로 바꾸고 기준값을 다시 계산한다(`UseJudgeVersion`, 버전 화면 `u`).
///
/// # Errors
/// 없는 버전이면 `UnknownVersion`, 기록 실패면 `Store`.
pub async fn approve_swap(store: &Store, version: &str) -> Result<(), TrainingError> {
    todo!("#91")
}

/// 학습 산출물 폴더. 승격된 모델도 여기에 버전별로 둔다. TODO(#91): 경로 미정
pub fn models_dir(home: &std::path::Path) -> PathBuf {
    todo!("#91")
}
