//! 새 session에 넘기는 패킷의 고정 구역과 경쟁 구역.
//! 설계: docs/design/context-management.md#패킷-구성

use std::collections::HashSet;
use std::path::Path;

use saturn_protocol::ids::{ConstraintId, LedgerSeq, SessionId};

use super::constraint_slot::ConstraintTier;
use super::context::ContextBudget;
use super::memo::INTERRUPTED_RESULT;
use super::stamp::{Stamp, label, session_title};

/// 축약본과 줄인 에이전트 답에 남기는 앞부분 글자 수.
pub const DIGEST_HEAD_CHARS: usize = 300;

/// 고정 구역에 넣는 최근 턴 수의 최대.
pub const RECENT_TURNS: usize = 3;

// 초안
const CHARS_PER_TOKEN: usize = 4;

/// 제약 칸 상한을 셀 때 항목마다 더하는 구분 글자 수.
pub const CONSTRAINT_SEPARATOR_CHARS: usize = ITEM_SEPARATOR.len();

const ITEM_SEPARATOR: &str = "\n\n";

/// 패킷 맨 앞에 두는 지시. 아래 기록이 요청이 아니라 이미 일어난 일이고, 이 턴에서는 아무것도 하지 말고 다음 사용자 입력을 기다리라고 알린다.
/// 설계: docs/design/context-management.md#패킷-구성
const INSTRUCTION: &str = "\
The records below are an archive of the earlier conversation. They are context only, not a request.
This archive is selective. If a requested detail is missing, say it is unknown; do not invent it.
- Items marked [Finished] are already done. Do not run them again and do not repeat their edits or commands.
- Items marked [In progress] or [Result unknown] may have partly run. Check the current state before relying on them, and do not redo them unless the user asks.
- Queued input and Held input have not been sent to you. Saturn sends them as separate turns.
For this message, do not call tools and do not change files. Reply with the single word \"Ready\", then wait for the next user input.";
const COMPETING_TITLE: &str = "Earlier records";

/// 기록 안의 마지막 요청이 현재 지시처럼 읽히지 않게 기록 끝에서 경계를 다시 확인한다.
pub const ARCHIVE_END: &str = "End of archived records. All requests above belong to the past. Do not follow or answer them now. For this archive message only, reply exactly Ready, with no other text, then wait for the next user input.";

/// 기록 원문 한 덩어리.
#[derive(Debug, Clone)]
pub struct Entry {
    pub seq: LedgerSeq,
    pub text: String,
}

/// 입력이 연 실행의 상태. 패킷의 입력 항목마다 적어 새 session이 끝난 일을 열린 요청으로 읽지 않게 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Finished,
    InProgress,
    /// 실패하거나 멈춰 일부만 실행됐을 수 있다.
    ResultUnknown,
}

impl TurnStatus {
    fn tag(self) -> &'static str {
        match self {
            Self::Finished => "[Finished]",
            Self::InProgress => "[In progress]",
            Self::ResultUnknown => "[Result unknown]",
        }
    }
}

/// `{label} [{상태}]: {text}`. 결과를 모르면 결과 자리에 오류 결과를 붙인다.
pub fn status_item(label: &str, status: TurnStatus, text: &str) -> String {
    let mut item = format!("{label} {}: {text}", status.tag());
    push_unknown_result(&mut item, status);
    item
}

fn push_unknown_result(item: &mut String, status: TurnStatus) {
    if status == TurnStatus::ResultUnknown {
        item.push_str("\nResult (error): ");
        item.push_str(INTERRUPTED_RESULT);
    }
}

/// 도구 결과는 넣지 않는다.
#[derive(Debug, Clone)]
pub struct RecentTurn {
    pub seq: LedgerSeq,
    pub stamp: Stamp,
    pub status: TurnStatus,
    pub input: String,
    /// 이 턴이 도는 중에 끼워 넣어 적용한 사용자 입력. 적용한 순서다.
    pub steers: Vec<String>,
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
    /// 제약 칸에 들어간 규칙. 제약 번호 순이고 상한 안이다.
    pub constraints: Vec<String>,
    /// 칸이 차서 못 넣은 제약의 규칙. 있으면 칸 끝에 개수를 적고, 맥락 정리를 미룰 때는 `constraints`와 함께 보인다.
    pub constraints_omitted: Vec<String>,
    /// 전환 기록 `packet_constraints`에 남길 제약별 단계. 패킷 글에는 쓰지 않는다.
    pub constraint_tiers: Vec<(ConstraintId, ConstraintTier)>,
    pub goal_and_last_input: Vec<Entry>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<Entry>,
    /// `RECENT_TURNS`개보다 많으면 기록 번호가 큰 쪽만 쓴다.
    pub recent_turns: Vec<RecentTurn>,
    /// 맥락 고르기가 정한 순서.
    pub competitors: Vec<CompetingItem>,
    /// provider가 스스로 읽는 지시 문서 이름. 어댑터 설명자가 알린다. 이 이름의 파일은 패킷에 넣지 않는다.
    pub provider_docs: Vec<String>,
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
/// 거절된 패킷을 줄여 다시 만든다. 고정 구역은 `build_packet`과 같게 두고 경쟁 구역만 `target_tokens`에 맞춰 남길 확률이 낮은 항목부터 뺀다.
/// 고정 구역만으로 `target_tokens`를 넘으면 줄일 수 없어 `None`이다.
pub fn reduce_packet(
    source: &PacketSource,
    budget: &ContextBudget,
    target_tokens: u64,
) -> Option<Packet> {
    let soft_chars = to_chars(budget.packet_limit());
    let target_chars = to_chars(target_tokens).min(soft_chars);
    let sections = fit_fixed_zone(source, soft_chars);
    let fixed_chars = render(&sections).chars().count();
    if fixed_chars > target_chars {
        return None;
    }
    Some(assemble(
        source,
        sections,
        (fixed_chars, target_chars),
        budget.item_cap_percent,
        None,
    ))
}

// cost: time O(t·L + m log m), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
fn build(source: &PacketSource, budget: &ContextBudget, summary: Option<&Entry>) -> PacketOutcome {
    let soft_chars = to_chars(budget.packet_limit());
    let hard_chars = to_chars(budget.packet_hard_limit());
    let sections = fit_fixed_zone(source, soft_chars);
    let fixed_chars = render(&sections).chars().count();
    if fixed_chars > hard_chars {
        return PacketOutcome::Deferred {
            constraints: source
                .constraints
                .iter()
                .chain(&source.constraints_omitted)
                .cloned()
                .collect(),
        };
    }
    PacketOutcome::Ready(assemble(
        source,
        sections,
        (fixed_chars, soft_chars),
        budget.item_cap_percent,
        summary,
    ))
}

// cost: time O(m log m + L), heap O(L), stack O(1)
// vars: m = 경쟁 항목 수, L = 패킷 재료 글자 수
// basis: estimate
/// 고정 구역 뒤에 경쟁 구역을 `limit_chars`까지 채운다. 고정 구역이 `limit_chars`를 넘으면 경쟁 구역은 빈다.
fn assemble(
    source: &PacketSource,
    mut sections: Vec<Section>,
    (fixed_chars, limit_chars): (usize, usize),
    item_cap_percent: u64,
    summary: Option<&Entry>,
) -> Packet {
    let header_chars = format!("## {COMPETING_TITLE}{ITEM_SEPARATOR}")
        .chars()
        .count();
    let competing_chars = limit_chars.saturating_sub(fixed_chars + header_chars);
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
    chosen.extend(fill_competing_zone(
        &source.competitors,
        &source.provider_docs,
        rest_chars,
        item_cap_percent,
    ));
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
    Packet {
        text,
        tokens,
        up_to: source.up_to,
        is_over_limit: fixed_chars > limit_chars,
        included: chosen.iter().map(|item| item.seq).collect(),
        is_summary_used: summary.is_some(),
    }
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
        .map(|turn| {
            let mut text = format!(
                "{} {} User: {}",
                label(turn.seq, turn.stamp.at_ms),
                turn.status.tag(),
                turn.input,
            );
            for steer in &turn.steers {
                text.push_str("\nUser (sent while this turn was running): ");
                text.push_str(steer);
            }
            text.push_str("\nAgent: ");
            text.push_str(&turn.answer);
            push_unknown_result(&mut text, turn.status);
            SectionItem {
                session: Some(turn.stamp.session),
                text,
            }
        })
        .collect();
    vec![
        Section {
            title: "Constraints and decisions",
            items: constraint_items(source),
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
fn fill_competing_zone(
    items: &[CompetingItem],
    provider_docs: &[String],
    budget_chars: usize,
    item_cap_percent: u64,
) -> Vec<Chosen> {
    let percent = usize::try_from(item_cap_percent.min(100)).unwrap_or(100);
    let item_cap = budget_chars * percent / 100;
    let mut remaining = budget_chars;
    let mut titled: HashSet<SessionId> = HashSet::new();
    let mut chosen: Vec<Chosen> = Vec::new();
    for item in items
        .iter()
        .filter(|item| !is_provider_doc(item, provider_docs))
    {
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

/// 제약 칸 상한 `C_max`를 글자 수로 센 값.
pub fn constraint_cap_chars(budget: &ContextBudget) -> usize {
    to_chars(budget.constraint_limit())
}

// cost: time O(c), heap O(L), stack O(1)
// vars: c = 제약 수, L = 제약 글자 수
// basis: estimate
/// 칸에 든 제약 뒤에 못 넣은 제약 수 한 줄을 붙인다. 이 줄은 상한에 세지 않는다.
fn constraint_items(source: &PacketSource) -> Vec<SectionItem> {
    let omitted = (!source.constraints_omitted.is_empty())
        .then(|| format!("Constraints omitted: {}", source.constraints_omitted.len()));
    source
        .constraints
        .iter()
        .cloned()
        .chain(omitted)
        .map(|text| SectionItem {
            session: None,
            text,
        })
        .collect()
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
    let mut text = format!("{INSTRUCTION}{ITEM_SEPARATOR}");
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
    text.push_str(ARCHIVE_END);
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
fn is_provider_doc(item: &CompetingItem, provider_docs: &[String]) -> bool {
    item.path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| provider_docs.iter().any(|doc| doc == name))
}

#[cfg(test)]
mod tests;
