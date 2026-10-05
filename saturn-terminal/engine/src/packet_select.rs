//! 실험 옵션 `context.select.packet = jev`: 패킷을 만들 때 router `compact` 판단으로 경쟁 구역의 순서를 정한다.
//! 설계: docs/design/context-management.md#패킷-판단의-적용, docs/design/context-selection.md#router에-넘기기

use std::time::Duration;

use saturn_core::routers::failure::RETRY_DEADLINE;
use saturn_core::routers::{
    CompactCandidate, RouterRequest, SET_COMPACT, compact_questions, compact_requests,
    compact_state, compact_verdicts,
};
use saturn_protocol::ids::{ChatId, InputId, LedgerSeq, Provider, SessionId, SettingsRevision};

use crate::handoff::compact_material;
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::store::JudgmentOutcome;
use crate::{Engine, EngineError};

/// router 답을 기다리는 시간. 넘어서 온 답은 늦은 답이라 쓰지 않는다. 호출 하나의 재시도 마감과 같다.
const COMPACT_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    RETRY_DEADLINE
};

/// 패킷을 만드는 계기. 키 비교에서 같은 입력인지 본다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    Input(InputId),
    /// 맥락 정리.
    Compaction,
}

/// 판단이 향한 전환. 채팅, 보내는 쪽 메인 session, 받는 쪽 provider와 모델, 전환을 일으킨 입력이다.
/// 기록은 추가만 되므로 기록 번호는 키에 넣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransitionKey {
    pub(crate) chat: ChatId,
    pub(crate) from: Option<SessionId>,
    pub(crate) provider: Provider,
    pub(crate) model: Option<String>,
    pub(crate) trigger: Trigger,
}

/// 적용 직전 키 비교의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settle {
    Apply,
    /// 판단을 `superseded`로 버리고 재료를 다시 모아 한 번 다시 묻는다.
    Retry,
    /// 다시 물었는데도 다르다. 판단 없이 진행한다.
    Fallback,
}

/// 판단을 요청한 때의 키와 적용 직전의 키를 비교한다. `is_retry`는 이미 한 번 다시 물은 뒤인지다.
pub(crate) fn settle(asked: &TransitionKey, now: &TransitionKey, is_retry: bool) -> Settle {
    match (asked == now, is_retry) {
        (true, _) => Settle::Apply,
        (false, false) => Settle::Retry,
        (false, true) => Settle::Fallback,
    }
}

/// `compact` 판단의 결과.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CompactOrder {
    /// 답을 받았다. 항목마다 호출과 결과 중 큰 남김 확률이고 답이 없는 항목은 빠져 있다.
    Judged(Vec<(LedgerSeq, f64)>),
    /// 물을 후보가 없다. router를 부르지 않았고 실패도 아니다.
    NoCandidates,
    /// 판단을 받지 못했다: router 실패, 답 전부 없음, 크기 한도, 늦은 답, 키가 두 번 달라짐.
    Unavailable,
}

/// 판단 한 번을 요청하는 쪽이 정하는 값.
pub(crate) struct CompactAsk {
    pub(crate) chat: ChatId,
    /// 입력과 무관한 호출(맥락 정리)은 `None`.
    pub(crate) input: Option<InputId>,
    pub(crate) settings: SettingsRevision,
    pub(crate) key: TransitionKey,
}

impl Engine {
    /// 후보 전체를 `compact`로 묻고 적용 직전 키를 비교한다. 키가 다르면 한 번 다시 묻고, 또 다르면 판단 없이 진행한다.
    /// 판단은 성공이든 실패든 판단 기록에 남는다.
    ///
    /// router 호출을 이 자리에서 기다리므로 그동안 engine의 다른 요청은 기다린다. 기다림은 `COMPACT_WAIT`로 제한한다.
    pub(crate) async fn compact_order(
        &self,
        ask: CompactAsk,
        key_now: impl Fn() -> TransitionKey,
    ) -> CompactOrder {
        let mut is_retry = false;
        loop {
            let rows = match self.store.ledger_since(ask.chat, LedgerSeq(0)).await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(error = %self.failure_line(&EngineError::Store(error)), "failed to read the records for the compact judgment");
                    return CompactOrder::Unavailable;
                }
            };
            let (candidates, inputs) = compact_material(&rows);
            if candidates.is_empty() {
                return CompactOrder::NoCandidates;
            }
            let Some(request) = self.compact_request(&candidates, &inputs) else {
                return CompactOrder::Unavailable;
            };
            let Some(exchange) = self.exchange_compact(&request).await else {
                return CompactOrder::Unavailable;
            };
            let seqs: Vec<LedgerSeq> = candidates.iter().map(|candidate| candidate.seq).collect();
            let verdicts = match &exchange.result {
                Ok(response) => compact_verdicts(&seqs, std::slice::from_ref(response))
                    .into_iter()
                    .filter(|(_, probability)| (0.0..=1.0).contains(probability))
                    .collect(),
                Err(_) => Vec::new(),
            };
            let settled = settle(&ask.key, &key_now(), is_retry);
            let outcome = match settled {
                Settle::Apply => outcome_of(&exchange.result),
                Settle::Retry | Settle::Fallback => JudgmentOutcome::Superseded,
            };
            self.record_compact(
                &ask,
                &request,
                &exchange,
                (outcome, verdicts.len(), seqs.len()),
            )
            .await;
            match settled {
                Settle::Apply if verdicts.is_empty() => return CompactOrder::Unavailable,
                Settle::Apply => return CompactOrder::Judged(verdicts),
                Settle::Fallback => return CompactOrder::Unavailable,
                Settle::Retry => is_retry = true,
            }
        }
    }

    /// 마지막 사용자 입력과 앞 입력 3개를 `state`에, 후보 내용을 질문에 싣는다. 비밀값은 가리고 절대 경로는 끝 이름만 남긴다.
    /// `state`와 질문 하나가 크기 한도를 넘으면 요청을 만들지 않는다.
    fn compact_request(
        &self,
        candidates: &[CompactCandidate],
        inputs: &[String],
    ) -> Option<RouterRequest> {
        let model = self.routers.active().model().to_owned();
        let state = sanitize_state(&compact_state(inputs), &self.masker);
        let clean: Vec<CompactCandidate> = candidates
            .iter()
            .map(|candidate| CompactCandidate {
                seq: candidate.seq,
                call: sanitize_state(&candidate.call, &self.masker),
                result: sanitize_state(&candidate.result, &self.masker),
            })
            .collect();
        if let Err(error) = compact_requests(&model, &state, &clean) {
            tracing::warn!(%error, "compact request is over the size limit");
            return None;
        }
        Some(RouterRequest {
            model,
            state,
            sets: vec![compact_questions(&clean)],
        })
    }

    /// 기다림 안에 끝난 호출만 돌려준다. 넘으면 호출을 끊고 늦은 답은 쓰지 않는다.
    async fn exchange_compact(&self, request: &RouterRequest) -> Option<RouterExchange> {
        let router = self.routers.active();
        match tokio::time::timeout(COMPACT_WAIT, router.exchange(request.clone())).await {
            Ok(exchange) => Some(exchange),
            Err(_) => {
                tracing::warn!("compact judgment took too long, ignoring it");
                None
            }
        }
    }

    async fn record_compact(
        &self,
        ask: &CompactAsk,
        request: &RouterRequest,
        exchange: &RouterExchange,
        (outcome, answered, asked): (JudgmentOutcome, usize, usize),
    ) {
        let reason = match outcome {
            JudgmentOutcome::Ok if answered < asked => Some("partial"),
            JudgmentOutcome::Ok => None,
            JudgmentOutcome::Superseded => Some("superseded"),
            JudgmentOutcome::Invalid => Some("invalid"),
            _ => Some("router-failed"),
        };
        let context = RecordContext {
            chat: ask.chat,
            input: ask.input,
            question_sets: request.sets.iter().map(|(id, _)| id.clone()).collect(),
            settings: ask.settings,
            fallbacks: reason
                .map(|reason| vec![(SET_COMPACT.to_owned(), reason.to_owned())])
                .unwrap_or_default(),
            outcome,
            thresholds: Vec::new(),
            asked_with: None,
        };
        if let Err(error) = self.routers.record(&self.store, context, exchange).await {
            tracing::warn!(error = %self.failure_line(&error), "failed to record a compact judgment");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(model: &str, trigger: Trigger) -> TransitionKey {
        TransitionKey {
            chat: ChatId(1),
            from: Some(SessionId(4)),
            provider: crate::providers::test_support::CLAUDE,
            model: Some(model.to_owned()),
            trigger,
        }
    }

    #[test]
    fn settle_applies_equal_keys_and_asks_again_once_when_they_differ() {
        let asked = key("opus", Trigger::Input(InputId(7)));
        let other_model = key("sonnet", Trigger::Input(InputId(7)));
        let other_input = key("opus", Trigger::Input(InputId(8)));
        let compaction = key("opus", Trigger::Compaction);
        let cases = [
            (&asked, false, Settle::Apply),
            (&asked, true, Settle::Apply),
            (&other_model, false, Settle::Retry),
            (&other_input, false, Settle::Retry),
            (&compaction, false, Settle::Retry),
            (&other_model, true, Settle::Fallback),
        ];
        for (now, is_retry, expected) in cases {
            assert_eq!(settle(&asked, now, is_retry), expected);
        }
    }
}
