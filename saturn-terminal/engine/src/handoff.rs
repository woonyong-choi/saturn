//! 기록 목록에서 새 session에 넘기는 패킷과, 돌아온 session에 붙이는 변경분을 만든다.
//! 설계: docs/design/context-management.md#패킷-구성, docs/design/providers-and-sessions.md

use std::collections::{HashMap, HashSet};

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
    CONSTRAINT_SEPARATOR_CHARS, CompetingItem, Entry, PacketItem, PacketOutcome, PacketSource,
    PacketZone, RECENT_TURNS, RecentTurn, TurnStatus, build_packet, constraint_cap_chars,
    reduce_packet, status_item,
};
use saturn_core::sessions::ranking::{Candidate, order_after_router, rank_candidates};
use saturn_core::sessions::stamp::Stamp;
use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory, ToolDetail};
use saturn_protocol::ids::{ChatId, ConstraintId, InputId, LedgerSeq, RunId, SessionId};
use saturn_protocol::state::InputState;

use crate::Engine;
use crate::store::{
    ConstraintState, ExceptionKind, LedgerRow, PacketId, PacketItemRow, RunChanges, RunEnd,
    SteeredInput, StoredConstraint, StoredException,
};
use crate::switch::Reduction;

/// 새 session의 첫 턴으로 보내는 글.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handoff {
    pub(crate) text: String,
    /// 추정 토큰 수.
    pub(crate) tokens: u64,
    /// 고정 구역이 `P_max`를 넘어 `P_hard`까지 허용했다.
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
    /// 패킷이 담은 기록의 마지막 번호.
    pub(crate) up_to: LedgerSeq,
    pub(crate) tokens: u64,
    /// 첫 시도가 1이고 맥락 한도로 거절돼 줄여 다시 보낼 때마다 1 늘어난다.
    pub(crate) attempt: u32,
    pub(crate) reduced_from: Option<PacketId>,
    /// 경쟁 구역을 고른 방식. `RANK_SELECTOR`나 `COMPACT_SELECTOR`다.
    selector: &'static str,
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
            up_to: source.up_to,
            tokens: handoff.tokens,
            attempt: 1,
            reduced_from: None,
        }
    }

    /// 거절된 시도 `previous`를 줄여 다시 만든 패킷의 근거. 같은 재료에서 항목만 다시 골랐다.
    pub(crate) fn reduced(
        handoff: &Handoff,
        reduction: &Reduction,
        (attempt, previous): (u32, Option<PacketId>),
    ) -> Self {
        Self {
            attempt: attempt + 1,
            reduced_from: previous,
            ..Self::first(handoff, &reduction.source, reduction.selector)
        }
    }

    /// 재료 없이 만든 근거. 시험이 보낼 수 없는 호출의 자리를 채운다.
    #[cfg(test)]
    pub(crate) fn empty(up_to: LedgerSeq) -> Self {
        Self {
            items: Vec::new(),
            constraints: Vec::new(),
            up_to,
            tokens: 0,
            attempt: 1,
            reduced_from: None,
            selector: RANK_SELECTOR,
        }
    }

    /// 제약 칸 후보마다의 단계. `Omitted` 포함, 제약 번호 순이다.
    pub(crate) fn tiers(&self) -> Vec<(ConstraintId, ConstraintTier)> {
        self.constraints.clone()
    }

    /// 항목 행. 제약 칸, 목표, 열린 항목, 최근 턴, 경쟁 구역 순이다.
    pub(crate) fn rows(&self) -> Vec<PacketItemRow> {
        let constraints = self.constraints.iter().map(|(id, tier)| {
            let is_omitted = *tier == ConstraintTier::Omitted;
            PacketItemRow {
                zone: "Constraints".to_owned(),
                ref_id: id.0,
                selector: tier.name(),
                form: (!is_omitted).then(|| tier.name().to_owned()),
                reason: is_omitted.then_some("slot_full"),
            }
        });
        let items = self.items.iter().map(|item| PacketItemRow {
            zone: item.zone.name().to_owned(),
            ref_id: item.seq.0,
            selector: match item.zone {
                PacketZone::Goal => "latest",
                PacketZone::Open => "pending",
                PacketZone::Recent => "recent_turns",
                PacketZone::Competing => self.selector,
            },
            form: item.form.map(|form| form.name().to_owned()),
            reason: item.reason,
        });
        constraints.chain(items).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HandoffOutcome {
    Ready(Handoff),
    /// 넘길 기록이 없다.
    Empty,
    /// 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 않는다.
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
fn constraint_slot(
    constraints: &[StoredConstraint],
    turns: &[RecentTurn],
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
        .map_or("", |turn| turn.input.as_str());
    let mut recent: Vec<&RunChanges> = changes.iter().collect();
    recent.sort_by_key(|run| run.seq);
    let start = recent.len().saturating_sub(RECENT_TURNS);
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
/// 순서는 후보 순위(RRF)만 쓴다. 목표 칸은 지금 작업의 첫 입력과 마지막 입력, 남은 일 칸은 대기·보류 입력과 결과를 모르는 작업, 결과 없는 도구 호출이다(docs/experiments/packet-goal-fields/report.md).
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
    let turns = recent_turns(rows, &steers);
    let tools = tools(rows);
    let mut open = pending.entries(last.seq);
    open.extend(open_items(&tools));
    let slot = constraint_slot(constraints, &turns, changes, budget);
    Some(PacketSource {
        constraints: slot.included.into_iter().map(|(_, rule)| rule).collect(),
        constraints_omitted: slot.omitted.into_iter().map(|(_, rule)| rule).collect(),
        constraint_tiers: slot.tiers,
        goal_and_last_input: goal_inputs(rows, &steers),
        open_items: open,
        competitors: changed_files_items(changes)
            .into_iter()
            .chain(ordered_competitors(&tools, &turns, budget.rrf_k, verdicts))
            .collect(),
        recent_turns: turns,
        provider_docs: provider_docs.to_vec(),
        evidence_lookup: budget.evidence_lookup,
        up_to: last.seq,
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
    let inputs = recent_turns(rows, &[])
        .into_iter()
        .map(|turn| turn.input)
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

/// 지금 작업(마지막 입력이 있는 행의 작업)의 첫 입력과 마지막 입력. 같으면 하나다. 실행 중에 끼워 넣어 적용한 입력도
/// 사용자의 입력이라 마지막 입력이 될 수 있다.
/// 기록 번호는 그 입력을 낸 실행의 첫 이벤트 번호(끼워 넣은 입력은 적용 때 쌓여 있던 마지막 번호)이고, 입력마다 그 실행의 상태를 적는다. 끝난 입력도 목표 칸에 남지만 끝났다고 적혀 요청으로 읽히지 않는다.
fn goal_inputs(rows: &[LedgerRow], steers: &[&SteeredInput]) -> Vec<Entry> {
    let silent = silent_runs(rows, steers);
    let Some(task) = rows
        .iter()
        .rev()
        .find(|row| row.input.is_some() && !silent.contains(&row.run))
        .map(|row| row.task)
    else {
        return Vec::new();
    };
    let mut runs: Vec<(RunId, LedgerSeq, TurnStatus)> = Vec::new();
    let mut inputs: Vec<(LedgerSeq, TurnStatus, String)> = Vec::new();
    for row in rows
        .iter()
        .filter(|row| row.task == task && !silent.contains(&row.run))
    {
        let Some(input) = &row.input else {
            continue;
        };
        if !runs.iter().any(|(run, _, _)| *run == row.run) {
            runs.push((row.run, row.seq, status_of(row.end)));
            inputs.push((row.seq, status_of(row.end), input.clone()));
        }
    }
    for steer in steers {
        if let Some((_, first, status)) = runs.iter().find(|(run, _, _)| *run == steer.run) {
            inputs.push(((*first).max(steer.after), *status, steer.text.clone()));
        }
    }
    // 같은 번호면 실행을 연 입력이 먼저 들어 있으므로 안정 정렬이 그 순서를 지킨다
    inputs.sort_by_key(|(seq, _, _)| *seq);
    let last = inputs.pop();
    inputs.truncate(1);
    let is_single = inputs.is_empty();
    let first = inputs.pop();
    let label = |is_first: bool| match (is_single, is_first) {
        (true, _) => "Input",
        (false, true) => "First input",
        (false, false) => "Last input",
    };
    first
        .map(|item| (true, item))
        .into_iter()
        .chain(last.map(|item| (false, item)))
        .map(|(is_first, (seq, status, text))| Entry {
            seq,
            text: status_item(label(is_first), status, &text),
        })
        .collect()
}

/// 실행이 끝난 방식에서 입력의 상태를 정한다. 정상 완료만 끝난 일이고 실패나 멈춤은 일부만 실행됐을 수 있다.
fn status_of(end: Option<RunEnd>) -> TurnStatus {
    match end {
        Some(RunEnd::Completed) => TurnStatus::Finished,
        None => TurnStatus::InProgress,
        Some(RunEnd::Failed | RunEnd::Stopped) => TurnStatus::ResultUnknown,
    }
}

/// 글도 도구 호출도 내지 않고 끝난 실행. provider 명령(`/compact` 등)의 턴이 이에 해당한다. 최근 턴 세 칸과 목표 칸의 마지막 입력을 이런 턴이 차지하면
/// 그 앞의 실제 대화(정정한 말 포함)가 밀려나므로 두 곳에서 뺀다. 입력 원문은 기록에 그대로 남는다. 끼워 넣은 입력이 붙은 실행은 뺄 수 없다.
fn silent_runs(rows: &[LedgerRow], steers: &[&SteeredInput]) -> HashSet<RunId> {
    let mut silent: HashSet<RunId> = HashSet::new();
    let mut loud: HashSet<RunId> = steers.iter().map(|steer| steer.run).collect();
    for row in rows {
        let is_quiet = matches!(
            row.event,
            ProviderEvent::ContextSize { .. }
                | ProviderEvent::TurnCompleted { .. }
                | ProviderEvent::Usage(_)
                | ProviderEvent::CacheWindow { .. }
                | ProviderEvent::SettingsApplied { .. }
        );
        if !is_quiet || row.end != Some(RunEnd::Completed) {
            loud.insert(row.run);
        }
        if row.input.is_some() {
            silent.insert(row.run);
        }
    }
    silent.retain(|run| !loud.contains(run));
    silent
}

/// 입력이 있는 실행마다 턴 하나. 기록 번호는 그 실행의 첫 이벤트 번호이고 답은 메인 에이전트 글을 이은 것이다.
/// 그 실행에 끼워 넣어 적용한 입력은 적용한 순서로 턴에 붙는다.
fn recent_turns(rows: &[LedgerRow], steers: &[&SteeredInput]) -> Vec<RecentTurn> {
    let silent = silent_runs(rows, steers);
    let mut order: Vec<RunId> = Vec::new();
    let mut turns: HashMap<RunId, RecentTurn> = HashMap::new();
    for row in rows {
        let Some(input) = &row.input else {
            continue;
        };
        if silent.contains(&row.run) {
            continue;
        }
        let turn = turns.entry(row.run).or_insert_with(|| {
            order.push(row.run);
            RecentTurn {
                seq: row.seq,
                stamp: stamp_of(row),
                status: status_of(row.end),
                input: input.clone(),
                steers: steers
                    .iter()
                    .filter(|steer| steer.run == row.run)
                    .map(|steer| steer.text.clone())
                    .collect(),
                answer: String::new(),
            }
        });
        if let ProviderEvent::Text {
            subagent: None,
            text,
            ..
        } = &row.event
        {
            turn.answer.push_str(text);
        }
    }
    order
        .into_iter()
        .filter_map(|run| turns.remove(&run))
        .collect()
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
    turns: &[RecentTurn],
    rrf_k: u32,
    verdicts: Option<&[(LedgerSeq, f64)]>,
) -> Vec<CompetingItem> {
    let last_input = turns.last().map_or("", |turn| turn.input.as_str());
    let first_recent = turns
        .len()
        .checked_sub(RECENT_TURNS)
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
        DEFAULT_CONSTRAINT_SLOT_PERCENT, DEFAULT_ITEM_CAP_PERCENT, DEFAULT_PACKET_HARD_PERCENT,
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
            window: 10_000,
            cache_read: 0.1,
            cache_write: 1.25,
            cache_ttl: Duration::from_secs(300),
            packet_hard_percent: DEFAULT_PACKET_HARD_PERCENT,
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
        assert!(text.contains("Input [Finished]: add two more lines"));
        assert!(text.contains("[Finished] User: add two more lines"));
        assert!(!text.contains("## Open items"));
        assert!(!text.contains("Input [Result unknown]"));
    }

    #[test]
    fn stopped_input_has_an_unknown_result_in_the_interrupted_format() {
        let rows = vec![ended(
            Some(RunEnd::Stopped),
            row(1, 1, 5, Some("run it"), text_event(AgentId(1), "started")),
        )];

        let text = handoff_text(&rows, &Pending::default());

        assert!(text.contains(&format!(
            "Input [Result unknown]: run it\nResult (error): {INTERRUPTED_RESULT}"
        )));
        assert!(!text.contains("Input [Finished]"));
    }

    #[test]
    fn running_input_is_marked_in_progress() {
        let rows = vec![ended(
            None,
            row(1, 1, 5, Some("run it"), text_event(AgentId(1), "started")),
        )];

        let text = handoff_text(&rows, &Pending::default());

        assert!(text.contains("Input [In progress]: run it"));
        assert!(text.contains("[In progress] User: run it"));
    }

    #[test]
    fn goal_holds_the_first_and_last_input_of_the_current_task() {
        let rows = vec![
            in_task(
                1,
                row(1, 1, 5, Some("old task"), text_event(AgentId(1), "a")),
            ),
            in_task(
                2,
                row(2, 2, 5, Some("first goal"), text_event(AgentId(1), "b")),
            ),
            in_task(
                2,
                row(3, 3, 5, Some("middle step"), text_event(AgentId(1), "c")),
            ),
            in_task(
                2,
                row(4, 4, 5, Some("last step"), text_event(AgentId(1), "d")),
            ),
        ];

        let text = handoff_text(&rows, &Pending::default());

        let goal = text.split("## Goal and last input").nth(1).unwrap();
        let goal = goal.split("## Recent turns").next().unwrap();
        assert!(goal.contains("first goal"));
        assert!(goal.contains("last step"));
        assert!(!goal.contains("middle step"));
        assert!(!goal.contains("old task"));
    }

    #[test]
    fn goal_is_one_entry_when_the_task_has_a_single_input() {
        let rows = vec![row(1, 1, 5, Some("only goal"), text_event(AgentId(1), "a"))];

        let text = handoff_text(&rows, &Pending::default());

        let goal = text.split("## Goal and last input").nth(1).unwrap();
        let goal = goal.split("## Recent turns").next().unwrap();
        assert_eq!(goal.matches("only goal").count(), 1);
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

    // #456
    #[test]
    fn steered_inputs_show_in_the_turn_and_the_goal() {
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
        type SteerCase = (&'static str, Vec<SteeredInput>, fn(&str, &str));
        let cases: [SteerCase; 2] = [
            (
                "steers join their turn in the order they were applied",
                vec![
                    steer(2, 1, 1, "use a hand written lexer"),
                    steer(3, 1, 2, "skip the docs"),
                ],
                |name, text| {
                    let first = text
                        .find("User (sent while this turn was running): use a hand written lexer");
                    let second =
                        text.find("User (sent while this turn was running): skip the docs");
                    let agent = text.find("Agent: starteddone");
                    assert!(
                        first.is_some() && second.is_some() && agent.is_some(),
                        "{name}: {text}"
                    );
                    assert!(first < second && second < agent, "{name}: {text}");
                },
            ),
            (
                "the last steered input is the last user input of the goal",
                vec![steer(2, 1, 2, "stop and use the lexer branch")],
                |name, text| {
                    assert!(
                        text.contains("First input [Finished]: write the parser"),
                        "{name}: {text}"
                    );
                    assert!(
                        text.contains("Last input [Finished]: stop and use the lexer branch"),
                        "{name}: {text}"
                    );
                },
            ),
        ];

        for (name, steers, check) in cases {
            let text = ready(&rows, &steers);
            check(name, &text);
        }
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
