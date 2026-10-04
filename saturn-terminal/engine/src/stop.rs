//! 멈춤과 보류: 트리 전체에 멈춤 신호를 보내고, 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 완료를 알린다.
//! 보류 재개와 보류 종료도 여기서 한다.
//! 설계: docs/design/input-handling.md#멈춤과-보류, docs/design/providers-and-sessions.md#트리-전체-중지

use std::collections::HashMap;

use saturn_core::agents::TreeStatus;
use saturn_core::providers::InterruptTarget;
use saturn_core::queue::{QueueError, QueuedInput};
use saturn_core::sessions::changes::{ChangeSet, describe};
use saturn_core::sessions::memo::INTERRUPTED_RESULT;
use saturn_protocol::ids::{AgentId, ChatId, InputId, RunId, TaskId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState, TaskState};

use crate::flow::LiveSession;
use crate::intake::direct_decision;
use crate::processes::{ProcessError, ProcessGroupId, StopOutcome, StopScope};
use crate::store::{NewInput, RunEnd, StoredHold};
use crate::{Engine, EngineError};

/// 멈추는 중인 채팅이 기다리는 것.
#[derive(Debug)]
pub(crate) struct StopProgress {
    /// 멈추는 에이전트. 값은 멈춘 뒤의 완료 신호(`TurnCompleted`나 `StreamLost`)를 받았는지다.
    agents: HashMap<AgentId, bool>,
    /// 결과를 기다리는 프로세스 묶음 수.
    pending_groups: usize,
    /// 묶음 밖에 남은 프로세스 수.
    unconfirmed: u32,
    held: Vec<TaskId>,
}

/// 프로세스 묶음 하나의 중지 결과.
pub(crate) struct StopDone {
    pub(crate) chat: ChatId,
    pub(crate) result: Result<StopOutcome, ProcessError>,
}

/// 보류한 작업이 멈출 때 하던 일. 재개할 때 확인 입력을 만들고, 보류를 닫을 때 session을 끝내는 데 쓴다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeldTask {
    pub(crate) agent: Option<AgentId>,
    /// 멈출 때 진행 중이던 실행을 연 입력. 실행 중이 아니었으면 `None`.
    pub(crate) input: Option<InputId>,
}

impl Engine {
    /// 모든 작업과 보내지 않은 입력을 보류하고 멈춤 신호를 보낸다. 완료는 트리 유휴와 프로세스 중지를 모두 확인한 뒤
    /// `ChatNotice::Stopped`로 알리고, 확인하지 못하면 `StopUnconfirmed`로 알린다. 이미 멈추는 중이면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 멈춤 기록을 쓰지 못하면 `Store`. 이때는 아무것도 멈추지 않는다.
    pub(crate) async fn stop_chat(&mut self, chat: ChatId) -> Result<(), EngineError> {
        self.end_children_of(chat).await;
        self.stop_chat_core(chat).await
    }

    /// 하위 접속을 거느린 채널이면 `stop_chat`이 먼저 하위 접속을 끝낸다. 이 함수는 하위 접속을 건드리지 않아
    /// `end_children_of`가 하위 채팅을 멈출 때 되돌아오지 않는다.
    pub(crate) async fn stop_chat_core(&mut self, chat: ChatId) -> Result<(), EngineError> {
        if self.flow.stopping.contains_key(&chat) {
            return Ok(());
        }
        self.store.begin_stop(chat).await?;
        if let Some(parked) = self.flow.deliveries.get_mut(&chat) {
            // provider 응답을 기다리는 전달은 응답이 와도 더 보내지 않고 보류한다
            parked.job.is_stopped = true;
        }
        let held = self.queue.stop(chat);
        let running = self.running_agents(chat);
        let mut groups: Vec<ProcessGroupId> = Vec::new();
        self.remember_held(chat, &held, &running).await;
        for agent in &running {
            let Some(live) = self.flow.live.get(agent).cloned() else {
                continue;
            };
            self.interrupt_tree(chat, &live);
            self.hold_session(&live).await;
            if let Some(group) = self.group_of(chat, &live)
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
        for input in self.queue.inputs_in_state(chat, InputState::Held) {
            let written = self
                .store
                .set_input_state(input, InputState::Held, None)
                .await;
            self.warn_failure("failed to record held input", written);
            self.notify_input(input).await;
        }
        let agents = running.iter().map(|agent| (*agent, false)).collect();
        self.flow.stopping.insert(
            chat,
            StopProgress {
                agents,
                pending_groups: groups.len(),
                unconfirmed: 0,
                held,
            },
        );
        for group in groups {
            self.spawn_stop(chat, group);
        }
        self.check_stop_done(chat).await
    }

    /// 채팅에서 계속 처리될 작업 수: 실행 중인 작업과 보내기 전에 판단하거나 기다리는 입력.
    pub(crate) fn continuing_work(&self, chat: ChatId) -> usize {
        let unsent = [InputState::Judging, InputState::Queued]
            .into_iter()
            .map(|state| self.queue.inputs_in_state(chat, state).len())
            .sum::<usize>();
        self.running_agents(chat).len()
            + unsent
            + usize::from(self.flow.deliveries.contains_key(&chat))
    }

    /// 진행 중인 실행이 있는 에이전트.
    pub(crate) fn running_agents(&self, chat: ChatId) -> Vec<AgentId> {
        let mut agents: Vec<AgentId> = self
            .runs
            .chat_of
            .iter()
            .filter(|(agent, owner)| **owner == chat && self.runs.active.contains_key(*agent))
            .map(|(agent, _)| *agent)
            .collect();
        agents.sort();
        agents
    }

    /// 보류한 작업마다 멈출 때 하던 일을 기억한다. 작업 이름표가 없는 작업에는 새로 붙인다.
    async fn remember_held(&mut self, chat: ChatId, held: &[TaskId], running: &[AgentId]) {
        for agent in running {
            let Some(task) = self.runs.task_of.get(agent).copied() else {
                continue;
            };
            let input = self.run_input(*agent).await;
            self.hold_task(
                chat,
                task,
                HeldTask {
                    agent: Some(*agent),
                    input,
                },
            )
            .await;
        }
        for task in held {
            self.flow.tasks.assign(*task);
            self.flow.held.entry(*task).or_insert(HeldTask {
                agent: None,
                input: None,
            });
        }
    }

    /// 보류를 메모리에 두고, 멈출 때 실행 중이던 작업(입력과 에이전트가 있는 보류)은 기록 저장소에도 남긴다. engine이 다시 떠도
    /// 재개를 제안하기 위해서다. 기록 실패는 경고만 남기고 멈춤을 막지 않는다.
    pub(crate) async fn hold_task(&mut self, chat: ChatId, task: TaskId, held: HeldTask) {
        self.flow.held.insert(task, held);
        if let (Some(agent), Some(input)) = (held.agent, held.input) {
            let saved = self
                .store
                .save_held_task(&StoredHold {
                    task,
                    chat,
                    agent,
                    input,
                })
                .await;
            self.warn_failure("failed to record held task", saved);
        }
    }

    /// 보류를 메모리와 기록 저장소에서 함께 지운다. 재개하거나 닫아 끝난 작업의 정보를 남기지 않기 위해서다.
    async fn release_held(&mut self, task: TaskId) -> Option<HeldTask> {
        let held = self.flow.held.remove(&task)?;
        if held.input.is_some() {
            let deleted = self.store.delete_held_task(task).await;
            self.warn_failure("failed to delete recorded held task", deleted);
        }
        Some(held)
    }

    /// 깊은 subagent부터 메인 순서로 멈춤 신호를 맡긴다. 연결 작업이 앞선 요청 뒤에 보내고, 연결이 끊겼으면 프로세스 중지로
    /// 넘어간다. 기다리지 않으므로 느린 provider 요청이 멈춤 요청 처리를 늦추지 않는다.
    fn interrupt_tree(&mut self, chat: ChatId, live: &LiveSession) {
        let targets: Vec<InterruptTarget> = self
            .agents
            .interrupt_order(live.agent)
            .into_iter()
            .map(|target| target.map_or(InterruptTarget::Main, InterruptTarget::Subagent))
            .collect();
        if let Some(connection) = self.providers.get(&(chat, live.provider)) {
            connection.interrupt_tree_detached(live.provider_session.clone(), targets);
        }
    }

    /// 멈춘 session은 보류로 두고 사용자가 이을 때까지 쓰지 않는다.
    async fn hold_session(&mut self, live: &LiveSession) {
        let held = self.sessions.set_state(live.session, SessionState::Held);
        if held.is_err() {
            self.warn_failure("session was not held", held);
            return;
        }
        let persisted = self.persist_sessions(live.session).await;
        self.warn_failure("failed to record held session", persisted);
    }

    fn group_of(&self, chat: ChatId, live: &LiveSession) -> Option<ProcessGroupId> {
        self.providers
            .get(&(chat, live.provider))?
            .process_group(&live.provider_session)
    }

    /// 묶음마다 별도 작업에서 중지한다. 결과는 요청 처리 루프로 돌아와 완료를 확인한다.
    fn spawn_stop(&self, chat: ChatId, group: ProcessGroupId) {
        let supervisor = self.supervisor.clone();
        let done = self.flow.stop_tx.clone();
        tokio::spawn(async move {
            let result = supervisor.stop_tree(group, StopScope::Descendants).await;
            let _ = done.send(StopDone { chat, result }); // engine이 끝난 뒤에는 받을 곳이 없다
        });
    }

    pub(crate) async fn on_stop_done(&mut self, done: StopDone) {
        let StopDone { chat, result } = done;
        let remaining = self.unconfirmed_after(&result);
        let Some(progress) = self.flow.stopping.get_mut(&chat) else {
            return;
        };
        progress.pending_groups = progress.pending_groups.saturating_sub(1);
        progress.unconfirmed = progress.unconfirmed.saturating_add(remaining);
        if let Err(error) = self.check_stop_done(chat).await {
            tracing::warn!(error = %self.failure_line(&error), "failed to finish stop");
        }
    }

    /// 확인하지 못한 프로세스 수. 중지 호출이 실패하면 하나로 센다.
    fn unconfirmed_after(&self, result: &Result<StopOutcome, ProcessError>) -> u32 {
        match result {
            Ok(StopOutcome::Stopped) => 0,
            Ok(StopOutcome::Unconfirmed { remaining }) => {
                u32::try_from(*remaining).unwrap_or(u32::MAX)
            }
            Err(error) => {
                tracing::warn!(error = %self.failure_line(error), "failed to confirm process stop");
                1
            }
        }
    }

    /// 멈춘 뒤의 완료 신호를 받았다.
    pub(crate) fn confirm_stopped_agent(&mut self, chat: ChatId, agent: AgentId) {
        if let Some(confirmed) = self
            .flow
            .stopping
            .get_mut(&chat)
            .and_then(|progress| progress.agents.get_mut(&agent))
        {
            *confirmed = true;
        }
    }

    /// 모든 에이전트의 트리가 유휴이고 프로세스 묶음 중지 결과가 모두 돌아왔을 때만 끝낸다.
    /// 이미 답을 마친 에이전트(`AnsweredTreeRunning`)는 subagent가 끝나기만 보면 된다.
    pub(crate) async fn check_stop_done(&mut self, chat: ChatId) -> Result<(), EngineError> {
        let Some(progress) = self.flow.stopping.get(&chat) else {
            return Ok(());
        };
        let is_tree_stopped = progress.agents.iter().all(|(agent, confirmed)| {
            let answered = self.agents.status(*agent) == Some(TreeStatus::AnsweredTreeRunning);
            (*confirmed || answered) && self.agents.is_tree_idle(*agent)
        });
        if progress.pending_groups > 0 || !is_tree_stopped {
            return Ok(());
        }
        let Some(progress) = self.flow.stopping.remove(&chat) else {
            return Ok(());
        };
        self.finish_stop(chat, progress).await
    }

    async fn finish_stop(
        &mut self,
        chat: ChatId,
        progress: StopProgress,
    ) -> Result<(), EngineError> {
        for agent in progress.agents.keys() {
            self.clear_permissions(*agent).await;
            self.end_stopped_run(*agent).await;
        }
        let ended = self.store.end_stop(chat).await;
        let mut labels = Vec::new();
        for task in &progress.held {
            if let Some(label) = self.flow.tasks.label(*task) {
                labels.push(label);
            }
            self.notify_task(chat, *task, TaskState::Held, None, None)
                .await;
        }
        let notice = if progress.unconfirmed > 0 {
            ChatNotice::StopUnconfirmed {
                remaining: progress.unconfirmed,
            }
        } else {
            ChatNotice::Stopped { held: labels }
        };
        self.notify_chat(chat, notice).await;
        self.run_after_stop(chat).await;
        Ok(ended?)
    }

    /// 멈춤을 고른 충돌 입력은 멈춤이 끝난 뒤에 그 입력이 붙은 작업을 재개해 실행한다. 멈춘 작업의 확인 입력과 쓰기
    /// 규칙은 재개 규칙을 그대로 따른다. 보류가 아니게 된 입력은 건너뛴다.
    async fn run_after_stop(&mut self, chat: ChatId) {
        let (mine, others): (Vec<InputId>, Vec<InputId>) =
            std::mem::take(&mut self.flow.run_after_stop)
                .into_iter()
                .partition(|input| {
                    self.queue
                        .input(*input)
                        .is_some_and(|record| record.chat == chat)
                });
        self.flow.run_after_stop = others;
        for input in mine {
            let is_held = self
                .queue
                .input(input)
                .is_some_and(|record| record.state == InputState::Held);
            if is_held {
                let resumed = self.continue_input(input).await;
                self.warn_failure("failed to run the input after stopping", resumed);
            }
        }
    }

    async fn end_stopped_run(&mut self, agent: AgentId) {
        let (chat, task) = (
            self.runs.chat_of.get(&agent).copied(),
            self.runs.task_of.get(&agent).copied(),
        );
        let Some(run) = self.runs.forget(agent) else {
            return;
        };
        if let (Some(chat), Some(task)) = (chat, task) {
            self.keep_stopped_changes(run, chat, task, agent).await;
        }
        let ended = self.store.finish_run(run, RunEnd::Stopped).await;
        self.warn_failure("failed to end stopped run", ended);
    }

    /// `task`가 없으면 채팅의 보류 전부를 접수 순서로 재개한다. 멈출 때 실행 중이던 작업에는 파일 상태부터
    /// 확인하게 하는 새 입력을 보내고, 같은 패킷은 다시 보내지 않는다. 결과를 모르는 작업(`NeedsCheck`)은
    /// `task`로 가리킬 때만 같은 확인 입력으로 잇는다.
    /// 멈출 때 바뀐 파일 목록이 있으면 확인 입력에 함께 적는다.
    ///
    /// # Errors
    /// 기록 저장소 쓰기 실패는 `Store`, 이어 보내는 중의 오류는 `dispatch_next`와 같다.
    pub(crate) async fn continue_held(
        &mut self,
        chat: ChatId,
        task: Option<TaskId>,
    ) -> Result<(), EngineError> {
        if let Some(task) = task
            && self
                .flow
                .needs_check
                .get(&task)
                .is_some_and(|check| check.chat == chat)
        {
            self.settle_unknown(chat, task).await?;
        }
        let held_inputs = self.queue.inputs_in_state(chat, InputState::Held);
        let interrupted = self.queue.resume(chat, task);
        for input in held_inputs {
            if self.queue.input(input).map(|record| record.state) == Some(InputState::Queued) {
                self.store
                    .set_input_state(input, InputState::Queued, None)
                    .await?;
                self.notify_input(input).await;
            }
        }
        for task in interrupted {
            self.send_confirmation(chat, task).await?;
        }
        self.dispatch_next(chat).await
    }

    /// 입력 하나를 재개하는 요청도 그 입력이 붙은 작업 전체를 재개한다. 같은 작업의 보류 입력은 접수 순서대로
    /// 모두 가고, 재개와 보류 종료를 작업 단위로 맞추기 위해서다.
    ///
    /// # Errors
    /// 보류가 아닌 입력이면 `Queue(InvalidTransition)`, 그 밖에는 `continue_held`와 같다.
    pub(crate) async fn continue_input(&mut self, input: InputId) -> Result<(), EngineError> {
        let record = self.queued(input)?;
        if record.state != InputState::Held {
            return Err(QueueError::InvalidTransition {
                from: record.state,
                to: InputState::Queued,
            }
            .into());
        }
        self.continue_held(record.chat, record.task).await
    }

    /// 보내지 않은 입력은 취소하고 session을 끝낸다. 기록과 수정된 파일은 그대로 둔다.
    ///
    /// # Errors
    /// 기록 저장소 쓰기 실패는 `Store`.
    pub(crate) async fn close_held(
        &mut self,
        chat: ChatId,
        task: TaskId,
    ) -> Result<(), EngineError> {
        let cancelled = self.queue.close_held(task);
        for input in cancelled {
            self.store
                .set_input_state(input, InputState::Cancelled, None)
                .await?;
            self.release_canceled_input(input).await;
            self.notify_input(input).await;
        }
        if let Some(held) = self.release_held(task).await {
            if let Some(agent) = held.agent {
                self.flow.interrupted_to_clean.remove(&agent);
                self.flow.interrupted_watch.remove(&agent);
                let deleted = self.store.delete_interrupted_subagents(agent).await;
                self.warn_failure("failed to delete interrupted subagents", deleted);
            }
            self.end_held_session(chat, held).await;
        }
        self.flow.tasks.release(task);
        Ok(())
    }

    /// 보류 종료 때 그 작업의 에이전트 session을 끝낸다.
    async fn end_held_session(&mut self, chat: ChatId, held: HeldTask) {
        let Some(agent) = held.agent else {
            return;
        };
        let Some(live) = self.flow.live.remove(&agent) else {
            return;
        };
        if let Ok(connection) = self.provider_mut(chat, live.provider) {
            connection.close_session_detached(live.provider_session.clone());
        }
        let ended = self.sessions.set_state(live.session, SessionState::Ended);
        if ended.is_err() {
            self.warn_failure("held session was not ended", ended);
            return;
        }
        let persisted = self.persist_sessions(live.session).await;
        self.warn_failure("failed to record ended session", persisted);
    }

    /// 결과를 모르는 실행은 닫고 그 입력은 `전달 중`으로 둔다. 같은 입력을 다시 보내지 않고 확인 입력으로 잇는다.
    async fn settle_unknown(&mut self, chat: ChatId, task: TaskId) -> Result<(), EngineError> {
        let Some(check) = self.flow.needs_check.remove(&task) else {
            return Ok(());
        };
        self.queue.finish_task(check.agent);
        if let Some(run) = self.runs.forget(check.agent) {
            self.keep_stopped_changes(run, chat, task, check.agent)
                .await;
            self.store.finish_run(run, RunEnd::Stopped).await?;
        }
        self.hold_task(
            chat,
            task,
            HeldTask {
                agent: Some(check.agent),
                input: Some(check.input),
            },
        )
        .await;
        self.send_confirmation(chat, task).await
    }

    /// 멈춘 실행이 시작 뒤 바꾼 파일 목록을 기록하고, 그 작업의 확인 입력에 쓰도록 둔다. 바뀐 파일이 없으면 두지 않는다.
    async fn keep_stopped_changes(
        &mut self,
        run: RunId,
        chat: ChatId,
        task: TaskId,
        agent: AgentId,
    ) {
        match self.settle_changes(run, chat, agent).await {
            Some(changes) if !changes.files.is_empty() => {
                self.flow.stopped_changes.insert(task, changes);
            }
            _ => {
                self.flow.stopped_changes.remove(&task);
            }
        }
    }

    /// 파일 상태를 확인한 뒤 이어 가게 하는 새 입력을 그 작업에 접수한다.
    async fn send_confirmation(&mut self, chat: ChatId, task: TaskId) -> Result<(), EngineError> {
        let Some(source) = self
            .flow
            .held
            .get(&task)
            .and_then(|held| held.input)
            .and_then(|input| self.queue.input(input).cloned())
        else {
            return Ok(());
        };
        self.release_held(task).await;
        let changes = self.flow.stopped_changes.remove(&task);
        let new = NewInput {
            chat,
            text: confirmation_text(&source.text, changes.as_ref()),
            settings: source.settings,
            permission: source.permission,
            workdir: source.workdir.clone(),
            pinned_model: source.pinned_model.clone(),
            skip_relation: true,
        };
        let id = self.store.accept_input(&new).await?;
        self.queue.accept(QueuedInput {
            id,
            chat,
            text: new.text,
            settings: new.settings,
            permission: new.permission,
            workdir: new.workdir,
            write_scope: source.write_scope,
            pinned_model: new.pinned_model,
            skip_relation: true,
            state: InputState::Judging,
            reason: None,
            task: Some(task),
        });
        let record = self.queued(id)?;
        let decision = direct_decision(&record, self.queue.revision(chat));
        self.apply_decision(id, decision, true).await
    }
}

/// 멈춘 턴은 별도 경고 문장 없이 결과 자리의 오류 결과로만 알린다. 멈추기 전까지 바뀐 파일이 있으면 결과 뒤에 적어
/// 에이전트가 처음부터 다시 훑지 않고 그 파일의 상태부터 확인하게 한다.
fn confirmation_text(original: &str, changes: Option<&ChangeSet>) -> String {
    let files = changes
        .and_then(describe)
        .map(|files| format!("\n\nFiles changed during that turn: {files}"))
        .unwrap_or_default();
    format!("Previous turn result (error): {INTERRUPTED_RESULT}{files}\n\nRequest:\n{original}")
}
