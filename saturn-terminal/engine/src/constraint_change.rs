//! 제약 해제와 예외: router 제안(`constraint_change`)과 사용자 명령을 같은 저장 변경으로 적용하고, 이번 작업 예외를 작업 수명에 묶는다.
//! 설계: docs/design/constraints.md#해제와-예외-판단

use saturn_core::constraints::pick_change_candidates;
use saturn_core::routers::constraint::{
    ChangeKind, ChangeVerdict, change_question, read_change, scoped_condition,
};
use saturn_core::routers::{JudgmentOutcome, RouterRequest, validate};
use saturn_protocol::ids::{ChatId, ConstraintId, InputId, JudgmentId, SettingsRevision, TaskId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::InputState;

use crate::flow::{JobKind, RouterDone, RouterJob};
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::store::{
    Actor, ChangeOutcome, ConstraintChange, ConstraintState, EventReason, NewChange,
};
use crate::{Engine, EngineError};

/// 해제·예외 판단의 답을 기다리는 입력. 요청을 보낼 때의 제약 revision과 질문에 실은 제약을 들고 있다.
#[derive(Debug, Clone)]
pub(crate) struct PendingChange {
    settings: SettingsRevision,
    /// 질문의 순서(`[k]`)대로의 제약 번호.
    constraints: Vec<ConstraintId>,
    /// 요청을 보낼 때 채팅의 제약 revision.
    revision: u64,
}

/// 판단 기록에 쓸 결과와 답이 유효한지. 유효한 답이 낡았으면 `Superseded`다.
fn judged_outcome(
    request: &RouterRequest,
    exchange: &RouterExchange,
    is_stale: bool,
) -> (JudgmentOutcome, bool) {
    let is_valid = exchange
        .result
        .as_ref()
        .is_ok_and(|response| validate(request, response).is_ok());
    let outcome = match (&exchange.result, is_valid, is_stale) {
        (Ok(_), true, true) => JudgmentOutcome::Superseded,
        (Ok(_), false, _) => JudgmentOutcome::Invalid,
        _ => outcome_of(&exchange.result),
    };
    (outcome, is_valid)
}

/// router가 읽은 변경 하나. 적용 전에 입력과 작업을 대조한다.
struct Proposal {
    constraint: ConstraintId,
    change: ConstraintChange,
    reason: Option<EventReason>,
}

impl Engine {
    /// 입력의 해제·예외 판단을 별도 작업으로 시작한다. `constraint.auto_apply`가 꺼져 있거나 유효 제약이 없거나 입력이
    /// 취소됐으면 보내지 않는다. 오류는 입력을 막지 않고 로그만 남긴다.
    pub(crate) async fn start_change(&mut self, input: InputId, retried: bool) {
        let started = self.try_start_change(input, retried).await;
        self.warn_failure("failed to start a constraint change judgment", started);
    }

    async fn try_start_change(&mut self, input: InputId, retried: bool) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        if record.state == InputState::Cancelled {
            return Ok(());
        }
        let settings = self.settings.at(&self.store, record.settings).await?;
        if !settings.constraint_auto_apply() {
            return Ok(());
        }
        let active: Vec<(ConstraintId, String, Vec<String>)> = self
            .store
            .constraints_of_chat(record.chat)
            .await?
            .into_iter()
            .filter(|constraint| constraint.state == ConstraintState::Active)
            .map(|constraint| (constraint.id, constraint.rule, constraint.scope))
            .collect();
        if active.is_empty() {
            return Ok(());
        }
        let revision = self.store.constraint_revision(record.chat).await?;
        let picked = pick_change_candidates(&active, &record.text);
        let numbered: Vec<String> = picked
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let (_, rule, _) = active.iter().find(|(candidate, _, _)| candidate == id)?;
                Some(format!("[{}] {rule}", index + 1))
            })
            .collect();
        let context = self.judge_context(&record).text;
        let state = format!(
            "constraints:\n{}\n{context}\nuser input: {}",
            numbered.join("\n"),
            record.text
        );
        let request = RouterRequest {
            model: self.routers.active().model().to_owned(),
            state: sanitize_state(&state, &self.masker),
            sets: vec![change_question(picked.len())],
        };
        let job = RouterJob {
            chat: record.chat,
            input,
            revision: self.queue.revision(record.chat),
            retried,
            kind: JobKind::Change,
        };
        self.flow.pending_change.insert(
            input,
            PendingChange {
                settings: record.settings,
                constraints: picked,
                revision,
            },
        );
        self.spawn_router_job(job, request);
        Ok(())
    }

    /// 해제·예외 판단의 답을 읽어 적용한다. 제약 revision이 요청 때와 다르면 한 번 다시 판단하고 또 다르면 적용하지 않는다.
    /// 판단은 항상 판단 기록에 남고, 적용은 `constraint.auto_apply`가 켜져 있을 때만 한다. 권한 모드 `full`도 이 정책을
    /// 바꾸지 않는다.
    pub(crate) async fn on_change_done(&mut self, done: RouterDone) {
        let RouterDone {
            job,
            request,
            exchange,
        } = done;
        let Some(pending) = self.flow.pending_change.remove(&job.input) else {
            return;
        };
        if let Some(alert) = self.routers.observe(&exchange) {
            self.notify_alert(job.chat, alert).await;
        }
        let Ok(record) = self.queued(job.input) else {
            return;
        };
        let Some(current) = self.current_revision(job.chat).await else {
            return;
        };
        let is_stale = current != pending.revision;
        let (outcome, is_valid) = judged_outcome(&request, &exchange, is_stale);
        let judgment = self
            .record_change_judgment(&pending, job.input, &request, &exchange, outcome)
            .await;
        if record.state == InputState::Cancelled || !is_valid {
            return;
        }
        if is_stale {
            if !job.retried {
                self.start_change(job.input, true).await;
            }
            return;
        }
        let Some(response) = exchange.result.as_ref().ok() else {
            return;
        };
        let applied = self
            .apply_verdict(&record, &pending, (&request, response), judgment)
            .await;
        self.warn_failure("failed to apply a constraint change", applied);
    }

    async fn current_revision(&self, chat: ChatId) -> Option<u64> {
        match self.store.constraint_revision(chat).await {
            Ok(revision) => Some(revision),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read the constraint revision");
                None
            }
        }
    }

    /// 낡지 않은 유효한 답을 읽어 적용한다. 저장된 설정이 꺼져 있으면 판단이 이미 도는 중이었어도 적용하지 않는다.
    async fn apply_verdict(
        &mut self,
        record: &saturn_core::queue::QueuedInput,
        pending: &PendingChange,
        (request, response): (&RouterRequest, &saturn_core::routers::RouterResponse),
        judgment: Option<JudgmentId>,
    ) -> Result<(), EngineError> {
        let settings = self.settings.at(&self.store, pending.settings).await?;
        if !settings.constraint_auto_apply() {
            return Ok(());
        }
        let verdict = read_change(
            request,
            response,
            pending.constraints.len(),
            &settings.thresholds(),
            self.routers.method(),
        );
        let is_full = self.is_full_mode(record.chat, record.settings).await;
        let Some(proposal) = self.proposal(record, pending, verdict, is_full) else {
            return Ok(());
        };
        self.apply_change(
            record.chat,
            proposal,
            (Actor::Router, Some(record.id), judgment),
            pending.revision,
        )
        .await
    }

    /// 읽은 판단을 적용할 변경으로 바꾼다. 종류를 확신하지 못한 요청은 `full`에서만 제약을 지우지 않는 이번 작업 예외로
    /// 적용하고(`확인 없이`), 그 밖에는 종류를 묻는 창이 생기기 전까지 아무것도 하지 않는다. 이번 작업 예외는 입력이 속한
    /// 작업을 알 때만 건다. 작업에 배정되기 전이면 제약을 그대로 둔다.
    fn proposal(
        &self,
        record: &saturn_core::queue::QueuedInput,
        pending: &PendingChange,
        verdict: ChangeVerdict,
        is_full: bool,
    ) -> Option<Proposal> {
        let (target, kind, reason) = match verdict {
            ChangeVerdict::None => return None,
            ChangeVerdict::Apply { target, kind } => (target, kind, None),
            ChangeVerdict::UnsureKind { target, .. } if is_full => {
                (target, ChangeKind::Once, Some(EventReason::Unconfirmed))
            }
            ChangeVerdict::UnsureKind { .. } => return None,
        };
        let constraint = *pending.constraints.get(target)?;
        let change = match kind {
            ChangeKind::Release => ConstraintChange::Release,
            ChangeKind::Once => ConstraintChange::Once { task: record.task? },
            ChangeKind::Scoped => ConstraintChange::Scoped {
                condition: scoped_condition(&record.text)?,
            },
        };
        Some(Proposal {
            constraint,
            change,
            reason,
        })
    }

    async fn record_change_judgment(
        &mut self,
        pending: &PendingChange,
        input: InputId,
        request: &RouterRequest,
        exchange: &RouterExchange,
        outcome: JudgmentOutcome,
    ) -> Option<JudgmentId> {
        let reason = match outcome {
            JudgmentOutcome::Ok => None,
            JudgmentOutcome::Superseded => Some("superseded"),
            JudgmentOutcome::Invalid => Some("invalid"),
            _ => Some("router-failed"),
        };
        let fallbacks = reason
            .map(|reason| {
                vec![(
                    saturn_core::routers::question_ids::CONSTRAINT_CHANGE.to_owned(),
                    reason.to_owned(),
                )]
            })
            .unwrap_or_default();
        let release = match self.settings.at(&self.store, pending.settings).await {
            Ok(settings) => settings.thresholds().constraint_release,
            Err(_) => saturn_core::routers::Thresholds::default().constraint_release,
        };
        let chat = self.queued(input).map(|record| record.chat).ok()?;
        let context = RecordContext {
            chat,
            input: Some(input),
            question_sets: request.sets.iter().map(|(id, _)| id.clone()).collect(),
            settings: pending.settings,
            fallbacks,
            outcome,
            thresholds: vec![("constraint_release".to_owned(), release)],
            asked_with: None,
        };
        match self.routers.record(&self.store, context, exchange).await {
            Ok(judgment) => judgment,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to record a constraint change judgment");
                None
            }
        }
    }

    /// 저장에 변경을 한 거래로 쓰고 TUI에 줄을 알린다. 제약 revision이 달라졌거나 대상이 유효하지 않으면 쓰지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Store`, 쓰지 못했으면 `StaleConstraint`.
    async fn apply_change(
        &mut self,
        chat: ChatId,
        proposal: Proposal,
        (actor, input, judgment): (Actor, Option<InputId>, Option<JudgmentId>),
        revision: u64,
    ) -> Result<(), EngineError> {
        let change = proposal.change.clone();
        let outcome = self
            .store
            .change_constraint(&NewChange {
                chat,
                constraint: proposal.constraint,
                change: proposal.change,
                actor,
                reason: proposal.reason,
                input,
                judgment,
                revision,
            })
            .await?;
        let ChangeOutcome::Applied { rule } = outcome else {
            return Err(EngineError::StaleConstraint);
        };
        let unconfirmed = proposal.reason == Some(EventReason::Unconfirmed);
        let notice = match change {
            ConstraintChange::Release => ChatNotice::ConstraintReleased { rule },
            ConstraintChange::Once { .. } => ChatNotice::ConstraintPaused { rule, unconfirmed },
            ConstraintChange::Scoped { condition } => {
                ChatNotice::ConstraintExcepted { rule, condition }
            }
        };
        self.notify_chat(chat, notice).await;
        Ok(())
    }

    /// 사용자가 제약 하나를 직접 영구 해제한다. router 제안과 달리 `constraint.auto_apply`와 권한 모드를 보지 않는다.
    /// `revision`은 화면이 본 제약 revision이다.
    ///
    /// # Errors
    /// 없는 제약이거나 revision이 낡았거나 유효 제약이 아니면 `StaleConstraint`, 쓰기 실패면 `Store`.
    pub(crate) async fn release_constraint(
        &mut self,
        constraint: ConstraintId,
        revision: u64,
    ) -> Result<(), EngineError> {
        let chat = self
            .store
            .chat_of_constraint(constraint)
            .await?
            .ok_or(EngineError::StaleConstraint)?;
        let proposal = Proposal {
            constraint,
            change: ConstraintChange::Release,
            reason: None,
        };
        self.apply_change(chat, proposal, (Actor::User, None, None), revision)
            .await
    }

    /// 작업이 끝나면 그 작업의 이번 작업 예외를 닫고 다시 유효해진 제약마다 줄을 남긴다. 오류는 로그만 남긴다.
    pub(crate) async fn end_task_exceptions(&mut self, task: TaskId) {
        match self.store.end_task_exceptions(task).await {
            Ok(resumed) => {
                for constraint in resumed {
                    self.notify_chat(
                        constraint.chat,
                        ChatNotice::ConstraintResumed {
                            rule: constraint.rule,
                        },
                    )
                    .await;
                }
            }
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to end the task exceptions");
            }
        }
    }

    /// 시작할 때 이미 없는 작업의 이번 작업 예외를 닫는다. 보류로 되살린 작업의 예외는 그 작업이 끝날 때까지 남는다.
    pub(crate) async fn reconcile_task_exceptions(&mut self) {
        let tasks = match self.store.open_once_exception_tasks().await {
            Ok(tasks) => tasks,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read the task exceptions");
                return;
            }
        };
        let alive: Vec<TaskId> = self
            .queue
            .main_tasks()
            .into_iter()
            .map(|info| info.task)
            .collect();
        for task in tasks.into_iter().filter(|task| !alive.contains(task)) {
            self.end_task_exceptions(task).await;
        }
    }
}
