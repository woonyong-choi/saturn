//! 입력 접수, 판단 차례, 판단 적용.
//! 설계: docs/design/input-handling.md, docs/design/judge.md

use std::path::Path;
use std::time::Instant;

use saturn_core::judges::{
    JudgeError, JudgeRequest, JudgeResponse, JudgmentOutcome, RouteDecision, decide_route,
    questions_for_input, validate,
};
use saturn_core::queue::{Permission, QueueError, QueuedInput};
use saturn_protocol::ids::{ChatId, ChatRevision, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::{Disposition, InputState};

use crate::flow::{Judged, Unrecorded};
use crate::judges::{JudgeExchange, RecordContext, outcome_of, sanitize_state};
use crate::requests::{settings_notification, trust_notification};
use crate::rpc::ClientId;
use crate::settings::{Settings, SettingsError};
use crate::{Engine, EngineError, masked_chain};

/// 모든 입력을 쓰기 권한으로 접수한다. 권한 모드를 읽어 읽기 전용 입력을 가르는 일은
/// TODO(#232): 권한 규칙 구현. 그 전에는 쓰기 대기가 길어져도 병렬 쓰기가 생기지 않는 쪽으로 둔다.
const INPUT_PERMISSION: Permission = Permission::Write;

/// 판단 한 번의 결과.
pub(crate) struct Verdict {
    pub(crate) decision: RouteDecision,
    /// judge 호출이 재시도 뒤에도 실패해 대체 판단을 쓴 경우.
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
        pinned_model: Option<String>,
        skip_relation: bool,
    ) -> Result<(), EngineError> {
        let input = self
            .accept_input(client, chat, text, pinned_model, skip_relation)
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

    /// 판단하고 보낸다. 오류는 로그만 남긴다.
    pub(crate) async fn advance(&mut self, chat: ChatId) {
        let result = match self.judge_next(chat).await {
            Ok(()) => self.dispatch_next(chat).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            tracing::warn!(chat = chat.0, error = %masked_chain(&self.masker, &error), "input flow stopped");
        }
    }

    async fn accept_input(
        &mut self,
        client: ClientId,
        chat: ChatId,
        text: String,
        pinned_model: Option<String>,
        skip_relation: bool,
    ) -> Result<InputId, EngineError> {
        let workdir = self.attached_workdir(client, chat)?;
        let settings = self.fix_settings(client, chat, &workdir).await?;
        let new = crate::store::NewInput {
            chat,
            text,
            settings,
            permission: INPUT_PERMISSION,
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

    /// 같은 채팅 입력을 접수 순서대로 하나씩 판단하고 적용한다. 모델을 고정했거나 관계 판단 없이 대기하는
    /// 입력(`skip_relation`)은 judge를 부르지 않는다.
    ///
    /// # Errors
    /// 판단 요청에 쓸 설정 번호를 읽지 못하면 `Settings`, 적용 오류는 `apply_decision`과 같다.
    pub(crate) async fn judge_next(&mut self, chat: ChatId) -> Result<(), EngineError> {
        while let Some((input, revision)) = self.queue.next_to_judge(chat) {
            let record = self.queued(input)?;
            let decision = if record.pinned_model.is_some() || record.skip_relation {
                direct_decision(&record, revision)
            } else {
                let running = self.chat_is_running(chat);
                self.ask_judge(&record, running, revision).await?.decision
            };
            self.apply_decision(input, decision, false).await?;
        }
        Ok(())
    }

    /// `RevisionConflict`면 `retried`가 거짓일 때만 한 번 다시 판단하고, 또 어긋나면 대기로 둔다.
    /// 어긋난 판단은 `Superseded`로 기록한다.
    ///
    /// # Errors
    /// 없는 입력이거나 `Judging`이 아니면 `Queue`, 기록 저장소 쓰기 실패면 `Store`.
    pub(crate) async fn apply_decision(
        &mut self,
        input: InputId,
        decision: RouteDecision,
        retried: bool,
    ) -> Result<(), EngineError> {
        let chat = self.queued(input)?.chat;
        let mut decision = decision;
        let mut retried = retried;
        loop {
            let current = self.queue.revision(chat);
            match self.queue.apply(input, &decision, current) {
                Ok(disposition) => return self.after_applied(input, disposition).await,
                Err(QueueError::RevisionConflict) => {
                    self.settle_record(input, true).await;
                    if retried {
                        return self.wait_in_queue(input).await;
                    }
                    decision = self.rejudge(input).await?;
                    retried = true;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// 어긋난 판단을 버리고 지금 revision으로 한 번 더 묻는다.
    async fn rejudge(&mut self, input: InputId) -> Result<RouteDecision, EngineError> {
        let record = self.queued(input)?;
        let revision = self.queue.revision(record.chat);
        if record.pinned_model.is_some() || record.skip_relation {
            return Ok(direct_decision(&record, revision));
        }
        let running = self.chat_is_running(record.chat);
        Ok(self.ask_judge(&record, running, revision).await?.decision)
    }

    async fn after_applied(
        &mut self,
        input: InputId,
        disposition: Disposition,
    ) -> Result<(), EngineError> {
        let chat = self.queued(input)?.chat;
        let judgment = self.settle_record(input, false).await;
        if let Some(judgment) = judgment {
            self.watch_judgment(chat, judgment, Instant::now());
        }
        self.flow.judged.insert(
            input,
            Judged {
                judgment,
                disposition,
            },
        );
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

    /// 판단 요청 하나를 보내 결과를 읽는다. 기록은 적용 결과를 안 뒤 `settle_record`가 쓴다.
    pub(crate) async fn ask_judge(
        &mut self,
        record: &QueuedInput,
        running: bool,
        revision: ChatRevision,
    ) -> Result<Verdict, EngineError> {
        let settings = self.settings.at(&self.store, record.settings).await?;
        let request = self.judge_request(record, running);
        let (exchange, alert) = self.judges.call(request.clone()).await;
        if let Some(alert) = alert {
            self.notify_alert(record.chat, alert).await;
        }
        let read = self.read_verdict(&request, &exchange, &settings, revision, record.settings);
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

    fn judge_request(&self, record: &QueuedInput, running: bool) -> JudgeRequest {
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
        // TODO(#168): 허용 모델 후보가 정해지면 `target_model`을 묻는다
        // TODO(#149): 보류 작업이 있으면 `resume_held`를 묻고 `note_resume_signal`로 잇는다
        JudgeRequest {
            model: self.judges.active().model().to_owned(),
            state: sanitize_state(&state, &self.masker),
            sets: questions_for_input(running, false, false, &[]),
        }
    }

    fn read_verdict(
        &self,
        request: &JudgeRequest,
        exchange: &JudgeExchange,
        settings: &Settings,
        revision: ChatRevision,
        settings_revision: SettingsRevision,
    ) -> ReadVerdict {
        let route = |response: &JudgeResponse| {
            decide_route(
                (request, response),
                &settings.thresholds(),
                self.judges.method(),
                revision,
                settings_revision,
            )
        };
        let invalid = || {
            let empty = JudgeResponse {
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
            Ok(_) | Err(JudgeError::Invalid { .. }) => invalid(),
            Err(_) => ReadVerdict {
                decision: self
                    .judges
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
            .judges
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
            (true, _) => "judge-failed",
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

/// judge 없이 정하는 판단. 처리 방식은 대기이고 모델은 고정 모델을 그대로 쓴다.
/// TODO(#168): 모델을 고정한 입력이 실행 중 도착했을 때의 처리 방식이 정해지면 대기 대신 따른다
fn direct_decision(record: &QueuedInput, revision: ChatRevision) -> RouteDecision {
    RouteDecision {
        revision,
        settings: record.settings,
        disposition: Disposition::Queue,
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
