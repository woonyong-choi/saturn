//! 새 session에 넘기는 패킷의 고정 구역과 경쟁 구역.
//! 설계: docs/design/context-management.md#패킷-구성

use std::collections::HashSet;
use std::path::Path;

use saturn_protocol::ids::{LedgerSeq, SessionId};

use super::context::ContextBudget;
use super::stamp::{Stamp, label, session_title};

/// 축약본과 줄인 에이전트 답에 남기는 앞부분 글자 수.
pub const DIGEST_HEAD_CHARS: usize = 300;

/// 고정 구역에 넣는 최근 턴 수의 최대.
pub const RECENT_TURNS: usize = 3;

// 초안
const ITEM_SHARE_PERCENT: usize = 30;

// 초안
const CHARS_PER_TOKEN: usize = 4;

/// provider가 스스로 읽으므로 패킷에 넣지 않는다.
const PROVIDER_DOCS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

const ITEM_SEPARATOR: &str = "\n\n";
const COMPETING_TITLE: &str = "Earlier records";

/// 기록 원문 한 덩어리.
#[derive(Debug, Clone)]
pub struct Entry {
    pub seq: LedgerSeq,
    pub text: String,
}

/// 도구 결과는 넣지 않는다.
#[derive(Debug, Clone)]
pub struct RecentTurn {
    pub seq: LedgerSeq,
    pub stamp: Stamp,
    pub input: String,
    pub answer: String,
}

/// 도구 호출과 결과, 다른 에이전트 결과 요약, 파일 경로.
#[derive(Debug, Clone)]
pub struct CompetingItem {
    pub seq: LedgerSeq,
    pub stamp: Stamp,
    /// 원문.
    pub text: String,
    /// 축약본 첫 줄. `memo::tool_memo`가 만든다.
    pub memo: String,
    /// 원문도 축약본도 들어가지 않을 때 넣는다.
    pub path: Option<String>,
}

/// provider 요약은 정본이 아니므로 모두 Saturn 기록 원문에서 고른다.
#[derive(Debug, Clone, Default)]
pub struct PacketSource {
    /// 대체된 제약은 뺀다.
    pub constraints: Vec<Entry>,
    pub goal_and_last_input: Vec<Entry>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<Entry>,
    /// `RECENT_TURNS`개보다 많으면 기록 번호가 큰 쪽만 쓴다.
    pub recent_turns: Vec<RecentTurn>,
    /// 맥락 고르기가 정한 순서.
    pub competitors: Vec<CompetingItem>,
    pub up_to: LedgerSeq,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub text: String,
    /// 추정치.
    pub tokens: u64,
    /// 새 session의 `delivered`가 된다.
    pub up_to: LedgerSeq,
    /// 고정 구역이 `P_max`를 넘어 `P_hard`까지 허용했다. engine이 초과를 기록한다.
    pub is_over_limit: bool,
    /// 경쟁 구역에 든 기록 번호. 기록 번호 순이고 provider 요약은 `up_to`가 아니라 요약 항목의 번호로 들어간다.
    pub included: Vec<LedgerSeq>,
    /// 요약이 경쟁 구역 예산에 들어가 첫 항목이 됐다. 요약을 주지 않았거나 예산을 넘어 원문으로 채웠으면 거짓.
    pub is_summary_used: bool,
}

#[derive(Debug, Clone)]
pub enum PacketOutcome {
    Ready(Packet),
    /// 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 않는다. TUI가 제약 목록을 보인다.
    Deferred {
        constraints: Vec<String>,
    },
}

#[derive(Debug, Clone)]
struct Section {
    title: &'static str,
    items: Vec<SectionItem>,
}

/// `session`이 있으면 같은 session의 이어진 항목 앞에 제목을 한 번 쓴다. 기록 번호 순이면 session도 이어진다.
#[derive(Debug, Clone)]
struct SectionItem {
    session: Option<SessionId>,
    text: String,
}

#[derive(Debug, Clone)]
struct Chosen {
    seq: LedgerSeq,
    session: Option<SessionId>,
    text: String,
}

// cost: time O(t·L + m log m), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
/// 고정 구역이 `P_max`를 넘으면 최근 턴을 줄이고, 그래도 넘으면 `P_hard`까지 허용하며 경쟁 구역은 비운다.
pub fn build_packet(source: &PacketSource, budget: &ContextBudget) -> PacketOutcome {
    build(source, budget, None)
}

// cost: time O(t·L + m log m), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
/// provider 압축 요약을 경쟁 구역 첫 항목으로 넣고 나머지는 `source.competitors`로 채운다. 호출하는 쪽이 요약 시점 뒤의 항목만 `competitors`에 둔다. 요약이 경쟁 구역 예산에 들어가지 않으면 요약 없이 `build_packet`과 같다.
pub fn build_packet_with_summary(
    source: &PacketSource,
    budget: &ContextBudget,
    summary: &Entry,
) -> PacketOutcome {
    build(source, budget, Some(summary))
}

// cost: time O(t·L + m log m), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
fn build(source: &PacketSource, budget: &ContextBudget, summary: Option<&Entry>) -> PacketOutcome {
    let soft_chars = to_chars(budget.packet_limit());
    let hard_chars = to_chars(budget.packet_hard_limit());
    let mut sections = fit_fixed_zone(source, soft_chars);
    let fixed_chars = render(&sections).chars().count();
    if fixed_chars > hard_chars {
        return PacketOutcome::Deferred {
            constraints: by_seq(&source.constraints)
                .into_iter()
                .map(|item| item.text)
                .collect(),
        };
    }
    let is_over_limit = fixed_chars > soft_chars;
    let header_chars = format!("## {COMPETING_TITLE}{ITEM_SEPARATOR}")
        .chars()
        .count();
    let competing_chars = soft_chars.saturating_sub(fixed_chars + header_chars);
    let summary = summary.filter(|entry| item_chars(&entry.text) <= competing_chars);
    let mut chosen: Vec<Chosen> = Vec::new();
    let mut rest_chars = competing_chars;
    if let Some(entry) = summary {
        rest_chars -= item_chars(&entry.text);
        chosen.push(Chosen {
            seq: entry.seq,
            session: None,
            text: entry.text.clone(),
        });
    }
    chosen.extend(fill_competing_zone(&source.competitors, rest_chars));
    sections.push(Section {
        title: COMPETING_TITLE,
        items: chosen
            .iter()
            .map(|item| SectionItem {
                session: item.session,
                text: item.text.clone(),
            })
            .collect(),
    });
    let text = render(&sections);
    let tokens = estimate_tokens(&text);
    PacketOutcome::Ready(Packet {
        text,
        tokens,
        up_to: source.up_to,
        is_over_limit,
        included: chosen.iter().map(|item| item.seq).collect(),
        is_summary_used: summary.is_some(),
    })
}

// cost: time O(t·L), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 고정 구역 글자 수
// basis: estimate
/// 오래된 턴부터 에이전트 답을 앞부분만 남기고, 그래도 넘치면 최근 턴 수를 3, 2, 1로 줄인다.
fn fit_fixed_zone(source: &PacketSource, soft_chars: usize) -> Vec<Section> {
    let mut turns = source.recent_turns.clone();
    turns.sort_by_key(|turn| turn.seq);
    let excess = turns.len().saturating_sub(RECENT_TURNS);
    turns.drain(..excess);
    let fits =
        |turns: &[RecentTurn]| render(&fixed_sections(source, turns)).chars().count() <= soft_chars;
    for index in 0..turns.len() {
        if fits(&turns) {
            return fixed_sections(source, &turns);
        }
        turns[index].answer = head(&turns[index].answer);
    }
    while turns.len() > 1 && !fits(&turns) {
        turns.remove(0);
    }
    fixed_sections(source, &turns)
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 고정 구역 글자 수
// basis: estimate
fn fixed_sections(source: &PacketSource, turns: &[RecentTurn]) -> Vec<Section> {
    let turns = turns
        .iter()
        .map(|turn| SectionItem {
            session: Some(turn.stamp.session),
            text: format!(
                "{} User: {}\nAgent: {}",
                label(turn.seq, turn.stamp.at_ms),
                turn.input,
                turn.answer
            ),
        })
        .collect();
    vec![
        Section {
            title: "Constraints and decisions",
            items: by_seq(&source.constraints),
        },
        Section {
            title: "Goal and last input",
            items: by_seq(&source.goal_and_last_input),
        },
        Section {
            title: "Open items",
            items: by_seq(&source.open_items),
        },
        Section {
            title: "Recent turns",
            items: turns,
        },
    ]
}

// cost: time O(L + m log m), heap O(L), stack O(1)
// vars: L = 경쟁 항목 글자 수, m = 경쟁 항목 수
// basis: estimate
/// 고른 순서대로 원문, 축약본, 경로 중 처음 들어가는 형태를 넣고 기록 번호 순으로 돌려준다.
/// 항목 앞의 기록 번호와 시각, session의 첫 항목이 쓰는 제목도 예산에 든다.
fn fill_competing_zone(items: &[CompetingItem], budget_chars: usize) -> Vec<Chosen> {
    let item_cap = budget_chars * ITEM_SHARE_PERCENT / 100;
    let mut remaining = budget_chars;
    let mut titled: HashSet<SessionId> = HashSet::new();
    let mut chosen: Vec<Chosen> = Vec::new();
    for item in items.iter().filter(|item| !is_provider_doc(item)) {
        let session = item.stamp.session;
        let title_chars = if titled.contains(&session) {
            0
        } else {
            item_chars(&session_title(session))
        };
        let prefix = label(item.seq, item.stamp.at_ms);
        let raw_chars = item.text.chars().count();
        let raw = (raw_chars <= item_cap).then(|| item.text.clone());
        let forms = [raw, Some(digest(item)), item.path.clone()];
        let Some(text) = forms
            .into_iter()
            .flatten()
            .map(|form| format!("{prefix} {form}"))
            .find(|text| item_chars(text) + title_chars <= remaining)
        else {
            continue;
        };
        remaining -= item_chars(&text) + title_chars;
        titled.insert(session);
        chosen.push(Chosen {
            seq: item.seq,
            session: Some(session),
            text,
        });
    }
    chosen.sort_by_key(|item| item.seq);
    chosen
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 축약본 글자 수
// basis: estimate
fn digest(item: &CompetingItem) -> String {
    let head = head(&item.text);
    let memo = item.memo.lines().next().unwrap_or_default();
    if memo.is_empty() {
        return head;
    }
    format!("{memo}\n{head}")
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 앞부분 글자 수
// basis: estimate
fn head(text: &str) -> String {
    text.chars().take(DIGEST_HEAD_CHARS).collect()
}

// cost: time O(l), heap O(1), stack O(1)
// vars: l = 항목 글자 수
// basis: estimate
fn item_chars(form: &str) -> usize {
    form.chars().count() + ITEM_SEPARATOR.len()
}

// cost: time O(e log e + L), heap O(L), stack O(1)
// vars: e = 항목 수, L = 항목 글자 수
// basis: estimate
fn by_seq(entries: &[Entry]) -> Vec<SectionItem> {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by_key(|entry| entry.seq);
    sorted
        .into_iter()
        .map(|entry| SectionItem {
            session: None,
            text: entry.text.clone(),
        })
        .collect()
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 패킷 원문 글자 수
// basis: estimate
fn render(sections: &[Section]) -> String {
    let mut text = String::new();
    for section in sections.iter().filter(|section| !section.items.is_empty()) {
        text.push_str("## ");
        text.push_str(section.title);
        text.push_str(ITEM_SEPARATOR);
        let mut titled: Option<SessionId> = None;
        for item in &section.items {
            if let Some(session) = item.session.filter(|session| titled != Some(*session)) {
                text.push_str(&session_title(session));
                text.push_str(ITEM_SEPARATOR);
                titled = Some(session);
            }
            text.push_str(&item.text);
            text.push_str(ITEM_SEPARATOR);
        }
    }
    text
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn to_chars(tokens: u64) -> usize {
    usize::try_from(tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(CHARS_PER_TOKEN)
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
fn is_provider_doc(item: &CompetingItem) -> bool {
    item.path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| PROVIDER_DOCS.contains(&name))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sessions::ranking::{Candidate, DEFAULT_RRF_K, order_after_judge, rank_candidates};

    // 2026-09-12T10:00Z
    const AT_MS: i64 = 1_789_207_200_000;

    // T = 4_000 → P_max = 400 토큰(1_600자), P_hard = 800 토큰(3_200자)
    fn budget() -> ContextBudget {
        ContextBudget {
            t_abs: 4_000,
            safety_percent: 100,
            window: 1_000_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
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
            packet.text,
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
            packet.text,
            "## Recent turns\n\n### Session 1\n\n#3 2026-09-12T10:00Z User: run tests\nAgent: ran them\n\n\
             ### Session 2\n\n#5 2026-09-12T11:00Z User: next\nAgent: ok\n\n"
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
        let long_rule = filler("rule", 3_500);
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

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    #[test]
    fn build_packet_skips_provider_docs() {
        let source = PacketSource {
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

        assert_eq!(packet.text, "");
        assert_eq!(packet.tokens, 0);
    }

    // cost: time O(c log c + c·l), heap O(c·l), stack O(1)
    // vars: c = 후보 수, l = 항목 글자 수
    // basis: estimate
    #[test]
    fn build_packet_judge_no_response_fills_in_rrf_order() {
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
        let ordered = order_after_judge(&ranked, &[]);
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
}
