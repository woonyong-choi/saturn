//! 입력 접수, 판단 차례, 판단 적용.
//! 설계: docs/design/input-handling.md, docs/design/router.md

use std::path::Path;
use std::time::Instant;

use saturn_core::queue::{QueueError, QueuedInput};
use saturn_core::routers::{
    JudgmentOutcome, RouteDecision, RouterError, RouterRequest, RouterResponse, decide_route,
    questions_for_input, validate,
};
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::{Disposition, InputState};

use crate::flow::{Routed, RouterDone, RouterJob, Unrecorded};
use crate::providers::parse_pinned;
use crate::requests::{settings_notification, trust_notification};
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::rpc::ClientId;
use crate::settings::{Settings, SettingsError};
use crate::{Engine, EngineError, masked_chain};

/// 판단 한 번의 결과.
pub(crate) struct Verdict {
    pub(crate) decision: RouteDecision,
    /// router 호출이 재시도 뒤에도 실패해 대체 판단을 쓴 경우.
    pub(crate) failed: bool,
}

impl Engine {
    /// 설정 번호와 권한은 접수 때 고정한다. 접수한 뒤의 판단과 전송 오류는 입력을 지우지 않고
    /// 로그만 남기며, 판단하지 못한 입력은 다음 판단 차례에 다시 판단한다.
    ///
    /// # Errors
    /// 붙지 않은 채팅이면 `ChatNotAttached`, 접수 기록 실패면 `Store`이고 입력은 어디에도 보내지 않는다.
    pub(crate) async fn submit_input(
        &mut self,
        client: ClientId,
        chat: ChatId,
        client_ref: u64,
        text: String,
        skip_relation: bool,
    ) -> Result<(), EngineError> {
        let input = self.accept_input(client, chat, text, skip_relation).await?;
        self.send(client, Notification::InputAccepted { client_ref, input })
            .await;
        self.notify_input(input).await;
        if let Err(error) = self.note_input_accepted(chat, Instant::now()).await {
            tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to settle judgment signals");
        }
        self.advance(chat).await;
        Ok(())
    }

    /// 보낼 것을 보내고 다음 입력의 판단을 시작한다. 판단 호출은 별도 작업으로 돌아 기다리지 않고,
    /// 결과는 `on_routed`로 돌아온다. 보내기를 먼저 해서 다음 판단이 앞 입력을 보낸 뒤의 상태를 본다.
    /// 오류는 로그만 남긴다.
    pub(crate) async fn advance(&mut self, chat: ChatId) {
        let result = self.advance_flow(chat).await;
        if let Err(error) = result {
            tracing::warn!(chat = chat.0, error = %masked_chain(&self.masker, &error), "input flow stopped");
        }
    }

    async fn advance_flow(&mut self, chat: ChatId) -> Result<(), EngineError> {
        self.dispatch_next(chat).await?;
        self.router_next(chat).await?;
        self.dispatch_next(chat).await
    }

    /// 별도 작업이 끝낸 router 호출의 결과를 받는다. 적용 직전에 채팅 revision을 비교하므로 호출이 도는 사이
    /// 멈춤이나 취소가 있었으면 결과는 버려진다. 적용하지 못한 오류는 입력을 지우지 않고 로그만 남긴다.
    pub(crate) async fn on_routed(&mut self, done: RouterDone) {
        let RouterDone {
            job,
            request,
            exchange,
        } = done;
        self.flow.judging.remove(&job.chat);
        match self.apply_routed(job, &request, exchange).await {
            Ok(()) => self.advance(job.chat).await,
            Err(error) => {
                tracing::warn!(chat = job.chat.0, error = %masked_chain(&self.masker, &error), "judgment not applied");
            }
        }
    }

    async fn apply_routed(
        &mut self,
        job: RouterJob,
        request: &RouterRequest,
        exchange: RouterExchange,
    ) -> Result<(), EngineError> {
        let verdict = self.finish_router(&job, request, exchange).await?;
        let waiting = self
            .queue
            .input(job.input)
            .is_some_and(|record| record.state == InputState::Judging);
        if waiting {
            self.apply_decision(job.input, verdict.decision, job.retried)
                .await
        } else {
            self.settle_record(job.input, true).await;
            Ok(())
        }
    }

    async fn accept_input(
        &mut self,
        client: ClientId,
        chat: ChatId,
        text: String,
        skip_relation: bool,
    ) -> Result<InputId, EngineError> {
        let workdir = self.attached_workdir(client, chat)?;
        let pinned_model = self.store.chat_model(chat).await?;
        let settings = self.fix_settings(client, chat, &workdir).await?;
        self.sync_provider_settings(chat, settings).await;
        let permission = self.input_permission(chat, settings).await;
        let new = crate::store::NewInput {
            chat,
            text,
            settings,
            permission,
            workdir,
            pinned_model,
            skip_relation,
        };
        let id = self.store.accept_input(&new).await?;
        self.queue.accept(QueuedInput {
            id,
            chat,
            text: new.text,
            settings,
            permission: new.permission,
            workdir: new.workdir,
            pinned_model: new.pinned_model,
            skip_relation,
            state: InputState::Judging,
            reason: None,
            task: None,
        });
        Ok(id)
    }

    /// 이 클라이언트가 그 채팅에 붙어 있어야 한다.
    pub(crate) fn require_attached(
        &self,
        client: ClientId,
        chat: ChatId,
    ) -> Result<(), EngineError> {
        let is_attached = self
            .attachments
            .get(&client)
            .is_some_and(|attachment| attachment.chat == chat);
        if is_attached {
            Ok(())
        } else {
            Err(EngineError::ChatNotAttached { chat })
        }
    }

    /// 이 클라이언트가 붙은 채팅의 작업 폴더.
    pub(crate) fn attached_workdir(
        &self,
        client: ClientId,
        chat: ChatId,
    ) -> Result<std::path::PathBuf, EngineError> {
        let attached = self
            .attachments
            .get(&client)
            .map(|attachment| attachment.chat);
        match (attached, self.chat_env(chat)) {
            (Some(attached), Some(env)) if attached == chat => Ok(env.workdir().to_path_buf()),
            _ => Err(EngineError::ChatNotAttached { chat }),
        }
    }

    /// 설정 파일이 바뀌었으면 다시 병합하고 그 번호를 쓴다. 새로 보이는 폴더 설정은 신뢰 창으로 묻는다.
    async fn fix_settings(
        &mut self,
        client: ClientId,
        chat: ChatId,
        workdir: &Path,
    ) -> Result<SettingsRevision, EngineError> {
        if !self.settings.changed(workdir).await? {
            return Ok(self
                .settings
                .current()
                .ok_or(SettingsError::NoPreviousRevision)?);
        }
        let (applied, prompt) = self
            .settings
            .apply_trusted(&self.store, Some(chat), workdir)
            .await?;
        let revision = applied.revision;
        if applied.warning.is_some() {
            self.rpc
                .broadcast(Some(chat), settings_notification(applied))
                .await;
        }
        if let Some(prompt) = prompt {
            self.send(client, trust_notification(&prompt)).await;
            self.set_folder_trust(client, Some(prompt));
        }
        Ok(revision)
    }

    /// 같은 채팅 입력을 접수 순서대로 하나씩 판단하고 적용한다. 관계 판단 없이 대기하는 입력(`skip_relation`)은
    /// router를 부르지 않는다. 모델을 고정한 입력은 관계 판단을 받고 `target_model`만 묻지 않는다.
    ///
    /// # Errors
    /// 판단 요청에 쓸 설정 번호를 읽지 못하면 `Settings`, 적용 오류는 `apply_decision`과 같다.
    pub(crate) async fn router_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        while !self.flow.judging.contains_key(&chat) {
            let Some((input, revision)) = self.queue.next_to_route(chat) else {
                break;
            };
            let record = self.queued(input)?;
            if record.skip_relation {
                let decision = direct_decision(&record, revision);
                self.apply_decision(input, decision, false).await?;
            } else {
                self.start_router(&record, revision, false);
            }
        }
        Ok(())
    }

    /// `RevisionConflict`면 `retried`가 거짓일 때만 한 번 다시 판단을 시작하고(결과는 `on_routed`로 온다),
    /// 또 어긋나면 대기로 둔다. 어긋난 판단은 `Superseded`로 기록한다.
    ///
    /// # Errors
    /// 없는 입력이거나 `Judging`이 아니면 `Queue`, 기록 저장소 쓰기 실패면 `Store`.
    pub(crate) async fn apply_decision(
        &mut self,
        input: InputId,
        decision: RouteDecision,
        retried: bool,
    ) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        let mut decision = decision;
        let mut retried = retried;
        loop {
            let current = self.queue.revision(record.chat);
            match self.queue.apply(input, &decision, current) {
                Ok(disposition) => {
                    return self
                        .after_applied(input, disposition, record.pinned_model.clone())
                        .await;
                }
                Err(QueueError::RevisionConflict) => {
                    self.settle_record(input, true).await;
                    if retried {
                        return self.wait_in_queue(input).await;
                    }
                    retried = true;
                    if !record.skip_relation {
                        self.start_router(&record, current, retried);
                        return Ok(());
                    }
                    decision = direct_decision(&record, current);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    async fn after_applied(
        &mut self,
        input: InputId,
        disposition: Disposition,
        before_model: Option<String>,
    ) -> Result<(), EngineError> {
        let applied = self.queued(input)?;
        let chat = applied.chat;
        if applied.pinned_model != before_model {
            self.store
                .set_input_model(input, applied.pinned_model.as_deref())
                .await?;
        }
        let judgment = self.settle_record(input, false).await;
        if let Some(judgment) = judgment {
            self.watch_judgment(chat, judgment, Instant::now());
        }
        self.flow.routed.insert(input, Routed { judgment });
        self.flow.last_disposition.insert(chat, disposition);
        self.flow.applied.push(input);
        self.store
            .set_input_state(input, InputState::Queued, None)
            .await?;
        Ok(())
    }

    /// 두 번 어긋난 입력은 처리 방식 없이 대기열에 둔다.
    async fn wait_in_queue(&mut self, input: InputId) -> Result<(), EngineError> {
        self.queue.set_state(input, InputState::Queued)?;
        self.flow.applied.push(input);
        self.store
            .set_input_state(input, InputState::Queued, None)
            .await?;
        Ok(())
    }

    /// router 호출을 별도 작업으로 시작하고 기다리지 않는다. 요청은 지금 상태로 만들고 `revision`은
    /// 결과를 적용할 때 비교하려고 호출 결과와 함께 돌려받는다.
    pub(crate) fn start_router(
        &mut self,
        record: &QueuedInput,
        revision: ChatRevision,
        retried: bool,
    ) {
        let running = self.chat_is_running(record.chat);
        let request = self.router_request(record, running);
        let job = RouterJob {
            chat: record.chat,
            input: record.id,
            revision,
            retried,
        };
        self.flow.judging.insert(record.chat, record.id);
        let router = self.routers.shared();
        let results = self.flow.router_tx.clone();
        tokio::spawn(async move {
            let exchange = router.exchange(request.clone()).await;
            // engine가 끝난 뒤에는 받을 곳이 없다
            let _ = results.send(RouterDone {
                job,
                request,
                exchange,
            });
        });
    }

    /// 돌아온 호출 결과를 읽는다. 기록은 적용 결과를 안 뒤 `settle_record`가 쓴다.
    pub(crate) async fn finish_router(
        &mut self,
        job: &RouterJob,
        request: &RouterRequest,
        exchange: RouterExchange,
    ) -> Result<Verdict, EngineError> {
        let record = self.queued(job.input)?;
        let settings = self.settings.at(&self.store, record.settings).await?;
        if let Some(alert) = self.routers.observe(&exchange) {
            self.notify_alert(record.chat, alert).await;
        }
        let mut read =
            self.read_verdict(request, &exchange, &settings, job.revision, record.settings);
        // 고정 모델은 `target_model` 선택만 대신하고 관계 판단은 그대로 받는다.
        // 고정하지 않았으면 후보가 아닌 글은 버려 현재 모델로 둔다
        if record.pinned_model.is_some() {
            read.decision.model.clone_from(&record.pinned_model);
        } else {
            read.decision.model = read
                .decision
                .model
                .take()
                .filter(|model| parse_pinned(model).is_some());
        }
        let fallbacks = read.fallback_reasons();
        let context = RecordContext {
            chat: record.chat,
            input: Some(record.id),
            question_sets: request.sets.iter().map(|(id, _)| id.clone()).collect(),
            settings: record.settings,
            fallbacks,
            outcome: read.outcome,
            thresholds: threshold_list(&settings),
            asked_with: None,
        };
        self.flow
            .unrecorded
            .insert(record.id, Unrecorded { context, exchange });
        Ok(Verdict {
            decision: read.decision,
            failed: read.failed,
        })
    }

    pub(crate) fn router_request(&self, record: &QueuedInput, running: bool) -> RouterRequest {
        let activity = if running { "running" } else { "idle" };
        let previous = self
            .flow
            .last_disposition
            .get(&record.chat)
            .map_or("none", |disposition| disposition_name(*disposition));
        let state = format!(
            "chat: {activity}\nprevious input handled as: {previous}\nuser input: {}",
            record.text
        );
        // TODO(#90): 보류 작업이 있으면 `resume_held`를 묻고 `note_resume_signal`로 잇는다
        RouterRequest {
            model: self.routers.active().model().to_owned(),
            state: sanitize_state(&state, &self.masker),
            sets: questions_for_input(
                running,
                record.pinned_model.is_some(),
                false,
                &self.model_candidates(record.chat),
            ),
        }
    }

    fn read_verdict(
        &self,
        request: &RouterRequest,
        exchange: &RouterExchange,
        settings: &Settings,
        revision: ChatRevision,
        settings_revision: SettingsRevision,
    ) -> ReadVerdict {
        let route = |response: &RouterResponse| {
            decide_route(
                (request, response),
                &settings.thresholds(),
                self.routers.method(),
                revision,
                settings_revision,
            )
        };
        let invalid = || {
            let empty = RouterResponse {
                model: request.model.clone(),
                answers: Vec::new(),
                tokens: (0, 0),
            };
            ReadVerdict {
                decision: route(&empty),
                outcome: JudgmentOutcome::Invalid,
                failed: false,
            }
        };
        match &exchange.result {
            Ok(response) if validate(request, response).is_ok() => ReadVerdict {
                decision: route(response),
                outcome: JudgmentOutcome::Ok,
                failed: false,
            },
            Ok(_) | Err(RouterError::Invalid { .. }) => invalid(),
            Err(_) => ReadVerdict {
                decision: self
                    .routers
                    .route_after_failure(request, revision, settings_revision),
                outcome: outcome_of(&exchange.result),
                failed: true,
            },
        }
    }

    /// 아직 기록하지 않은 판단을 쓴다. 어긋난 판단은 `Superseded`로 쓰고, 쓰기 실패는 로그만 남긴다.
    pub(crate) async fn settle_record(
        &mut self,
        input: InputId,
        superseded: bool,
    ) -> Option<JudgmentId> {
        let mut unrecorded = self.flow.unrecorded.remove(&input)?;
        if superseded {
            unrecorded.context.outcome = JudgmentOutcome::Superseded;
        }
        let recorded = self
            .routers
            .record(&self.store, unrecorded.context, &unrecorded.exchange)
            .await;
        match recorded {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to record judgment");
                None
            }
        }
    }

    /// 실행 중인 에이전트가 있는 채팅이면 참.
    pub(crate) fn chat_is_running(&self, chat: ChatId) -> bool {
        self.runs
            .chat_of
            .iter()
            .any(|(agent, owner)| *owner == chat && self.runs.active.contains_key(agent))
    }

    pub(crate) fn queued(&self, input: InputId) -> Result<QueuedInput, EngineError> {
        self.queue
            .input(input)
            .cloned()
            .ok_or_else(|| QueueError::NotFound(input).into())
    }
}

struct ReadVerdict {
    decision: RouteDecision,
    outcome: JudgmentOutcome,
    failed: bool,
}

impl ReadVerdict {
    /// 대체 규칙을 쓴 질문과 사유. 초안 사유 이름이다.
    fn fallback_reasons(&self) -> Vec<(String, String)> {
        let reason = match (self.failed, self.outcome) {
            (true, _) => "router-failed",
            (false, JudgmentOutcome::Invalid) => "invalid",
            _ => "fallback",
        };
        self.decision
            .fallbacks
            .iter()
            .map(|question| (question.clone(), reason.to_owned()))
            .collect()
    }
}

/// router 없이 정하는 판단. 처리 방식은 대기이고 모델은 고정 모델을 그대로 쓴다.
/// TODO(#168): 모델을 고정한 입력이 실행 중 도착했을 때의 처리 방식이 정해지면 대기 대신 따른다
pub(crate) fn direct_decision(record: &QueuedInput, revision: ChatRevision) -> RouteDecision {
    RouteDecision {
        revision,
        settings: record.settings,
        disposition: Disposition::Queue,
        is_conflict: false,
        keep_current: true,
        model: record.pinned_model.clone(),
        resume_held: false,
        fallbacks: Vec::new(),
    }
}

/// 판단에 쓴 질문별 기준값. 초안 목록이다.
fn threshold_list(settings: &Settings) -> Vec<(String, f64)> {
    let thresholds = settings.thresholds();
    vec![
        ("keep_current".to_owned(), thresholds.keep_current),
        ("is_actionable".to_owned(), thresholds.is_actionable),
        ("min_confidence".to_owned(), thresholds.min_confidence),
        ("resume_held".to_owned(), thresholds.resume_held),
    ]
}

fn disposition_name(disposition: Disposition) -> &'static str {
    match disposition {
        Disposition::Steer => "steer",
        Disposition::NewTask => "new-task",
        Disposition::Queue => "queue",
    }
}
