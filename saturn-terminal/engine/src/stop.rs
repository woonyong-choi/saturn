//! 멈춤과 보류: 트리 전체에 멈춤 신호를 보내고, 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 완료를 알린다.
//! 보류 재개와 보류 종료도 여기서 한다.
//! 설계: docs/design/input-handling.md#멈춤과-보류, docs/design/providers-and-sessions.md#트리-전체-중지

use std::collections::HashMap;

use saturn_core::agents::TreeStatus;
use saturn_core::providers::{InterruptTarget, ProviderClient, ProviderError};
use saturn_core::queue::{QueueError, QueuedInput};
use saturn_protocol::ids::{AgentId, ChatId, InputId, TaskId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState, TaskState};

use crate::flow::LiveSession;
use crate::intake::direct_decision;
use crate::processes::{ProcessError, ProcessGroupId, StopOutcome, StopScope};
use crate::store::{NewInput, RunEnd};
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
        if self.flow.stopping.contains_key(&chat) {
            return Ok(());
        }
        self.store.begin_stop(chat).await?;
        let held = self.queue.stop(chat);
        let running = self.running_agents(chat);
        let mut groups: Vec<ProcessGroupId> = Vec::new();
        self.remember_held(&held, &running).await;
        for agent in &running {
            let Some(live) = self.flow.live.get(agent).cloned() else {
                continue;
            };
            self.interrupt_tree(chat, &live).await;
            self.hold_session(&live).await;
            if let Some(group) = self.group_of(chat, &live)
                && !groups.contains(&group)
            {
                groups.push(group);
            }
        }
        for input in self.queue.inputs_in_state(chat, InputState::Held) {
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

    /// 진행 중인 실행이 있는 에이전트.
    fn running_agents(&self, chat: ChatId) -> Vec<AgentId> {
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
    async fn remember_held(&mut self, held: &[TaskId], running: &[AgentId]) {
        for agent in running {
            let Some(task) = self.runs.task_of.get(agent).copied() else {
                continue;
            };
            let input = self.run_input(*agent).await;
            self.flow.held.insert(
                task,
                HeldTask {
                    agent: Some(*agent),
                    input,
                },
            );
        }
        for task in held {
            self.flow.tasks.assign(*task);
            self.flow.held.entry(*task).or_insert(HeldTask {
                agent: None,
                input: None,
            });
        }
    }

    /// 깊은 subagent부터 메인 순서로 멈춤 신호를 보낸다. 연결이 끊겼으면 프로세스 중지로 넘어간다.
    async fn interrupt_tree(&mut self, chat: ChatId, live: &LiveSession) {
        for target in self.agents.interrupt_order(live.agent) {
            let target = target.map_or(InterruptTarget::Main, InterruptTarget::Subagent);
            let sent = match self.provider_mut(chat, live.provider) {
                Ok(connection) => connection.interrupt(&live.provider_session, target).await,
                Err(error) => Err(error),
            };
            match sent {
                Ok(()) => {}
                Err(ProviderError::ConnectionLost) => break,
                Err(error) => {
                    tracing::warn!(error = %self.failure_line(&error), "interrupt was not delivered");
                }
            }
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
        Ok(ended?)
    }

    async fn end_stopped_run(&mut self, agent: AgentId) {
        let Some(run) = self.runs.forget(agent) else {
            return;
        };
        let ended = self.store.finish_run(run, RunEnd::Stopped).await;
        self.warn_failure("failed to end stopped run", ended);
    }

    /// `task`가 없으면 채팅의 보류 전부를 접수 순서로 재개한다. 멈출 때 실행 중이던 작업에는 파일 상태부터
    /// 확인하게 하는 새 입력을 보내고, 같은 패킷은 다시 보내지 않는다. 결과를 모르는 작업(`NeedsCheck`)은
    /// `task`로 가리킬 때만 같은 확인 입력으로 잇는다.
    /// TODO(#65): 수정 파일 목록을 파일 상태 차이로 셀지 provider 이벤트로 셀지 정해지기 전에는 에이전트가 직접 확인한다
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

    /// 보류 입력 하나만 재개하는 규칙이 설계에 없어 입력이 붙은 작업을 재개한다.
    /// TODO(#90): 입력 하나만 재개할지, 같은 작업의 보류 입력을 함께 재개할지
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
            self.notify_input(input).await;
        }
        if let Some(held) = self.flow.held.remove(&task) {
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
            let closed = connection.close_session(&live.provider_session).await;
            self.warn_failure("failed to close held session", closed);
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
            self.store.finish_run(run, RunEnd::Stopped).await?;
        }
        self.flow.held.insert(
            task,
            HeldTask {
                agent: Some(check.agent),
                input: Some(check.input),
            },
        );
        self.send_confirmation(chat, task).await
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
        self.flow.held.remove(&task);
        let new = NewInput {
            chat,
            text: confirmation_text(&source.text),
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

/// 멈춘 턴이 어디까지 갔는지는 에이전트가 파일 상태로 확인하게 한다.
fn confirmation_text(original: &str) -> String {
    format!(
        "Your previous turn was stopped before it finished, so some of its changes may be half done. \
         Check the current state of the working files first, then continue the request below from that state \
         without repeating what is already done.\n\nRequest:\n{original}"
    )
}
