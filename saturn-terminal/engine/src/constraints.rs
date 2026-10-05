//! 제약 등록: router 답으로 등록 후보를 정하고, 자동 등록하거나 사용자에게 묻고, 입력이 취소되면 함께 해제한다.
//! 설계: docs/design/constraints.md

use saturn_core::constraints::{scope_of, split_sentences};
use saturn_core::queue::QueuedInput;
use saturn_core::routers::calibration::AskedAnswer;
use saturn_core::routers::constraint::{
    RegistrationAction, RegistrationVerdict, line_questions, read_lines, read_registration,
};
use saturn_core::routers::{JudgmentOutcome, RouterRequest, validate};
use saturn_protocol::ids::{ChatId, ConstraintAskId, InputId, JudgmentId, SettingsRevision};
use saturn_protocol::rpc::{ChatNotice, ConstraintAskAnswer, Notification};
use saturn_protocol::state::InputState;

use crate::flow::{JobKind, RouterDone, RouterJob};
use crate::routers::{RecordContext, RouterExchange, outcome_of, sanitize_state};
use crate::rpc::ClientId;
use crate::settings::Settings;
use crate::store::{
    Actor, AnswerOutcome, ConstraintState, EventReason, NewRegistration, NewRule, StoredAsk,
};
use crate::{Engine, EngineError, masked_chain};

/// router 답에서 읽은 등록 계획. 판단 기록을 쓴 뒤에 적용한다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstraintPlan {
    pub(crate) verdict: RegistrationVerdict,
    /// 권한 모드가 `full`이라 묻지 않고 지키는 쪽으로 등록한다.
    pub(crate) full: bool,
}

impl ConstraintPlan {
    /// 사용자에게 묻는 계획이다. 물은 판단은 확률 q를 1로 기록한다.
    pub(crate) fn asks_user(&self) -> bool {
        self.verdict.action == RegistrationAction::Ask && !self.full
    }
}

/// 문장 나누기 답을 기다리는 등록.
#[derive(Debug, Clone)]
pub(crate) struct PendingLines {
    chat: ChatId,
    plan: ConstraintPlan,
    settings: SettingsRevision,
    judgment: Option<JudgmentId>,
    sentences: Vec<String>,
}

impl Engine {
    // cost: time O(q + a), heap O(1), stack O(1)
    // vars: q = 요청 질문 수, a = 답 수
    // basis: estimate
    /// 유효한 router 답의 `is_constraint`를 읽어 등록 계획을 만든다. 답이 없거나 형식이 틀렸거나 기준값 미만이면 `None`이다.
    /// 자동 적용(`constraint.auto_apply`)이 꺼져 있으면 `full` 모드여도 계획을 만들지 않는다. 판단은 판단 기록에만 남고
    /// 제약 표와 사용자 질문과 문장 나누기 호출로 이어지지 않으며, 입력 원문은 평소대로 작업과 인계 맥락에 남는다.
    pub(crate) async fn constraint_plan(
        &self,
        request: &RouterRequest,
        exchange: &RouterExchange,
        record: &QueuedInput,
        settings: &Settings,
    ) -> Option<ConstraintPlan> {
        if !settings.constraint_auto_apply() {
            return None;
        }
        let Ok(response) = &exchange.result else {
            return None;
        };
        if validate(request, response).is_err() {
            return None;
        }
        let verdict = read_registration(
            request,
            response,
            &settings.thresholds(),
            self.routers.method(),
        );
        if verdict.action == RegistrationAction::Skip {
            return None;
        }
        let full = self.is_full_mode(record.chat, record.settings).await;
        Some(ConstraintPlan { verdict, full })
    }

    /// 판단 기록을 쓴 뒤 부른다. 입력이 이미 취소됐으면 등록하지 않는다. 긴 입력이면 문장 나누기 질문을 별도 작업으로
    /// 시작하고 답이 오면 등록한다. 오류는 입력을 막지 않고 로그만 남긴다.
    pub(crate) async fn apply_constraint_plan(
        &mut self,
        input: InputId,
        plan: ConstraintPlan,
        judgment: Option<JudgmentId>,
    ) {
        let Ok(record) = self.queued(input) else {
            return;
        };
        if record.state == InputState::Cancelled {
            return;
        }
        let Some(sentences) = split_sentences(&record.text) else {
            let rules = vec![(0, record.text.clone())];
            let applied = self.register_rules(&record, rules, plan, judgment).await;
            self.warn_failure("failed to register a constraint", applied);
            return;
        };
        self.start_lines(&record, sentences, plan, judgment);
    }

    /// 문장마다 제약인지 묻는 요청을 별도 작업으로 보낸다. 답은 `on_lines_done`으로 온다.
    fn start_lines(
        &mut self,
        record: &QueuedInput,
        sentences: Vec<String>,
        plan: ConstraintPlan,
        judgment: Option<JudgmentId>,
    ) {
        let numbered: Vec<String> = sentences
            .iter()
            .enumerate()
            .map(|(index, sentence)| format!("[{}] {sentence}", index + 1))
            .collect();
        let state = format!("user input sentences:\n{}", numbered.join("\n"));
        let request = RouterRequest {
            model: self.routers.active().model().to_owned(),
            state: sanitize_state(&state, &self.masker),
            sets: vec![line_questions(sentences.len())],
        };
        let job = RouterJob {
            chat: record.chat,
            input: record.id,
            revision: self.queue.revision(record.chat),
            retried: false,
            kind: JobKind::Lines,
        };
        self.flow.pending_lines.insert(
            record.id,
            PendingLines {
                chat: record.chat,
                plan,
                settings: record.settings,
                judgment,
                sentences,
            },
        );
        self.spawn_router_job(job, request);
    }

    /// 문장 나누기 답을 읽어 규칙을 정하고 등록한다. 질문이 실패했거나 제약인 문장이 없으면 입력 전체를 한 건으로 등록한다.
    /// 놓치는 것보다 길게 넣는 쪽이 안전하기 때문이다.
    pub(crate) async fn on_lines_done(&mut self, done: RouterDone) {
        let RouterDone {
            job,
            request,
            exchange,
        } = done;
        let Some(pending) = self.flow.pending_lines.remove(&job.input) else {
            return;
        };
        if let Some(alert) = self.routers.observe(&exchange) {
            self.notify_alert(pending.chat, alert).await;
        }
        let picked = self.picked_lines(&pending, &request, &exchange).await;
        self.record_lines_judgment(&pending, job.input, &request, &exchange)
            .await;
        let Ok(record) = self.queued(job.input) else {
            return;
        };
        if record.state == InputState::Cancelled {
            return;
        }
        let rules = match picked {
            Some(lines) if !lines.is_empty() => lines,
            _ => vec![(0, record.text.clone())],
        };
        let applied = self
            .register_rules(&record, rules, pending.plan, pending.judgment)
            .await;
        self.warn_failure("failed to register constraints", applied);
    }

    /// 답이 유효하면 제약인 문장의 `(문장 번호, 글)`. 질문이 실패했으면 `None`.
    async fn picked_lines(
        &self,
        pending: &PendingLines,
        request: &RouterRequest,
        exchange: &RouterExchange,
    ) -> Option<Vec<(u32, String)>> {
        let response = exchange.result.as_ref().ok()?;
        validate(request, response).ok()?;
        let settings = self.settings.at(&self.store, pending.settings).await.ok()?;
        let lines = read_lines(response, pending.sentences.len(), &settings.thresholds())?;
        Some(
            lines
                .into_iter()
                .filter_map(|line| {
                    let sentence = pending.sentences.get(line - 1)?;
                    Some((u32::try_from(line).ok()?, sentence.clone()))
                })
                .collect(),
        )
    }

    /// 문장 나누기 호출도 판단 기록에 남긴다.
    async fn record_lines_judgment(
        &mut self,
        pending: &PendingLines,
        input: InputId,
        request: &RouterRequest,
        exchange: &RouterExchange,
    ) {
        let is_valid = exchange
            .result
            .as_ref()
            .is_ok_and(|response| validate(request, response).is_ok());
        let outcome = if exchange.result.is_ok() && !is_valid {
            JudgmentOutcome::Invalid
        } else {
            outcome_of(&exchange.result)
        };
        let reason = if exchange.result.is_err() {
            "router-failed"
        } else {
            "invalid"
        };
        let fallbacks = if is_valid {
            Vec::new()
        } else {
            vec![(
                saturn_core::routers::constraint::line_question_id(1),
                reason.to_owned(),
            )]
        };
        let context = RecordContext {
            chat: pending.chat,
            input: Some(input),
            question_sets: request.sets.iter().map(|(id, _)| id.clone()).collect(),
            settings: pending.settings,
            fallbacks,
            outcome,
            thresholds: vec![(
                "constraint_ask".to_owned(),
                self.constraint_ask_threshold(pending).await,
            )],
            asked_with: None,
        };
        let recorded = self.routers.record(&self.store, context, exchange).await;
        self.warn_failure("failed to record a line judgment", recorded);
    }

    async fn constraint_ask_threshold(&self, pending: &PendingLines) -> f64 {
        match self.settings.at(&self.store, pending.settings).await {
            Ok(settings) => settings.thresholds().constraint_ask,
            Err(_) => saturn_core::routers::Thresholds::default().constraint_ask,
        }
    }

    // cost: time O(r), heap O(r), stack O(1), io r
    // vars: r = 규칙 수
    // basis: estimate
    /// 규칙을 저장하고 TUI에 알린다. 자동 등록과 `full`의 묻지 않은 등록은 `Active`와 줄이고, 그 밖의 묻는 구간은
    /// `Candidate`와 확인 창이다.
    async fn register_rules(
        &mut self,
        record: &QueuedInput,
        rules: Vec<(u32, String)>,
        plan: ConstraintPlan,
        judgment: Option<JudgmentId>,
    ) -> Result<(), EngineError> {
        let new_rules: Vec<NewRule> = rules
            .iter()
            .map(|(line, rule)| NewRule {
                line: *line,
                scope: scope_of(rule),
                rule: rule.clone(),
            })
            .collect();
        let rules: Vec<String> = rules.into_iter().map(|(_, rule)| rule).collect();
        let is_candidate = plan.asks_user();
        let unconfirmed = plan.verdict.action == RegistrationAction::Ask && plan.full;
        let registered = self
            .store
            .register_constraints(&NewRegistration {
                chat: record.chat,
                input: record.id,
                rules: &new_rules,
                state: if is_candidate {
                    ConstraintState::Candidate
                } else {
                    ConstraintState::Active
                },
                actor: Actor::Router,
                reason: unconfirmed.then_some(EventReason::Unconfirmed),
                judgment,
            })
            .await?;
        match registered.ask {
            Some(ask) => {
                let leaned_yes = plan.verdict.probability.is_none_or(|yes| yes >= 0.5);
                self.flow.ask_leaned_yes.insert(ask, leaned_yes);
                self.offer_ask(record.chat, ask, &rules).await;
            }
            None => {
                for rule in rules {
                    self.notify_chat(
                        record.chat,
                        ChatNotice::ConstraintAdded { rule, unconfirmed },
                    )
                    .await;
                }
            }
        }
        Ok(())
    }

    /// 확인 알림을 보관해 붙은 TUI에 보내고 나중에 붙는 TUI에도 보낸다.
    async fn offer_ask(&mut self, chat: ChatId, ask: ConstraintAskId, rules: &[String]) {
        let notification = Notification::ConstraintAsked {
            ask,
            chat,
            rule: rules.join("\n"),
        };
        self.rpc.offer_constraint_ask(chat, ask, notification).await;
    }

    /// 사용자의 답을 한 거래로 적용한다. 등록하면 `제약 등록됨` 줄이 남고 거절하면 줄이 남지 않는다.
    /// 사용자의 답은 그 판단 기록의 물은 답으로 남기고, router가 더 높게 본 쪽과 다르면 `Wrong`이다.
    ///
    /// # Errors
    /// 없는 확인이면 `Store(NotFound)`, 이미 답했거나 대상이 바뀌어 닫힌 확인이면 `UnexpectedAnswer`.
    pub(crate) async fn answer_constraint_ask(
        &mut self,
        client: ClientId,
        ask: ConstraintAskId,
        answer: ConstraintAskAnswer,
    ) -> Result<(), EngineError> {
        let register = answer == ConstraintAskAnswer::Yes;
        let outcome = self.store.answer_constraint_ask(ask, register).await?;
        let AnswerOutcome::Applied {
            chat,
            judgment,
            registered,
            rules,
        } = outcome
        else {
            return Err(EngineError::UnexpectedAnswer {
                what: "constraint ask",
            });
        };
        self.rpc.resolve_constraint_ask(client, ask).await;
        let leaned_yes = self.flow.ask_leaned_yes.remove(&ask).unwrap_or(true);
        if let Some(judgment) = judgment {
            let agreed = if leaned_yes { register } else { !register };
            let asked = if agreed {
                AskedAnswer::Correct
            } else {
                AskedAnswer::Wrong
            };
            let recorded = self.store.record_asked_answer(judgment, asked).await;
            self.warn_failure("failed to record the constraint answer", recorded);
        }
        if registered {
            for rule in rules {
                self.notify_chat(
                    chat,
                    ChatNotice::ConstraintAdded {
                        rule,
                        unconfirmed: false,
                    },
                )
                .await;
            }
        }
        Ok(())
    }

    /// 시작할 때 답을 기다리는 확인을 TUI에 되살린다. 질문은 기록 저장소에 있어 engine을 다시 켜도 남는다.
    ///
    /// # Errors
    /// 확인을 읽지 못하면 `Store`.
    pub(crate) async fn restore_constraint_asks(&mut self) -> Result<(), EngineError> {
        let asks: Vec<StoredAsk> = self.store.open_constraint_asks().await?;
        for ask in asks {
            let notification = Notification::ConstraintAsked {
                ask: ask.id,
                chat: ask.chat,
                rule: ask.rules.join("\n"),
            };
            self.rpc
                .offer_constraint_ask(ask.chat, ask.id, notification)
                .await;
        }
        Ok(())
    }

    /// 입력이 취소되면 그 입력의 제약을 함께 해제하고 해제 줄을 남긴다. 사용자가 거둔 말이 제약으로 남지 않게 하기 위해서다.
    /// 오류는 로그만 남긴다.
    pub(crate) async fn release_canceled_input(&mut self, input: InputId) {
        let released = match self.store.release_input_constraints(input).await {
            Ok(released) => released,
            Err(error) => {
                tracing::warn!(error = %masked_chain(&self.masker, &error), "failed to release the constraints of a canceled input");
                return;
            }
        };
        let Some(released) = released else { return };
        for ask in released.closed_asks {
            self.flow.ask_leaned_yes.remove(&ask);
            self.rpc.withdraw_constraint_ask(ask).await;
        }
        for rule in released.rules {
            self.notify_chat(released.chat, ChatNotice::ConstraintReleased { rule })
                .await;
        }
    }
}
