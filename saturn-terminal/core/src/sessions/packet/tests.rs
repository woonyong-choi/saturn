use std::time::Duration;

use super::*;
use crate::sessions::context::{DEFAULT_ITEM_CAP_PERCENT, DEFAULT_PACKET_HARD_PERCENT};
use crate::sessions::ranking::{Candidate, DEFAULT_RRF_K, order_after_router, rank_candidates};

// 2026-09-12T10:00Z
const AT_MS: i64 = 1_789_207_200_000;

// 지시문이 먹는 토큰. 지시문을 뺀 기록 예산이 T = 4_000일 때와 같도록 T에 더한다.
fn instruction_tokens() -> u64 {
    estimate_tokens(&format!("{INSTRUCTION}{ITEM_SEPARATOR}"))
}

// 기록 몫이 T = 4_000 → P_max = 400 토큰(1_600자), P_hard = 800 토큰(3_200자)일 때와 같다.
fn budget() -> ContextBudget {
    ContextBudget {
        t_abs: 4_000 + 10 * instruction_tokens(),
        safety_percent: 100,
        window: 1_000_000,
        cache_read: 0.1,
        cache_write: 1.25,
        cache_ttl: Duration::from_secs(300),
        packet_hard_percent: DEFAULT_PACKET_HARD_PERCENT,
        item_cap_percent: DEFAULT_ITEM_CAP_PERCENT,
        rrf_k: DEFAULT_RRF_K,
    }
}

fn entry(seq: u64, text: &str) -> Entry {
    Entry {
        seq: LedgerSeq(seq),
        text: text.into(),
    }
}

fn stamp(session: u64, at_ms: i64) -> Stamp {
    Stamp {
        session: SessionId(session),
        at_ms: Some(at_ms),
    }
}

fn turn(seq: u64, input: &str, answer: &str) -> RecentTurn {
    RecentTurn {
        seq: LedgerSeq(seq),
        stamp: stamp(1, AT_MS),
        status: TurnStatus::Finished,
        input: input.into(),
        answer: answer.into(),
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
fn item(seq: u64, text: &str, path: Option<&str>) -> CompetingItem {
    CompetingItem {
        seq: LedgerSeq(seq),
        stamp: stamp(1, AT_MS),
        text: text.into(),
        memo: format!("memo {seq}"),
        path: path.map(str::to_string),
    }
}

/// 맨 앞의 지시문을 뺀 기록 부분.
fn records(packet: &Packet) -> &str {
    packet
        .text
        .strip_prefix(&format!("{INSTRUCTION}{ITEM_SEPARATOR}"))
        .expect("packet should start with the instruction")
}

fn ready(outcome: PacketOutcome) -> Packet {
    match outcome {
        PacketOutcome::Ready(packet) => packet,
        PacketOutcome::Deferred { .. } => panic!("packet should be ready"),
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 글자 수
// basis: estimate
fn filler(prefix: &str, chars: usize) -> String {
    let mut text = prefix.to_string();
    text.extend(std::iter::repeat_n('.', chars - prefix.chars().count()));
    text
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_fixed_zone_in_order_and_tool_results_only_in_competing() {
    let source = PacketSource {
        constraints: vec![entry(3, "keep api stable")],
        goal_and_last_input: vec![entry(40, "fix login message")],
        open_items: vec![entry(38, "tests pending")],
        recent_turns: vec![turn(39, "run tests", "ran them")],
        competitors: vec![item(37, "cargo test output", None)],
        provider_docs: Vec::new(),
        up_to: LedgerSeq(42),
    };

    let packet = ready(build_packet(&source, &budget()));

    let order = [
        "keep api stable",
        "fix login message",
        "tests pending",
        "User: run tests\nAgent: ran them",
        "## Earlier records",
        "cargo test output",
    ]
    .map(|needle| packet.text.find(needle).expect("item should be in packet"));
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(packet.up_to, LedgerSeq(42));
    assert!(!packet.is_over_limit);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_fills_competing_in_chosen_order_raw_then_digest() {
    let source = PacketSource {
        competitors: vec![
            item(10, &filler("A", 380), None),
            item(11, &filler("B", 380), None),
            item(12, &filler("C", 380), None),
            item(13, &filler("D", 380), None),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    for name in ["A", "B", "C"] {
        assert!(packet.text.contains(&filler(name, 380)));
    }
    assert!(
        packet
            .text
            .contains(&format!("memo 13\n{}", filler("D", 300)))
    );
    assert!(!packet.text.contains(&filler("D", 301)));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_small_top_item_goes_raw_before_large_lower_item() {
    let source = PacketSource {
        competitors: vec![
            item(20, &filler("small", 400), None),
            item(10, &filler("large", 470), None),
            item(11, &filler("large2", 470), None),
            item(12, &filler("large3", 470), None),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.contains(&filler("small", 400)));
    assert!(!packet.text.contains(&filler("large3", 470)));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_item_over_cap_goes_as_digest() {
    let source = PacketSource {
        competitors: vec![item(5, &filler("log", 1_000), None)],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(
        packet
            .text
            .contains(&format!("memo 5\n{}", filler("log", 300)))
    );
    assert!(!packet.text.contains(&filler("log", 301)));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_falls_back_to_path_then_skips() {
    let source = PacketSource {
        competitors: vec![
            item(1, &filler("A", 450), None),
            item(2, &filler("B", 450), None),
            item(3, &filler("C", 450), None),
            item(4, &filler("D", 400), Some("src/d.rs")),
            item(5, &filler("E", 400), None),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.contains("src/d.rs\n\n"));
    assert!(!packet.text.contains("memo 4"));
    assert!(!packet.text.contains("E."));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_competing_groups_by_session_with_seq_and_time() {
    let mut first = item(12, "twelve", None);
    first.stamp = stamp(2, AT_MS + 120_000);
    let mut second = item(10, "ten", None);
    second.stamp = stamp(1, AT_MS);
    let mut third = item(11, "eleven", None);
    third.stamp = stamp(1, AT_MS + 60_000);
    let source = PacketSource {
        competitors: vec![first, second, third],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert_eq!(
        records(&packet),
        "## Earlier records\n\n### Session 1\n\n#10 2026-09-12T10:00Z ten\n\n\
         #11 2026-09-12T10:01Z eleven\n\n### Session 2\n\n#12 2026-09-12T10:02Z twelve\n\n"
    );
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_recent_turns_carry_session_title_seq_and_time() {
    let mut later = turn(5, "next", "ok");
    later.stamp = stamp(2, AT_MS + 3_600_000);
    let source = PacketSource {
        recent_turns: vec![later, turn(3, "run tests", "ran them")],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert_eq!(
        records(&packet),
        "## Recent turns\n\n### Session 1\n\n#3 2026-09-12T10:00Z [Finished] User: run tests\nAgent: ran them\n\n\
         ### Session 2\n\n#5 2026-09-12T11:00Z [Finished] User: next\nAgent: ok\n\n"
    );
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_many_sessions_stay_within_packet_limit() {
    let source = PacketSource {
        competitors: (10..200)
            .map(|seq| {
                let mut item = item(seq, &filler("x", 2_000), Some("src/x.rs"));
                item.stamp = stamp(seq, AT_MS);
                item
            })
            .collect(),
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.tokens <= budget().packet_limit());
    assert!(packet.text.contains("### Session 10\n\n#10 "));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_without_time_writes_seq_only() {
    let mut undated = item(7, "seven", None);
    undated.stamp = Stamp {
        session: SessionId(1),
        at_ms: None,
    };
    let source = PacketSource {
        competitors: vec![undated],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.contains("### Session 1\n\n#7 seven\n\n"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_writes_competing_in_seq_order() {
    let source = PacketSource {
        competitors: vec![
            item(9, "NINE", None),
            item(3, "THREE", None),
            item(5, "FIVE", None),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    let order = ["THREE", "FIVE", "NINE"]
        .map(|needle| packet.text.find(needle).expect("item should be in packet"));
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_large_record_stays_within_packet_limit() {
    let source = PacketSource {
        constraints: vec![entry(1, "rule")],
        recent_turns: vec![turn(2, "go", "done")],
        competitors: (10..200)
            .map(|seq| item(seq, &filler("x", 2_000), Some("src/x.rs")))
            .collect(),
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.tokens <= budget().packet_limit());
    assert_eq!(packet.tokens, estimate_tokens(&packet.text));
    assert!(!packet.is_over_limit);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_fixed_overflow_trims_oldest_answer_first() {
    let source = PacketSource {
        recent_turns: vec![
            turn(1, "q1", &filler("a", 560)),
            turn(2, "q2", &filler("b", 560)),
            turn(3, "q3", &filler("c", 560)),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.contains(&filler("a", 300)));
    assert!(!packet.text.contains(&filler("a", 301)));
    assert!(packet.text.contains(&filler("b", 560)));
    assert!(packet.text.contains(&filler("c", 560)));
    assert!(!packet.is_over_limit);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_fixed_overflow_drops_oldest_turns() {
    let source = PacketSource {
        recent_turns: vec![
            turn(1, &filler("x", 700), "ok"),
            turn(2, &filler("y", 700), "ok"),
            turn(3, &filler("z", 700), "ok"),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(!packet.text.contains("x."));
    assert!(packet.text.contains(&filler("y", 700)));
    assert!(packet.text.contains(&filler("z", 700)));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_fixed_over_limit_allows_hard_limit_without_competing() {
    let source = PacketSource {
        constraints: vec![entry(1, &filler("rule", 2_000))],
        competitors: vec![item(5, "tool output", None)],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.is_over_limit);
    assert!(packet.tokens <= budget().packet_hard_limit());
    assert!(packet.text.contains(&filler("rule", 2_000)));
    assert!(!packet.text.contains("tool output"));
}

#[test]
fn build_packet_fixed_over_hard_limit_defers_with_constraints() {
    let long_rule = filler(
        "rule",
        3_500 + 8 * usize::try_from(instruction_tokens()).unwrap(),
    );
    let source = PacketSource {
        constraints: vec![entry(2, &long_rule), entry(1, "first rule")],
        ..PacketSource::default()
    };

    let outcome = build_packet(&source, &budget());

    let PacketOutcome::Deferred { constraints } = outcome else {
        panic!("packet should be deferred");
    };
    assert_eq!(constraints, vec!["first rule".to_string(), long_rule]);
}

#[test]
fn reduce_packet_drops_the_lowest_items_and_keeps_the_fixed_zone() {
    let source = PacketSource {
        goal_and_last_input: vec![entry(40, "fix login message")],
        competitors: vec![
            item(1, &filler("high", 300), None),
            item(2, &filler("mid", 300), None),
            item(3, &filler("low", 300), None),
        ],
        ..PacketSource::default()
    };
    let full = ready(build_packet(&source, &budget()));

    let target = instruction_tokens() + (full.tokens - instruction_tokens()) / 2;

    let reduced = reduce_packet(&source, &budget(), target).expect("room for the fixed zone");

    assert!(reduced.tokens <= target);
    assert!(reduced.text.contains("fix login message"));
    assert!(reduced.text.contains(&filler("high", 300)));
    assert!(!reduced.text.contains(&filler("low", 300)));
    assert_eq!(reduced.included, vec![LedgerSeq(1)]);
}

#[test]
fn reduce_packet_is_none_when_the_fixed_zone_alone_is_over_the_target() {
    let source = PacketSource {
        goal_and_last_input: vec![entry(40, &filler("goal", 800))],
        competitors: vec![item(1, "tool output", None)],
        ..PacketSource::default()
    };

    assert!(reduce_packet(&source, &budget(), 100).is_none());
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_skips_provider_docs() {
    let source = PacketSource {
        provider_docs: vec!["AGENTS.md".to_owned(), "CLAUDE.md".to_owned()],
        competitors: vec![
            item(1, "agents doc body", Some("AGENTS.md")),
            item(2, "claude doc body", Some("docs/CLAUDE.md")),
            item(3, "source body", Some("src/a.rs")),
        ],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.contains("source body"));
    assert!(!packet.text.contains("doc body"));
}

#[test]
fn build_packet_empty_source_is_empty() {
    let packet = ready(build_packet(&PacketSource::default(), &budget()));

    assert_eq!(records(&packet), "");
    assert_eq!(packet.tokens, instruction_tokens());
}

// cost: time O(c log c + c·l), heap O(c·l), stack O(1)
// vars: c = 후보 수, l = 항목 글자 수
// basis: estimate
#[test]
fn build_packet_router_no_response_fills_in_rrf_order() {
    let mut candidates: Vec<Candidate> = (0..150)
        .map(|seq| Candidate {
            seq: LedgerSeq(seq),
            text: format!("unrelated {seq}"),
            files: Vec::new(),
        })
        .collect();
    candidates[12].text = "/v2/auth 로그인 실패".into();
    candidates[12].files = vec!["src/auth/login.rs".into()];
    let ranked = rank_candidates(
        &candidates,
        &["src/auth/login.rs".into()],
        "로그인 실패 고쳐 줘",
        DEFAULT_RRF_K,
    );
    let ordered = order_after_router(&ranked, &[]);
    let source = PacketSource {
        competitors: ordered
            .iter()
            .map(|seq| item(seq.0, &filler(&format!("#{:03}", seq.0), 450), None))
            .collect(),
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    for seq in &ranked[..3] {
        assert!(packet.text.contains(&format!("#{:03}.", seq.0)));
    }
    assert!(!packet.text.contains(&format!("#{:03}.", ranked[3].0)));
    assert!(packet.text.contains("#012."));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_included_lists_competing_seqs_in_seq_order() {
    let source = PacketSource {
        competitors: vec![item(12, "twelve", None), item(10, "ten", None)],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert_eq!(packet.included, vec![LedgerSeq(10), LedgerSeq(12)]);
    assert!(!packet.is_summary_used);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_with_summary_puts_summary_first_in_competing_zone() {
    let source = PacketSource {
        competitors: vec![item(20, "after summary", None)],
        ..PacketSource::default()
    };

    let packet = ready(build_packet_with_summary(
        &source,
        &budget(),
        &entry(9, "summary body"),
    ));

    assert!(packet.is_summary_used);
    assert_eq!(packet.included, vec![LedgerSeq(9), LedgerSeq(20)]);
    assert!(packet.text.find("summary body") < packet.text.find("after summary"));
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 테스트 데이터 크기
// basis: estimate
#[test]
fn build_packet_with_summary_over_competing_budget_falls_back_to_records() {
    let source = PacketSource {
        competitors: vec![item(20, "after summary", None)],
        ..PacketSource::default()
    };

    let packet = ready(build_packet_with_summary(
        &source,
        &budget(),
        &entry(9, &filler("S", 1_600)),
    ));

    assert!(!packet.is_summary_used);
    assert_eq!(packet.included, vec![LedgerSeq(20)]);
    assert!(!packet.text.contains("SSS"));
}

#[test]
fn build_packet_starts_with_the_do_not_act_instruction() {
    let source = PacketSource {
        goal_and_last_input: vec![entry(1, "Input [Finished]: add two lines")],
        recent_turns: vec![turn(1, "add two lines", "done")],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    assert!(packet.text.starts_with(INSTRUCTION));
    assert!(INSTRUCTION.contains("do not call tools and do not change files"));
    assert!(INSTRUCTION.contains("wait for the next user input"));
}

#[test]
fn build_packet_recent_turn_states_and_unknown_result_format() {
    let mut stopped = turn(3, "run it", "started");
    stopped.status = TurnStatus::ResultUnknown;
    let mut running = turn(5, "keep going", "ok");
    running.status = TurnStatus::InProgress;
    let source = PacketSource {
        recent_turns: vec![turn(1, "add two lines", "done"), stopped, running],
        ..PacketSource::default()
    };

    let packet = ready(build_packet(&source, &budget()));

    let text = records(&packet);
    assert!(text.contains("[Finished] User: add two lines\nAgent: done"));
    assert!(text.contains(&format!(
        "[Result unknown] User: run it\nAgent: started\nResult (error): {INTERRUPTED_RESULT}"
    )));
    assert!(text.contains("[In progress] User: keep going"));
}
