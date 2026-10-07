//! 턴 끝: 마지막 턴 값 기록, 작업 끝 알림, 턴 경계에서만 하는 맥락 정리, 다음 입력 전송.
//! 설계: docs/design/context-management.md#compaction-판정, docs/design/providers-and-sessions.md

use std::time::{Duration, Instant, SystemTime};

use saturn_core::sessions::LastTurn;
use saturn_core::sessions::context::{CompactionDecision, ContextMeasure, decide};
use saturn_protocol::ids::{AgentId, ChatId, TaskId};
use saturn_protocol::rpc::{ChatNotice, Notification};

use crate::flow::LiveSession;
use crate::handoff::{HandoffOutcome, PacketEvidence, RECENT_SELECTOR, handoff_of, handoff_source};
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
        self.after_turn_value(chat, &live).await
    }

    /// 맥락 정리를 판정하고, 새 session을 열지 않으면 기다리던 입력을 보낸다.
    pub(crate) async fn after_turn_value(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
    ) -> Result<(), EngineError> {
        match self.compact_at_boundary(chat, live).await {
            // 새 session 열기를 기다린다. 끝나면 거기서 이어 간다
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(chat = chat.0, error = %self.failure_line(&error), "context compaction skipped");
            }
        }
        self.restart_stale_connections(chat).await;
        self.dispatch_next(chat).await
    }

    /// 마지막 활성 맥락과 끝 시각을 기록하고 session을 유휴로 둔다. `A`를 모르면 값을 기록하지 않는다.
    pub(crate) async fn record_turn_value(
        &mut self,
        live: &LiveSession,
    ) -> Result<(), EngineError> {
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

    /// 트리 유휴이고 합칠 대기 입력이 없으며 `A`를 알 때만 `decide`를 부르고, `Restart`면 새 session 열기를 맡긴다.
    /// 맡겼으면 참이고, 다음 입력은 새 session이 열린 뒤에 보낸다.
    /// 유휴 복귀 조건(경과 시간)은 다음 입력이 올 때 판정할 일(`plan_open`)이라 여기서는 경과 0으로 본다.
    /// `context.mode`가 `provider`면 판정하지 않고 provider 자동 압축에 맡긴다. 기대 잔여 턴은 모르는 값(기본 3)으로 둔다.
    async fn compact_at_boundary(
        &mut self,
        chat: ChatId,
        live: &LiveSession,
    ) -> Result<bool, EngineError> {
        let Some(active) = self.flow.context_tokens.get(&live.agent).copied().flatten() else {
            return Ok(false);
        };
        if self.context_mode(chat, live.agent).await? == ContextMode::Provider {
            return Ok(false);
        }
        let budget = self.context_budget(chat, live.agent, live.provider).await?;
        if active < budget.threshold() {
            return Ok(false);
        }
        let (rows, steers, changes) = self.packet_material(chat).await?;
        let constraints = self.store.constraints_of_chat(chat).await?;
        let source = handoff_source(
            &rows,
            &steers,
            &changes,
            &self.pending_work(chat, None),
            (&constraints, &self.registry.instruction_docs()),
            &budget,
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
            return Ok(false);
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
                // 패킷은 재료가 있어야 만들어지므로 `Ready`면 재료가 있다
                let Some(source) = source else {
                    return Ok(false);
                };
                let evidence = PacketEvidence::first(&handoff, &source, RECENT_SELECTOR);
                let reduction = Reduction {
                    source,
                    budget,
                    selector: RECENT_SELECTOR,
                    sent_tokens: handoff.tokens,
                };
                self.restart_session(chat, live, handoff.text, Some(reduction), evidence)
                    .await?;
                return Ok(true);
            }
            HandoffOutcome::Deferred { constraints } => {
                self.notify_chat(chat, ChatNotice::ContextDeferred { constraints })
                    .await;
            }
            HandoffOutcome::Empty => {}
        }
        Ok(false)
    }

    pub(crate) async fn notify_chat(&self, chat: ChatId, notice: ChatNotice) {
        let notification = Notification::ChatNotice {
            chat,
            task: None,
            notice,
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }

    /// 끝난 작업에 붙는 알림. TUI가 그 작업 이름표 옆에 그린다.
    pub(crate) async fn notify_chat_task(&self, chat: ChatId, task: TaskId, notice: ChatNotice) {
        let notification = Notification::ChatNotice {
            chat,
            task: Some(task),
            notice,
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }
}
