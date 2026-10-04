//! compaction 판정과 보관 session으로 돌아갈 때의 재개 판정.
//! 설계: docs/design/context-management.md, docs/design/providers-and-sessions.md#그-provider로-돌아가기

use std::time::Duration;

/// 기대 잔여 턴을 모를 때 쓰는 값.
pub const DEFAULT_EXPECTED_TURNS: u32 = 3;

/// provider가 더 긴 유지 시간을 알려 주지 않을 때 쓰는 캐시 유지 시간.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);

/// 고정 구역의 하한 `packet_hard_limit`이 `threshold`에서 차지하는 비율(%)(초안).
pub const DEFAULT_PACKET_HARD_PERCENT: u64 = 20;

/// 항목 하나가 경쟁 구역 예산에서 원문으로 들어갈 수 있는 비율(%)(초안).
pub const DEFAULT_ITEM_CAP_PERCENT: u64 = 30;

#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    /// 절대 기준(토큰).
    pub t_abs: u64,
    /// 백분율(0~100).
    pub safety_percent: u8,
    /// 맥락 창 크기(토큰).
    pub window: u64,
    /// 단가 배수.
    pub cache_read: f64,
    /// 단가 배수.
    pub cache_write: f64,
    /// 마지막 턴 뒤 이만큼 지나면 캐시가 끝났다고 본다. provider가 알려 준 값이고 모르면 `DEFAULT_CACHE_TTL`.
    pub cache_ttl: Duration,
    /// 설정 `context.packet_hard_percent`. 1~100.
    pub packet_hard_percent: u64,
    /// 설정 `context.item_cap_percent`. 1~100.
    pub item_cap_percent: u64,
    /// 설정 `context.select.rrf_k`.
    pub rrf_k: u32,
}

impl ContextBudget {
    /// `safety_percent`가 100을 넘으면 100으로 본다.
    pub fn threshold(&self) -> u64 {
        let percent = u128::from(self.safety_percent.min(100));
        let by_window = u128::from(self.window) * percent / 100;
        let by_window = u64::try_from(by_window).expect("window share should fit in u64");
        self.t_abs.min(by_window)
    }

    /// `p95_growth`를 관측하기 전(`None`)에는 창 크기의 10%를 더한다.
    pub fn hard_limit(&self, p95_growth: Option<u64>) -> u64 {
        let growth = p95_growth.unwrap_or(self.window / 10);
        self.threshold().saturating_add(growth)
    }

    pub fn packet_limit(&self) -> u64 {
        self.threshold() / 10
    }

    /// 고정 구역이 `packet_limit`을 넘는 패킷에만 쓴다(초안).
    pub fn packet_hard_limit(&self) -> u64 {
        self.threshold() * self.packet_hard_percent.clamp(1, 100) / 100
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ContextMeasure {
    /// 잴 수 없으면 `None`이고 provider 자동 압축에 맡긴다.
    /// 루트(메인) 에이전트 메시지 기준이다.
    pub active: Option<u64>,
    pub packet: u64,
    pub tree_idle: bool,
    pub has_mergeable_queue: bool,
    pub since_last_turn: Duration,
    /// 모르면 `DEFAULT_EXPECTED_TURNS`로 본다.
    pub expected_turns: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnDecision {
    /// 보관 session을 재개하고 마지막으로 받은 기록 번호 뒤의 변경분만 붙인다.
    Resume,
    /// 새 session을 열고 패킷을 넘긴다.
    NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionDecision {
    Continue,
    Defer,
    Restart,
}

/// `measure.active`가 `None`이면 새 session을 열지 않고 provider 자동 압축에 맡긴다.
pub fn decide(budget: &ContextBudget, measure: &ContextMeasure) -> CompactionDecision {
    if !measure.tree_idle || measure.has_mergeable_queue {
        return CompactionDecision::Defer;
    }
    let Some(active) = measure.active else {
        return CompactionDecision::Continue;
    };
    let is_cache_expired = measure.since_last_turn > budget.cache_ttl;
    if is_cache_expired && measure.packet < active {
        return CompactionDecision::Restart;
    }
    if active < budget.threshold() {
        return CompactionDecision::Continue;
    }
    let expected = measure.expected_turns.unwrap_or(DEFAULT_EXPECTED_TURNS);
    if break_even_turns(budget, active, measure.packet) <= f64::from(expected) {
        return CompactionDecision::Restart;
    }
    CompactionDecision::Continue
}

/// 음수는 0으로, 옮겨도 턴당 비용이 줄지 않으면(`active ≤ packet`, `cache_read ≤ 0`) 무한대로 돌려준다.
pub fn break_even_turns(budget: &ContextBudget, active: u64, packet: u64) -> f64 {
    if active <= packet || budget.cache_read <= 0.0 {
        return f64::INFINITY;
    }
    let active = active as f64;
    let packet = packet as f64;
    let turns = (packet * budget.cache_write - active * budget.cache_read)
        / ((active - packet) * budget.cache_read);
    if turns.is_nan() {
        return f64::INFINITY;
    }
    turns.max(0.0)
}

/// `budget`은 돌아갈 provider의 것이고, `since_last_turn`이 `cache_ttl` 이하면 캐시 유지 시간 안으로 본다.
/// 유지 시간 안이면 재개한다. 지났으면 패킷이 session 맥락 `active`보다 작을 때만 새 session을 연다.
pub fn decide_return(
    budget: &ContextBudget,
    since_last_turn: Duration,
    active: u64,
    packet: u64,
) -> ReturnDecision {
    let is_cache_warm = since_last_turn <= budget.cache_ttl;
    if !is_cache_warm && packet < active {
        return ReturnDecision::NewSession;
    }
    ReturnDecision::Resume
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::ranking::DEFAULT_RRF_K;

    fn budget() -> ContextBudget {
        ContextBudget {
            t_abs: 100_000,
            safety_percent: 60,
            window: 200_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
            packet_hard_percent: DEFAULT_PACKET_HARD_PERCENT,
            item_cap_percent: DEFAULT_ITEM_CAP_PERCENT,
            rrf_k: DEFAULT_RRF_K,
        }
    }

    fn measure(active: u64, packet: u64) -> ContextMeasure {
        ContextMeasure {
            active: Some(active),
            packet,
            tree_idle: true,
            has_mergeable_queue: false,
            since_last_turn: Duration::from_secs(10),
            expected_turns: None,
        }
    }

    #[test]
    fn threshold_takes_smaller_of_abs_and_window_share() {
        let small_abs = ContextBudget {
            t_abs: 50_000,
            ..budget()
        };

        assert_eq!(budget().threshold(), 100_000);
        assert_eq!(small_abs.threshold(), 50_000);
        assert_eq!(
            ContextBudget {
                window: 100_000,
                ..budget()
            }
            .threshold(),
            60_000
        );
    }

    #[test]
    fn threshold_percent_over_hundred_is_clamped() {
        let over = ContextBudget {
            t_abs: u64::MAX,
            safety_percent: 250,
            ..budget()
        };

        assert_eq!(over.threshold(), 200_000);
    }

    #[test]
    fn hard_limit_without_growth_uses_tenth_of_window() {
        assert_eq!(budget().hard_limit(None), 120_000);
    }

    #[test]
    fn hard_limit_with_growth_adds_p95() {
        assert_eq!(budget().hard_limit(Some(7_000)), 107_000);
    }

    #[test]
    fn packet_limit_is_tenth_of_threshold() {
        assert_eq!(budget().packet_limit(), 10_000);
    }

    #[test]
    fn decide_follows_the_context_measure() {
        let expensive_write = ContextBudget {
            cache_write: 10.0,
            ..budget()
        };
        // (사례, 예산, 측정값, 예상 결정)
        let cases = [
            (
                "not tree idle defers",
                budget(),
                ContextMeasure {
                    tree_idle: false,
                    ..measure(150_000, 5_000)
                },
                CompactionDecision::Defer,
            ),
            (
                "mergeable queue defers",
                budget(),
                ContextMeasure {
                    has_mergeable_queue: true,
                    ..measure(150_000, 5_000)
                },
                CompactionDecision::Defer,
            ),
            (
                "unmeasured active continues",
                budget(),
                ContextMeasure {
                    active: None,
                    since_last_turn: Duration::from_secs(3_600),
                    ..measure(0, 5_000)
                },
                CompactionDecision::Continue,
            ),
            (
                "cache expired and packet smaller restarts",
                budget(),
                ContextMeasure {
                    since_last_turn: Duration::from_secs(301),
                    ..measure(20_000, 5_000)
                },
                CompactionDecision::Restart,
            ),
            (
                "cache expired but packet larger continues",
                budget(),
                ContextMeasure {
                    since_last_turn: Duration::from_secs(301),
                    ..measure(4_000, 5_000)
                },
                CompactionDecision::Continue,
            ),
            (
                "below threshold continues",
                budget(),
                measure(99_999, 5_000),
                CompactionDecision::Continue,
            ),
            (
                // k* = (5000×1.25 − 100000×0.1) / (95000×0.1) < 0 → 0 ≤ 3
                "at threshold with break even within turns restarts",
                budget(),
                measure(100_000, 5_000),
                CompactionDecision::Restart,
            ),
            (
                // k* = (9000×10 − 100000×0.1) / (91000×0.1) ≈ 8.8 > 3
                "at threshold with break even beyond turns continues",
                expensive_write,
                measure(100_000, 9_000),
                CompactionDecision::Continue,
            ),
            (
                "expected turns overrides default",
                expensive_write,
                ContextMeasure {
                    expected_turns: Some(10),
                    ..measure(100_000, 9_000)
                },
                CompactionDecision::Restart,
            ),
        ];

        for (name, budget, measure, expected) in cases {
            assert_eq!(decide(&budget, &measure), expected, "{name}");
        }
    }

    #[test]
    fn break_even_turns_matches_formula() {
        let turns = break_even_turns(
            &ContextBudget {
                cache_write: 10.0,
                ..budget()
            },
            100_000,
            9_000,
        );

        let expected = (9_000.0 * 10.0 - 100_000.0 * 0.1) / (91_000.0 * 0.1);
        assert!((turns - expected).abs() < 1e-9);
    }

    #[test]
    fn break_even_turns_packet_not_smaller_is_infinite() {
        assert!(break_even_turns(&budget(), 5_000, 5_000).is_infinite());
        assert!(break_even_turns(&budget(), 4_000, 5_000).is_infinite());
    }

    #[test]
    fn break_even_turns_negative_is_zero() {
        assert_eq!(break_even_turns(&budget(), 100_000, 1_000), 0.0);
    }

    #[test]
    fn packet_hard_limit_is_a_fifth_of_threshold() {
        assert_eq!(budget().packet_hard_limit(), 20_000);
    }

    #[test]
    fn packet_hard_limit_follows_the_percent() {
        let quarter = ContextBudget {
            packet_hard_percent: 25,
            ..budget()
        };
        assert_eq!(quarter.packet_hard_limit(), 25_000);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    #[test]
    fn decide_return_matches_rule_table() {
        let warm = Duration::from_secs(300);
        let cold = Duration::from_secs(301);
        // (경과 시간, A, P, 기대 판정)
        let cases = [
            (warm, 99_999, 5_000, ReturnDecision::Resume),
            (warm, 99_999, 200_000, ReturnDecision::Resume),
            (warm, 100_000, 5_000, ReturnDecision::Resume),
            (warm, 900_000, 5_000, ReturnDecision::Resume),
            (cold, 20_000, 5_000, ReturnDecision::NewSession),
            (cold, 150_000, 5_000, ReturnDecision::NewSession),
            (cold, 5_000, 5_000, ReturnDecision::Resume),
            (cold, 4_000, 5_000, ReturnDecision::Resume),
        ];

        for (since, active, packet, expected) in cases {
            let result = decide_return(&budget(), since, active, packet);

            assert_eq!(result, expected, "since={since:?} A={active} P={packet}");
        }
    }
}
