//! compaction 판정과 패킷 구성.
//!
//! 설계: docs/design/context-management.md. 기호:
//! - `A`: 턴 끝마다 잰 활성 맥락 크기(토큰)
//! - `T = min(T_abs, N% × 창 크기)`: 발동 기준. `T_abs`는 provider별 절대 토큰, `N`은 안전 비율
//! - `P`: 새 session에 넘길 패킷 크기, `P_max = T / 10`
//! - `r`, `w`: 캐시 읽기 배수, 캐시 쓰기 배수
//! - `k* = (P×w − A×r) / ((A − P) × r)`: 새 session으로 옮기는 비용과 그대로 잇는 비용이 같아지는 턴 수
//! - `H`: 턴당 증가량의 p95(관측 전에는 창의 10%), `T_hard = T + H`: provider 자동 압축 안전망

use std::path::Path;
use std::time::Duration;

use saturn_protocol::ids::LedgerSeq;

/// 기대 잔여 턴을 모를 때 쓰는 값.
pub const DEFAULT_EXPECTED_TURNS: u32 = 3;

/// 도구 결과에서 남기는 앞부분 글자 수.
pub const TOOL_RESULT_CHARS: usize = 300;

/// 토큰 수 추정에 쓰는 글자 수. 초안: 한 토큰을 네 글자로 본다.
const CHARS_PER_TOKEN: usize = 4;

/// provider가 스스로 읽는 문서. 패킷에 넣지 않는다.
const PROVIDER_DOCS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// provider별 기준값. 설정에서 읽는다. TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    /// provider별 절대 토큰 `T_abs`.
    pub t_abs: u64,
    /// 안전 비율 `N`(0~100).
    pub safety_percent: u8,
    /// 모델 맥락 창 크기.
    pub window: u64,
    /// 캐시 읽기 배수 `r`, 쓰기 배수 `w`.
    pub cache_read: f64,
    /// 캐시 쓰기 배수.
    pub cache_write: f64,
    /// provider 캐시 유지 시간. 마지막 턴 뒤 이만큼 지나면 유휴 복귀 조건을 본다.
    pub cache_ttl: Duration,
}

impl ContextBudget {
    /// 발동 기준 `T`. `N`이 100을 넘으면 100으로 본다.
    pub fn threshold(&self) -> u64 {
        let percent = u128::from(self.safety_percent.min(100));
        let by_window = u128::from(self.window) * percent / 100;
        let by_window = u64::try_from(by_window).expect("window share should fit in u64");
        self.t_abs.min(by_window)
    }

    /// provider 자동 압축에 넘길 안전망 `T_hard = T + H`. 사용자가 provider 설정에 값을 정해 두면 넣지 않는다.
    /// `H`를 관측하기 전(`None`)에는 창 크기의 10%를 쓴다.
    pub fn hard_limit(&self, p95_growth: Option<u64>) -> u64 {
        let growth = p95_growth.unwrap_or(self.window / 10);
        self.threshold().saturating_add(growth)
    }

    /// 패킷 크기 상한 `P_max = T / 10`.
    pub fn packet_limit(&self) -> u64 {
        self.threshold() / 10
    }
}

/// 턴 끝에 판정에 쓰는 값.
#[derive(Debug, Clone, Copy)]
pub struct ContextMeasure {
    /// `A`. 잴 수 없으면 `None`이고 Saturn 재시작 없이 provider 자동 압축에 맡긴다. TODO(#62): 루트 메시지로만 계산할지
    pub active: Option<u64>,
    /// 예상 패킷 크기 `P`.
    pub packet: u64,
    /// 트리 유휴인지.
    pub tree_idle: bool,
    /// A에 합칠 대기 입력이 있는지.
    pub has_mergeable_queue: bool,
    /// 마지막 턴 뒤 지난 시간.
    pub since_last_turn: Duration,
    /// 기대 잔여 턴. 모르면 `None`(3으로 본다).
    pub expected_turns: Option<u32>,
}

/// compaction 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionDecision {
    /// 그대로 계속한다.
    Continue,
    /// 다음 턴 경계까지 미룬다(트리 유휴가 아니거나 합칠 대기 입력이 있음).
    Defer,
    /// 새 session을 열고 패킷을 넘긴다.
    Restart,
}

/// 판정 순서: 유휴 아님이나 합칠 입력 → `Defer`, 캐시 만료이고 `P < A` → `Restart`, `A < T` → `Continue`,
/// `A ≥ T`이고 `k*` ≤ 기대 잔여 턴 → `Restart`, 그 밖 → `Continue`.
/// `A`를 잴 수 없으면 새 session을 열지 않고 `Continue`(provider 자동 압축에 맡긴다).
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

/// 본전 턴 수 `k*`. 0보다 작으면 0(바로 옮기는 쪽이 이득)이다.
/// `A ≤ P`이거나 `r ≤ 0`이면 옮겨도 턴당 비용이 줄지 않으므로 무한대다.
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

/// 패킷 재료. 모두 Saturn 기록 원문에서 고른다(provider 요약은 정본이 아니다).
#[derive(Debug, Clone, Default)]
pub struct PacketSource {
    /// 사용자가 명시한 제약과 결정 원문. 항상 넣는다.
    pub pinned: Vec<String>,
    /// 현재 목표와 마지막 사용자 입력 원문.
    pub goal_and_last_input: Vec<String>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<String>,
    /// 최근 3턴 원문.
    pub recent_turns: Vec<String>,
    /// 그 이전 도구 호출 `(결과 원문, 한 줄 메모)`. `compact` 질문이 남기라고 한 것만 들어온다.
    /// 결과는 앞 300자와 한 줄 메모로 줄인다.
    pub kept_tool_calls: Vec<(String, String)>,
    /// 파일은 내용 대신 경로만.
    pub file_paths: Vec<String>,
    /// 이 패킷이 포함하는 마지막 기록 번호.
    pub up_to: LedgerSeq,
}

/// 새 session에 넘길 패킷.
#[derive(Debug, Clone)]
pub struct Packet {
    /// 본문.
    pub text: String,
    /// 토큰 수 추정.
    pub tokens: u64,
    /// 포함한 마지막 기록 번호. 새 session의 `delivered`가 된다.
    pub up_to: LedgerSeq,
}

// cost: time O(m·L), heap O(L), stack O(1)
// vars: m = 패킷 항목 수, L = 패킷 원문 글자 수
// basis: estimate
// alt: 항목별 토큰 수를 한 번 세어 두고 빼기. time O(L). 잃는 것: 절 머리 줄 계산이 따로 필요
/// 패킷을 위 순서대로 만들고 `P_max`를 넘으면 뒤쪽 항목부터 줄인다.
/// provider가 스스로 읽는 문서(`AGENTS.md`, `CLAUDE.md`)는 넣지 않는다.
///
/// 줄이는 순서는 파일 경로, 도구 호출, 최근 턴, 미완 항목, 목표, 고정 항목이고, 절 안에서는 끝 항목부터다.
/// 마지막으로 줄이는 항목은 남은 예산만큼 앞부분을 남긴다. 고정 항목도 상한을 넘으면 줄인다.
/// 토큰 수는 네 글자를 한 토큰으로 추정한다.
pub fn build_packet(source: &PacketSource, max_tokens: u64) -> Packet {
    let mut sections = packet_sections(source);
    let mut text = render(&sections);
    let mut tokens = estimate_tokens(&text);
    while tokens > max_tokens {
        let Some(index) = sections
            .iter()
            .rposition(|section| !section.items.is_empty())
        else {
            break;
        };
        shrink_last_item(&mut sections[index], tokens - max_tokens);
        text = render(&sections);
        tokens = estimate_tokens(&text);
    }
    Packet {
        text,
        tokens,
        up_to: source.up_to,
    }
}

/// 패킷의 절 하나.
#[derive(Debug, Clone)]
struct Section {
    title: &'static str,
    items: Vec<String>,
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수
// basis: estimate
/// 패킷 재료를 넣는 순서대로 절로 나눈다. 도구 결과는 앞 300자와 한 줄 메모로 줄이고,
/// provider가 스스로 읽는 문서의 경로는 뺀다.
fn packet_sections(source: &PacketSource) -> Vec<Section> {
    let tool_calls = source
        .kept_tool_calls
        .iter()
        .map(|(result, memo)| {
            let memo = memo.lines().next().unwrap_or_default();
            let head: String = result.chars().take(TOOL_RESULT_CHARS).collect();
            format!("{memo}\n{head}")
        })
        .collect();
    let file_paths = source
        .file_paths
        .iter()
        .filter(|path| !is_provider_doc(path))
        .cloned()
        .collect();
    vec![
        Section {
            title: "Constraints and decisions",
            items: source.pinned.clone(),
        },
        Section {
            title: "Goal and last input",
            items: source.goal_and_last_input.clone(),
        },
        Section {
            title: "Open items",
            items: source.open_items.clone(),
        },
        Section {
            title: "Recent turns",
            items: source.recent_turns.clone(),
        },
        Section {
            title: "Earlier tool calls",
            items: tool_calls,
        },
        Section {
            title: "Files",
            items: file_paths,
        },
    ]
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 패킷 원문 글자 수
// basis: estimate
/// 빈 절은 빼고 `## 제목`과 항목을 빈 줄로 나눠 잇는다.
fn render(sections: &[Section]) -> String {
    let mut text = String::new();
    for section in sections.iter().filter(|section| !section.items.is_empty()) {
        text.push_str("## ");
        text.push_str(section.title);
        text.push_str("\n\n");
        for item in &section.items {
            text.push_str(item);
            text.push_str("\n\n");
        }
    }
    text
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 끝 항목 글자 수
// basis: estimate
/// 끝 항목을 넘친 토큰만큼 줄인다. 다 줄여야 하면 뺀다.
fn shrink_last_item(section: &mut Section, excess_tokens: u64) {
    let Some(item) = section.items.last_mut() else {
        return;
    };
    let excess_chars = usize::try_from(excess_tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(CHARS_PER_TOKEN);
    let item_chars = item.chars().count();
    if item_chars <= excess_chars {
        section.items.pop();
        return;
    }
    *item = item.chars().take(item_chars - excess_chars).collect();
}

// cost: time O(L), heap O(1), stack O(1)
// vars: L = 글자 수
// basis: estimate
/// 토큰 수 추정. 초안: 네 글자를 한 토큰으로 보고 올림한다.
fn estimate_tokens(text: &str) -> u64 {
    let tokens = text.chars().count().div_ceil(CHARS_PER_TOKEN);
    u64::try_from(tokens).expect("token count should fit in u64")
}

// cost: time O(l), heap O(1), stack O(1)
// vars: l = 경로 글자 수
// basis: estimate
fn is_provider_doc(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| PROVIDER_DOCS.contains(&name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ContextBudget {
        ContextBudget {
            t_abs: 100_000,
            safety_percent: 60,
            window: 200_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
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

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 원문 글자 수
    // basis: estimate
    fn words(count: usize, word: &str) -> String {
        vec![word; count].join(" ")
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
    fn decide_not_tree_idle_defers() {
        let busy = ContextMeasure {
            tree_idle: false,
            ..measure(150_000, 5_000)
        };

        assert_eq!(decide(&budget(), &busy), CompactionDecision::Defer);
    }

    #[test]
    fn decide_mergeable_queue_defers() {
        let queued = ContextMeasure {
            has_mergeable_queue: true,
            ..measure(150_000, 5_000)
        };

        assert_eq!(decide(&budget(), &queued), CompactionDecision::Defer);
    }

    #[test]
    fn decide_unmeasured_active_continues() {
        let unknown = ContextMeasure {
            active: None,
            since_last_turn: Duration::from_secs(3_600),
            ..measure(0, 5_000)
        };

        assert_eq!(decide(&budget(), &unknown), CompactionDecision::Continue);
    }

    #[test]
    fn decide_cache_expired_and_packet_smaller_restarts() {
        let idle = ContextMeasure {
            since_last_turn: Duration::from_secs(301),
            ..measure(20_000, 5_000)
        };

        assert_eq!(decide(&budget(), &idle), CompactionDecision::Restart);
    }

    #[test]
    fn decide_cache_expired_but_packet_larger_continues() {
        let idle = ContextMeasure {
            since_last_turn: Duration::from_secs(301),
            ..measure(4_000, 5_000)
        };

        assert_eq!(decide(&budget(), &idle), CompactionDecision::Continue);
    }

    #[test]
    fn decide_below_threshold_continues() {
        assert_eq!(
            decide(&budget(), &measure(99_999, 5_000)),
            CompactionDecision::Continue
        );
    }

    #[test]
    fn decide_at_threshold_with_break_even_within_turns_restarts() {
        // k* = (5000×1.25 − 100000×0.1) / (95000×0.1) < 0 → 0 ≤ 3
        assert_eq!(
            decide(&budget(), &measure(100_000, 5_000)),
            CompactionDecision::Restart
        );
    }

    #[test]
    fn decide_at_threshold_with_break_even_beyond_turns_continues() {
        let expensive_write = ContextBudget {
            cache_write: 10.0,
            ..budget()
        };
        // k* = (9000×10 − 100000×0.1) / (91000×0.1) ≈ 8.8 > 3
        let result = decide(&expensive_write, &measure(100_000, 9_000));

        assert_eq!(result, CompactionDecision::Continue);
    }

    #[test]
    fn decide_expected_turns_overrides_default() {
        let expensive_write = ContextBudget {
            cache_write: 10.0,
            ..budget()
        };
        let long_run = ContextMeasure {
            expected_turns: Some(10),
            ..measure(100_000, 9_000)
        };

        assert_eq!(
            decide(&expensive_write, &long_run),
            CompactionDecision::Restart
        );
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

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 원문 글자 수
    // basis: estimate
    #[test]
    fn build_packet_puts_sections_in_order() {
        let source = PacketSource {
            pinned: vec!["keep api stable".into()],
            goal_and_last_input: vec!["add login".into()],
            open_items: vec!["tests pending".into()],
            recent_turns: vec!["turn 3".into()],
            kept_tool_calls: vec![("cargo test output".into(), "ran tests".into())],
            file_paths: vec!["src/login.rs".into()],
            up_to: LedgerSeq(42),
        };

        let packet = build_packet(&source, 10_000);

        let order = [
            "keep api stable",
            "add login",
            "tests pending",
            "turn 3",
            "ran tests",
            "src/login.rs",
        ]
        .map(|needle| packet.text.find(needle).expect("item should be in packet"));
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(packet.up_to, LedgerSeq(42));
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 원문 글자 수
    // basis: estimate
    #[test]
    fn build_packet_tool_result_keeps_head_and_one_line_memo() {
        let source = PacketSource {
            kept_tool_calls: vec![("x".repeat(1_000), "first line\nsecond line".into())],
            ..PacketSource::default()
        };

        let packet = build_packet(&source, 10_000);

        assert!(
            packet
                .text
                .contains(&format!("first line\n{}", "x".repeat(300)))
        );
        assert!(!packet.text.contains(&"x".repeat(301)));
        assert!(!packet.text.contains("second line"));
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 원문 글자 수
    // basis: estimate
    #[test]
    fn build_packet_skips_provider_docs() {
        let source = PacketSource {
            file_paths: vec![
                "AGENTS.md".into(),
                "docs/CLAUDE.md".into(),
                "src/a.rs".into(),
            ],
            ..PacketSource::default()
        };

        let packet = build_packet(&source, 10_000);

        assert!(packet.text.contains("src/a.rs"));
        assert!(!packet.text.contains("AGENTS.md"));
        assert!(!packet.text.contains("CLAUDE.md"));
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 원문 글자 수
    // basis: estimate
    #[test]
    fn build_packet_over_limit_shrinks_from_the_back() {
        let source = PacketSource {
            pinned: vec!["PINNED".into()],
            goal_and_last_input: vec!["GOAL".into()],
            recent_turns: vec![words(200, "turn")],
            file_paths: (0..100).map(|n| format!("src/file_{n}.rs")).collect(),
            ..PacketSource::default()
        };

        let packet = build_packet(&source, 300);

        assert!(packet.tokens <= 300);
        assert_eq!(packet.tokens, estimate_tokens(&packet.text));
        assert!(packet.text.contains("PINNED"));
        assert!(packet.text.contains("GOAL"));
        assert!(packet.text.contains("turn turn"));
        assert!(packet.text.contains("src/file_0.rs"));
        assert!(!packet.text.contains("src/file_99.rs"));
    }

    #[test]
    fn build_packet_tiny_limit_still_fits() {
        let source = PacketSource {
            pinned: vec![words(100, "rule")],
            goal_and_last_input: vec![words(100, "goal")],
            ..PacketSource::default()
        };

        let packet = build_packet(&source, 5);

        assert!(packet.tokens <= 5);
    }

    #[test]
    fn build_packet_zero_limit_is_empty() {
        let source = PacketSource {
            pinned: vec!["rule".into()],
            ..PacketSource::default()
        };

        let packet = build_packet(&source, 0);

        assert_eq!(packet.text, "");
        assert_eq!(packet.tokens, 0);
    }
}
