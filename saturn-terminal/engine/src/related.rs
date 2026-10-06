//! 새 session의 첫 작업 입력 앞에 같은 채팅의 관련 원문을 미리 넣는 실험 옵션 `context.select.related = rank`.
//! 후보 집합, 권한 범위, 순위는 근거 검색(`evidence.rs`)과 같고, 고른 원문은 기존 패킷의 고정 구역에 실린다.
//! 설계: docs/design/context-management.md#단계별-기억-확장, docs/design/context-selection.md#보존-우선-선별-계약

use std::collections::HashSet;

use saturn_core::queue::QueuedInput;
use saturn_core::sessions::SendTarget;
use saturn_core::sessions::context::ContextBudget;
use saturn_core::sessions::packet::{PacketSource, RelatedRecord, Role, related_room_chars};
use saturn_protocol::rpc::EvidenceKind;

use crate::Engine;
use crate::handoff::{HandoffOutcome, RelatedLog, handoff_of};
use crate::settings::{RelatedSelect, Settings};

/// 순위 위에서 예산을 따져 보는 후보 수의 상한. 초안.
const MAX_CONSIDERED: usize = 20;

impl Engine {
    /// 새 session에서만 선주입을 시도하고, 실제로 원문을 고른 경우 패킷을 다시 만든다.
    pub(crate) async fn related_handoff(
        &self,
        (record, settings, budget): (&QueuedInput, &Settings, &ContextBudget),
        target: &SendTarget,
        source: Option<PacketSource>,
        outcome: HandoffOutcome,
    ) -> (Option<PacketSource>, HandoffOutcome, Option<RelatedLog>) {
        if !matches!(target, SendTarget::New { .. }) {
            return (source, outcome, None);
        }
        let (source, related) = self
            .attach_related((record, settings, budget), source)
            .await;
        let outcome = if related.as_ref().is_some_and(|log| !log.picked.is_empty()) {
            source
                .as_ref()
                .map_or(HandoffOutcome::Empty, |source| handoff_of(source, budget))
        } else {
            outcome
        };
        (source, outcome, related)
    }

    // cost: time O(r + n log n), heap O(L), stack O(1)
    // vars: r = 기록 행 수, n = 후보 수, L = 대화 글자 수
    // basis: estimate
    /// 설정이 켜져 있으면 `source`에 관련 원문을 더한 재료와 그 근거를 돌려준다. 꺼져 있으면 `source`를 그대로 돌려주고 근거는 없다.
    /// 후보는 입력 본문으로 순위를 매기고, 패킷이 이미 싣는 대화 본문은 종류 있는 번호(`종류:번호`)로 뺀다.
    /// 실패하거나 못 넣어도 입력은 막지 않고 이유를 근거에 남기며 오래된 내용으로 대신하지 않는다.
    pub(crate) async fn attach_related(
        &self,
        (record, settings, budget): (&QueuedInput, &Settings, &ContextBudget),
        source: Option<PacketSource>,
    ) -> (Option<PacketSource>, Option<RelatedLog>) {
        if settings.related_select() != RelatedSelect::Rank {
            return (source, None);
        }
        let base = source.clone().unwrap_or_default();
        let search = match self.related_search(record.chat, &record.text).await {
            Ok(search) => search,
            Err(error) => {
                tracing::warn!(chat = record.chat.0, error = %self.failure_line(&error), "related record search failed");
                let log = RelatedLog::none(base.up_to, "retrieval_failed");
                return (source, Some(log));
            }
        };
        let carried: HashSet<(EvidenceKind, u64)> = base
            .protected()
            .iter()
            .map(|item| (kind_of(item.role), item.id))
            .collect();
        let room = related_room_chars(&base, budget);
        let mut used = 0;
        let mut picked = Vec::new();
        let mut omitted = Vec::new();
        let candidates = search
            .ranked
            .into_iter()
            .filter(|found| !carried.contains(&(found.kind, found.id)))
            .take(MAX_CONSIDERED);
        for found in candidates {
            let reference = (found.kind, found.id, found.hash.clone());
            let item = RelatedRecord {
                kind: found.kind,
                chat: search.chat,
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
        if picked.is_empty() && omitted.is_empty() {
            return (source, Some(RelatedLog::none(search.tail, "no_candidates")));
        }
        let log = RelatedLog {
            tail: search.tail,
            picked: picked
                .iter()
                .map(|item| (item.kind, item.id, item.hash.clone()))
                .collect(),
            omitted,
        };
        if picked.is_empty() {
            return (source, Some(log));
        }
        let mut merged = base;
        // 기록 없이 제약만 있던 재료도 패킷 기록에 후보를 읽은 채팅 revision이 남게 한다
        merged.up_to = merged.up_to.max(search.tail);
        merged.related = picked;
        (Some(merged), Some(log))
    }
}

/// 패킷의 대화 본문 역할에 맞는 근거 종류. 패킷과 근거 조회가 같은 번호를 쓴다.
fn kind_of(role: Role) -> EvidenceKind {
    match role {
        Role::User => EvidenceKind::Input,
        Role::Steer => EvidenceKind::Steer,
        Role::Assistant => EvidenceKind::Text,
    }
}
