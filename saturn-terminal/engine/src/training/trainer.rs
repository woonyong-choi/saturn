//! 로컬 학습기 실행(Python, MLX 프로세스)과 평가.
//!
//! 설계: docs/design/judge-training.md(`/train` 실행, 승격 게이트, 베이스 모델 교체).
//! Apple Silicon에서 로컬로 학습한다. 학습기는 `processes::Supervisor`로 띄워 멈춤과 종료 때 같이 정리한다.
//! 자식 환경은 provider와 같이 `secrets::scrub`을 두 번 거친다(judge 키를 학습기에 넘기지 않는다).
//! TODO(#43): 학습 출발점(공개 체크포인트에서 이어 학습, 항목별 yes/no 확률 직접 학습) 미정
//! TODO(#91): 학습 스크립트 위치, Python 실행 파일 찾기, 산출물 형식 미정

use std::path::PathBuf;

use saturn_core::judges::calibration::EvalReport;

use super::{LabeledSets, TrainingError};
use crate::processes::Supervisor;

/// 학습기 실행 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainerSpec {
    /// Python 실행 파일.
    pub python: PathBuf,
    /// 학습 스크립트(MLX).
    pub script: PathBuf,
    /// 베이스 모델.
    pub base_model: String,
    /// 이어 학습할 judge 버전. `None`이면 현재 버전.
    pub from: Option<String>,
    /// 학습용 라벨 파일(JSONL).
    pub train_set: PathBuf,
    /// 평가용 라벨 파일(JSONL).
    pub eval_set: PathBuf,
    /// 산출물 폴더.
    pub output: PathBuf,
}

/// 학습한 후보 모델.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateModel {
    /// 후보 버전 이름.
    pub version: String,
    /// 모델 경로.
    pub path: PathBuf,
    /// 베이스 모델.
    pub base_model: String,
}

/// 라벨을 파일로 쓰고 학습기를 실행해 끝날 때까지 기다린다. stderr는 가린 뒤에만 로그에 남긴다.
///
/// # Errors
/// 실행 실패면 `Process`, 0이 아닌 종료면 `Trainer`.
pub async fn run_trainer(
    supervisor: &Supervisor,
    spec: &TrainerSpec,
    labels: &LabeledSets,
) -> Result<CandidateModel, TrainingError> {
    todo!("#91")
}

/// 같은 평가용 라벨로 후보와 현재 모델을 비교한다. 정확도 차이의 95% 신뢰구간, Brier, ECE, 처리 비율, 선택지 순서 일관성.
/// 승격 여부는 호출자가 `calibration::should_promote`로 정한다.
///
/// # Errors
/// 평가 실행 실패면 `Trainer`나 `Process`.
pub async fn evaluate(
    supervisor: &Supervisor,
    candidate: &CandidateModel,
    current: &str,
    labels: &LabeledSets,
) -> Result<EvalReport, TrainingError> {
    todo!("#91")
}
