//! 턴 끝: 마지막 턴 값 기록, 작업 끝 알림, 턴 경계에서만 하는 맥락 정리, 다음 입력 전송.
//! 설계: docs/design/context-management.md#compaction-판정, docs/design/providers-and-sessions.md

use std::time::{Duration, Instant, SystemTime};

use saturn_core::providers::ProviderError;
use saturn_core::sessions::LastTurn;
use saturn_core::sessions::context::{CompactionDecision, ContextMeasure, decide};
use saturn_protocol::ids::{AgentId, ChatId, LedgerSeq};
use saturn_protocol::rpc::{ChatNotice, Notification};

use crate::flow::LiveSession;
use crate::handoff::{HandoffOutcome, handoff_of, handoff_source};
use crate::settings::ContextMode;
use crate::switch::Reduction;
use crate::{Engine, EngineError};

impl Engine {
    /// 트리가 유휴가 된 턴 끝에서 다음 순서로 처리한다. 마지막 턴 값을 기록하고, 작업을 끝내고,
    /// 맥락 정리를 판정하고, 그 뒤에야 기다리던 입력을 보낸다. 정리가 새 session으로 바꾸는 일은 이 경계에서만 한다.
    ///
    /// # Errors
    /// 열린 session이 없으면 `Provider(NotSent)`, 기록 실패면 `Store`. 맥락 정리 오류는 로그만 남기고 입력은 보낸다.
    pub(crate) async fn on_turn_end(
        &mut self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<(), EngineError> {
        let live = self
            .flow
            .live
            .get(&agent)
            .cloned()
            .ok_or(EngineError::NoRun { agent })?;
        self.clear_permissions(agent).await;
        self.record_turn_value(&live).await?;
        self.end_task(chat, agent).await?;
        if let Err(error) = self.compact_at_boundary(chat, &live).await {
            tracing::warn!(chat = chat.0, error = %self.failure_line(&error), "context compaction skipped");
        }
        self.restart_stale_connections(chat).await;
        self.dispatch_next(chat).await
    }

    /// 마지막 활성 맥락과 끝 시각을 기록하고 session을 유휴로 둔다. `A`를 모르면 값을 기록하지 않는다.
    async fn record_turn_value(&mut self, live: &LiveSession) -> Result<(), EngineError> {
        let active = self.flow.context_tokens.get(&live.agent).copied().flatten();
        if let Some(active) = active {
            let last_turn = LastTurn {
                active,
                ended_at: SystemTime::now(),
            };
            self.finish_turn(live.session, last_turn).await?;
        }
        self.sessions.mark_idle(live.session, Instant::now());
        self.persist_sessions(live.session).await
    }

    /// 트리 유휴이고 합칠 대기 입력이 없으며 `A`를 알 때만 `decide`를 부르고, `Restart`면 새 session으로 이어 간다.
    /// 유휴 복귀 조건(경과 시간)은 다음 입력이 올 때 판정할 일(`plan_open`)이라 여기서는 경과 0으로 본다.
    /// `context.mode`가 `provider`면 판정하지 않고 provider 자동 압축에 맡긴다. 기대 잔여 턴은 모르는 값(기본 3)으로 둔다.
    async fn compact_at_boundary(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
    ) -> Result<(), EngineError> {
        let Some(active) = self.flow.context_tokens.get(&live.agent).copied().flatten() else {
            return Ok(());
        };
        if self.context_mode().await? == ContextMode::Provider {
            return Ok(());
        }
        let budget = self.context_budget(live.provider).await?;
        if active < budget.threshold() {
            return Ok(());
        }
        let rows = self.store.ledger_since(chat, LedgerSeq(0)).await?;
        let source = handoff_source(
            &rows,
            &self.pending_work(chat, None),
            &self.registry.instruction_docs(),
        );
        let outcome = source
            .as_ref()
            .map_or(HandoffOutcome::Empty, |source| handoff_of(source, &budget));
        let packet = match &outcome {
            HandoffOutcome::Ready(handoff) => handoff.tokens,
            HandoffOutcome::Empty | HandoffOutcome::Deferred { .. } => 0,
        };
        let measure = ContextMeasure {
            active: Some(active),
            packet,
            tree_idle: self.agents.is_tree_idle(live.agent),
            has_mergeable_queue: self.queue.has_waiting(chat),
            since_last_turn: Duration::ZERO,
            expected_turns: None,
        };
        if decide(&budget, &measure) != CompactionDecision::Restart {
            return Ok(());
        }
        match outcome {
            HandoffOutcome::Ready(handoff) => {
                if handoff.is_over_limit {
                    tracing::warn!(
                        chat = chat.0,
                        tokens = handoff.tokens,
                        "packet exceeds its limit"
                    );
                }
                let up_to = rows.last().map_or_else(Default::default, |row| row.seq);
                let reduction = source.map(|source| Reduction {
                    source,
                    budget,
                    sent_tokens: handoff.tokens,
                });
                self.restart_and_notify(chat, live, handoff.text, reduction, up_to)
                    .await?;
            }
            HandoffOutcome::Deferred { constraints } => {
                self.notify_chat(chat, ChatNotice::ContextDeferred { constraints })
                    .await;
            }
            HandoffOutcome::Empty => {}
        }
        Ok(())
    }

    /// 새 session으로 이어 가고 알린다. 줄인 패킷도 맥락 한도로 거절되면 옛 session을 그대로 두고 `PacketOverflow`를 알린다.
    /// 맥락 정리는 기다리는 입력이 없을 때만 하므로 보류할 입력은 없다.
    async fn restart_and_notify(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
        packet: String,
        reduction: Option<Reduction>,
        up_to: LedgerSeq,
    ) -> Result<(), EngineError> {
        match self
            .restart_session(chat, live, packet, reduction, up_to)
            .await
        {
            Ok(()) => self.notify_chat(chat, ChatNotice::Compacted).await,
            Err(EngineError::Provider(ProviderError::ContextExceeded { .. })) => {
                self.notify_chat(chat, ChatNotice::PacketOverflow).await;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub(crate) async fn notify_chat(&self, chat: ChatId, notice: ChatNotice) {
        let notification = Notification::ChatNotice {
            chat,
            task: None,
            notice,
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }
}
