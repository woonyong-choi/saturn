//! 실험 옵션 `context.select.packet = jev`: 패킷을 만들 때 router `compact` 판단으로 경쟁 구역의 순서를 정한다.
//! 설계: docs/design/context-management.md#패킷-판단의-적용, docs/design/context-selection.md#router에-넘기기

use std::time::{Duration, SystemTime};

use saturn_core::queue::SendAction;
use saturn_core::routers::failure::RETRY_DEADLINE;
use saturn_core::routers::{
    CompactCandidate, RouterError, RouterRequest, SET_COMPACT, compact_omitted_bytes,
    compact_overflow_bytes, compact_questions, compact_state, compact_verdicts,
};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, Provider, SessionId, SettingsRevision,
};
use saturn_protocol::state::InputState;

use crate::flow::{JobKind, RouterDone, RouterJob};

use crate::handoff::{Selection, compact_material};
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::store::{JudgmentOutcome, LedgerRow, SteeredInput, StoreError};
use crate::{Engine, EngineError, masked_chain};

/// router 답을 기다리는 시간. 넘어서 온 답은 늦은 답이라 쓰지 않는다. 호출 하나의 재시도 마감과 같다.
const COMPACT_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    RETRY_DEADLINE
};

/// 패킷을 만드는 계기. 키 비교에서 같은 입력인지 본다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// 답을 받았다. 항목마다 호출과 결과 중 큰 남김 확률이고 답이 없는 항목은 빠져 있다. 선별 근거를 함께 싣는다.
    Judged(Vec<(LedgerSeq, f64)>, Selection),
    /// 물을 후보가 없다. router를 부르지 않았고 실패도 아니다.
    NoCandidates,
    /// 판단을 받지 못했다: router 실패, 답 전부 없음, 필수 state가 한도에 안 들어감, 늦은 답, 키가 두 번 달라짐.
    /// 순위로 대체하는 근거를 싣는다.
    Unavailable(Selection),
}

/// 판단 한 번을 요청하는 쪽이 정하는 값.
#[derive(Debug, Clone)]
pub(crate) struct CompactAsk {
    pub(crate) chat: ChatId,
    /// 입력과 무관한 호출(맥락 정리)은 `None`.
    pub(crate) input: Option<InputId>,
    pub(crate) settings: SettingsRevision,
    pub(crate) key: TransitionKey,
}

/// 물을 준비를 마친 호출 하나. 후보와 요청 글은 이 시점의 기록으로 만들었다.
#[derive(Debug, Clone)]
pub(crate) struct CompactCall {
    ask: CompactAsk,
    request: RouterRequest,
    seqs: Vec<LedgerSeq>,
    /// 질문에 싣지 못해 잘린 후보 글의 바이트.
    omitted_bytes: u64,
    /// 키가 달라 다시 묻는 호출이다.
    is_retry: bool,
}

/// 호출이 돌아온 뒤 이어 갈 일.
#[derive(Debug, Clone)]
pub(crate) enum CompactResume {
    /// 판단을 기다리던 입력 전달.
    Send(SendAction),
    /// 판단을 기다리던 턴 경계의 맥락 정리. 끝난 턴의 에이전트.
    Boundary(AgentId),
}

/// 답을 기다리는 호출. 채팅마다 하나.
#[derive(Debug)]
pub(crate) struct CompactWait {
    call: CompactCall,
    resume: CompactResume,
}

/// 돌아온 답. 적용 직전에 키를 비교해 쓰거나 버린다. 기다림 안에 오지 않았으면 `exchange`가 없다.
#[derive(Debug)]
pub(crate) struct CompactReply {
    call: CompactCall,
    exchange: Option<RouterExchange>,
    /// 판단 기록을 썼다.
    is_recorded: std::sync::atomic::AtomicBool,
}

/// 적용 직전 자리의 결과.
pub(crate) enum CompactGate {
    Ready(CompactOrder),
    /// 물을 호출이 남았다. 호출자가 `spawn_compact`로 별도 작업에 맡긴다.
    Ask(CompactCall),
}

impl Engine {
    /// 적용 직전 자리. 돌아온 답이 있으면 키를 비교해 쓰고, 키가 다르면 답을 `superseded`로 버리고 한 번 다시 묻는다.
    /// 다시 물은 답도 키가 다르면 판단 없이 진행한다. 답이 없으면 물을 호출을 만든다. router는 여기서 부르지 않는다.
    /// 판단은 성공이든 실패든 판단 기록에 남는다.
    pub(crate) async fn compact_gate(
        &self,
        ask: CompactAsk,
        key_now: TransitionKey,
    ) -> CompactGate {
        let slot = (ask.chat, ask.key.trigger);
        let Some(reply) = self.flow.compact_replies.get(&slot) else {
            return self.compact_call(ask, false).await;
        };
        let call = &reply.call;
        let Some(exchange) = &reply.exchange else {
            return CompactGate::Ready(CompactOrder::Unavailable(Selection::fell_back(
                "late",
                call.omitted_bytes,
            )));
        };
        let verdicts: Vec<(LedgerSeq, f64)> = match &exchange.result {
            Ok(response) => compact_verdicts(&call.seqs, std::slice::from_ref(response))
                .into_iter()
                .filter(|(_, probability)| (0.0..=1.0).contains(probability))
                .collect(),
            Err(_) => Vec::new(),
        };
        let settled = settle(&call.ask.key, &key_now, call.is_retry);
        let outcome = match settled {
            Settle::Apply => outcome_of(&exchange.result),
            Settle::Retry | Settle::Fallback => JudgmentOutcome::Superseded,
        };
        self.record_compact(
            &call.ask,
            &call.request,
            exchange,
            (outcome, verdicts.len(), call.seqs.len()),
        )
        .await;
        reply
            .is_recorded
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let omitted = call.omitted_bytes;
        match settled {
            Settle::Apply if verdicts.is_empty() => CompactGate::Ready(CompactOrder::Unavailable(
                Selection::fell_back(unusable_reason(outcome), omitted),
            )),
            Settle::Apply => {
                let partial = (verdicts.len() < call.seqs.len()).then_some("partial");
                CompactGate::Ready(CompactOrder::Judged(
                    verdicts,
                    Selection::judged(partial, omitted),
                ))
            }
            Settle::Fallback => CompactGate::Ready(CompactOrder::Unavailable(
                Selection::fell_back("superseded", omitted),
            )),
            Settle::Retry => {
                let again = CompactAsk {
                    key: key_now,
                    ..call.ask.clone()
                };
                self.compact_call(again, true).await
            }
        }
    }

    /// 판단 재료로 읽는 기록 행과 끼워 넣어 적용한 입력. 보호 본문을 만드는 읽기와 같다.
    async fn compact_records(
        &self,
        chat: ChatId,
    ) -> Result<(Vec<LedgerRow>, Vec<SteeredInput>), StoreError> {
        Ok((
            self.store.ledger_since(chat, LedgerSeq(0)).await?,
            self.store.steered_inputs(chat).await?,
        ))
    }

    /// 지금 기록으로 후보와 요청을 만든다. 물을 후보가 없거나 요청을 만들 수 없으면 호출 없이 끝낸다.
    async fn compact_call(&self, ask: CompactAsk, is_retry: bool) -> CompactGate {
        let (rows, steers) = match self.compact_records(ask.chat).await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&EngineError::Store(error)), "failed to read the records for the compact judgment");
                return CompactGate::Ready(CompactOrder::Unavailable(Selection::fell_back(
                    "store-failed",
                    0,
                )));
            }
        };
        let current = ask
            .input
            .and_then(|input| self.queue.input(input))
            .map(|input| input.text.as_str());
        let (candidates, inputs) = compact_material(&rows, &steers, current);
        if candidates.is_empty() {
            return CompactGate::Ready(CompactOrder::NoCandidates);
        }
        let (request, omitted_bytes) = match self.compact_request(&candidates, &inputs) {
            Ok(built) => built,
            Err(overflow) => {
                return CompactGate::Ready(CompactOrder::Unavailable(Selection::over_limit(
                    overflow as u64,
                )));
            }
        };
        CompactGate::Ask(CompactCall {
            seqs: candidates.iter().map(|candidate| candidate.seq).collect(),
            omitted_bytes,
            ask,
            request,
            is_retry,
        })
    }

    /// 호출을 별도 작업으로 보내고 기다리지 않는다. 같은 채팅은 답이 올 때까지 다음 입력을 보내지 않는다.
    /// 답은 `on_routed`가 `on_compact_done`으로 넘긴다. 기다림(`COMPACT_WAIT`)을 넘긴 답은 버린다.
    pub(crate) fn spawn_compact(&mut self, call: CompactCall, resume: CompactResume) {
        let chat = call.ask.chat;
        let trigger = call.ask.key.trigger;
        let job = RouterJob {
            chat,
            input: call.ask.input.unwrap_or(InputId(0)),
            revision: self.queue.revision(chat),
            retried: call.is_retry,
            kind: JobKind::Compact {
                trigger,
                is_late: false,
            },
        };
        let request = call.request.clone();
        self.flow
            .compact_waiting
            .insert(chat, CompactWait { call, resume });
        let router = self.routers.shared();
        let results = self.flow.router_tx.clone();
        tokio::spawn(async move {
            let done =
                match tokio::time::timeout(COMPACT_WAIT, router.exchange(request.clone())).await {
                    Ok(exchange) => RouterDone {
                        job,
                        request,
                        exchange,
                    },
                    Err(_) => {
                        tracing::warn!("compact judgment took too long, ignoring it");
                        RouterDone {
                            job: RouterJob {
                                kind: JobKind::Compact {
                                    trigger,
                                    is_late: true,
                                },
                                ..job
                            },
                            request,
                            exchange: RouterExchange {
                                sent: String::new(),
                                received: None,
                                result: Err(RouterError::NoResponse),
                                started_at: SystemTime::now(),
                                elapsed: COMPACT_WAIT,
                                unknown_cost_calls: 0,
                            },
                        }
                    }
                };
            // engine가 끝난 뒤에는 받을 곳이 없다
            let _ = results.send(done);
        });
    }

    /// 적용 자리를 거치지 않은 답을 지운다. 계획이 `compact` 판단을 쓰지 않게 바뀌었거나(전환이 사라짐) 입력이 사라진 경우라
    /// 답은 쓰지 않았으므로 `superseded`로 기록한다.
    pub(crate) async fn discard_compact_reply(&mut self, chat: ChatId, trigger: Trigger) {
        let Some(reply) = self.flow.compact_replies.remove(&(chat, trigger)) else {
            return;
        };
        let is_recorded = reply.is_recorded.load(std::sync::atomic::Ordering::Relaxed);
        let Some(exchange) = reply.exchange.filter(|_| !is_recorded) else {
            return;
        };
        let call = reply.call;
        self.record_compact(
            &call.ask,
            &call.request,
            &exchange,
            (JudgmentOutcome::Superseded, 0, call.seqs.len()),
        )
        .await;
    }

    /// 돌아온 답을 보관하고 기다리던 일을 이어 간다. 적용 직전 키 비교는 이어 가는 쪽의 `compact_gate`가 한다.
    pub(crate) async fn on_compact_done(&mut self, done: RouterDone) {
        let JobKind::Compact { trigger, is_late } = done.job.kind else {
            return;
        };
        let chat = done.job.chat;
        let Some(wait) = self.flow.compact_waiting.remove(&chat) else {
            return;
        };
        let exchange = (!is_late).then_some(done.exchange);
        self.flow.compact_replies.insert(
            (chat, trigger),
            CompactReply {
                call: wait.call,
                exchange,
                is_recorded: std::sync::atomic::AtomicBool::new(false),
            },
        );
        self.resume_after_compact(chat, trigger, wait.resume).await;
    }

    /// 답을 보관한 뒤 판단을 기다리던 입력 전달이나 턴 경계 처리를 이어 간다.
    async fn resume_after_compact(
        &mut self,
        chat: ChatId,
        trigger: Trigger,
        resume: CompactResume,
    ) {
        match resume {
            CompactResume::Send(action) => self.resume_send(chat, trigger, action).await,
            CompactResume::Boundary(agent) => {
                let Some(live) = self.flow.live.get(&agent).cloned() else {
                    self.discard_compact_reply(chat, trigger).await;
                    return;
                };
                if let Err(error) = self.after_turn_value(chat, &live).await {
                    tracing::warn!(chat = chat.0, error = %masked_chain(&self.masker, &error), "failed to continue after the compact judgment");
                }
            }
        }
    }

    /// 기다리는 사이 입력이 사라졌거나 상태가 바뀌었으면 보내지 않고 답을 버린다.
    async fn resume_send(&mut self, chat: ChatId, trigger: Trigger, action: SendAction) {
        let input = match action {
            SendAction::Steer { input, .. }
            | SendAction::NewTurn { input, .. }
            | SendAction::NewTask { input, .. } => input,
        };
        let is_waiting = self
            .queue
            .input(input)
            .is_some_and(|record| record.state == InputState::Queued);
        if !is_waiting {
            self.discard_compact_reply(chat, trigger).await;
        } else if let Err(error) = self.deliver(chat, action).await {
            tracing::warn!(chat = chat.0, error = %masked_chain(&self.masker, &error), "failed to send the input after the compact judgment");
        }
        self.advance(chat).await;
    }

    /// 현재 요청과 기록된 사용자 입력 전체를 `state`에, 후보 내용을 질문에 싣는다. 비밀값은 가리고 절대 경로는 끝 이름만 남긴다.
    /// 목표와 정정은 줄이지 않으므로 `state`와 질문 하나가 크기 한도를 넘으면 요청을 만들지 않고 한도를 넘은 바이트를 돌려준다.
    /// 요청과 함께 돌려주는 값은 가린 뒤 실제 질문을 만드는 글 기준으로 센, 질문에 싣지 못한 후보 글의 바이트다.
    fn compact_request(
        &self,
        candidates: &[CompactCandidate],
        inputs: &[String],
    ) -> Result<(RouterRequest, u64), usize> {
        let model = self.routers.active().model().to_owned();
        let state = sanitize_state(&compact_state(inputs, candidates.len()), &self.masker);
        let clean: Vec<CompactCandidate> = candidates
            .iter()
            .map(|candidate| CompactCandidate {
                seq: candidate.seq,
                call: sanitize_state(&candidate.call, &self.masker),
                result: sanitize_state(&candidate.result, &self.masker),
            })
            .collect();
        let overflow = compact_overflow_bytes(&model, &state, &clean);
        if overflow > 0 {
            tracing::warn!(
                overflow,
                "compact state and question are over the size limit"
            );
            return Err(overflow);
        }
        let omitted = compact_omitted_bytes(&clean) as u64;
        let request = RouterRequest {
            model,
            state,
            sets: vec![compact_questions(&clean)],
        };
        Ok((request, omitted))
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

/// 판단은 있었지만 쓸 답이 없을 때 패킷 근거에 남기는 고정 사유.
fn unusable_reason(outcome: JudgmentOutcome) -> &'static str {
    match outcome {
        JudgmentOutcome::Ok => "unanswered",
        JudgmentOutcome::Invalid => "invalid",
        _ => "router-failed",
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
