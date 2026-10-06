//! 기록 목록에서 새 session에 넘기는 패킷과, 돌아온 session에 붙이는 변경분을 만든다.
//! 설계: docs/design/context-management.md#패킷-구성, docs/design/providers-and-sessions.md

use std::collections::HashMap;

use saturn_core::constraints::scope_of;
use saturn_core::routers::CompactCandidate;
use saturn_core::sessions::changes::describe;
use saturn_core::sessions::constraint_slot::ConstraintTier;
use saturn_core::sessions::constraint_slot::{
    ConstraintSlot, SlotConstraint, SlotContext, fill_constraint_slot,
};
use saturn_core::sessions::context::ContextBudget;
use saturn_core::sessions::memo::{INTERRUPTED_RESULT, ToolKind, tool_memo};
use saturn_core::sessions::packet::{
    CONSTRAINT_SEPARATOR_CHARS, CompetingItem, Entry, Message, PacketItem, PacketOutcome,
    PacketSource, PacketZone, Protected, Role, Turn, TurnStatus, build_packet,
    constraint_cap_chars, fixed_zone, leads_with_fixed_zone, reduce_packet,
};
use saturn_core::sessions::ranking::{Candidate, order_after_router, rank_candidates};
use saturn_core::sessions::stamp::Stamp;
use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory, ToolDetail};
use saturn_protocol::ids::{ChatId, ConstraintId, InputId, LedgerSeq, RunId, SessionId};
use saturn_protocol::rpc::EvidenceKind;
use saturn_protocol::state::InputState;

use crate::Engine;
use crate::store::{
    ConstraintState, ExceptionKind, LedgerRow, PacketId, PacketItemRow, RunChanges, RunEnd,
    SteeredInput, StoredConstraint, StoredException, sha256_hex,
};
use crate::switch::Reduction;

/// 제약 기준 파일과 도구 후보 순위가 보는 최근 턴 수. 대화 본문을 싣는 범위가 아니다.
const REFERENCE_TURNS: usize = 3;

/// 새 session의 첫 턴으로 보내는 글.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    pub(crate) text: String,
    /// 추정 토큰 수.
    pub(crate) tokens: u64,
    /// 고정 구역이 `P_max`를 넘어 `P_send`까지 허용했다.
    pub(crate) is_over_limit: bool,
    /// 기록 번호가 있는 재료 항목마다 들어갔는지.
    pub(crate) items: Vec<PacketItem>,
}

/// 보내는 패킷 한 시도의 근거 재료. 전달 패킷 기록(`handoff_packets`)이 이 값으로 항목을 남긴다.
#[derive(Debug, Clone)]
pub(crate) struct PacketEvidence {
    items: Vec<PacketItem>,
    /// 제약 칸 후보마다의 단계. 칸이 차서 빠진 제약은 `Omitted`다.
    constraints: Vec<(ConstraintId, ConstraintTier)>,
    /// 줄이거나 빼지 않고 실은 대화 본문. 실제 순서다.
    protected: Vec<Protected>,
    /// 패킷을 만들 때 직렬화한 고정 구역(제약 칸, 남은 일, 대화 본문). 보낼 글이 이 글로 시작해야 한다.
    fixed: String,
    /// 패킷을 만든 시점의 전체 글 해시. 보낼 글이 만든 글과 같은지 가린다.
    text_sha: String,
    /// 패킷이 담은 기록의 마지막 번호.
    pub(crate) up_to: LedgerSeq,
    pub(crate) tokens: u64,
    /// 첫 시도가 1이고 맥락 한도로 거절돼 줄여 다시 보낼 때마다 1 늘어난다.
    pub(crate) attempt: u32,
    pub(crate) reduced_from: Option<PacketId>,
    /// 경쟁 구역을 고른 방식. `RANK_SELECTOR`나 `COMPACT_SELECTOR`다.
    selector: &'static str,
    /// 관련 원문 선주입(`context.select.related`)을 시도했으면 그 근거. 꺼져 있으면 `None`이다.
    related: Option<RelatedLog>,
}

/// 관련 원문 선주입이 항목을 고른 방식. 근거 검색과 같은 후보 집합의 RRF 순위다.
pub(crate) const RELATED_SELECTOR: &str = "related_rank";

/// 관련 원문 한 건의 종류, 번호, 원문 해시. `종류:채팅:번호:해시` 참조의 재료다.
pub(crate) type RelatedRef = (EvidenceKind, u64, String);

/// 관련 원문 선주입의 근거: 후보를 읽은 때의 채팅 revision, 패킷에 든 기록, 못 든 기록과 이유.
/// 한 건에 속하지 않는 이유(`no_candidates`, `retrieval_failed`)는 기록을 가리키지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelatedLog {
    pub(crate) tail: LedgerSeq,
    pub(crate) picked: Vec<RelatedRef>,
    pub(crate) omitted: Vec<(Option<RelatedRef>, &'static str)>,
}

impl RelatedLog {
    /// 고른 기록 없이 `reason`으로 모두 못 넣었다.
    pub(crate) fn none(tail: LedgerSeq, reason: &'static str) -> Self {
        Self {
            tail,
            picked: Vec::new(),
            omitted: vec![(None, reason)],
        }
    }

    /// 고른 기록 전부를 `reason`으로 뺀 근거. 이미 못 든 기록은 그대로 둔다.
    pub(crate) fn all_dropped(&self, reason: &'static str) -> Self {
        let verdicts = vec![Some(reason); self.picked.len()];
        self.after_recheck(&verdicts)
    }

    /// 적용 직전 재검사 결과로 고친 근거. `verdicts`는 `picked`와 같은 순서이고 `Some`이면 그 이유로 뺀다.
    pub(crate) fn after_recheck(&self, verdicts: &[Option<&'static str>]) -> Self {
        let mut kept = Vec::new();
        let mut omitted = self.omitted.clone();
        for (found, verdict) in self.picked.iter().zip(verdicts) {
            match verdict {
                None => kept.push(found.clone()),
                Some(reason) => omitted.push((Some(found.clone()), *reason)),
            }
        }
        Self {
            tail: self.tail,
            picked: kept,
            omitted,
        }
    }

    fn rows(&self) -> Vec<PacketItemRow> {
        let row = |found: &Option<RelatedRef>, reason: Option<&'static str>| match found {
            Some((kind, id, hash)) => PacketItemRow {
                zone: format!("Related-{}", kind.name()),
                ref_id: *id,
                selector: RELATED_SELECTOR,
                form: reason.is_none().then(|| "Full".to_owned()),
                reason,
                hash: Some(hash.clone()),
            },
            None => PacketItemRow {
                zone: "Related".to_owned(),
                ref_id: self.tail.0,
                selector: RELATED_SELECTOR,
                form: None,
                reason,
                hash: None,
            },
        };
        self.picked
            .iter()
            .map(|found| row(&Some(found.clone()), None))
            .chain(
                self.omitted
                    .iter()
                    .map(|(found, reason)| row(found, Some(*reason))),
            )
            .collect()
    }
}

/// 경쟁 구역을 후보 순위(RRF) 순서로 채웠다.
pub(crate) const RANK_SELECTOR: &str = "rank";

/// 경쟁 구역을 router `compact` 판단의 남김 확률 순서로 채웠다. 전달 패킷 기록의 `selector` 값이다.
pub(crate) const COMPACT_SELECTOR: &str = "compact";

impl PacketEvidence {
    /// 재료에서 처음 만든 패킷의 근거. `selector`는 경쟁 구역을 고른 방식이다.
    pub(crate) fn first(handoff: &Handoff, source: &PacketSource, selector: &'static str) -> Self {
        Self {
            selector,
            items: handoff.items.clone(),
            constraints: source.constraint_tiers.clone(),
            protected: source.protected(),
            fixed: fixed_zone(source),
            text_sha: sha256_hex(handoff.text.as_bytes()),
            up_to: source.up_to,
            tokens: handoff.tokens,
            attempt: 1,
            reduced_from: None,
            related: None,
        }
    }

    /// 관련 원문 선주입의 근거를 붙인다.
    pub(crate) fn with_related(self, related: Option<RelatedLog>) -> Self {
        Self { related, ..self }
    }

    pub(crate) fn related(&self) -> Option<&RelatedLog> {
        self.related.as_ref()
    }

    /// 거절된 시도 `previous`를 줄여 다시 만든 패킷의 근거. 같은 재료에서 항목만 다시 골랐다.
    /// 관련 원문은 줄이지 않고 통째로 빼므로 `reduction.source`가 아니라 관련 원문을 뺀 재료와 맞춘다.
    pub(crate) fn reduced(
        handoff: &Handoff,
        reduction: &Reduction,
        (attempt, previous): (u32, Option<PacketId>),
        related: Option<&RelatedLog>,
    ) -> Self {
        Self {
            attempt: attempt + 1,
            reduced_from: previous,
            related: related.map(|log| log.all_dropped("reduced")),
            ..Self::first(
                handoff,
                &reduction.source_without_related(),
                reduction.selector,
            )
        }
    }

    /// 재료 없이 만든 근거. 시험이 보낼 수 없는 호출의 자리를 채운다.
    #[cfg(test)]
    pub(crate) fn empty(up_to: LedgerSeq) -> Self {
        Self {
            items: Vec::new(),
            constraints: Vec::new(),
            protected: Vec::new(),
            fixed: String::new(),
            text_sha: String::new(),
            up_to,
            tokens: 0,
            attempt: 1,
            reduced_from: None,
            selector: RANK_SELECTOR,
            related: None,
        }
    }

    /// 제약 칸 후보마다의 단계. `Omitted` 포함, 제약 번호 순이다.
    pub(crate) fn tiers(&self) -> Vec<(ConstraintId, ConstraintTier)> {
        self.constraints.clone()
    }

    /// 보낼 글이 만든 패킷과 같은 글이고, 고정 구역의 대화 본문이 줄거나 빠지거나 옮겨지지 않았는지. 아니면 보내지 않는다.
    pub(crate) fn carries_dialogue(&self, body: &str) -> bool {
        sha256_hex(body.as_bytes()) == self.text_sha && leads_with_fixed_zone(&self.fixed, body)
    }

    /// 항목 행. 제약 칸, 대화 본문, 열린 항목, 경쟁 구역 순이다. 대화 본문은 역할, 번호, 원문 해시를 남기고 행 순서가 실제 순서다.
    pub(crate) fn rows(&self) -> Vec<PacketItemRow> {
        let constraints = self.constraints.iter().map(|(id, tier)| {
            let is_omitted = *tier == ConstraintTier::Omitted;
            PacketItemRow {
                zone: "Constraints".to_owned(),
                ref_id: id.0,
                selector: tier.name(),
                form: (!is_omitted).then(|| tier.name().to_owned()),
                reason: is_omitted.then_some("slot_full"),
                hash: None,
            }
        });
        let dialogue = self.protected.iter().map(|item| PacketItemRow {
            zone: item.role.name().to_owned(),
            ref_id: item.id,
            selector: "protected",
            form: Some("Full".to_owned()),
            reason: None,
            hash: Some(sha256_hex(item.text.as_bytes())),
        });
        let items = self.items.iter().map(|item| PacketItemRow {
            zone: item.zone.name().to_owned(),
            ref_id: item.seq.0,
            selector: match item.zone {
                PacketZone::Open => "pending",
                PacketZone::Competing => self.selector,
            },
            form: item.form.map(|form| form.name().to_owned()),
            reason: item.reason,
            hash: None,
        });
        let related = self.related.iter().flat_map(RelatedLog::rows);
        constraints
            .chain(dialogue)
            .chain(related)
            .chain(items)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandoffOutcome {
    Ready(Handoff),
    /// 넘길 기록이 없다.
    Empty,
    /// 고정 구역이 `P_send`도 넘어 새 session으로 옮기지 않는다.
    Deferred {
        constraints: Vec<String>,
    },
}

/// 예외가 걸린 제약은 규칙 뒤에 예외 표기를 붙여 새 session이 규칙을 그대로 지키거나 제약이 없는 줄 아는 일을 막는다.
/// 표기는 제약의 크기에 세므로 표기까지 들어가지 않으면 그 제약은 칸에서 건너뛴다.
fn with_exception_note(constraint: &StoredConstraint) -> String {
    match &constraint.exception {
        None => constraint.rule.clone(),
        Some(StoredException {
            kind: ExceptionKind::Once,
            ..
        }) => format!("{} [paused for the current task]", constraint.rule),
        Some(StoredException {
            kind: ExceptionKind::Scoped,
            condition,
        }) => format!(
            "{} [exception: {}]",
            constraint.rule,
            condition.as_deref().unwrap_or_default()
        ),
    }
}

// cost: time O(c log c + c·(s·r + w)), heap O(c), stack O(1)
// vars: c = 제약 수, s = 범위 경로 수, r = 기준 경로 수, w = 규칙 단어 수
// basis: estimate
/// 유효 제약으로 제약 칸을 채운다. 지금 작업의 기준 파일은 마지막 입력에 나온 경로와 최근 3턴이 건드린 파일이다.
/// 제약 순위의 기준일 뿐 대화 본문을 싣는 범위가 아니다.
fn constraint_slot(
    constraints: &[StoredConstraint],
    turns: &[Turn],
    changes: &[RunChanges],
    budget: &ContextBudget,
) -> ConstraintSlot {
    let mut valid: Vec<SlotConstraint> = constraints
        .iter()
        .filter(|constraint| constraint.state != ConstraintState::Released)
        .map(|constraint| SlotConstraint {
            id: constraint.id,
            rule: with_exception_note(constraint),
            scope: constraint.scope.clone(),
        })
        .collect();
    valid.sort_by_key(|constraint| constraint.id);
    let last_input = turns
        .iter()
        .max_by_key(|turn| turn.seq)
        .map_or("", opening_input);
    let mut recent: Vec<&RunChanges> = changes.iter().collect();
    recent.sort_by_key(|run| run.seq);
    let start = recent.len().saturating_sub(REFERENCE_TURNS);
    let reference: Vec<String> = scope_of(last_input)
        .into_iter()
        .chain(
            recent[start..]
                .iter()
                .flat_map(|run| run.set.files.iter().map(|file| file.path.clone())),
        )
        .collect();
    fill_constraint_slot(
        &valid,
        SlotContext {
            reference_paths: &reference,
            last_input,
        },
        constraint_cap_chars(budget),
        CONSTRAINT_SEPARATOR_CHARS,
    )
}

/// 한 도구 호출과 그 결과. 결과가 없으면 중단돼 결과를 모르는 호출이다.
struct Tool {
    seq: LedgerSeq,
    stamp: Stamp,
    title: String,
    kind: ToolKind,
    path: Option<String>,
    files: Vec<String>,
    output: Option<String>,
}

// cost: time O(L + c log c), heap O(L), stack O(1)
// vars: L = 기록 글자 수, c = 도구 호출 수
// basis: estimate
/// 도구 후보의 순서는 후보 순위(RRF)만 쓴다. 대화 본문은 기록된 사용자 입력, 적용된 끼워 넣은 입력, 에이전트 글 전부를 순위와 개수에 상관없이 실제 순서로 싣고,
/// 남은 일 칸은 대기·보류 입력과 결과를 모르는 작업, 결과 없는 도구 호출이다.
/// 제약 칸에는 해제되지 않은 저장 제약을 상한(`C_max`) 안에서 넣는다(docs/design/constraints.md#패킷의-제약-칸). 예외가 걸린 제약은 규칙 뒤에 예외 표기를 붙여 넣는다.
/// 넘길 기록이 없으면 `None`.
pub(crate) fn handoff_source(
    rows: &[LedgerRow],
    steers: &[SteeredInput],
    changes: &[RunChanges],
    pending: &Pending,
    materials: (&[StoredConstraint], &[String]),
    budget: &ContextBudget,
) -> Option<PacketSource> {
    handoff_source_ordered(rows, steers, changes, pending, materials, budget, None)
}

/// `handoff_source`에 router `compact` 판단을 더한 것. `verdicts`가 있으면 경쟁 구역의 도구 항목을 남김 확률 순으로
/// 두고(같은 확률이면 RRF 순, 답이 없는 항목은 뒤에 RRF 순), 없으면 RRF 순이다. 수정 파일 항목은 어느 쪽이든 맨 앞이다.
pub(crate) fn handoff_source_ordered(
    rows: &[LedgerRow],
    steers: &[SteeredInput],
    changes: &[RunChanges],
    pending: &Pending,
    (constraints, provider_docs): (&[StoredConstraint], &[String]),
    budget: &ContextBudget,
    verdicts: Option<&[(LedgerSeq, f64)]>,
) -> Option<PacketSource> {
    let last = rows.last()?;
    // 실행의 이벤트가 이 재료에 없으면(다른 session이 낸 실행을 뺀 변경분 등) 그 실행에 끼운 입력도 뺀다
    let steers: Vec<&SteeredInput> = steers
        .iter()
        .filter(|steer| rows.iter().any(|row| row.run == steer.run))
        .collect();
    let turns = turns(rows, &steers);
    let tools = tools(rows);
    let mut open = pending.entries(last.seq);
    open.extend(open_items(&tools));
    let slot = constraint_slot(constraints, &turns, changes, budget);
    Some(PacketSource {
        constraints: slot.included.into_iter().map(|(_, rule)| rule).collect(),
        constraints_omitted: slot.omitted.into_iter().map(|(_, rule)| rule).collect(),
        constraint_tiers: slot.tiers,
        open_items: open,
        competitors: changed_files_items(changes)
            .into_iter()
            .chain(ordered_competitors(&tools, &turns, budget.rrf_k, verdicts))
            .collect(),
        turns,
        related: Vec::new(),
        provider_docs: provider_docs.to_vec(),
        evidence_lookup: budget.evidence_lookup,
        up_to: last.seq,
    })
}

/// 새 작업으로 열 session의 패킷 재료. 앞 맥락 없이 시작하는 것이 설계라 기록은 넣지 않고 제약 칸만 채운다
/// (docs/design/constraints.md#새-작업-session의-제약). 넣을 제약이 없으면 `None`.
pub(crate) fn constraint_only_source(
    constraints: &[StoredConstraint],
    budget: &ContextBudget,
) -> Option<PacketSource> {
    let slot = constraint_slot(constraints, &[], &[], budget);
    if slot.included.is_empty() && slot.omitted.is_empty() {
        return None;
    }
    Some(PacketSource {
        constraints: slot.included.into_iter().map(|(_, rule)| rule).collect(),
        constraints_omitted: slot.omitted.into_iter().map(|(_, rule)| rule).collect(),
        constraint_tiers: slot.tiers,
        evidence_lookup: false,
        ..PacketSource::default()
    })
}

pub(crate) fn build_handoff(
    rows: &[LedgerRow],
    steers: &[SteeredInput],
    changes: &[RunChanges],
    pending: &Pending,
    (constraints, provider_docs): (&[StoredConstraint], &[String]),
    budget: &ContextBudget,
) -> HandoffOutcome {
    match handoff_source(
        rows,
        steers,
        changes,
        pending,
        (constraints, provider_docs),
        budget,
    ) {
        Some(source) => handoff_of(&source, budget),
        None => HandoffOutcome::Empty,
    }
}

// cost: time O(t·L + m log m), heap O(L), stack O(1)
// vars: t = 최근 턴 수(3 이하), L = 패킷 재료 글자 수, m = 경쟁 항목 수
// basis: estimate
pub(crate) fn handoff_of(source: &PacketSource, budget: &ContextBudget) -> HandoffOutcome {
    match build_packet(source, budget) {
        PacketOutcome::Ready(packet) => HandoffOutcome::Ready(Handoff {
            text: packet.text,
            tokens: packet.tokens,
            is_over_limit: packet.is_over_limit,
            items: packet.items,
        }),
        PacketOutcome::Deferred { constraints } => HandoffOutcome::Deferred { constraints },
    }
}

/// 거절된 패킷을 줄인 것. 고정 구역만으로 `target_tokens`를 넘으면 `None`이다.
pub(crate) fn reduce_handoff(
    source: &PacketSource,
    budget: &ContextBudget,
    target_tokens: u64,
) -> Option<Handoff> {
    let packet = reduce_packet(source, budget, target_tokens)?;
    Some(Handoff {
        text: packet.text,
        tokens: packet.tokens,
        is_over_limit: packet.is_over_limit,
        items: packet.items,
    })
}

/// router `compact`로 묻는 후보 전체와 `state`에 쓸 사용자 입력(오래된 순). 후보는 경쟁 구역에 들어갈 수 있는 도구 호출 전체다.
pub(crate) fn compact_material(rows: &[LedgerRow]) -> (Vec<CompactCandidate>, Vec<String>) {
    let candidates = tools(rows)
        .iter()
        .map(|tool| CompactCandidate {
            seq: tool.seq,
            call: tool.title.clone(),
            result: tool
                .output
                .clone()
                .unwrap_or_else(|| INTERRUPTED_RESULT.to_owned()),
        })
        .collect();
    let inputs = turns(rows, &[])
        .iter()
        .map(|turn| opening_input(turn).to_owned())
        .collect();
    (candidates, inputs)
}

/// 아직 기록에 실행이 없는 입력과 결과를 모르는 작업의 입력 원문. 남은 일 칸 재료다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Pending {
    /// 대기(`Queued`) 입력. 접수 순서.
    pub(crate) waiting: Vec<String>,
    /// 보류(`Held`) 입력. 접수 순서.
    pub(crate) held: Vec<String>,
    /// 보낸 뒤 결과를 모르는(`NeedsCheck`) 작업의 입력. 입력 번호 순서.
    pub(crate) unchecked: Vec<String>,
}

impl Pending {
    /// 기록 번호가 없는 항목이라 모두 `at`에 둔다. 같은 번호 안에서는 대기, 보류, 결과 모름 순서다.
    fn entries(&self, at: LedgerSeq) -> Vec<Entry> {
        let labeled = [("Queued input", &self.waiting), ("Held input", &self.held)];
        labeled
            .into_iter()
            .flat_map(|(label, texts)| texts.iter().map(move |text| (label, text)))
            .map(|(label, text)| Entry {
                seq: at,
                text: format!("{label}: {text}"),
            })
            .chain(self.unchecked.iter().map(|text| Entry {
                seq: at,
                text: interrupted_text("Input", text),
            }))
            .collect()
    }
}

impl Engine {
    /// 패킷을 만들 재료 전체: 기록 행, 끼워 넣어 적용한 입력, 실행별 수정 파일.
    ///
    /// # Errors
    /// 기록 저장소를 읽지 못하면 `Store`.
    pub(crate) async fn packet_material(
        &self,
        chat: ChatId,
    ) -> Result<(Vec<LedgerRow>, Vec<SteeredInput>, Vec<RunChanges>), crate::EngineError> {
        Ok((
            self.store.ledger_since(chat, LedgerSeq(0)).await?,
            self.store.steered_inputs(chat).await?,
            self.store.run_changes(chat).await?,
        ))
    }

    /// 채팅의 대기·보류 입력과 결과를 모르는 작업의 입력. `sending`은 지금 보내는 입력이라 뺀다.
    pub(crate) fn pending_work(&self, chat: ChatId, sending: Option<InputId>) -> Pending {
        let texts = |state| -> Vec<String> {
            self.queue
                .inputs_in_state(chat, state)
                .into_iter()
                .filter(|id| Some(*id) != sending)
                .filter_map(|id| self.queue.input(id))
                .map(|input| input.text.clone())
                .collect()
        };
        let mut checks: Vec<InputId> = self
            .flow
            .needs_check
            .values()
            .filter(|check| check.chat == chat)
            .map(|check| check.input)
            .collect();
        checks.sort();
        Pending {
            waiting: texts(InputState::Queued),
            held: texts(InputState::Held),
            unchecked: checks
                .into_iter()
                .filter_map(|id| self.queue.input(id))
                .map(|input| input.text.clone())
                .collect(),
        }
    }
}

/// 실행이 끝난 방식에서 입력의 상태를 정한다. 정상 완료만 끝난 일이고 실패나 멈춤은 일부만 실행됐을 수 있다.
fn status_of(end: Option<RunEnd>) -> TurnStatus {
    match end {
        Some(RunEnd::Completed) => TurnStatus::Finished,
        None => TurnStatus::InProgress,
        Some(RunEnd::Failed | RunEnd::Stopped) => TurnStatus::ResultUnknown,
    }
}

/// 턴을 연 사용자 입력 원문.
fn opening_input(turn: &Turn) -> &str {
    turn.messages
        .first()
        .map_or("", |message| message.text.as_str())
}

// cost: time O(r + s·r_run), heap O(L), stack O(1)
// vars: r = 기록 행 수, s = 끼워 넣은 입력 수, r_run = 한 실행의 행 수, L = 대화 글자 수
// basis: estimate
/// 입력이 있는 실행마다 턴 하나. 기록 번호는 그 실행의 첫 이벤트 번호다. 본문은 실제 순서로, 실행을 연 입력, 메인 에이전트 글, 끼워 넣어 적용한 입력이다.
/// 끼워 넣은 입력은 적용 때 쌓여 있던 기록 번호 뒤에 들어가고 같은 번호 안에서는 적용한 순서를 지킨다. 같은 글자의 입력도 합치지 않는다.
fn turns(rows: &[LedgerRow], steers: &[&SteeredInput]) -> Vec<Turn> {
    let mut order: Vec<RunId> = Vec::new();
    let mut turns: HashMap<RunId, (Turn, Vec<&SteeredInput>)> = HashMap::new();
    for row in rows {
        let Some(input) = &row.input else {
            continue;
        };
        let (turn, waiting) = turns.entry(row.run).or_insert_with(|| {
            order.push(row.run);
            let turn = Turn {
                seq: row.seq,
                stamp: stamp_of(row),
                status: status_of(row.end),
                messages: vec![Message {
                    role: Role::User,
                    id: row.seq.0,
                    text: input.clone(),
                }],
            };
            let waiting = steers
                .iter()
                .filter(|steer| steer.run == row.run)
                .copied()
                .collect();
            (turn, waiting)
        });
        place_steers(turn, waiting, |steer| steer.after < row.seq);
        if let ProviderEvent::Text {
            subagent: None,
            text,
            ..
        } = &row.event
        {
            // provider가 조각으로 낸 답도 기록 이벤트마다 번호와 해시를 남기려고 합치지 않는다. 글에서는 core가 이어 붙여 보인다
            turn.messages.push(Message {
                role: Role::Assistant,
                id: row.seq.0,
                text: text.clone(),
            });
        }
    }
    order
        .into_iter()
        .filter_map(|run| turns.remove(&run))
        .map(|(mut turn, mut waiting)| {
            place_steers(&mut turn, &mut waiting, |_| true);
            turn
        })
        .collect()
}

/// 기다리던 끼워 넣은 입력 중 `is_due`인 것을 적용한 순서대로 턴 끝에 붙인다.
fn place_steers(
    turn: &mut Turn,
    waiting: &mut Vec<&SteeredInput>,
    is_due: impl Fn(&SteeredInput) -> bool,
) {
    let (due, rest): (Vec<&SteeredInput>, Vec<&SteeredInput>) =
        waiting.iter().copied().partition(|steer| is_due(steer));
    *waiting = rest;
    turn.messages.extend(due.into_iter().map(|steer| Message {
        role: Role::Steer,
        id: steer.input.0,
        text: steer.text.clone(),
    }));
}

fn tools(rows: &[LedgerRow]) -> Vec<Tool> {
    let results: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolResult {
                call_id, output, ..
            } => Some((call_id.as_str(), output.as_str())),
            _ => None,
        })
        .collect();
    let exit_codes: HashMap<&str, Option<i32>> = rows
        .iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolResult {
                call_id, exit_code, ..
            } => Some((call_id.as_str(), *exit_code)),
            _ => None,
        })
        .collect();
    rows.iter()
        .filter_map(|row| match &row.event {
            ProviderEvent::ToolCall {
                call_id,
                activity,
                detail,
                ..
            } if detail.category.is_candidate() => Some(tool_of(
                row,
                activity,
                detail,
                results.get(call_id.as_str()).copied(),
                exit_codes.get(call_id.as_str()).copied().flatten(),
            )),
            _ => None,
        })
        .collect()
}

fn tool_of(
    row: &LedgerRow,
    activity: &Activity,
    detail: &ToolDetail,
    output: Option<&str>,
    exit_code: Option<i32>,
) -> Tool {
    let path = detail.paths.first().cloned();
    let (title, command) = match activity {
        Activity::RunningCommand { command } => (format!("Run {command}"), Some(command.clone())),
        _ if detail.paths.is_empty() => (format!("{:?}", detail.category), None),
        _ => (
            format!("{:?} {}", detail.category, detail.paths.join(" ")),
            None,
        ),
    };
    Tool {
        seq: row.seq,
        stamp: stamp_of(row),
        title,
        kind: kind_of(detail, path.as_ref(), command, exit_code),
        path,
        files: detail.paths.clone(),
        output: output.map(str::to_owned),
    }
}

/// 값을 얻은 도구 종류만 메모 틀을 쓰고 나머지는 `Other`다. 테스트 통과 수는 구조로 얻지 못해 `TestRun`을 쓰지 않는다.
fn kind_of(
    detail: &ToolDetail,
    path: Option<&String>,
    command: Option<String>,
    exit_code: Option<i32>,
) -> ToolKind {
    match (detail.category, path, command) {
        (ToolCategory::FileRead, Some(path), _) => ToolKind::FileRead {
            path: path.clone(),
            lines: detail.read_lines.map(|range| (range.first, range.last)),
        },
        (ToolCategory::FileEdit, Some(path), _) => match detail.changed {
            Some(change) => ToolKind::FileEdit {
                path: path.clone(),
                added: change.added,
                removed: change.removed,
            },
            None => ToolKind::Other,
        },
        (ToolCategory::Shell, _, Some(command)) => ToolKind::Shell { command, exit_code },
        _ => ToolKind::Other,
    }
}

fn open_items(tools: &[Tool]) -> Vec<Entry> {
    tools
        .iter()
        .filter(|tool| tool.output.is_none())
        .map(|tool| Entry {
            seq: tool.seq,
            text: interrupted_text("Tool call", &tool.title),
        })
        .collect()
}

/// 결과를 모르는 항목은 별도 경고 문장 없이 결과 자리에 오류 결과를 둔다.
fn interrupted_text(label: &str, subject: &str) -> String {
    format!("{label}: {subject}\nResult (error): {INTERRUPTED_RESULT}")
}

fn ordered_competitors(
    tools: &[Tool],
    turns: &[Turn],
    rrf_k: u32,
    verdicts: Option<&[(LedgerSeq, f64)]>,
) -> Vec<CompetingItem> {
    let last_input = turns.last().map_or("", opening_input);
    let first_recent = turns
        .len()
        .checked_sub(REFERENCE_TURNS)
        .map_or(0, |skip| turns[skip].seq.0);
    let base_files: Vec<String> = tools
        .iter()
        .filter(|tool| tool.seq.0 >= first_recent)
        .flat_map(|tool| tool.files.clone())
        .collect();
    let candidates: Vec<Candidate> = tools
        .iter()
        .map(|tool| Candidate {
            seq: tool.seq,
            text: item_text(tool),
            files: tool.files.clone(),
        })
        .collect();
    let ranked = rank_candidates(&candidates, &base_files, last_input, rrf_k);
    let ordered = match verdicts {
        Some(verdicts) => order_after_router(&ranked, verdicts),
        None => ranked,
    };
    ordered
        .into_iter()
        .filter_map(|seq| tools.iter().find(|tool| tool.seq == seq))
        .map(|tool| CompetingItem {
            seq: tool.seq,
            stamp: tool.stamp,
            text: item_text(tool),
            memo: tool.output.as_deref().map_or_else(
                || INTERRUPTED_RESULT.to_owned(),
                |output| tool_memo(&tool.kind, output),
            ),
            path: tool.path.clone(),
        })
        .collect()
}

/// 실행마다 폴더 상태 차이로 센 수정 파일 목록을 경쟁 항목으로 만든다. 도구 호출 이벤트에 없는 수정(셸 명령, 자식 프로세스)도 담긴다.
/// 도구 순위와 따로 맨 앞에 두어 예산이 모자라도 먼저 들어간다. 바뀐 파일이 없는 실행은 항목이 없다.
fn changed_files_items(changes: &[RunChanges]) -> Vec<CompetingItem> {
    changes
        .iter()
        .filter_map(|run| {
            let text = describe(&run.set)?;
            Some(CompetingItem {
                seq: run.seq,
                stamp: Stamp {
                    session: run.session,
                    at_ms: Some(run.at_ms),
                },
                text: format!("Files changed in this run: {text}"),
                memo: format!("Files changed in this run: {} files", run.set.files.len()),
                path: run.set.files.first().map(|change| change.path.clone()),
            })
        })
        .collect()
}

/// 기록 번호 뒤의 변경분 중 그 session이 낸 실행의 수정 파일 목록은 이미 알고 있으므로 뺀다.
pub(crate) fn changes_of_others(
    changes: Vec<RunChanges>,
    session: SessionId,
    after: LedgerSeq,
) -> Vec<RunChanges> {
    changes
        .into_iter()
        .filter(|run| run.session != session && run.seq > after)
        .collect()
}

/// 패킷의 경쟁 구역 후보와 같은 도구 호출 하나. `text`는 패킷이 넣는 원문과 같다.
#[derive(Debug, Clone)]
pub(crate) struct ToolRecord {
    pub(crate) seq: LedgerSeq,
    pub(crate) at_ms: i64,
    pub(crate) text: String,
    pub(crate) files: Vec<String>,
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 기록 글자 수
// basis: estimate
/// 근거 검색과 원문 조회가 패킷과 같은 후보와 같은 원문을 쓰게 한다.
pub(crate) fn tool_records(rows: &[LedgerRow]) -> Vec<ToolRecord> {
    tools(rows)
        .into_iter()
        .map(|tool| ToolRecord {
            seq: tool.seq,
            at_ms: tool.stamp.at_ms.unwrap_or_default(),
            text: item_text(&tool),
            files: tool.files,
        })
        .collect()
}

fn item_text(tool: &Tool) -> String {
    format!(
        "{}\n{}",
        tool.title,
        tool.output.as_deref().unwrap_or(INTERRUPTED_RESULT)
    )
}

fn stamp_of(row: &LedgerRow) -> Stamp {
    Stamp {
        session: row.session,
        at_ms: Some(row.at_ms),
    }
}

/// 기록 번호 뒤의 변경분 중 그 session이 낸 것은 이미 받았으므로 뺀다.
pub(crate) fn others_only(rows: Vec<LedgerRow>, session: SessionId) -> Vec<LedgerRow> {
    rows.into_iter()
        .filter(|row| row.session != session)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use saturn_core::sessions::changes::{ChangeKind, ChangeSet, FileChange};
    use saturn_core::sessions::context::{
        DEFAULT_CONSTRAINT_SLOT_PERCENT, DEFAULT_ITEM_CAP_PERCENT,
    };
    use saturn_core::sessions::ranking::DEFAULT_RRF_K;
    use saturn_protocol::event::LineRange;
    use saturn_protocol::ids::{AgentId, TaskId};

    use super::*;

    fn text_event(agent: AgentId, text: &str) -> ProviderEvent {
        ProviderEvent::Text {
            agent,
            subagent: None,
            text: text.to_owned(),
        }
    }

    fn budget() -> ContextBudget {
        ContextBudget {
            t_abs: 10_000,
            safety_percent: 100,
            window: 200_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
            item_cap_percent: DEFAULT_ITEM_CAP_PERCENT,
            constraint_slot_percent: DEFAULT_CONSTRAINT_SLOT_PERCENT,
            rrf_k: DEFAULT_RRF_K,
            evidence_lookup: false,
        }
    }

    fn row(
        seq: u64,
        run: u64,
        session: u64,
        input: Option<&str>,
        event: ProviderEvent,
    ) -> LedgerRow {
        LedgerRow {
            seq: LedgerSeq(seq),
            run: RunId(run),
            session: SessionId(session),
            task: TaskId(1),
            input: input.map(str::to_owned),
            is_amendment: false,
            end: Some(RunEnd::Completed),
            at_ms: 1_700_000_000_000,
            event,
        }
    }

    fn in_task(task: u64, mut row: LedgerRow) -> LedgerRow {
        row.task = TaskId(task);
        row
    }

    fn handoff_text(rows: &[LedgerRow], pending: &Pending) -> String {
        let HandoffOutcome::Ready(handoff) =
            build_handoff(rows, &[], &[], pending, (&[], &[]), &budget())
        else {
            panic!("packet should be ready");
        };
        handoff.text
    }

    fn read_call(call: &str, path: &str) -> ProviderEvent {
        ProviderEvent::ToolCall {
            agent: AgentId(1),
            subagent: None,
            call_id: call.to_owned(),
            activity: Activity::ReadingFile,
            detail: ToolDetail {
                category: ToolCategory::FileRead,
                paths: vec![path.to_owned()],
                read_lines: Some(LineRange { first: 1, last: 9 }),
                changed: None,
            },
        }
    }

    fn result(call: &str, output: &str) -> ProviderEvent {
        ProviderEvent::ToolResult {
            agent: AgentId(1),
            subagent: None,
            call_id: call.to_owned(),
            output: output.to_owned(),
            exit_code: None,
        }
    }

    fn run_changes(session: u64, seq: u64, files: &[(&str, &[&str])]) -> RunChanges {
        RunChanges {
            run: RunId(session),
            session: SessionId(session),
            seq: LedgerSeq(seq),
            at_ms: 0,
            set: ChangeSet {
                files: files
                    .iter()
                    .map(|(path, actors)| FileChange {
                        path: (*path).to_owned(),
                        kind: ChangeKind::Modified,
                        actors: actors.iter().map(|actor| (*actor).to_owned()).collect(),
                    })
                    .collect(),
                is_partial: false,
            },
        }
    }

    #[test]
    fn packet_lists_files_changed_by_shell_even_without_tool_events() {
        let rows = vec![row(
            1,
            1,
            5,
            Some("fix the cache"),
            text_event(AgentId(1), "done"),
        )];
        let changes = vec![run_changes(
            1,
            1,
            &[("src/cache.rs", &[]), ("src/lib.rs", &["main agent"])],
        )];

        let HandoffOutcome::Ready(handoff) = build_handoff(
            &rows,
            &[],
            &changes,
            &Pending::default(),
            (&[], &[]),
            &budget(),
        ) else {
            panic!("packet should be ready");
        };

        assert!(handoff.text.contains(
            "Files changed in this run: src/cache.rs (modified, by shell or child process), src/lib.rs (modified, by main agent)"
        ));
    }

    #[test]
    fn changes_of_others_drops_the_sessions_own_and_already_delivered_runs() {
        let changes = vec![
            run_changes(1, 3, &[("own.rs", &[])]),
            run_changes(2, 2, &[("delivered.rs", &[])]),
            run_changes(2, 9, &[("new.rs", &[])]),
        ];

        let kept = changes_of_others(changes, SessionId(1), LedgerSeq(5));

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].set.files[0].path, "new.rs");
    }

    #[test]
    fn empty_rows_have_nothing_to_hand_over() {
        assert_eq!(
            build_handoff(&[], &[], &[], &Pending::default(), (&[], &[]), &budget()),
            HandoffOutcome::Empty
        );
    }

    #[test]
    fn packet_carries_input_answer_and_tool_result_with_session_title() {
        let rows = vec![
            row(
                1,
                1,
                5,
                Some("fix the cache"),
                text_event(AgentId(1), "looking"),
            ),
            row(
                2,
                1,
                5,
                Some("fix the cache"),
                read_call("c1", "src/cache.rs"),
            ),
            row(3, 1, 5, Some("fix the cache"), result("c1", "cache body")),
            row(
                4,
                1,
                5,
                Some("fix the cache"),
                text_event(AgentId(1), " done"),
            ),
        ];

        let HandoffOutcome::Ready(handoff) =
            build_handoff(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget())
        else {
            panic!("packet should be ready");
        };

        assert!(handoff.text.contains("fix the cache"));
        assert!(handoff.text.contains("looking done"));
        assert!(handoff.text.contains("cache body"));
        assert!(handoff.text.contains("### Session 5"));
        assert!(!handoff.is_over_limit);
    }

    #[test]
    fn interrupted_tool_call_is_an_error_result_not_a_warning() {
        let rows = vec![
            row(1, 1, 5, Some("run it"), read_call("c1", "a.rs")),
            row(2, 1, 5, Some("run it"), text_event(AgentId(1), "waiting")),
        ];

        let HandoffOutcome::Ready(handoff) =
            build_handoff(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget())
        else {
            panic!("packet should be ready");
        };

        assert!(handoff.text.contains("Tool call: FileRead a.rs"));
        assert!(handoff.text.contains(INTERRUPTED_RESULT));
    }

    fn ended(end: Option<RunEnd>, mut row: LedgerRow) -> LedgerRow {
        row.end = end;
        row
    }

    #[test]
    fn finished_input_is_marked_finished_and_never_an_open_item() {
        let rows = vec![ended(
            Some(RunEnd::Completed),
            row(
                1,
                1,
                5,
                Some("add two more lines"),
                text_event(AgentId(1), "added"),
            ),
        )];

        let text = handoff_text(&rows, &Pending::default());

        assert!(text.starts_with("The records below are an archive"));
        assert!(text.contains("Do not run them again"));
        assert!(text.contains("[Finished] User: add two more lines\nAgent: added"));
        assert_eq!(text.matches("add two more lines").count(), 1);
        assert!(!text.contains("## Open items"));
        assert!(!text.contains("[Result unknown] User"));
    }

    #[test]
    fn stopped_input_has_an_unknown_result_in_the_interrupted_format() {
        let rows = vec![ended(
            Some(RunEnd::Stopped),
            row(1, 1, 5, Some("run it"), text_event(AgentId(1), "started")),
        )];

        let text = handoff_text(&rows, &Pending::default());

        assert!(text.contains(&format!(
            "[Result unknown] User: run it\nAgent: started\nResult (error): {INTERRUPTED_RESULT}"
        )));
        assert!(!text.contains("[Finished] User"));
    }

    #[test]
    fn running_input_is_marked_in_progress() {
        let rows = vec![ended(
            None,
            row(1, 1, 5, Some("run it"), text_event(AgentId(1), "started")),
        )];

        let text = handoff_text(&rows, &Pending::default());

        assert!(text.contains("[In progress] User: run it\nAgent: started"));
    }

    // #592: 정정 뒤에 읽기 턴이 여러 개 이어져도 판정이 수정으로 분류하지 않아도 정정 원문은 한 번, 제자리에 남는다
    #[test]
    fn a_correction_survives_four_following_reads_without_any_judgment() {
        let correction = "actually the header is X-Route-Key";
        let mut rows = vec![
            row(
                1,
                1,
                5,
                Some("use the header X-Req-Id"),
                text_event(AgentId(1), "ok"),
            ),
            row(2, 2, 5, Some(correction), text_event(AgentId(1), "noted")),
        ];
        for (n, path) in ["docs", "layout", "tests", "build"].iter().enumerate() {
            let run = 3 + n as u64;
            let call = format!("c{run}");
            let input = format!("read the {path}");
            rows.push(row(
                run * 10,
                run,
                5,
                Some(&input),
                read_call(&call, &format!("src/{path}.rs")),
            ));
            rows.push(row(
                run * 10 + 1,
                run,
                5,
                Some(&input),
                result(&call, "body"),
            ));
        }

        let text = handoff_text(&rows, &Pending::default());

        assert_eq!(text.matches(correction).count(), 1);
        let order = [
            "User: use the header X-Req-Id",
            correction,
            "User: read the docs",
            "User: read the layout",
            "User: read the tests",
            "User: read the build",
        ]
        .map(|needle| text.find(needle).expect("input should be in the packet"));
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    }

    // #592: 같은 글자의 서로 다른 입력은 문자열로 합치지 않고 입력마다 남는다
    #[test]
    fn identical_inputs_of_different_runs_are_both_kept() {
        let rows = vec![
            row(1, 1, 5, Some("yes"), text_event(AgentId(1), "first")),
            row(2, 2, 5, Some("yes"), text_event(AgentId(1), "second")),
        ];

        let text = handoff_text(&rows, &Pending::default());

        assert_eq!(text.matches("User: yes").count(), 2);
        assert!(text.contains("Agent: first"));
        assert!(text.contains("Agent: second"));
    }

    // #592: 여러 작업의 입력도 작업과 개수에 상관없이 모두 실제 순서로 남는다
    #[test]
    fn every_input_of_every_task_is_kept_in_order() {
        let inputs = ["old task", "first goal", "middle step", "last step"];
        let rows: Vec<LedgerRow> = inputs
            .iter()
            .zip(1..)
            .map(|(input, n)| {
                in_task(
                    n / 2 + 1,
                    row(n, n, 5, Some(input), text_event(AgentId(1), "x")),
                )
            })
            .collect();

        let text = handoff_text(&rows, &Pending::default());

        let at = inputs.map(|input| text.find(input).expect("input should be in the packet"));
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(!text.contains("## Goal and last input"));
    }

    #[test]
    fn waiting_held_and_interrupted_inputs_are_open_items() {
        let rows = vec![row(7, 1, 5, Some("work"), text_event(AgentId(1), "a"))];
        let pending = Pending {
            waiting: vec!["run the suite".to_owned()],
            held: vec!["refactor later".to_owned()],
            unchecked: vec!["deploy it".to_owned()],
        };

        let text = handoff_text(&rows, &pending);

        let open = text.split("## Open items").nth(1).unwrap();
        let open = open.split("## Recent turns").next().unwrap();
        assert!(open.contains("Queued input: run the suite"));
        assert!(open.contains("Held input: refactor later"));
        assert!(open.contains(&format!(
            "Input: deploy it\nResult (error): {INTERRUPTED_RESULT}"
        )));
    }

    #[test]
    fn others_only_drops_the_sessions_own_rows() {
        let rows = vec![
            row(1, 1, 1, Some("a"), text_event(AgentId(1), "x")),
            row(2, 2, 2, Some("b"), text_event(AgentId(1), "y")),
        ];

        let kept = others_only(rows, SessionId(1));

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].session, SessionId(2));
    }

    fn steer(input: u64, run: u64, after: u64, text: &str) -> SteeredInput {
        SteeredInput {
            input: InputId(input),
            run: RunId(run),
            after: LedgerSeq(after),
            text: text.to_owned(),
            is_amendment: false,
        }
    }

    fn ready(rows: &[LedgerRow], steers: &[SteeredInput]) -> String {
        let HandoffOutcome::Ready(handoff) = build_handoff(
            rows,
            steers,
            &[],
            &Pending::default(),
            (&[], &[]),
            &budget(),
        ) else {
            panic!("packet should be ready");
        };
        handoff.text
    }

    // #592: 보낼 글이 기록한 본문을 같은 순서로 담지 않으면 근거가 보내지 않는다고 말한다
    #[test]
    fn evidence_refuses_a_body_that_lost_or_swapped_recorded_dialogue() {
        let rows = vec![
            row(1, 1, 5, Some("first input"), text_event(AgentId(1), "one")),
            row(2, 2, 5, Some("second input"), text_event(AgentId(1), "two")),
        ];
        let source =
            handoff_source(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget()).unwrap();
        let HandoffOutcome::Ready(handoff) = handoff_of(&source, &budget()) else {
            panic!("packet should be ready");
        };
        let evidence = PacketEvidence::first(&handoff, &source, RANK_SELECTOR);

        assert!(evidence.carries_dialogue(&handoff.text));
        assert!(!evidence.carries_dialogue(&handoff.text.replace("second input", "second")));
        assert!(!evidence.carries_dialogue(&handoff.text.replace("Agent: one", "Agent: 1")));
        let roles: Vec<(String, u64)> = evidence
            .rows()
            .into_iter()
            .filter(|item| item.hash.is_some())
            .map(|item| (item.zone, item.ref_id))
            .collect();
        assert_eq!(
            roles,
            [
                ("User".to_owned(), 1),
                ("Assistant".to_owned(), 1),
                ("User".to_owned(), 2),
                ("Assistant".to_owned(), 2),
            ]
        );
    }

    // #592: 같은 답을 조각으로 낸 기록은 조각마다 번호와 원문 해시를 남기고, 글에서는 한 답으로 이어 읽힌다
    #[test]
    fn streamed_answer_fragments_keep_their_own_ids_and_hashes() {
        let agent = AgentId(1);
        let rows = vec![
            row(1, 1, 5, Some("greet"), text_event(agent, "Hel")),
            row(2, 1, 5, Some("greet"), text_event(agent, "lo ")),
            row(3, 1, 5, Some("greet"), text_event(agent, "there")),
        ];
        let source =
            handoff_source(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget()).unwrap();
        let HandoffOutcome::Ready(handoff) = handoff_of(&source, &budget()) else {
            panic!("packet should be ready");
        };
        let evidence = PacketEvidence::first(&handoff, &source, RANK_SELECTOR);

        assert!(handoff.text.contains("User: greet\nAgent: Hello there\n"));
        let recorded: Vec<(String, u64, Option<String>)> = evidence
            .rows()
            .into_iter()
            .filter(|item| item.hash.is_some())
            .map(|item| (item.zone, item.ref_id, item.hash))
            .collect();
        let hash = |text: &str| Some(sha256_hex(text.as_bytes()));
        assert_eq!(
            recorded,
            [
                ("User".to_owned(), 1, hash("greet")),
                ("Assistant".to_owned(), 1, hash("Hel")),
                ("Assistant".to_owned(), 2, hash("lo ")),
                ("Assistant".to_owned(), 3, hash("there")),
            ]
        );
        assert!(evidence.carries_dialogue(&handoff.text));
    }

    // #592: 만든 뒤 글이 바뀌었거나 같은 본문을 다른 구역에 옮겨 놓은 글은 보내지 않는다
    #[test]
    fn evidence_refuses_a_body_that_differs_from_the_built_packet() {
        let rows = vec![row(
            1,
            1,
            5,
            Some("only input"),
            text_event(AgentId(1), "ok"),
        )];
        let source =
            handoff_source(&rows, &[], &[], &Pending::default(), (&[], &[]), &budget()).unwrap();
        let HandoffOutcome::Ready(handoff) = handoff_of(&source, &budget()) else {
            panic!("packet should be ready");
        };
        let evidence = PacketEvidence::first(&handoff, &source, RANK_SELECTOR);
        let moved = handoff
            .text
            .replace("User: only input\nAgent: ok", "")
            .replace("## Conversation", "## Earlier records")
            + "User: only input\nAgent: ok\n\n";

        assert!(!evidence.carries_dialogue(&format!("{}\n", handoff.text)));
        assert!(!evidence.carries_dialogue(&moved));
    }

    // #456, #592: 끼워 넣은 입력은 적용한 기록 번호 자리에 적용한 순서로 들어가고, 입력 번호와 역할이 기록된다
    #[test]
    fn steered_inputs_keep_their_applied_position_and_order() {
        let rows = vec![
            row(
                1,
                1,
                5,
                Some("write the parser"),
                text_event(AgentId(1), "started"),
            ),
            row(
                4,
                1,
                5,
                Some("write the parser"),
                text_event(AgentId(1), "done"),
            ),
        ];
        let steers = vec![
            steer(12, 1, 1, "use a hand written lexer"),
            steer(13, 1, 2, "use a hand written lexer"),
            steer(14, 1, 3, "skip the docs"),
        ];
        let lexer = "User (sent while this turn was running): use a hand written lexer";
        let order = [
            "User: write the parser\nAgent: started",
            lexer,
            lexer,
            "User (sent while this turn was running): skip the docs",
            "Agent: done",
        ];

        let text = ready(&rows, &steers);

        let mut at = 0;
        for needle in order {
            at += text[at..]
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} should follow the previous item: {text}"))
                + needle.len();
        }
        let source = handoff_source(
            &rows,
            &steers,
            &[],
            &Pending::default(),
            (&[], &[]),
            &budget(),
        )
        .unwrap();
        let ids: Vec<(Role, u64)> = source
            .protected()
            .iter()
            .map(|item| (item.role, item.id))
            .collect();
        assert_eq!(
            ids,
            [
                (Role::User, 1),
                (Role::Assistant, 1),
                (Role::Steer, 12),
                (Role::Steer, 13),
                (Role::Steer, 14),
                (Role::Assistant, 4),
            ]
        );
    }

    // #456
    #[test]
    fn a_steer_into_another_sessions_run_stays_out_of_a_catch_up_for_the_first() {
        let rows = vec![
            row(
                1,
                1,
                5,
                Some("write the parser"),
                text_event(AgentId(1), "done"),
            ),
            row(
                2,
                2,
                6,
                Some("review it"),
                text_event(AgentId(1), "looks fine"),
            ),
        ];
        let steers = [
            steer(3, 1, 1, "private to session five"),
            steer(4, 2, 2, "only nits please"),
        ];

        let text = ready(&others_only(rows, SessionId(5)), &steers);

        assert!(text.contains("only nits please"), "{text}");
        assert!(!text.contains("private to session five"), "{text}");
    }
}
