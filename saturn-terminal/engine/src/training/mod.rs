//! router 학습 실행. 계산 규칙은 `saturn_core::routers::calibration`에 있고 여기는 실행만 맡는다.
//! 설계: docs/design/router-training.md
//! TODO(#91): 채점 안 된 판단 수, 채점 후보, 라벨, router 버전을 읽고 쓰는 `store` 메서드 없음
//! TODO(#55): 멈춤 명령이 진행 중인 학습도 멈출지 미정

#![expect(clippy::todo, reason = "#91 뼈대")]

mod grading;
mod trainer;

use std::path::PathBuf;

use saturn_core::routers::calibration::{EvalReport, ThresholdState};
use saturn_protocol::rpc::Notification;
use tokio::sync::mpsc;

use crate::processes::{ProcessError, Supervisor};
use crate::routers::{RemoteRouter, RoutersError};
use crate::settings::Settings;
use crate::store::{Store, StoreError};

pub(crate) use grading::{GradingCandidate, LabeledSets};

/// 느린 조정의 질문별 최소 채점 건수로도 쓴다.
pub(crate) const MIN_UNLABELED: u32 = 200;

#[derive(Debug, thiserror::Error)]
pub enum TrainingError {
    #[error("not enough unlabeled judgments: {have} of {need}")]
    NotEnough { have: u32, need: u32 },
    #[error("grading model is not configured")]
    NoGrader,
    #[error("grading model call failed")]
    Grader(#[source] RoutersError),
    #[error("local trainer failed: {detail}")]
    Trainer {
        /// 종료 코드와 가린 stderr 첫 줄.
        detail: String,
    },
    #[error("trainer process failed")]
    Process(#[from] ProcessError),
    #[error("training record access failed")]
    Store(#[from] StoreError),
    #[error("router version not found: {version}")]
    UnknownVersion { version: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrainOptions {
    /// 기준값을 1차 영점으로 되돌린다.
    pub reset_thresholds: bool,
    /// 이 router 버전에서 다시 학습한다.
    pub from: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrainPlan {
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
    pub(crate) fn to_notification(&self) -> Notification {
        todo!("#91")
    }
}

#[derive(Debug, Clone)]
pub(crate) enum TrainOutcome {
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
pub(crate) async fn preview(
    store: &Store,
    settings: &Settings,
    options: TrainOptions,
) -> Result<TrainPlan, TrainingError> {
    todo!("#91")
}

/// # Errors
/// 채점 실패면 `Grader`, 학습기 실패면 `Trainer`나 `Process`, 기록 실패면 `Store`.
pub(crate) async fn run(
    store: &Store,
    grader: &RemoteRouter,
    supervisor: &Supervisor,
    plan: TrainPlan,
    progress: mpsc::Sender<Notification>,
) -> Result<TrainOutcome, TrainingError> {
    todo!("#91")
}

/// 느린 조정. 결과 신호를 확정한 모든 판단 기록으로 `Observation` 목록을 만들어 질문마다 `recenter`에 넘긴다. 쓰인 결과가 `MIN_RECENTER_RESULTS` 미만인 질문은 그대로 둔다. 중심값과 최저값·최고값이 든 상태는 호출하는 쪽이 만든다.
///
/// # Errors
/// 기록 조회 실패면 `Store`.
pub(crate) async fn recenter_thresholds(
    store: &Store,
    mut states: Vec<ThresholdState>,
) -> Result<Vec<ThresholdState>, TrainingError> {
    let observations = store.observations().await?;
    for state in &mut states {
        state.recenter(&observations);
    }
    Ok(states)
}

/// `reset`이면 1차 영점으로 되돌린다.
///
/// # Errors
/// 없는 버전이면 `UnknownVersion`, 기록 실패면 `Store`.
pub(crate) async fn recompute_thresholds(
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
pub(crate) async fn on_base_model_change(
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
pub(crate) async fn approve_swap(store: &Store, version: &str) -> Result<(), TrainingError> {
    todo!("#91")
}

/// 승격된 모델도 버전별로 둔다. TODO(#91): 경로 미정
pub(crate) fn models_dir(home: &std::path::Path) -> PathBuf {
    todo!("#91")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_core::routers::calibration::{MIN_RECENTER_RESULTS, Signal};

    use super::*;
    use crate::store::test_judgment;
    use crate::store::tests::temp_store;

    const QUESTION: &str = "keep_current";

    /// 행동한 판단(p = 0.9) 중 모두 틀림 신호를 받은 `count`건.
    async fn store_with_wrong_actions(count: usize) -> (tempfile::TempDir, Store) {
        let (dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        for _ in 0..count {
            let id = store
                .record_judgment(&test_judgment(chat))
                .await
                .unwrap()
                .unwrap();
            store.record_signal(id, Signal::Wrong).await.unwrap();
        }
        (dir, store)
    }

    fn state() -> ThresholdState {
        ThresholdState::new(QUESTION, 0.8, (0.5, 0.95))
    }

    #[tokio::test]
    async fn recenter_thresholds_with_enough_recorded_results_moves_center() {
        let (_dir, store) = store_with_wrong_actions(MIN_RECENTER_RESULTS).await;

        let states = recenter_thresholds(&store, vec![state()]).await.unwrap();

        assert!(states[0].center > 0.8);
    }

    #[tokio::test]
    async fn recenter_thresholds_below_min_results_keeps_center() {
        let (_dir, store) = store_with_wrong_actions(MIN_RECENTER_RESULTS - 1).await;

        let states = recenter_thresholds(&store, vec![state()]).await.unwrap();

        assert_eq!(states[0].center, 0.8);
    }

    #[tokio::test]
    async fn recenter_thresholds_ignores_judgments_still_being_observed() {
        let (_dir, store) = temp_store().await;
        let chat = store.create_chat(PathBuf::from("/work")).await.unwrap();
        for _ in 0..MIN_RECENTER_RESULTS {
            store.record_judgment(&test_judgment(chat)).await.unwrap();
        }

        let states = recenter_thresholds(&store, vec![state()]).await.unwrap();

        assert_eq!(states[0].center, 0.8);
    }
}
