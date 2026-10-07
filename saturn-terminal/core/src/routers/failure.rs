//! router 호출이 실패했을 때의 재시도 간격과 대체 행동.
//! 설계: docs/design/router.md#router-실패

use std::time::Duration;

use saturn_protocol::ids::{ChatRevision, SettingsRevision};
use saturn_protocol::state::Disposition;

use super::{RouteDecision, RouterRequest, question_ids};

/// 실패한 호출을 다시 보내기 전에 기다리는 시간.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// 첫 호출 뒤 다시 보내는 최대 횟수.
pub const MAX_RETRIES: u32 = 2;

/// 첫 실패 시각부터 포기까지의 전체 마감. 진행 중인 시도도 이 시각에 끊는다.
pub const RETRY_DEADLINE: Duration = Duration::from_secs(10);

/// 입력 처리 판단을 포기하고 현재 모델로 진행할 때 남기는 로그.
pub const SKIP_MODEL_MESSAGE: &str = "판단 모델 실패로 모델 선택을 건너뜁니다";

/// 첫 실패 뒤 아직 쓸 수 있는 시간. 마감이 지났으면 0.
pub fn remaining_until_deadline(since_first_failure: Duration) -> Duration {
    RETRY_DEADLINE.saturating_sub(since_first_failure)
}

/// `failed_attempts`는 지금까지 실패한 호출 수(1 이상), `since_first_failure`는 첫 실패 뒤 흐른 시간.
/// 횟수를 다 썼거나 간격을 기다린 시점이 마감을 넘으면 포기해 `None`.
pub fn retry_delay(failed_attempts: u32, since_first_failure: Duration) -> Option<Duration> {
    let within_count = failed_attempts <= MAX_RETRIES;
    let within_deadline = since_first_failure + RETRY_INTERVAL <= RETRY_DEADLINE;
    (within_count && within_deadline).then_some(RETRY_INTERVAL)
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 요청 질문 수
// basis: estimate
/// 입력 처리 판단이 실패하면 모델 선택과 끼워 넣기·대기·새 작업 판단을 건너뛰고 현재 에이전트와 현재 모델로 보낸다.
///
/// 실행 중이면 현재 에이전트의 턴에 끼워 넣고, 아니면 현재 에이전트에 바로 보낸다. 입력을 대기로 두지 않는다.
/// `fallbacks`에는 묻지 못한 질문을 모두 남긴다.
pub fn route_after_failure(
    request: &RouterRequest,
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
        is_conflict: false,
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: asked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::{ConstraintQuestion, questions_for_input};

    const REVISION: ChatRevision = ChatRevision(3);
    const SETTINGS: SettingsRevision = SettingsRevision(2);

    fn request(running: bool) -> RouterRequest {
        RouterRequest {
            model: "router".into(),
            state: "state".into(),
            sets: questions_for_input(
                running,
                false,
                false,
                &["model-a".to_string()],
                ConstraintQuestion::Without,
            ),
        }
    }

    #[test]
    fn retry_delay_waits_five_seconds_twice_then_gives_up() {
        let none = Duration::ZERO;
        assert_eq!(retry_delay(1, none), Some(Duration::from_secs(5)));
        assert_eq!(
            retry_delay(2, Duration::from_secs(5)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(retry_delay(3, Duration::from_secs(10)), None);
    }

    #[test]
    fn retry_delay_gives_up_when_waiting_would_pass_the_deadline() {
        assert_eq!(retry_delay(1, Duration::from_secs(5)), Some(RETRY_INTERVAL));
        assert_eq!(retry_delay(1, Duration::from_millis(5001)), None);
        assert_eq!(retry_delay(1, Duration::from_secs(10)), None);
    }

    #[test]
    fn remaining_until_deadline_counts_down_to_zero() {
        assert_eq!(
            remaining_until_deadline(Duration::from_secs(3)),
            Duration::from_secs(7)
        );
        assert_eq!(
            remaining_until_deadline(Duration::from_secs(10)),
            Duration::ZERO
        );
        assert_eq!(
            remaining_until_deadline(Duration::from_secs(99)),
            Duration::ZERO
        );
    }

    #[test]
    fn route_after_failure_keeps_the_current_agent_and_model() {
        // (사례, 실행 중인지, 예상 처리 방식, 건너뛴 질문으로 기록돼야 할 목록)
        let cases = [
            (
                "idle sends to the current agent",
                false,
                Disposition::Queue,
                &[][..],
            ),
            (
                "running steers instead of queueing",
                true,
                Disposition::Steer,
                &[
                    "keep_current",
                    "is_actionable",
                    "target_model",
                    "relation_to_running",
                    "steer_or_spawn",
                ][..],
            ),
        ];

        for (name, running, disposition, skipped) in cases {
            let decision = route_after_failure(&request(running), REVISION, SETTINGS);

            assert_eq!(decision.disposition, disposition, "{name}");
            assert!(decision.keep_current, "{name}");
            assert_eq!(decision.model, None, "{name}");
            assert!(!decision.resume_held, "{name}");
            assert_eq!(decision.revision, REVISION, "{name}");
            assert_eq!(decision.settings, SETTINGS, "{name}");
            for id in skipped {
                assert!(
                    decision.fallbacks.iter().any(|recorded| recorded == id),
                    "{name}: {id}"
                );
            }
        }
    }

    #[test]
    fn skip_messages_match_the_decided_wording() {
        assert_eq!(
            SKIP_MODEL_MESSAGE,
            "판단 모델 실패로 모델 선택을 건너뜁니다"
        );
    }
}
