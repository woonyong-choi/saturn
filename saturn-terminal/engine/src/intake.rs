//! 입력 접수, 판단 차례, 판단 적용.
//! 설계: docs/design/input-handling.md, docs/design/router.md

use std::path::Path;
use std::time::Instant;

use saturn_core::queue::{QueueError, QueuedInput};
use saturn_core::routers::shadow::split_shadow;
use saturn_core::routers::{
    ConstraintQuestion, JudgmentOutcome, RouteDecision, RouterError, RouterRequest, RouterResponse,
    decide_route, question_ids, questions_for_input, validate,
};
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision, TaskId};
use saturn_protocol::rpc::{ModelMode, Notification};
use saturn_protocol::state::{Disposition, InputState};

use crate::Attachment;
use crate::constraints::ConstraintPlan;
use crate::flow::{JobKind, Routed, RouterDone, RouterJob, Unrecorded};
use crate::models::ModelPlan;
use crate::requests::{settings_notification, trust_notification};
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::rpc::ClientId;
use crate::settings::{Settings, SettingsError};
use crate::{Engine, EngineError, masked_chain};

/// 사용자에게 묻는 제약 등록의 확률 q. 구간에서는 항상 묻는다.
const ASKED_CONSTRAINT_Q: f64 = 1.0;

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
        self.submit(client, chat, client_ref, text, skip_relation, None)
            .await
    }

    /// router를 부르지 않고 `task`에 바로 끼워 넣는다. 관계 판단 없이 그 작업으로 가므로 router가 다른 작업으로
    /// 보낼 위험이 없다. 그 작업이 이미 끝났거나 없으면 `submit_input`처럼 판단을 받는다.
    ///
    /// # Errors
    /// `submit_input`과 같다.
    pub(crate) async fn submit_to_task(
        &mut self,
        client: ClientId,
        chat: ChatId,
        client_ref: u64,
        task: TaskId,
        text: String,
    ) -> Result<(), EngineError> {
        let is_open = self
            .queue
            .main_tasks()
            .iter()
            .any(|info| info.chat == chat && info.task == task);
        if is_open {
            self.submit(client, chat, client_ref, text, true, Some(task))
                .await
        } else {
            self.submit(client, chat, client_ref, text, false, None)
                .await
        }
    }

    async fn submit(
        &mut self,
        client: ClientId,
        chat: ChatId,
        client_ref: u64,
        text: String,
        skip_relation: bool,
        task: Option<TaskId>,
    ) -> Result<(), EngineError> {
        let input = self
            .accept_input(client, chat, text, skip_relation, task)
            .await?;
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
        match done.job.kind {
            JobKind::Route => {}
            JobKind::Lines => {
                self.on_lines_done(done).await;
                return;
            }
            JobKind::Change => {
                self.on_change_done(done).await;
                return;
            }
        }
        let RouterDone {
            job,
            request,
            exchange,
        } = done;
        self.flow.judging.remove(&job.chat);
        match self.apply_routed(job, &request, exchange).await {
            Ok(()) => {
                self.follow_resume_signals().await;
                self.advance(job.chat).await;
            }
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
        task: Option<TaskId>,
    ) -> Result<InputId, EngineError> {
        let workdir = self.attached_workdir(client, chat)?;
        self.open_request(chat);
        let pinned_model = self.store.chat_model(chat).await?;
        let settings = self.fix_settings(client, chat, &workdir).await?;
        if let Ok(mode) = self.chat_mode(chat, settings).await {
            self.passes.set_mode(chat, mode);
        }
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
        let write_scope = self.write_scope_of(chat, &new.workdir);
        self.note_judge_input(chat, id, &new.text);
        self.queue.accept(QueuedInput {
            id,
            chat,
            text: new.text,
            settings,
            permission: new.permission,
            workdir: new.workdir,
            write_scope,
            pinned_model: new.pinned_model,
            skip_relation,
            state: InputState::Judging,
            reason: None,
            task,
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
        let run = self.run_layer_of(client);
        if !self.settings.changed(Some(chat), workdir, &run).await? {
            return Ok(self
                .settings
                .revision_in(chat, &run)
                .ok_or(SettingsError::NoPreviousRevision)?);
        }
        self.apply_changed_settings(&[client], chat, workdir).await
    }

    /// 이 접속의 실행 `-c`(`키=값`). 붙지 않은 접속이면 없다.
    pub(crate) fn run_layer_of(&self, client: ClientId) -> Vec<String> {
        self.attachments
            .get(&client)
            .map(Attachment::run_layer)
            .unwrap_or_default()
    }

    /// 이 채팅에 붙은 첫 접속의 실행 `-c`. 접속을 특정할 수 없는 일(종료 때 설정 읽기)이 쓴다.
    pub(crate) fn run_layer_of_chat(&self, chat: ChatId) -> Vec<String> {
        self.attachments
            .values()
            .find(|attachment| attachment.chat == chat)
            .map(Attachment::run_layer)
            .unwrap_or_default()
    }

    /// 바뀐 설정 파일을 병합해 경고를 채팅에 알리고, 새로 보이는 폴더 설정의 신뢰 창을 `clients`에 연다.
    /// `clients`는 같은 접속 `-c`를 쓰는 접속들이다. 입력 접수와 설정 파일 감시가 함께 쓴다.
    pub(crate) async fn apply_changed_settings(
        &mut self,
        clients: &[ClientId],
        chat: ChatId,
        workdir: &Path,
    ) -> Result<SettingsRevision, EngineError> {
        let run = clients
            .first()
            .map(|client| self.run_layer_of(*client))
            .unwrap_or_default();
        let (applied, prompt) = self
            .settings
            .apply_trusted(&self.store, Some(chat), workdir, &run)
            .await?;
        let revision = applied.revision;
        if applied.warning.is_some() {
            self.rpc
                .broadcast(Some(chat), settings_notification(applied))
                .await;
        }
        if let Some(prompt) = prompt {
            for client in clients {
                self.send(*client, trust_notification(&prompt)).await;
                self.set_folder_trust(*client, Some(prompt.clone()));
            }
        }
        self.announce_model_settings(chat, clients, revision)
            .await?;
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
                self.start_router(&record, revision, false).await?;
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
                    self.pin_default_model_without_current(&record, disposition)
                        .await;
                    if !record.skip_relation {
                        // 판단이 없으면(장애, 응답 없음, 이 질문의 답 없음) 재개도 무시도 아니다
                        let judged = !decision
                            .fallbacks
                            .iter()
                            .any(|id| id == question_ids::RESUME_HELD);
                        self.flow
                            .resume_signals
                            .push((record.chat, judged.then_some(decision.resume_held)));
                    }
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
                        self.start_router(&record, current, retried).await?;
                        return Ok(());
                    }
                    decision = direct_decision(&record, current);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// 이어 가기와 끼워 넣기는 현재 모델을 유지하지만 채팅에 이어 갈 메인 session이 없으면 유지할 모델이 없다. router 판단이 실패한
    /// 첫 입력이 그렇다. 그때는 기본 모델로 연다(router의 `target_model` 선택은 이어 가기에 쓰지 않는다). 기본 모델이 없거나
    /// 이미 고정한 모델이 있으면 아무것도 하지 않는다.
    async fn pin_default_model_without_current(
        &mut self,
        record: &QueuedInput,
        disposition: Disposition,
    ) {
        if disposition == Disposition::NewTask || self.sessions.live_main(record.chat).is_some() {
            return;
        }
        let default = match self.model_plan(record.settings).await {
            Ok(plan) => plan.default,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "model plan not read, keeping the provider default");
                return;
            }
        };
        let Some(model) = default else {
            return;
        };
        let pinned = self.queue.pin_model_if_unset(record.id, &model);
        self.warn_failure("failed to pin the default model", pinned);
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

    /// 적용한 판단의 `resume_held`를 접수 순서대로 보류 작업에 반영한다. 오류는 로그만 남긴다.
    /// `apply_decision`은 확인 입력을 보내는 재개 경로에서도 불리므로 재개는 거기서 하지 않고 여기서 한다.
    async fn follow_resume_signals(&mut self) {
        for (chat, resume) in std::mem::take(&mut self.flow.resume_signals) {
            let Some(resume) = resume else { continue };
            let followed = self.follow_resume_signal(chat, resume).await;
            self.warn_failure("failed to follow resume signal", followed);
        }
    }

    /// 새 입력의 `resume_held` 판단을 보류 작업에 반영한다. 재개 뜻이면 채팅의 보류를 모두 재개하고, 아니면 무시 횟수를
    /// 올려 재개 뜻이 없는 입력이 쌓인 때 보류를 닫는다. 보류 작업이 없으면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// `continue_held`, `close_held`와 같다.
    async fn follow_resume_signal(
        &mut self,
        chat: ChatId,
        resume: bool,
    ) -> Result<(), EngineError> {
        if !self.queue.has_held_task(chat) {
            return Ok(());
        }
        match self.queue.note_resume_signal(chat, resume) {
            Some(tasks) => {
                for task in tasks {
                    self.close_held(chat, task).await?;
                }
                Ok(())
            }
            None if resume => self.continue_held(chat, None).await,
            None => Ok(()),
        }
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
    ///
    /// # Errors
    /// 입력의 설정 번호를 읽지 못하면 `Settings`.
    pub(crate) async fn start_router(
        &mut self,
        record: &QueuedInput,
        revision: ChatRevision,
        retried: bool,
    ) -> Result<(), EngineError> {
        // 최초 목표나 최신 수정을 온전히 담지 못한 맥락으로는 판단하지 않고, 사용자가 확인하도록 대기에 둔다
        if self.judge_context(record).is_incomplete {
            return self.wait_in_queue(record.id).await;
        }
        let running = self.chat_is_running(record.chat);
        let plan = self.model_plan(record.settings).await?;
        // 입력 처리를 다시 판단하는 요청에는 제약 질문을 넣지 않아 같은 입력을 두 번 등록하지 않는다
        let request = self.router_request(record, running, &plan, !retried);
        let job = RouterJob {
            chat: record.chat,
            input: record.id,
            revision,
            retried,
            kind: JobKind::Route,
        };
        self.flow.judging.insert(record.chat, record.id);
        self.spawn_router_job(job, request);
        Ok(())
    }

    /// router 호출을 별도 작업으로 보내고 기다리지 않는다. 결과는 `on_routed`로 온다.
    pub(crate) fn spawn_router_job(&mut self, job: RouterJob, request: RouterRequest) {
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
        let full_sets: Vec<_> = request.sets.iter().map(|(id, _)| id.clone()).collect();
        let active = self.routers.active();
        let policy = crate::policy::policy_digest(
            &settings,
            active.router_id(),
            active.model(),
            &self.catalog.version,
        );
        tracing::debug!(%policy, "judgment policy");
        if let Some(alert) = self.routers.observe(&exchange) {
            self.notify_alert(record.chat, alert).await;
        }
        // 그림자 질문은 실제 판단이 읽는 요청과 답에서 떼어 낸다
        let split = split_shadow(request, exchange.result.as_ref().ok());
        let mut shadow = self.shadow_plan(request, &split, (job.revision, policy));
        let request = &split.request;
        let mut read = self.read_verdict(
            (request, split.response.as_ref()),
            &exchange,
            &settings,
            (job.revision, record.settings),
        );
        // 고정 모델은 `target_model` 선택만 대신하고 관계 판단은 그대로 받는다.
        // 고정하지 않았으면 오토 모드에서 후보 글인 router 선택을, 아니면 기본 모델을 쓴다. 둘 다 없으면 현재 모델이다
        let plan = ModelPlan::from_settings(&settings, &self.registry);
        read.decision.model = if record.pinned_model.is_some() {
            record.pinned_model.clone()
        } else {
            let routed = read
                .decision
                .model
                .take()
                .filter(|model| self.registry.parse_pinned(model).is_some())
                .filter(|_| plan.mode == ModelMode::Auto);
            routed.or(plan.default)
        };
        if let Some(shadow) = &mut shadow {
            shadow.decided = read.decision.model.clone();
        }
        let fallbacks = read.fallback_reasons();
        let constraint = self
            .constraint_plan(request, split.response.as_ref(), &record, &settings)
            .await;
        let context = RecordContext {
            chat: record.chat,
            input: Some(record.id),
            question_sets: full_sets,
            settings: record.settings,
            fallbacks,
            outcome: read.outcome,
            thresholds: threshold_list(&settings),
            asked_with: constraint
                .filter(ConstraintPlan::asks_user)
                .map(|_| ASKED_CONSTRAINT_Q),
        };
        // 등록 대상이 아닌 입력만 해제·예외 판단을 받는다. router가 답하지 못했으면 제약 판단을 모두 건너뛴다
        let change = constraint.is_none()
            && settings.constraint_auto_apply()
            && exchange.result.is_ok()
            && !job.retried;
        self.flow.unrecorded.insert(
            record.id,
            Unrecorded {
                context,
                exchange,
                constraint,
                change,
                shadow,
            },
        );
        Ok(Verdict {
            decision: read.decision,
            failed: read.failed,
        })
    }

    pub(crate) fn router_request(
        &self,
        record: &QueuedInput,
        running: bool,
        plan: &ModelPlan,
        with_constraint: bool,
    ) -> RouterRequest {
        // 매뉴얼 모드는 후보를 주지 않아 `target_model`을 묻지 않는다
        let candidates = match plan.mode {
            ModelMode::Auto => self.model_candidates(record.chat),
            ModelMode::Manual => Vec::new(),
        };
        let activity = if running { "running" } else { "idle" };
        let previous = self
            .flow
            .last_disposition
            .get(&record.chat)
            .map_or("none", |disposition| disposition_name(*disposition));
        let context = self.judge_context(record).text;
        let state = format!(
            "chat: {activity}\nprevious input handled as: {previous}\n{context}\nuser input: {}",
            record.text
        );
        let mut request = RouterRequest {
            model: self.routers.active().model().to_owned(),
            state: sanitize_state(&state, &self.masker),
            sets: questions_for_input(
                running,
                record.pinned_model.is_some(),
                self.queue.has_held_task(record.chat),
                &candidates,
                if with_constraint {
                    ConstraintQuestion::With
                } else {
                    ConstraintQuestion::Without
                },
            ),
        };
        self.add_shadow(record, plan.shadow, &mut request);
        request
    }

    /// `real`은 그림자 질문을 뗀 요청과 그 답이다.
    fn read_verdict(
        &self,
        (request, real): (&RouterRequest, Option<&RouterResponse>),
        exchange: &RouterExchange,
        settings: &Settings,
        (revision, settings_revision): (ChatRevision, SettingsRevision),
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
            Ok(_) if real.is_some_and(|response| validate(request, response).is_ok()) => {
                ReadVerdict {
                    decision: route(real.unwrap_or(&empty_response(request))),
                    outcome: JudgmentOutcome::Ok,
                    failed: false,
                }
            }
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
        let (chat, settings) = (unrecorded.context.chat, unrecorded.context.settings);
        let recorded = self
            .routers
            .record(&self.store, unrecorded.context, &unrecorded.exchange)
            .await;
        let judgment = match recorded {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to record judgment");
                None
            }
        };
        if let (Some(judgment), Some(shadow)) = (judgment, unrecorded.shadow) {
            self.record_shadow((judgment, input, chat, settings), shadow, superseded)
                .await;
        }
        self.follow_constraint_judgment(
            input,
            (unrecorded.constraint, unrecorded.change),
            judgment,
        )
        .await;
        judgment
    }

    /// 판단 기록을 쓴 뒤 그 입력의 제약 등록을 적용하고, 등록 대상이 아니면 해제·예외 판단을 시작한다.
    async fn follow_constraint_judgment(
        &mut self,
        input: InputId,
        (plan, change): (Option<ConstraintPlan>, bool),
        judgment: Option<JudgmentId>,
    ) {
        if let Some(plan) = plan {
            self.apply_constraint_plan(input, plan, judgment).await;
        }
        if change {
            self.start_change(input, false).await;
        }
    }

    /// 실행 중인 에이전트가 있는 채팅이면 참.
    /// provider 응답을 기다리는 전달도 시작하는 작업이라 실행 중으로 본다.
    pub(crate) fn chat_is_running(&self, chat: ChatId) -> bool {
        self.flow.deliveries.contains_key(&chat)
            || self
                .runs
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

fn empty_response(request: &RouterRequest) -> RouterResponse {
    RouterResponse {
        model: request.model.clone(),
        answers: Vec::new(),
        tokens: (0, 0),
    }
}

/// router 없이 정하는 판단. 처리 방식은 대기이고 모델은 고정 모델을 그대로 쓴다.
/// TODO(#168): 모델을 고정한 입력이 실행 중 도착했을 때의 처리 방식이 정해지면 대기 대신 따른다
pub(crate) fn direct_decision(record: &QueuedInput, revision: ChatRevision) -> RouteDecision {
    // 대상 작업이 정해진 입력은 대기하지 않고 그 작업에 끼워 넣는다
    let disposition = if record.task.is_some() {
        Disposition::Steer
    } else {
        Disposition::Queue
    };
    RouteDecision {
        revision,
        settings: record.settings,
        disposition,
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
        ("is_constraint".to_owned(), thresholds.is_constraint),
        ("constraint_ask".to_owned(), thresholds.constraint_ask),
        (
            "constraint_release".to_owned(),
            thresholds.constraint_release,
        ),
    ]
}

fn disposition_name(disposition: Disposition) -> &'static str {
    match disposition {
        Disposition::Steer => "steer",
        Disposition::NewTask => "new-task",
        Disposition::Queue => "queue",
    }
}
