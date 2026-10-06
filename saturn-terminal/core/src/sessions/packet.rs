//! 새 session에 넘기는 패킷의 고정 구역과 경쟁 구역.
//! 설계: docs/design/context-management.md#패킷-구성

use std::collections::HashSet;
use std::path::Path;

use saturn_protocol::ids::{ConstraintId, LedgerSeq, SessionId, TaskId};

use super::constraint_slot::ConstraintTier;
use super::context::ContextBudget;
use super::memo::INTERRUPTED_RESULT;
use super::stamp::{Stamp, label, session_title};

/// 축약본과 줄인 에이전트 답에 남기는 앞부분 글자 수.
pub const DIGEST_HEAD_CHARS: usize = 300;

/// 고정 구역에 넣는 최근 턴 수의 최대.
pub const RECENT_TURNS: usize = 3;

/// 최신 수정을 고정 구역에 넣는 작업 수의 최대(초안). 오래된 대화의 수정이 쌓여 고정 구역을 채우지 않게 한다.
pub const AMENDED_TASKS: usize = 5;

// 초안
const CHARS_PER_TOKEN: usize = 4;

/// 제약 칸 상한을 셀 때 항목마다 더하는 구분 글자 수.
pub const CONSTRAINT_SEPARATOR_CHARS: usize = ITEM_SEPARATOR.len();

const ITEM_SEPARATOR: &str = "\n\n";

/// 패킷 맨 앞에 두는 지시. 아래 기록이 요청이 아니라 이미 일어난 일이고, 이 턴에서는 아무것도 하지 말고 다음 사용자 입력을 기다리라고 알린다.
/// 설계: docs/design/context-management.md#패킷-구성
const INSTRUCTION: &str = "\
The records below are an archive of the earlier conversation. They are context only, not a request.
- Items marked [Finished] are already done. Do not run them again and do not repeat their edits or commands.
- Items marked [In progress] or [Result unknown] may have partly run. Check the current state before relying on them, and do not redo them unless the user asks.
- Queued input and Held input have not been sent to you. Saturn sends them as separate turns.
For this message, do not call tools and do not change files. Reply with the single word \"Ready\", then wait for the next user input.";
const COMPETING_TITLE: &str = "Earlier records";

/// 경쟁 구역 끝에 붙이는 안내. 줄이거나 뺀 기록의 번호(`#` 뒤 숫자)로 원문을 다시 읽는 방법이다.
/// 설계: docs/design/context-selection.md#근거-검색과-원문-조회
const LOOKUP_HINT: &str = "\
Records shown as a digest or a path, and records left out, can be read in full in a later turn with `saturn evidence read <number>` (the number after #). `saturn evidence search <words>` lists matching records.";

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

/// 작업의 최신 수정. 그 작업에서 수정으로 판단된 가장 나중의 사용자 입력 원문이다.
/// 최근 턴 세 칸에서 밀려나도 고정 구역에 남기려고 따로 둔다.
#[derive(Debug, Clone)]
pub struct Amendment {
    /// 수정을 낸 실행의 첫 이벤트 번호. 끼워 넣은 입력은 적용 때 쌓여 있던 마지막 번호다.
    pub seq: LedgerSeq,
    pub task: TaskId,
    pub status: TurnStatus,
    /// 사용자 입력 원문.
    pub text: String,
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
    /// 작업마다의 최신 수정. 같은 원문이 최근 턴이나 목표 칸에 이미 있으면 넣지 않고, 고정 구역이 넘치면 오래된 것부터 빼고 생략을 표시한다.
    pub amendments: Vec<Amendment>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<Entry>,
    /// `RECENT_TURNS`개보다 많으면 기록 번호가 큰 쪽만 쓴다.
    pub recent_turns: Vec<RecentTurn>,
    /// 맥락 고르기가 정한 순서.
    pub competitors: Vec<CompetingItem>,
    /// 참이면 경쟁 구역이 원문 아닌 모양으로 넣거나 뺀 기록이 있을 때 다시 읽는 방법을 한 줄 알린다. 설정 `context.evidence.lookup`.
    pub evidence_lookup: bool,
    /// provider가 스스로 읽는 지시 문서 이름. 어댑터 설명자가 알린다. 이 이름의 파일은 패킷에 넣지 않는다.
    pub provider_docs: Vec<String>,
    pub up_to: LedgerSeq,
}

/// 패킷 항목이 들어가는 구역. 제약 칸은 항목이 기록 번호가 아니라 제약 번호라 `PacketSource::constraint_tiers`가 따로 말한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketZone {
    Goal,
    Open,
    Recent,
    Competing,
}

impl PacketZone {
    /// 전달 패킷 기록에 쓰는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::Goal => "Goal",
            Self::Open => "Open",
            Self::Recent => "Recent",
            Self::Competing => "Competing",
        }
    }
}

/// 항목이 패킷에 들어간 모양.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemForm {
    Full,
    /// 에이전트 답을 앞부분만 남겼다.
    Trimmed,
    Digest,
    Path,
    /// provider 압축 요약.
    Summary,
}

impl ItemForm {
    /// 전달 패킷 기록에 쓰는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Trimmed => "Trimmed",
            Self::Digest => "Digest",
            Self::Path => "Path",
            Self::Summary => "Summary",
        }
    }
}

/// 패킷 재료 항목 하나가 이 패킷에 들어갔는지. 들어가지 못했으면 이유가 있다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketItem {
    pub zone: PacketZone,
    pub seq: LedgerSeq,
    /// 들어간 모양. 들어가지 못했으면 `None`.
    pub form: Option<ItemForm>,
    /// 들어가지 못한 이유. `recent_limit`(최근 턴 수 상한), `packet_limit`(고정 구역이 넘쳐 뺌), `budget`(경쟁 구역
    /// 예산), `provider_doc`(provider가 스스로 읽는 문서). 들어갔으면 `None`.
    pub reason: Option<&'static str>,
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
    /// 기록 번호가 있는 재료 항목마다 들어갔는지. 구역 순서이고 구역 안에서는 재료 순서다.
    pub items: Vec<PacketItem>,
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
    form: ItemForm,
}

/// 최신 수정의 기록 번호와 빠진 이유. 들어갔으면 이유가 없다.
type AmendmentFate = (LedgerSeq, Option<&'static str>);

/// 고정 구역을 맞춘 결과. `turns`는 남은 최근 턴이고, `trimmed`는 답을 앞부분만 남긴 턴의 기록 번호다.
struct Fitted {
    sections: Vec<Section>,
    turns: Vec<RecentTurn>,
    trimmed: Vec<LedgerSeq>,
    /// 최신 수정마다 들어갔는지. 들어가지 못했으면 이유(`duplicate`, `packet_limit`).
    amendments: Vec<AmendmentFate>,
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
    let fitted = fit_fixed_zone(source, soft_chars);
    let fixed_chars = render(&fitted.sections).chars().count();
    if fixed_chars > target_chars {
        return None;
    }
    Some(assemble(
        source,
        fitted,
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
    let fitted = fit_fixed_zone(source, soft_chars);
    let fixed_chars = render(&fitted.sections).chars().count();
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
        fitted,
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
    fitted: Fitted,
    (fixed_chars, limit_chars): (usize, usize),
    item_cap_percent: u64,
    summary: Option<&Entry>,
) -> Packet {
    let Fitted {
        mut sections,
        turns,
        trimmed,
        amendments,
    } = fitted;
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
            form: ItemForm::Summary,
        });
    }
    let fill = |chars| {
        fill_competing_zone(
            &source.competitors,
            &source.provider_docs,
            chars,
            item_cap_percent,
        )
    };
    let mut filled = fill(rest_chars);
    let hint = source
        .evidence_lookup
        .then_some(item_chars(LOOKUP_HINT))
        .filter(|_| has_unread_original(source, &filled));
    if let Some(hint_chars) = hint {
        filled = fill(rest_chars.saturating_sub(hint_chars));
    }
    chosen.extend(filled);
    let mut texts: Vec<SectionItem> = chosen
        .iter()
        .map(|item| SectionItem {
            session: item.session,
            text: item.text.clone(),
        })
        .collect();
    if hint.is_some() && has_unread_original(source, &chosen) {
        texts.push(SectionItem {
            session: None,
            text: LOOKUP_HINT.to_owned(),
        });
    }
    sections.push(Section {
        title: COMPETING_TITLE,
        items: texts,
    });
    let text = render(&sections);
    let tokens = estimate_tokens(&text);
    let items = packet_items(source, (&turns, &trimmed, &amendments), &chosen);
    Packet {
        text,
        tokens,
        up_to: source.up_to,
        is_over_limit: fixed_chars > limit_chars,
        included: chosen.iter().map(|item| item.seq).collect(),
        is_summary_used: summary.is_some(),
        items,
    }
}

// cost: time O(m·c), heap O(1), stack O(1)
// vars: m = 경쟁 항목 수, c = 고른 항목 수
// basis: estimate
/// 경쟁 항목 중 원문이 그대로 들어가지 못한 것(뺐거나 축약본, 경로로 넣음)이 있는지. provider 문서는 provider가 스스로 읽으므로 세지 않는다.
fn has_unread_original(source: &PacketSource, chosen: &[Chosen]) -> bool {
    source
        .competitors
        .iter()
        .filter(|item| !is_provider_doc(item, &source.provider_docs))
        .any(|item| {
            !chosen
                .iter()
                .any(|picked| picked.seq == item.seq && picked.form == ItemForm::Full)
        })
}

// cost: time O(m + t), heap O(m + t), stack O(1)
// vars: m = 재료 항목 수, t = 최근 턴 수
// basis: estimate
/// 재료 항목마다 이 패킷에 들어갔는지 정리한다. 고정 구역의 목표와 열린 항목은 항상 들어간다.
fn packet_items(
    source: &PacketSource,
    (turns, trimmed, amendments): (&[RecentTurn], &[LedgerSeq], &[AmendmentFate]),
    chosen: &[Chosen],
) -> Vec<PacketItem> {
    let entry = |zone, seq| PacketItem {
        zone,
        seq,
        form: Some(ItemForm::Full),
        reason: None,
    };
    let mut items: Vec<PacketItem> = source
        .goal_and_last_input
        .iter()
        .map(|item| entry(PacketZone::Goal, item.seq))
        .chain(amendments.iter().map(|(seq, reason)| PacketItem {
            zone: PacketZone::Goal,
            seq: *seq,
            form: reason.is_none().then_some(ItemForm::Full),
            reason: *reason,
        }))
        .chain(
            source
                .open_items
                .iter()
                .map(|item| entry(PacketZone::Open, item.seq)),
        )
        .collect();
    let newest: HashSet<LedgerSeq> = {
        let mut seqs: Vec<LedgerSeq> = source.recent_turns.iter().map(|turn| turn.seq).collect();
        seqs.sort_unstable();
        seqs.split_off(seqs.len().saturating_sub(RECENT_TURNS))
            .into_iter()
            .collect()
    };
    for turn in &source.recent_turns {
        let kept = turns.iter().any(|kept| kept.seq == turn.seq);
        let (form, reason) = match (kept, newest.contains(&turn.seq)) {
            (true, _) if trimmed.contains(&turn.seq) => (Some(ItemForm::Trimmed), None),
            (true, _) => (Some(ItemForm::Full), None),
            (false, true) => (None, Some("packet_limit")),
            (false, false) => (None, Some("recent_limit")),
        };
        items.push(PacketItem {
            zone: PacketZone::Recent,
            seq: turn.seq,
            form,
            reason,
        });
    }
    items.extend(
        chosen
            .iter()
            .filter(|item| item.form == ItemForm::Summary)
            .map(|item| PacketItem {
                zone: PacketZone::Competing,
                seq: item.seq,
                form: Some(ItemForm::Summary),
                reason: None,
            }),
    );
    for item in &source.competitors {
        let picked = chosen
            .iter()
            .find(|picked| picked.seq == item.seq && picked.form != ItemForm::Summary);
        let reason = match picked {
            Some(_) => None,
            None if is_provider_doc(item, &source.provider_docs) => Some("provider_doc"),
            None => Some("budget"),
        };
        items.push(PacketItem {
            zone: PacketZone::Competing,
            seq: item.seq,
            form: picked.map(|picked| picked.form),
            reason,
        });
    }
    items
}

// cost: time O(t·L), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 고정 구역 글자 수
// basis: estimate
/// 오래된 턴부터 에이전트 답을 앞부분만 남기고, 그래도 넘치면 최근 턴 수를 3, 2, 1로 줄이고, 그래도 넘치면 다른 작업의 최신 수정을 오래된 것부터 뺀다.
/// 최신 수정이 최근 턴보다 나중에 줄어드는 것은 밀려난 정정 원문을 최근 턴보다 먼저 지키기 위해서다.
fn fit_fixed_zone(source: &PacketSource, soft_chars: usize) -> Fitted {
    let mut turns = source.recent_turns.clone();
    turns.sort_by_key(|turn| turn.seq);
    let excess = turns.len().saturating_sub(RECENT_TURNS);
    turns.drain(..excess);
    let mut pool: Vec<&Amendment> = source.amendments.iter().collect();
    pool.sort_by_key(|amendment| amendment.seq);
    // 오래된 쪽부터 빠진 개수
    let mut dropped = 0;
    let fits = |turns: &[RecentTurn], dropped: usize| {
        let (kept, omitted) = kept_amendments(source, &pool, turns, dropped);
        render(&fixed_sections(source, turns, &kept, omitted))
            .chars()
            .count()
            <= soft_chars
    };
    let mut trimmed = Vec::new();
    let mut settled = false;
    for index in 0..turns.len() {
        if fits(&turns, dropped) {
            settled = true;
            break;
        }
        let shortened = head(&turns[index].answer);
        if shortened != turns[index].answer {
            trimmed.push(turns[index].seq);
        }
        turns[index].answer = shortened;
    }
    if !settled {
        while turns.len() > 1 && !fits(&turns, dropped) {
            turns.remove(0);
        }
        while dropped < pool.len() && !fits(&turns, dropped) {
            dropped += 1;
        }
    }
    let (kept, omitted) = kept_amendments(source, &pool, &turns, dropped);
    let outcome = pool
        .iter()
        .enumerate()
        .map(|(index, amendment)| {
            let reason = if index < dropped {
                Some("packet_limit")
            } else if kept.iter().all(|k| k.seq != amendment.seq) {
                Some("duplicate")
            } else {
                None
            };
            (amendment.seq, reason)
        })
        .collect();
    Fitted {
        sections: fixed_sections(source, &turns, &kept, omitted),
        turns,
        trimmed,
        amendments: outcome,
    }
}

// cost: time O(a·(t + g)), heap O(a), stack O(1)
// vars: a = 최신 수정 수, t = 최근 턴 수, g = 목표 칸 항목 수
// basis: estimate
/// 앞에서 `dropped`개를 뺀 최신 수정 중 같은 원문이 최근 턴이나 목표 칸에 없는 것과, 빼서 보이지 않게 된 수정의 수.
fn kept_amendments<'a>(
    source: &PacketSource,
    pool: &[&'a Amendment],
    turns: &[RecentTurn],
    dropped: usize,
) -> (Vec<&'a Amendment>, usize) {
    let is_shown = |text: &str| {
        turns
            .iter()
            .any(|turn| turn.input == text || turn.steers.iter().any(|steer| steer == text))
            || source
                .goal_and_last_input
                .iter()
                .any(|entry| entry.text.contains(text))
    };
    let omitted = pool
        .iter()
        .take(dropped)
        .filter(|amendment| !is_shown(&amendment.text))
        .count();
    let kept = pool
        .iter()
        .skip(dropped)
        .filter(|amendment| !is_shown(&amendment.text))
        .copied()
        .collect();
    (kept, omitted)
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 고정 구역 글자 수
// basis: estimate
fn fixed_sections(
    source: &PacketSource,
    turns: &[RecentTurn],
    amendments: &[&Amendment],
    omitted: usize,
) -> Vec<Section> {
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
            items: goal_items(source, amendments, omitted),
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

// cost: time O(a log a + L), heap O(L), stack O(1)
// vars: a = 항목 수, L = 목표 칸 글자 수
// basis: estimate
/// 지금 작업의 첫·마지막 입력 뒤에 작업별 최신 수정을 기록 번호 순으로 붙이고, 뺀 것이 있으면 개수를 적는다.
fn goal_items(
    source: &PacketSource,
    amendments: &[&Amendment],
    omitted: usize,
) -> Vec<SectionItem> {
    let mut items = by_seq(&source.goal_and_last_input);
    items.extend(amendments.iter().map(|amendment| SectionItem {
        session: None,
        text: status_item(
            &format!("Latest amendment (task {})", amendment.task.0),
            amendment.status,
            &amendment.text,
        ),
    }));
    if omitted > 0 {
        items.push(SectionItem {
            session: None,
            text: format!("({omitted} earlier amendments left out to fit this packet)"),
        });
    }
    items
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
        let forms = [
            (ItemForm::Full, raw),
            (ItemForm::Digest, Some(digest(item))),
            (ItemForm::Path, item.path.clone()),
        ];
        let Some((form, text)) = forms
            .into_iter()
            .filter_map(|(form, text)| Some((form, format!("{prefix} {}", text?))))
            .find(|(_, text)| item_chars(text) + title_chars <= remaining)
        else {
            continue;
        };
        remaining -= item_chars(&text) + title_chars;
        titled.insert(session);
        chosen.push(Chosen {
            seq: item.seq,
            session: Some(session),
            text,
            form,
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
