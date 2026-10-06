//! 새 session에 넘기는 패킷의 고정 구역과 경쟁 구역.
//! 설계: docs/design/context-management.md#패킷-구성

use std::collections::HashSet;
use std::path::Path;

use saturn_protocol::ids::{ChatId, ConstraintId, LedgerSeq, SessionId};
use saturn_protocol::rpc::EvidenceKind;

use super::constraint_slot::ConstraintTier;
use super::context::ContextBudget;
use super::memo::INTERRUPTED_RESULT;
use super::stamp::{Stamp, label, session_title};

/// 축약본에 남기는 앞부분 글자 수.
pub const DIGEST_HEAD_CHARS: usize = 300;

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
const RELATED_TITLE: &str = "Related original records";

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

fn push_unknown_result(item: &mut String, status: TurnStatus) {
    if status == TurnStatus::ResultUnknown {
        item.push_str("\nResult (error): ");
        item.push_str(INTERRUPTED_RESULT);
    }
}

/// 보호 본문의 역할. 전달 패킷 기록의 항목 구역 이름으로도 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 실행을 연 사용자 입력.
    User,
    /// 실행 중에 끼워 넣어 적용한 사용자 입력.
    Steer,
    /// 메인 에이전트가 낸 글.
    Assistant,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Steer => "Steer",
            Self::Assistant => "Assistant",
        }
    }
}

/// 턴 안의 본문 하나. 글을 줄이거나 합치지 않고 실제 순서대로 둔다.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    /// `User`는 실행의 첫 이벤트 번호, `Steer`는 입력 번호, `Assistant`는 그 글을 기록한 이벤트 번호다. 한 답이 여러 이벤트로 나뉘면 이벤트마다 `Assistant` 하나다.
    pub id: u64,
    pub text: String,
}

/// 입력이 연 실행 하나. 도구 결과는 넣지 않는다.
#[derive(Debug, Clone)]
pub struct Turn {
    pub seq: LedgerSeq,
    pub stamp: Stamp,
    pub status: TurnStatus,
    /// 실제 순서의 본문. 첫 항목은 실행을 연 `User`다.
    pub messages: Vec<Message>,
}

/// 기록된 본문 하나의 역할, 번호, 원문. 패킷 기록이 항목마다 해시를 남기는 재료다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    pub role: Role,
    pub id: u64,
    /// 기록된 원문.
    pub text: String,
}

impl Turn {
    // cost: time O(L), heap O(L), stack O(1)
    // vars: L = 턴 본문 글자 수
    // basis: estimate
    /// 본문마다 앞 표지를 붙여 줄로 잇는다. 기록이 조각으로 나뉜 한 답은 앞 조각에 바로 이어 붙여 읽기 편하게 하고 표지는 첫 조각에만 쓴다.
    fn render(&self) -> String {
        let mut text = String::new();
        let mut previous: Option<Role> = None;
        for message in &self.messages {
            match (previous, message.role) {
                (Some(Role::Assistant), Role::Assistant) => text.push_str(&message.text),
                (previous, role) => {
                    if previous.is_some() {
                        text.push('\n');
                    }
                    match role {
                        Role::User => text.push_str(&format!(
                            "{} {} User: {}",
                            label(self.seq, self.stamp.at_ms),
                            self.status.tag(),
                            message.text
                        )),
                        Role::Steer => {
                            text.push_str("User (sent while this turn was running): ");
                            text.push_str(&message.text);
                        }
                        Role::Assistant => {
                            text.push_str("Agent: ");
                            text.push_str(&message.text);
                        }
                    }
                }
            }
            previous = Some(message.role);
        }
        push_unknown_result(&mut text, self.status);
        text
    }
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

/// 새 session의 첫 작업 입력 앞에 미리 넣는 관련 원문 한 건. 종류와 채팅, 번호, 원문 해시가 `saturn evidence read`가 읽는
/// `종류:채팅:번호:해시` 참조와 같다. 줄이거나 바꾸지 않은 원문이고, 못 넣으면 통째로 뺀다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedRecord {
    pub kind: EvidenceKind,
    pub chat: ChatId,
    pub id: u64,
    /// 원문의 SHA-256(16진수 소문자).
    pub hash: String,
    pub text: String,
}

impl RelatedRecord {
    // cost: time O(L), heap O(L), stack O(1), alloc 1
    // vars: L = 원문 글자 수
    // basis: estimate
    fn render(&self) -> String {
        format!(
            "[{}:{}:{}:{}]\n{}",
            self.kind.name(),
            self.chat.0,
            self.id,
            self.hash,
            self.text
        )
    }

    // cost: time O(L), heap O(L), stack O(1)
    // vars: L = 원문 글자 수
    // basis: estimate
    /// 패킷에 이 항목이 차지하는 글자 수. 항목 뒤 구분 글자를 포함한다.
    pub fn packet_chars(&self) -> usize {
        item_chars(&self.render())
    }
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
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<Entry>,
    /// 이 채팅에서 기록된 사용자 입력과 에이전트 글. 개수와 길이에 상한이 없고 줄이거나 빼지 않는다.
    pub turns: Vec<Turn>,
    /// 맥락 고르기가 정한 순서.
    pub competitors: Vec<CompetingItem>,
    /// 참이면 경쟁 구역이 원문 아닌 모양으로 넣거나 뺀 기록이 있을 때 다시 읽는 방법을 한 줄 알린다. 설정 `context.evidence.lookup`.
    pub evidence_lookup: bool,
    /// provider가 스스로 읽는 지시 문서 이름. 어댑터 설명자가 알린다. 이 이름의 파일은 패킷에 넣지 않는다.
    pub provider_docs: Vec<String>,
    /// 실험 설정 `context.select.related`가 고른 관련 원문. 대화 본문 뒤의 고정 구역이라 줄이지 않고, 경쟁 구역은 이 뒤 남은 예산으로 채운다.
    pub related: Vec<RelatedRecord>,
    pub up_to: LedgerSeq,
}

impl PacketSource {
    // cost: time O(L), heap O(L), stack O(1)
    // vars: L = 보호 본문 글자 수
    // basis: estimate
    /// 패킷이 줄이지 않고 실어야 하는 본문. 기록 번호 순으로 정렬한 턴 안의 실제 순서다.
    pub fn protected(&self) -> Vec<Protected> {
        sorted_turns(&self.turns)
            .into_iter()
            .flat_map(|turn| &turn.messages)
            .map(|message| Protected {
                role: message.role,
                id: message.id,
                text: message.text.clone(),
            })
            .collect()
    }
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 고정 구역 글자 수
// basis: estimate
/// 패킷 앞쪽의 고정 구역(제약 칸, 남은 일, 대화 본문)을 직렬화한 글. 패킷 글은 항상 이 글로 시작하고,
/// 그 뒤에는 경쟁 구역만 올 수 있다. 전송 직전 검사의 기준 snapshot이다.
pub fn fixed_zone(source: &PacketSource) -> String {
    render(&fixed_sections(source))
}

/// `text`가 `fixed_zone` 글로 시작하고 그 뒤가 비었거나 경쟁 구역 제목으로 이어지는지. 본문이 빠지거나 바뀌거나 순서가 달라지면 거짓이라 보내면 안 된다.
/// 구역을 제목 글자로 찾지 않고 고정 구역 전체를 앞에서부터 글자 그대로 맞추므로, 본문 안의 제목 모양 글이나 다른 구역에 같은 글이 있어도 속지 않는다.
pub fn leads_with_fixed_zone(fixed: &str, text: &str) -> bool {
    text.strip_prefix(fixed)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(&competing_header()))
}

/// 패킷 항목이 들어가는 구역. 제약 칸은 항목이 기록 번호가 아니라 제약 번호라 `PacketSource::constraint_tiers`가 따로 말하고,
/// 대화 본문은 줄이거나 빼지 않으므로 `PacketSource::protected`가 따로 말한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketZone {
    Open,
    Competing,
}

impl PacketZone {
    /// 전달 패킷 기록에 쓰는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Competing => "Competing",
        }
    }
}

/// 항목이 패킷에 들어간 모양.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemForm {
    Full,
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
    /// 들어가지 못한 이유. `budget`(경쟁 구역 예산), `provider_doc`(provider가 스스로 읽는 문서). 들어갔으면 `None`.
    pub reason: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct Packet {
    pub text: String,
    /// 추정치.
    pub tokens: u64,
    /// 새 session의 `delivered`가 된다.
    pub up_to: LedgerSeq,
    /// 보호 본문을 담은 고정 구역이 `P_max`를 넘어 `P_send`까지 허용했다. engine이 초과를 기록한다.
    pub is_over_limit: bool,
    /// 전송 가능 여부를 가린 보수적 추정(토큰). 글자 수가 아니라 ASCII가 아닌 글자를 글자당 1토큰으로 센다. 실제 provider 계수가 아니라 추정이다.
    pub send_tokens: u64,
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
    /// 고정 구역의 보수적 추정이 전송 가능 상한 `P_send`도 넘거나 상한을 알 수 없어 새 session으로 옮기지 않는다. 본문은 자르지 않는다. TUI가 제약 목록을 보인다.
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

// cost: time O(L + m log m), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
/// 보호 본문은 `P_max`를 넘어도 줄이지 않고 전송 가능 상한 `P_send`(`ContextBudget::send_limit`)까지 허용하며 이때 경쟁 구역은 비운다. `P_send`도 넘거나 `P_send`를 알 수 없으면 `Deferred`다.
pub fn build_packet(source: &PacketSource, budget: &ContextBudget) -> PacketOutcome {
    build(source, budget, None)
}

// cost: time O(L + m log m), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
/// provider 압축 요약을 경쟁 구역 첫 항목으로 넣고 나머지는 `source.competitors`로 채운다. 호출하는 쪽이 요약 시점 뒤의 항목만 `competitors`에 둔다. 요약이 경쟁 구역 예산에 들어가지 않으면 요약 없이 `build_packet`과 같다.
pub fn build_packet_with_summary(
    source: &PacketSource,
    budget: &ContextBudget,
    summary: &Entry,
) -> PacketOutcome {
    build(source, budget, Some(summary))
}

// cost: time O(L + m log m), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
/// 거절된 패킷을 줄여 다시 만든다. 보호 본문과 제약, 남은 일은 `build_packet`과 같게 두고 경쟁 구역만 `target_tokens`에 맞춰 남길 확률이 낮은 항목부터 뺀다.
/// 보호 본문만으로 `target_tokens`를 넘으면 본문을 자르지 않고 줄일 수 없어 `None`이다.
pub fn reduce_packet(
    source: &PacketSource,
    budget: &ContextBudget,
    target_tokens: u64,
) -> Option<Packet> {
    let soft_chars = to_chars(budget.packet_limit());
    let target_chars = to_chars(target_tokens).min(soft_chars);
    let sections = fixed_sections(source);
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

// cost: time O(L + m log m), heap O(L), stack O(1)
// vars: L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
fn build(source: &PacketSource, budget: &ContextBudget, summary: Option<&Entry>) -> PacketOutcome {
    let soft_chars = to_chars(budget.packet_limit());
    let send_limit = budget.send_limit();
    let sections = fixed_sections(source);
    let fixed_text = render(&sections);
    if estimate_send_tokens(&fixed_text) > send_limit {
        return PacketOutcome::Deferred {
            constraints: source
                .constraints
                .iter()
                .chain(&source.constraints_omitted)
                .cloned()
                .collect(),
        };
    }
    let fixed_chars = fixed_text.chars().count();
    let assemble_with = |limit_chars| {
        assemble(
            source,
            sections.clone(),
            (fixed_chars, limit_chars),
            budget.item_cap_percent,
            summary,
        )
    };
    let mut packet = assemble_with(soft_chars);
    // 경쟁 구역까지 더한 글이 전송 가능 상한을 넘으면(글자 수 예산과 보수적 추정의 차이) 경쟁 구역을 비운다
    if estimate_send_tokens(&packet.text) > send_limit {
        packet = assemble_with(0);
    }
    packet.is_over_limit = fixed_chars > soft_chars;
    PacketOutcome::Ready(packet)
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
    // 관련 원문으로 이미 전문이 들어간 도구 기록은 경쟁 구역에서 뺀다. 기록 번호는 기록 행마다 겹치지 않고 도구 기록의 번호가 이 번호다
    let deduped;
    let source = if source
        .related
        .iter()
        .any(|record| record.kind == EvidenceKind::Tool)
    {
        deduped = PacketSource {
            competitors: source
                .competitors
                .iter()
                .filter(|item| {
                    !source
                        .related
                        .iter()
                        .any(|record| record.kind == EvidenceKind::Tool && record.id == item.seq.0)
                })
                .cloned()
                .collect(),
            ..source.clone()
        };
        &deduped
    } else {
        source
    };
    let header_chars = competing_header().chars().count();
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
        .filter(|hint_chars| *hint_chars <= rest_chars)
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
    let send_tokens = estimate_send_tokens(&text);
    let items = packet_items(source, &chosen);
    Packet {
        text,
        tokens,
        send_tokens,
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

// cost: time O(m + o), heap O(m + o), stack O(1)
// vars: m = 경쟁 항목 수, o = 남은 일 항목 수
// basis: estimate
/// 재료 항목마다 이 패킷에 들어갔는지 정리한다. 남은 일 항목은 항상 들어간다. 대화 본문은 줄이거나 빼지 않으므로 여기에 없다.
fn packet_items(source: &PacketSource, chosen: &[Chosen]) -> Vec<PacketItem> {
    let mut items: Vec<PacketItem> = source
        .open_items
        .iter()
        .map(|item| PacketItem {
            zone: PacketZone::Open,
            seq: item.seq,
            form: Some(ItemForm::Full),
            reason: None,
        })
        .collect();
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

// cost: time O(t log t), heap O(t), stack O(1)
// vars: t = 턴 수
// basis: estimate
/// 기록 번호 순의 턴. 같은 번호면 들어온 순서를 지킨다.
fn sorted_turns(turns: &[Turn]) -> Vec<&Turn> {
    let mut sorted: Vec<&Turn> = turns.iter().collect();
    sorted.sort_by_key(|turn| turn.seq);
    sorted
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 고정 구역 글자 수
// basis: estimate
/// 제약 칸, 남은 일, 대화 본문. 대화는 줄이거나 뺀 것 없이 모두 넣는다.
fn fixed_sections(source: &PacketSource) -> Vec<Section> {
    let turns = sorted_turns(&source.turns)
        .into_iter()
        .map(|turn| SectionItem {
            session: Some(turn.stamp.session),
            text: turn.render(),
        })
        .collect();
    vec![
        Section {
            title: "Constraints and decisions",
            items: constraint_items(source),
        },
        Section {
            title: "Open items",
            items: by_seq(&source.open_items),
        },
        Section {
            title: "Conversation",
            items: turns,
        },
        Section {
            title: RELATED_TITLE,
            items: source
                .related
                .iter()
                .map(|record| SectionItem {
                    session: None,
                    text: record.render(),
                })
                .collect(),
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

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 고정 구역 글자 수
// basis: estimate
/// 관련 원문을 넣을 수 있는 글자 수. 목표 예산 `P_max`와 전송 상한 `P_send` 중 작은 값에서
/// 고정 구역과 관련 원문 구역 제목을 뺀 값이다. 선택 원문 때문에 보내던 패킷이 보류되지 않게 한다.
/// `source.related`는 비어 있어야 한다.
pub fn related_room_chars(source: &PacketSource, budget: &ContextBudget) -> usize {
    let fixed = fixed_zone(source).chars().count();
    let title = format!("## {RELATED_TITLE}{ITEM_SEPARATOR}")
        .chars()
        .count();
    to_chars(budget.packet_limit().min(budget.send_limit())).saturating_sub(fixed + title)
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

fn competing_header() -> String {
    format!("## {COMPETING_TITLE}{ITEM_SEPARATOR}")
}

// cost: time O(L), heap O(1), stack O(1)
// vars: L = 글자 수
// basis: estimate
/// 전송 가능 여부를 가리는 보수적 추정. ASCII는 `estimate_tokens`처럼 4자에 1토큰, 그 밖의 글자(한글 등)는 글자마다 1토큰으로 센다.
/// provider 계수가 아니므로 이 값이 한도 안이어도 provider가 거절할 수 있다.
fn estimate_send_tokens(text: &str) -> u64 {
    let ascii = text.chars().filter(char::is_ascii).count();
    let other = text.chars().count() - ascii;
    let tokens = ascii.div_ceil(CHARS_PER_TOKEN) + other;
    u64::try_from(tokens).expect("token count should fit in u64")
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
