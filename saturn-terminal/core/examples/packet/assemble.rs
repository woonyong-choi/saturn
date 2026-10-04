//! 기록 목록에서 `PacketSource`를 만들고 후보 순위와 router 판단으로 경쟁 구역 순서를 정한다.
//! 설계: docs/design/context-management.md#패킷-구성, docs/design/context-selection.md

use std::collections::HashSet;
use std::time::Duration;

use saturn_core::sessions::context::{
    ContextBudget, DEFAULT_ITEM_CAP_PERCENT, DEFAULT_PACKET_HARD_PERCENT,
};
use saturn_core::sessions::memo::{ToolKind, tool_memo};
use saturn_core::sessions::packet::{
    CompetingItem, Entry, PacketSource, RECENT_TURNS, RecentTurn, TurnStatus,
};
use saturn_core::sessions::ranking::{
    Candidate, DEFAULT_RRF_K, order_after_router, rank_candidates,
};
use saturn_core::sessions::stamp::Stamp;
use saturn_protocol::ids::{LedgerSeq, SessionId};
use serde::Deserialize;

use crate::args::PacketCondition;

/// 개발용 예제라 어댑터 설명자 없이 두 provider의 지시 문서 이름을 직접 적는다.
const PROVIDER_DOCS: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];
use crate::records::{Body, Record};

/// `T`는 `P_max`의 10배다(`P_max = T / 10`).
const BUDGET_TO_THRESHOLD: u64 = 10;

/// 경쟁 구역 후보. 도구 호출 하나가 후보 하나다.
struct Tool<'a> {
    seq: u64,
    stamp: Stamp,
    name: &'a str,
    args: &'a serde_json::Value,
    result: Option<&'a str>,
}

/// `--judgments` 파일. 둘 다 없어도 된다.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Judgments {
    /// `compact` 판단. 후보 기록 번호와 남길 확률.
    #[serde(default)]
    pub(crate) compact: Vec<Verdict>,
    /// 대체되지 않은 제약으로 판단한 사용자 입력의 기록 번호.
    #[serde(default)]
    pub(crate) constraints: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub(crate) struct Verdict {
    pub(crate) seq: u64,
    pub(crate) probability: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FixedFields {
    pub(crate) goal: Vec<FixedEntry>,
    pub(crate) open_items: Vec<FixedEntry>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FixedEntry {
    pub(crate) seq: u64,
    pub(crate) text: String,
}

/// 순서를 정하는 규칙.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OrderRule {
    pub(crate) condition: PacketCondition,
    pub(crate) k: u32,
    pub(crate) top_n: usize,
}

#[derive(Debug)]
pub(crate) struct Assembled {
    pub(crate) source: PacketSource,
    /// 후보 전체의 RRF 순서.
    pub(crate) rrf_order: Vec<LedgerSeq>,
    /// 순서를 정하는 데 쓴 router 판단 수.
    pub(crate) routed: usize,
}

// cost: time O(L + c log c), heap O(L), stack O(1)
// vars: L = 기록 글자 수, c = 후보 수
// basis: estimate
/// `after`가 있으면 그 번호 뒤의 도구 호출만 후보로 둔다(provider 요약 모드). 고정 구역은 기록 전체에서 만든다.
pub(crate) fn assemble(
    records: &[Record],
    after: Option<u64>,
    judgments: &Judgments,
    rule: &OrderRule,
    fixed: Option<&FixedFields>,
) -> Assembled {
    let last_input = last_user(records).map_or("", |(_, text)| text);
    let turns = recent_turns(records);
    let tools: Vec<Tool> = tools(records, after);
    let candidates: Vec<Candidate> = tools.iter().map(candidate).collect();
    let base_files = base_files(records, &turns, last_input);
    let rrf_order = rank_candidates(&candidates, &base_files, last_input, rule.k);
    let verdicts = usable_verdicts(&rrf_order, judgments, rule);
    let ordered = order_after_router(&rrf_order, &verdicts);
    let competitors = ordered
        .iter()
        .filter_map(|seq| tools.iter().find(|tool| tool.seq == seq.0))
        .map(competing_item)
        .collect();
    let (goal_and_last_input, open_items) = fixed.map_or_else(
        || {
            (
                last_user(records)
                    .map(|(seq, text)| {
                        vec![Entry {
                            seq: LedgerSeq(seq),
                            text: text.to_owned(),
                        }]
                    })
                    .unwrap_or_default(),
                open_items(&tools),
            )
        },
        |fields| {
            (
                fields.goal.iter().map(entry).collect(),
                fields.open_items.iter().map(entry).collect(),
            )
        },
    );
    let source = PacketSource {
        constraints: constraints(records, &judgments.constraints),
        goal_and_last_input,
        open_items,
        recent_turns: turns,
        competitors,
        provider_docs: PROVIDER_DOCS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        up_to: LedgerSeq(records.iter().map(|record| record.seq).max().unwrap_or(0)),
    };
    Assembled {
        source,
        rrf_order,
        routed: verdicts.len(),
    }
}

fn entry(value: &FixedEntry) -> Entry {
    Entry {
        seq: LedgerSeq(value.seq),
        text: value.text.clone(),
    }
}

/// `P_max = budget_tokens`가 되는 예산. 안전 비율은 100%, 창 크기는 `T`와 같게 둬 `T`가 그대로 기준이 된다.
pub(crate) fn budget_for(budget_tokens: u64) -> ContextBudget {
    let threshold = budget_tokens.saturating_mul(BUDGET_TO_THRESHOLD);
    ContextBudget {
        t_abs: threshold,
        safety_percent: 100,
        window: threshold,
        cache_read: 0.1,
        cache_write: 1.25,
        cache_ttl: Duration::from_secs(300),
        packet_hard_percent: DEFAULT_PACKET_HARD_PERCENT,
        item_cap_percent: DEFAULT_ITEM_CAP_PERCENT,
        rrf_k: DEFAULT_RRF_K,
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// 조건이 router 판단을 쓰지 않거나 판단이 없으면 빈 목록(RRF 순서 대체)이다. `rrf-router`는 RRF 상위 `top_n`개의 판단만 쓴다.
fn usable_verdicts(
    rrf_order: &[LedgerSeq],
    judgments: &Judgments,
    rule: &OrderRule,
) -> Vec<(LedgerSeq, f64)> {
    let allowed: HashSet<u64> = match rule.condition {
        PacketCondition::RrfOnly => return Vec::new(),
        PacketCondition::RouterOnly => rrf_order.iter().map(|seq| seq.0).collect(),
        PacketCondition::RrfRouter => rrf_order.iter().take(rule.top_n).map(|seq| seq.0).collect(),
    };
    judgments
        .compact
        .iter()
        .filter(|verdict| allowed.contains(&verdict.seq))
        .map(|verdict| (LedgerSeq(verdict.seq), verdict.probability))
        .collect()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn last_user(records: &[Record]) -> Option<(u64, &str)> {
    records.iter().rev().find_map(|record| match &record.body {
        Body::User(text) => Some((record.seq, text.as_str())),
        _ => None,
    })
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// 사용자 입력마다 턴 하나이고, 다음 입력 전까지의 에이전트 글을 답으로 모은다.
fn recent_turns(records: &[Record]) -> Vec<RecentTurn> {
    let mut turns: Vec<RecentTurn> = Vec::new();
    for record in records {
        match &record.body {
            Body::User(text) => turns.push(RecentTurn {
                seq: LedgerSeq(record.seq),
                stamp: stamp_of(record),
                status: TurnStatus::Finished,
                input: text.clone(),
                answer: String::new(),
            }),
            Body::Agent(text) => {
                if let Some(turn) = turns.last_mut() {
                    turn.answer.push_str(text);
                }
            }
            Body::Tool { .. } => {}
        }
    }
    turns
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn tools(records: &[Record], after: Option<u64>) -> Vec<Tool<'_>> {
    records
        .iter()
        .filter(|record| after.is_none_or(|boundary| record.seq > boundary))
        .filter_map(|record| match &record.body {
            Body::Tool { name, args, result } => Some(Tool {
                seq: record.seq,
                stamp: stamp_of(record),
                name,
                args,
                result: result.as_deref(),
            }),
            _ => None,
        })
        .collect()
}

fn stamp_of(record: &Record) -> Stamp {
    Stamp {
        session: SessionId(record.session),
        at_ms: record.at_ms,
    }
}

fn candidate(tool: &Tool) -> Candidate {
    Candidate {
        seq: LedgerSeq(tool.seq),
        text: tool_text(tool),
        files: tool_files(tool),
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn competing_item(tool: &Tool) -> CompetingItem {
    let files = tool_files(tool);
    CompetingItem {
        seq: LedgerSeq(tool.seq),
        stamp: tool.stamp,
        text: tool_text(tool),
        memo: tool_memo(
            &kind_of(tool, files.first()),
            tool.result.unwrap_or_default(),
        ),
        path: files.into_iter().next(),
    }
}

fn tool_text(tool: &Tool) -> String {
    format!(
        "{} {}\n{}",
        tool.name,
        tool.args,
        tool.result.unwrap_or_default()
    )
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn tool_files(tool: &Tool) -> Vec<String> {
    ["path", "file_path"]
        .iter()
        .filter_map(|key| tool.args[key].as_str())
        .map(str::to_owned)
        .collect()
}

/// 파일 읽기만 틀이 있고 나머지는 메모가 없다. 입력에 종료 코드나 줄 수가 없어 값을 지어내지 않기 위해서다.
fn kind_of(tool: &Tool, path: Option<&String>) -> ToolKind {
    match (tool.name.to_lowercase().as_str(), path) {
        ("read", Some(path)) => ToolKind::FileRead {
            path: path.clone(),
            lines: None,
        },
        _ => ToolKind::Other,
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn constraints(records: &[Record], seqs: &[u64]) -> Vec<Entry> {
    records
        .iter()
        .filter(|record| seqs.contains(&record.seq))
        .filter_map(|record| match &record.body {
            Body::User(text) => Some(Entry {
                seq: LedgerSeq(record.seq),
                text: text.clone(),
            }),
            _ => None,
        })
        .collect()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
fn open_items(tools: &[Tool]) -> Vec<Entry> {
    tools
        .iter()
        .filter(|tool| tool.result.is_none())
        .map(|tool| Entry {
            seq: LedgerSeq(tool.seq),
            text: format!("Unfinished tool call: {} {}", tool.name, tool.args),
        })
        .collect()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// 마지막 입력에 나온 경로와 최근 3턴의 도구 호출이 건드린 파일.
fn base_files(records: &[Record], turns: &[RecentTurn], last_input: &str) -> Vec<String> {
    let first_recent = turns
        .len()
        .checked_sub(RECENT_TURNS)
        .map_or(0, |skip| turns[skip].seq.0);
    let mut files = paths_in(last_input);
    files.extend(
        tools(records, None)
            .iter()
            .filter(|tool| tool.seq >= first_recent)
            .flat_map(tool_files),
    );
    files
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 입력 항목 수
// basis: estimate
/// 경로 모양의 낱말: `/`가 있거나 `이름.확장자` 꼴.
fn paths_in(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| {
                !(c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
            })
        })
        .filter(|word| word.contains('/') || word.trim_end_matches('.').contains('.'))
        .map(str::to_owned)
        .collect()
}
