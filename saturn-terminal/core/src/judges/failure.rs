//! judge 호출이 실패했을 때의 재시도 간격과 대체 행동.
//! 설계: docs/design/judge.md#judge-실패

use std::time::Duration;

use saturn_protocol::ids::{ChatRevision, SettingsRevision};
use saturn_protocol::state::Disposition;

use super::{JudgeRequest, RouteDecision, question_ids};

/// 실패한 호출을 다시 보내기 전에 기다리는 시간.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// 첫 호출 뒤 다시 보내는 최대 횟수. 간격 두 번이라 기다림은 10초에서 끝난다.
pub const MAX_RETRIES: u32 = 2;

/// 입력 처리 판단을 포기하고 현재 모델로 진행할 때 남기는 로그.
pub const SKIP_MODEL_MESSAGE: &str = "판단 모델 실패로 모델 선택을 건너뜁니다";

/// 패킷의 `compact` 판단을 포기하고 순위 순서로 채울 때 남기는 로그.
pub const SKIP_RECORD_MESSAGE: &str = "판단 모델 실패로 기록 선택을 건너뜁니다";

/// 누가 session 전환을 시작했는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionStarter {
    /// judge가 고른 모델이 달라 시작한 전환.
    Judge,
    /// 사용자가 모델을 고정했거나 맥락 크기 규칙이 시작한 전환.
    Forced,
}

/// `compact` 판단이 실패했을 때 패킷에 대한 행동.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactFailure {
    /// 전환하지 않고 현재 session과 모델로 진행한다.
    SkipTransition,
    /// 전환하고 경쟁 구역을 순위 순서로 채운다.
    FillByRank,
}

/// `failed_attempts`는 지금까지 실패한 호출 수(1 이상). 포기하면 `None`.
pub fn retry_delay(failed_attempts: u32) -> Option<Duration> {
    (failed_attempts <= MAX_RETRIES).then_some(RETRY_INTERVAL)
}

/// 판단 없이 전환하면 패킷 없음과 같은 정답률이라 judge가 시작한 전환만 건너뛴다.
pub fn compact_failure(starter: TransitionStarter) -> CompactFailure {
    match starter {
        TransitionStarter::Judge => CompactFailure::SkipTransition,
        TransitionStarter::Forced => CompactFailure::FillByRank,
    }
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 요청 질문 수
// basis: estimate
/// 입력 처리 판단이 실패하면 모델 선택과 끼워 넣기·대기·새 작업 판단을 건너뛰고 현재 에이전트와 현재 모델로 보낸다.
///
/// 실행 중이면 현재 에이전트의 턴에 끼워 넣고, 아니면 현재 에이전트에 바로 보낸다. 입력을 대기로 두지 않는다.
/// `fallbacks`에는 묻지 못한 질문을 모두 남긴다.
pub fn route_after_failure(
    request: &JudgeRequest,
    revision: ChatRevision,
    settings: SettingsRevision,
) -> RouteDecision {
    let asked: Vec<String> = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions.iter().map(|question| question.id.clone()))
        .collect();
    let is_running = asked
        .iter()
        .any(|id| id == question_ids::RELATION_TO_RUNNING);
    RouteDecision {
        revision,
        settings,
        disposition: if is_running {
            Disposition::Steer
        } else {
            Disposition::Queue
        },
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: asked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judges::questions_for_input;
    use crate::sessions::ranking::order_after_judge;

    use saturn_protocol::ids::LedgerSeq;

    const REVISION: ChatRevision = ChatRevision(3);
    const SETTINGS: SettingsRevision = SettingsRevision(2);

    fn request(running: bool) -> JudgeRequest {
        JudgeRequest {
            model: "judge".into(),
            state: "state".into(),
            sets: questions_for_input(running, false, false, &["model-a".to_string()]),
        }
    }

    #[test]
    fn retry_delay_waits_five_seconds_twice_then_gives_up() {
        assert_eq!(retry_delay(1), Some(Duration::from_secs(5)));
        assert_eq!(retry_delay(2), Some(Duration::from_secs(5)));
        assert_eq!(retry_delay(3), None);
        let waited: Duration = (1..).map_while(retry_delay).sum();
        assert_eq!(waited, Duration::from_secs(10));
    }

    #[test]
    fn route_after_failure_idle_sends_to_current_agent_and_model() {
        let decision = route_after_failure(&request(false), REVISION, SETTINGS);

        assert_eq!(decision.disposition, Disposition::Queue);
        assert!(decision.keep_current);
        assert_eq!(decision.model, None);
        assert!(!decision.resume_held);
        assert_eq!(decision.revision, REVISION);
        assert_eq!(decision.settings, SETTINGS);
    }

    #[test]
    fn route_after_failure_running_steers_instead_of_queueing() {
        let decision = route_after_failure(&request(true), REVISION, SETTINGS);

        assert_eq!(decision.disposition, Disposition::Steer);
        assert!(decision.keep_current);
        assert_eq!(decision.model, None);
    }

    #[test]
    fn route_after_failure_records_every_skipped_question() {
        let decision = route_after_failure(&request(true), REVISION, SETTINGS);

        for id in [
            "keep_current",
            "is_actionable",
            "target_model",
            "relation_to_running",
            "steer_or_spawn",
        ] {
            assert!(
                decision.fallbacks.iter().any(|skipped| skipped == id),
                "{id}"
            );
        }
    }

    #[test]
    fn compact_failure_skips_judge_transition_and_fills_forced_one() {
        assert_eq!(
            compact_failure(TransitionStarter::Judge),
            CompactFailure::SkipTransition
        );
        assert_eq!(
            compact_failure(TransitionStarter::Forced),
            CompactFailure::FillByRank
        );
    }

    #[test]
    fn compact_failure_forced_transition_orders_competing_zone_by_rank() {
        let ranked: Vec<LedgerSeq> = [7, 3, 9].map(LedgerSeq).to_vec();

        let action = compact_failure(TransitionStarter::Forced);

        assert_eq!(action, CompactFailure::FillByRank);
        assert_eq!(order_after_judge(&ranked, &[]), ranked);
    }

    #[test]
    fn skip_messages_match_the_decided_wording() {
        assert_eq!(
            SKIP_MODEL_MESSAGE,
            "판단 모델 실패로 모델 선택을 건너뜁니다"
        );
        assert_eq!(
            SKIP_RECORD_MESSAGE,
            "판단 모델 실패로 기록 선택을 건너뜁니다"
        );
    }
}
