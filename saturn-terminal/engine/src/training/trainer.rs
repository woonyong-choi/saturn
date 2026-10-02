//! 로컬 학습기(Python, MLX) 실행과 평가. 자식 환경에서 router 키를 지운다(`secrets::scrub`).
//! 설계: docs/design/router-training.md
//! TODO(#43): 학습 출발점 미정
//! TODO(#91): 학습 스크립트 위치, Python 실행 파일 찾기, 산출물 형식 미정

use std::path::PathBuf;

use saturn_core::routers::calibration::EvalReport;

use super::{LabeledSets, TrainingError};
use crate::processes::Supervisor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrainerSpec {
    pub python: PathBuf,
    pub script: PathBuf,
    pub base_model: String,
    /// `None`이면 현재 버전.
    pub from: Option<String>,
    /// JSONL.
    pub train_set: PathBuf,
    /// JSONL.
    pub eval_set: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CandidateModel {
    pub version: String,
    pub path: PathBuf,
    pub base_model: String,
}

/// stderr는 가린 뒤에만 로그에 남긴다.
///
/// # Errors
/// 실행 실패면 `Process`, 0이 아닌 종료면 `Trainer`.
pub(crate) async fn run_trainer(
    supervisor: &Supervisor,
    spec: &TrainerSpec,
    labels: &LabeledSets,
) -> Result<CandidateModel, TrainingError> {
    todo!("#91")
}

/// 승격 여부는 호출자가 `calibration::should_promote`로 정한다.
///
/// # Errors
/// 평가 실행 실패면 `Trainer`나 `Process`.
pub(crate) async fn evaluate(
    supervisor: &Supervisor,
    candidate: &CandidateModel,
    current: &str,
    labels: &LabeledSets,
) -> Result<EvalReport, TrainingError> {
    todo!("#91")
}
