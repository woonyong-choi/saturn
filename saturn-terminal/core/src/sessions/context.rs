//! compaction 판정과 패킷 구성.
//! 설계: docs/design/context-management.md

use std::path::Path;
use std::time::Duration;

use saturn_protocol::ids::LedgerSeq;

/// 기대 잔여 턴을 모를 때 쓰는 값.
pub const DEFAULT_EXPECTED_TURNS: u32 = 3;

/// 도구 결과에서 남기는 앞부분 글자 수.
pub const TOOL_RESULT_CHARS: usize = 300;

// 초안
const CHARS_PER_TOKEN: usize = 4;

/// provider가 스스로 읽으므로 패킷에 넣지 않는다.
const PROVIDER_DOCS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// TODO(#49): 설정 키 이름과 기본값
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
    /// 마지막 턴 뒤 이만큼 지나면 유휴 복귀 조건을 본다.
    pub cache_ttl: Duration,
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
}

#[derive(Debug, Clone, Copy)]
pub struct ContextMeasure {
    /// 잴 수 없으면 `None`이고 provider 자동 압축에 맡긴다.
    /// TODO(#62): 루트 메시지로만 계산할지
    pub active: Option<u64>,
    pub packet: u64,
    pub tree_idle: bool,
    pub has_mergeable_queue: bool,
    pub since_last_turn: Duration,
    /// 모르면 `DEFAULT_EXPECTED_TURNS`로 본다.
    pub expected_turns: Option<u32>,
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

/// provider 요약은 정본이 아니므로 모두 Saturn 기록 원문에서 고른다.
#[derive(Debug, Clone, Default)]
pub struct PacketSource {
    /// 사용자가 명시한 제약과 결정 원문.
    pub pinned: Vec<String>,
    pub goal_and_last_input: Vec<String>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<String>,
    /// 최근 3턴.
    pub recent_turns: Vec<String>,
    /// `(결과 원문, 한 줄 메모)`이고 `compact` 질문이 남기라고 한 것만 들어온다.
    pub kept_tool_calls: Vec<(String, String)>,
    /// 파일은 내용 대신 경로만.
    pub file_paths: Vec<String>,
    pub up_to: LedgerSeq,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub text: String,
    /// 추정치.
    pub tokens: u64,
    /// 새 session의 `delivered`가 된다.
    pub up_to: LedgerSeq,
}

// cost: time O(m·L), heap O(L), stack O(1)
// vars: m = 패킷 항목 수, L = 패킷 원문 글자 수
// basis: estimate
// alt: 항목별 토큰 수를 한 번 세어 두고 빼기. time O(L). 잃는 것: 절 머리 줄 계산이 따로 필요
/// `max_tokens`를 넘으면 뒤 절의 끝 항목부터 줄이고, 고정 항목도 예외가 아니다.
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

#[derive(Debug, Clone)]
struct Section {
    title: &'static str,
    items: Vec<String>,
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수
// basis: estimate
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
