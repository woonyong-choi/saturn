//! 새 session의 첫 작업 입력 앞에 같은 채팅의 관련 원문을 미리 넣는 실험 옵션 `context.select.related`(`rank`, `jev`).
//! 후보 집합, 권한 범위, 순위는 근거 검색(`evidence.rs`)과 같고, 고른 원문은 기존 패킷의 고정 구역에 실린다.
//! `jev`는 같은 후보를 router가 후보별로 판단해 확신 있는 긍정만 앞으로 올리고(제외 없음), 판단을 못 쓰거나 바뀐 것이 없으면 `rank`로 고정한다.
//! 설계: docs/design/context-management.md#단계별-기억-확장, docs/design/context-selection.md#관련-원문-jev-실험-계약

use std::collections::HashSet;

use saturn_core::queue::QueuedInput;
use saturn_core::routers::{
    Answer, RELATED_HEAD_CHARS, RELATED_TAIL_CHARS, RelatedCandidate, RouterRequest, SET_RELATED,
    related_questions, related_requests, related_state, related_verdicts,
};
use saturn_core::sessions::SendTarget;
use saturn_core::sessions::context::ContextBudget;
use saturn_core::sessions::packet::{PacketSource, RelatedRecord, Role, related_room_chars};
use saturn_protocol::ids::{ChatId, InputId, LedgerSeq, SettingsRevision};
use saturn_protocol::rpc::EvidenceKind;

use crate::Engine;
use crate::evidence::RelatedFound;
use crate::handoff::{
    HandoffOutcome, RELATED_JEV_SELECTOR, RELATED_SELECTOR, RelatedLog, RelatedRef, handoff_of,
};
use crate::packet_select::{CompactAsk, CompactCall, TransitionKey, Trigger};
use crate::routers::{ActiveRouter, RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::settings::{RelatedSelect, Settings};
use crate::store::JudgmentOutcome;

/// 순위 위에서 예산을 따져 보는 후보 수의 상한. 초안.
const MAX_CONSIDERED: usize = 20;

/// 판단 요청 때 고정한 값. 적용 직전에 지금 값으로 다시 만든 스냅샷과 모두 같아야 답을 쓴다.
/// `refs[i]`의 임시 번호(ordinal)는 `i + 1`이고 요청 안에서만 뜻이 있다. 저장하거나 패킷에 남기지 않는다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RelatedSnapshot {
    pub(crate) chat: ChatId,
    pub(crate) input: InputId,
    pub(crate) text: String,
    pub(crate) settings: SettingsRevision,
    pub(crate) related: RelatedSelect,
    pub(crate) min_confidence: f64,
    /// 후보를 읽은 때의 채팅 revision.
    pub(crate) tail: LedgerSeq,
    /// 새 session 대상과 고른 모델.
    pub(crate) target: (SendTarget, Option<String>),
    /// 후보 순서대로 `(종류, 번호, 해시)`. 채팅은 `chat`이다.
    pub(crate) refs: Vec<RelatedRef>,
}

/// 판단을 적용 직전에 거른 결과.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RelatedOrder {
    /// 모든 후보의 `candidates` 위치. 확신 있는 긍정을 확률 내림차순(같으면 기존 순위 순)으로 앞에 두고 나머지는 기존 순위 순이다.
    Judged(Vec<usize>),
    /// 판단을 쓰지 않고 `rank`로 고정한다. 값은 판단 근거에 남는 이유다.
    Rank(&'static str),
}

enum RelatedGate {
    Ready(RelatedOrder),
    Ask(Box<CompactCall>),
}

/// 종류 있는 번호로 패킷이 이미 싣는 대화 본문을 뺀 상위 `MAX_CONSIDERED`건. `rank`와 `jev`가 같이 쓴다.
fn shared_candidates(
    ranked: Vec<RelatedFound>,
    carried: &HashSet<(EvidenceKind, u64)>,
) -> Vec<RelatedFound> {
    ranked
        .into_iter()
        .filter(|found| !carried.contains(&(found.kind, found.id)))
        .take(MAX_CONSIDERED)
        .collect()
}

/// 후보를 주어진 순서로 같은 전문 예산에 채운다. 못 든 후보는 `budget`으로 뺀다. 선택 방식과 관계없이 같은 규칙이다.
fn fill_room(
    ordered: impl IntoIterator<Item = RelatedFound>,
    chat: ChatId,
    room: usize,
) -> (Vec<RelatedRecord>, Vec<(Option<RelatedRef>, &'static str)>) {
    let mut used = 0;
    let mut picked = Vec::new();
    let mut omitted = Vec::new();
    for found in ordered {
        let reference = (found.kind, found.id, found.hash.clone());
        let item = RelatedRecord {
            kind: found.kind,
            chat,
            id: found.id,
            hash: found.hash,
            text: found.text,
        };
        if used + item.packet_chars() <= room {
            used += item.packet_chars();
            picked.push(item);
        } else {
            omitted.push((Some(reference), "budget"));
        }
    }
    (picked, omitted)
}

/// 판단 확률로 확신 있는 긍정(`P(yes) > 0.5`이고 확신이 `min_confidence` 이상)만 앞으로 올린다.
/// 긍정은 확률 내림차순, 같으면 기존 순위 순이고 나머지는 기존 순위 순으로 모두 뒤에 붙인다. 아무도 빼지 않는다.
/// 확률이 하나라도 틀렸으면 `jev_invalid`, 긍정이 없거나 순서가 그대로면 `jev_no_promotion`으로 `rank`를 쓴다.
fn order_by_verdicts(verdicts: &[Option<f64>], min_confidence: f64) -> RelatedOrder {
    let mut probabilities = Vec::with_capacity(verdicts.len());
    for verdict in verdicts {
        match verdict {
            Some(yes) if yes.is_finite() && (0.0..=1.0).contains(yes) => {
                probabilities.push(*yes);
            }
            _ => return RelatedOrder::Rank("jev_invalid"),
        }
    }
    let mut promoted: Vec<usize> = (0..probabilities.len())
        .filter(|index| {
            probabilities[*index] > 0.5
                && Answer::Noul(probabilities[*index]).confidence() >= min_confidence
        })
        .collect();
    promoted.sort_by(|a, b| {
        probabilities[*b]
            .total_cmp(&probabilities[*a])
            .then(a.cmp(b))
    });
    let order: Vec<usize> = promoted
        .iter()
        .copied()
        .chain((0..probabilities.len()).filter(|index| !promoted.contains(index)))
        .collect();
    if promoted.is_empty() || order.iter().copied().eq(0..order.len()) {
        return RelatedOrder::Rank("jev_no_promotion");
    }
    RelatedOrder::Judged(order)
}

impl Engine {
    /// 새 session에서만 선주입을 시도하고, 실제로 원문을 고른 경우 패킷을 다시 만든다.
    /// 판단을 기다려야 하면 물을 호출을 돌려준다. 호출자가 별도 작업에 맡기고 답이 오면 계획을 다시 세운다.
    pub(crate) async fn related_handoff(
        &self,
        (record, settings, budget): (&QueuedInput, &Settings, &ContextBudget),
        (target, model): (&SendTarget, Option<&str>),
        source: Option<PacketSource>,
        outcome: HandoffOutcome,
    ) -> Result<(Option<PacketSource>, HandoffOutcome, Option<RelatedLog>), Box<CompactCall>> {
        if !matches!(target, SendTarget::New { .. }) {
            return Ok((source, outcome, None));
        }
        let (source, related) = self
            .attach_related((record, settings, budget), (target, model), source)
            .await?;
        let outcome = if related.as_ref().is_some_and(|log| !log.picked.is_empty()) {
            source
                .as_ref()
                .map_or(HandoffOutcome::Empty, |source| handoff_of(source, budget))
        } else {
            outcome
        };
        Ok((source, outcome, related))
    }

    // cost: time O(r + n log n), heap O(L), stack O(1)
    // vars: r = 기록 행 수, n = 후보 수, L = 대화 글자 수
    // basis: estimate
    /// 설정이 켜져 있으면 `source`에 관련 원문을 더한 재료와 그 근거를 돌려준다. 꺼져 있으면 `source`를 그대로 돌려주고 근거는 없다.
    /// 후보는 입력 본문으로 순위를 매기고, 패킷이 이미 싣는 대화 본문은 종류 있는 번호(`종류:번호`)로 뺀다.
    /// `jev`는 같은 후보와 같은 전문 예산에서 선택 순서만 router 판단으로 바꾸고, 판단이 없거나 못 쓰면 `rank`로 고정한다.
    /// 실패하거나 못 넣어도 입력은 막지 않고 이유를 근거에 남기며 오래된 내용으로 대신하지 않는다.
    ///
    /// # Errors
    /// `jev` 판단을 아직 받지 못해 물을 호출이 남았으면 그 호출.
    pub(crate) async fn attach_related(
        &self,
        (record, settings, budget): (&QueuedInput, &Settings, &ContextBudget),
        target: (&SendTarget, Option<&str>),
        source: Option<PacketSource>,
    ) -> Result<(Option<PacketSource>, Option<RelatedLog>), Box<CompactCall>> {
        let requested = settings.related_select();
        let requested_selector = match requested {
            RelatedSelect::Off => return Ok((source, None)),
            RelatedSelect::Rank => RELATED_SELECTOR,
            RelatedSelect::Jev => RELATED_JEV_SELECTOR,
        };
        let base = source.clone().unwrap_or_default();
        let search = match self.related_search(record.chat, &record.text).await {
            Ok(search) => search,
            Err(error) => {
                tracing::warn!(chat = record.chat.0, error = %self.failure_line(&error), "related record search failed");
                let log =
                    RelatedLog::none(base.up_to, "retrieval_failed").requested(requested_selector);
                return Ok((source, Some(log)));
            }
        };
        let carried: HashSet<(EvidenceKind, u64)> = base
            .protected()
            .iter()
            .map(|item| (kind_of(item.role), item.id))
            .collect();
        let room = related_room_chars(&base, budget);
        let tail = search.tail;
        let chat = search.chat;
        let candidates = shared_candidates(search.ranked, &carried);
        if candidates.is_empty() {
            let log = RelatedLog::none(tail, "no_candidates").requested(requested_selector);
            return Ok((source, Some(log)));
        }
        let (applied, ordered, mut omitted) = if requested == RelatedSelect::Jev {
            let snapshot = RelatedSnapshot::of(record, settings, tail, target, &candidates);
            match self.related_gate(snapshot, &candidates).await? {
                RelatedOrder::Judged(order) => {
                    let ordered = order
                        .iter()
                        .map(|index| candidates[*index].clone())
                        .collect();
                    (RELATED_JEV_SELECTOR, ordered, Vec::new())
                }
                RelatedOrder::Rank(reason) => (RELATED_SELECTOR, candidates, vec![(None, reason)]),
            }
        } else {
            (RELATED_SELECTOR, candidates, Vec::new())
        };
        let (picked, over_budget) = fill_room(ordered, chat, room);
        omitted.extend(over_budget);
        let log = RelatedLog {
            requested: requested_selector,
            applied,
            tail,
            picked: picked
                .iter()
                .map(|item| (item.kind, item.id, item.hash.clone()))
                .collect(),
            omitted,
        };
        if picked.is_empty() {
            return Ok((source, Some(log)));
        }
        let mut merged = base;
        // 기록 없이 제약만 있던 재료도 패킷 기록에 후보를 읽은 채팅 revision이 남게 한다
        merged.up_to = merged.up_to.max(tail);
        merged.related = picked;
        Ok((Some(merged), Some(log)))
    }

    /// 적용 직전 자리. 돌아온 답이 있으면 스냅샷을 지금 값과 맞춰 쓰거나 `rank`로 고정하고, 없으면 물을 호출을 만든다.
    /// 판단은 성공이든 실패든 판단 기록에 한 번 남는다. router는 여기서 부르지 않는다.
    pub(crate) async fn related_gate(
        &self,
        now: RelatedSnapshot,
        candidates: &[RelatedFound],
    ) -> Result<RelatedOrder, Box<CompactCall>> {
        let slot = (now.chat, Trigger::Related(now.input));
        let Some(reply) = self.flow.compact_replies.get(&slot) else {
            return match self.related_call(&now, candidates) {
                Ok(call) => Err(Box::new(call)),
                Err(reason) => Ok(RelatedOrder::Rank(reason)),
            };
        };
        let Some(exchange) = &reply.exchange else {
            return Ok(RelatedOrder::Rank("jev_late"));
        };
        let call = &reply.call;
        let (order, outcome, fallbacks): (_, _, &[&str]) = if call.related.as_deref() == Some(&now)
        {
            match &exchange.result {
                Err(_) => (
                    RelatedOrder::Rank("jev_failed"),
                    outcome_of(&exchange.result),
                    &["router-failed"],
                ),
                Ok(response) => {
                    let ordinals: Vec<u32> = (1..=now.refs.len())
                        .filter_map(|ordinal| u32::try_from(ordinal).ok())
                        .collect();
                    let verdicts = related_verdicts(&ordinals, std::slice::from_ref(response));
                    match order_by_verdicts(&verdicts, now.min_confidence) {
                        order @ RelatedOrder::Rank("jev_invalid") => {
                            (order, JudgmentOutcome::Invalid, &["invalid"])
                        }
                        order @ RelatedOrder::Rank(_) => {
                            (order, JudgmentOutcome::Ok, &["no-promotion"])
                        }
                        order @ RelatedOrder::Judged(_) => (order, JudgmentOutcome::Ok, &[]),
                    }
                }
            }
        } else {
            (
                RelatedOrder::Rank("jev_stale"),
                JudgmentOutcome::Superseded,
                &["superseded"],
            )
        };
        if !reply.is_recorded.load(std::sync::atomic::Ordering::Relaxed) {
            let clipped = candidates.iter().any(is_clipped);
            let mut reasons = fallbacks.to_vec();
            if clipped {
                reasons.push("clipped");
            }
            self.record_related(call, exchange, outcome, &reasons).await;
            reply
                .is_recorded
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(order)
    }

    /// 지금 후보로 요청을 만든다. 요청 한도를 넘으면 호출 없이 `rank`로 간다. 후보 번호는 이 요청 안에서만 쓰는 임시 번호다.
    pub(crate) fn related_call(
        &self,
        snapshot: &RelatedSnapshot,
        candidates: &[RelatedFound],
    ) -> Result<CompactCall, &'static str> {
        let SendTarget::New { provider, .. } = &snapshot.target.0 else {
            return Err("jev_failed");
        };
        let masked: Vec<RelatedCandidate> = candidates
            .iter()
            .zip(1u32..)
            .map(|(found, ordinal)| RelatedCandidate {
                ordinal,
                kind: found.kind,
                chat: snapshot.chat,
                id: found.id,
                hash: found.hash.clone(),
                text: sanitize_state(&found.text, &self.masker),
            })
            .collect();
        let model = self.routers.active().model().to_owned();
        let scope = self
            .queued(snapshot.input)
            .map(|record| record.workdir.display().to_string())
            .unwrap_or_default();
        let state = sanitize_state(&related_state(&snapshot.text, &scope), &self.masker);
        let pieces = related_requests(&model, &state, &masked).map_err(|error| {
            tracing::warn!(%error, "related request is over the size limit");
            "jev_too_large"
        })?;
        // 원격 router는 전송 직전에 분할하지만 로컬 서버는 요청을 한 번에 보낸다.
        if !can_send_related(self.routers.active(), pieces.len()) {
            return Err("jev_too_large");
        }
        let request = RouterRequest {
            model,
            state,
            sets: vec![related_questions(&masked)],
        };
        let ask = CompactAsk {
            chat: snapshot.chat,
            input: Some(snapshot.input),
            settings: snapshot.settings,
            key: TransitionKey {
                chat: snapshot.chat,
                from: None,
                provider: *provider,
                model: snapshot.target.1.clone(),
                trigger: Trigger::Related(snapshot.input),
            },
        };
        Ok(CompactCall::related(ask, request, snapshot.clone()))
    }

    pub(crate) async fn record_related(
        &self,
        call: &CompactCall,
        exchange: &RouterExchange,
        outcome: JudgmentOutcome,
        reasons: &[&str],
    ) {
        let context = RecordContext {
            chat: call.ask.chat,
            input: call.ask.input,
            question_sets: call.request.sets.iter().map(|(id, _)| id.clone()).collect(),
            settings: call.ask.settings,
            fallbacks: reasons
                .iter()
                .map(|reason| (SET_RELATED.to_owned(), (*reason).to_owned()))
                .collect(),
            outcome,
            thresholds: call
                .related
                .iter()
                .map(|snapshot| ("min_confidence".to_owned(), snapshot.min_confidence))
                .collect(),
            asked_with: None,
        };
        if let Err(error) = self.routers.record(&self.store, context, exchange).await {
            tracing::warn!(error = %self.failure_line(&error), "failed to record a related judgment");
        }
    }
}

impl RelatedSnapshot {
    pub(crate) fn of(
        record: &QueuedInput,
        settings: &Settings,
        tail: LedgerSeq,
        (target, model): (&SendTarget, Option<&str>),
        candidates: &[RelatedFound],
    ) -> Self {
        Self {
            chat: record.chat,
            input: record.id,
            text: record.text.clone(),
            settings: record.settings,
            related: settings.related_select(),
            min_confidence: settings.thresholds().min_confidence,
            tail,
            target: (target.clone(), model.map(str::to_owned)),
            refs: candidates.iter().map(ref_of).collect(),
        }
    }
}

fn ref_of(found: &RelatedFound) -> RelatedRef {
    (found.kind, found.id, found.hash.clone())
}

fn is_clipped(found: &RelatedFound) -> bool {
    found.text.chars().count() > RELATED_HEAD_CHARS + RELATED_TAIL_CHARS
}

/// 원격은 조각별 전송을 지원한다. 로컬 서버는 원본 한 요청만 보내므로 여러 조각이면 보내지 않는다.
fn can_send_related(router: &ActiveRouter, pieces: usize) -> bool {
    !matches!(router, ActiveRouter::Local(_)) || pieces == 1
}

/// 패킷의 대화 본문 역할에 맞는 근거 종류. 패킷과 근거 조회가 같은 번호를 쓴다.
fn kind_of(role: Role) -> EvidenceKind {
    match role {
        Role::User => EvidenceKind::Input,
        Role::Steer => EvidenceKind::Steer,
        Role::Assistant => EvidenceKind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::{LocalRouter, LocalSource};

    #[test]
    fn only_confident_positives_are_promoted_and_nothing_is_excluded() {
        let cases = [
            (
                vec![Some(0.9), Some(f64::NAN)],
                RelatedOrder::Rank("jev_invalid"),
            ),
            (
                vec![Some(0.9), Some(-0.1)],
                RelatedOrder::Rank("jev_invalid"),
            ),
            (
                vec![Some(0.9), Some(1.1)],
                RelatedOrder::Rank("jev_invalid"),
            ),
            (vec![Some(0.9), None], RelatedOrder::Rank("jev_invalid")),
            // 확신 있는 긍정 하나가 불확실한 후보들 사이에서 앞으로 간다
            (
                vec![Some(0.4), Some(0.55), Some(0.95), Some(0.3)],
                RelatedOrder::Judged(vec![2, 0, 1, 3]),
            ),
            // 긍정이 없거나 확신이 모자라면 승격하지 않는다
            (
                vec![Some(0.1), Some(0.05)],
                RelatedOrder::Rank("jev_no_promotion"),
            ),
            (
                vec![Some(0.55), Some(0.75)],
                RelatedOrder::Rank("jev_no_promotion"),
            ),
            // 이미 맨 앞이면 순서가 같다
            (
                vec![Some(0.9), Some(0.2), Some(0.5)],
                RelatedOrder::Rank("jev_no_promotion"),
            ),
            // 동률은 기존 순위 순
            (
                vec![Some(0.1), Some(0.9), Some(0.9)],
                RelatedOrder::Judged(vec![1, 2, 0]),
            ),
            (
                vec![Some(0.85), Some(0.95), Some(0.85)],
                RelatedOrder::Judged(vec![1, 0, 2]),
            ),
        ];
        for (verdicts, expected) in cases {
            assert_eq!(order_by_verdicts(&verdicts, 0.6), expected, "{verdicts:?}");
        }
    }

    #[test]
    fn local_router_sends_a_related_request_only_as_one_piece() {
        let local = ActiveRouter::Local(LocalRouter::new(
            LocalSource::Model {
                path: std::path::PathBuf::new(),
            },
            "local".to_owned(),
        ));
        assert!(can_send_related(&local, 1));
        assert!(!can_send_related(&local, 2));
    }
}
