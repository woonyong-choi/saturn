//! 크래시 복구: 끝나지 않은 실행을 보류로 되살리고, 효과 범위가 증명된 실행만 자동으로 이어 간다.
//! 설계: docs/design/engine-lifecycle.md#크래시-뒤-복구

use saturn_core::queue::QueuedInput;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, InputId, SubagentId, TaskId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState};

use crate::chat_env::ChatEnv;
use crate::stop::HeldTask;
use crate::store::{NewInput, RunEnd, RunRecord, StoredHold};
use crate::{Engine, EngineError};

impl Engine {
    /// 시작 직후 한 번 부른다. 끝나지 않은 실행마다 보류로 되살린 뒤, 효과 범위가 증명된 실행은 파일 상태를 확인하게 하는
    /// 새 입력으로 이어 가고 나머지는 보류한 채 TUI가 붙으면 `/continue`를 제안한다.
    /// 크래시 전에 보낸 패킷은 어느 경우에도 다시 보내지 않는다.
    /// 앞서 정상 종료한 engine이 기록에 남긴 보류 작업과 끊긴 하위 에이전트는 이 복구보다 먼저 되살린다.
    /// 이 복구를 마친 뒤에는 기록에 남은 보내지 않은 입력을 대기열에 되살린다.
    ///
    /// # Errors
    /// 끝나지 않은 실행을 읽지 못하면 `Store`. 실행 하나의 복구 실패는 경고로 남기고 나머지를 이어서 복구한다.
    pub(crate) async fn recover_after_crash(&mut self) -> Result<(), EngineError> {
        self.restore_recorded_holds().await?;
        for run in self.latest_unfinished_runs().await? {
            self.recover_run(run).await?;
        }
        self.restore_constraint_asks().await?;
        self.restore_open_inputs().await
    }

    /// 기록 저장소에 남은 보내지 않은 입력을 접수 순서대로 대기열에 되살리고, 채팅마다 보낼 것을 보내고 판단할 것을 판단한다.
    /// 앞선 복구가 되살린 입력은 이미 대기열에 있어 건너뛴다. 입력 하나의 복원 실패는 경고로 남기고 나머지를 잇는다.
    async fn restore_open_inputs(&mut self) -> Result<(), EngineError> {
        let mut chats: Vec<ChatId> = Vec::new();
        for (id, stored, state) in self.store.open_inputs().await? {
            if self.queue.input(id).is_some() {
                continue;
            }
            let chat = stored.chat;
            let restored = self.restore_open_input(id, stored, state).await;
            let is_restored = restored.is_ok();
            self.warn_failure("failed to restore an open input", restored);
            if is_restored && !chats.contains(&chat) {
                chats.push(chat);
            }
        }
        for chat in chats {
            self.advance(chat).await;
        }
        Ok(())
    }

    /// 보냈는지 모르는 `Delivering` 입력은 대기열에 보이게만 두고 다시 보내지 않는다. 이어 가는 것은 사용자의 확인이다.
    /// 채팅에 보류 작업이 있으면 판단 중이거나 대기하던 입력도 멈춤과 같게 보류해 그 작업과 함께 `/continue`를 기다린다.
    /// 보류 입력은 보류 작업에 붙여 `/continue`를 제안한다.
    async fn restore_open_input(
        &mut self,
        id: InputId,
        stored: NewInput,
        state: InputState,
    ) -> Result<(), EngineError> {
        let chat = stored.chat;
        self.ensure_chat_env(chat).await?;
        let is_unsent = matches!(state, InputState::Judging | InputState::Queued);
        let state = if is_unsent && self.queue.has_held_task(chat) {
            self.store
                .set_input_state(id, InputState::Held, None)
                .await?;
            InputState::Held
        } else {
            state
        };
        let write_scope = self.write_scope_of(chat, &stored.workdir);
        self.queue.restore_unsent(
            QueuedInput {
                id,
                chat,
                text: stored.text,
                settings: stored.settings,
                permission: stored.permission,
                workdir: stored.workdir,
                write_scope,
                pinned_model: stored.pinned_model,
                skip_relation: stored.skip_relation,
                state,
                reason: None,
                task: None,
            },
            state,
        );
        match state {
            InputState::Held => self.hold_restored_input(chat, id),
            InputState::Delivering => {
                tracing::warn!(
                    input = id.0,
                    "input delivery result is unknown, not resending"
                );
            }
            _ => {}
        }
        Ok(())
    }

    /// 되살린 보류 입력의 작업을 보류로 등록하고 처음 붙는 TUI에 재개를 제안한다.
    fn hold_restored_input(&mut self, chat: ChatId, input: InputId) {
        let Some(task) = self.queue.input(input).and_then(|record| record.task) else {
            return;
        };
        self.flow.tasks.assign(task);
        self.flow.held.entry(task).or_insert(HeldTask {
            agent: None,
            input: None,
        });
        self.suggest_resume(chat, task);
    }

    /// 채팅의 작업 폴더와 더한 폴더를 읽고, 붙은 TUI가 없으면 `engine` 프로세스 환경으로 채팅 환경을 만든다.
    async fn ensure_chat_env(&mut self, chat: ChatId) -> Result<(), EngineError> {
        self.load_chat_dirs(chat).await?;
        if self.chat_env(chat).is_none() {
            let workdir = self.store.chat_workdir(chat).await?;
            self.chats
                .insert(chat, ChatEnv::new(workdir, std::env::vars().collect()));
        }
        Ok(())
    }

    /// 실행 하나를 보류로 되살리고 증명된 실행은 이어 간다. 이 실행의 복구 실패는 경고로 남긴다.
    async fn recover_run(&mut self, run: RunRecord) -> Result<(), EngineError> {
        let restored = self.hold_interrupted(&run).await;
        match restored {
            Ok(true) if run.effect_scope.allows_auto_resume() => self.resume_proven(run).await,
            Ok(true) => self.hold_unproven(run).await,
            Ok(false) => Ok(()),
            Err(error) => {
                tracing::warn!(run = run.id.0, error = %self.failure_line(&error), "failed to recover interrupted run");
                Ok(())
            }
        }
    }

    /// 작업마다 가장 나중 실행만 돌려준다. 같은 작업의 앞선 실행은 가장 나중 실행이 대신하므로 닫기만 한다.
    async fn latest_unfinished_runs(&mut self) -> Result<Vec<RunRecord>, EngineError> {
        let mut latest: Vec<RunRecord> = Vec::new();
        for run in self.store.unfinished_runs().await? {
            match latest.iter_mut().find(|kept| kept.task == run.task) {
                Some(kept) => {
                    let older = std::mem::replace(kept, run);
                    let ended = self.store.finish_run(older.id, RunEnd::Stopped).await;
                    self.warn_failure("failed to end an older interrupted run", ended);
                }
                None => latest.push(run),
            }
        }
        Ok(latest)
    }

    /// 파일 상태를 확인한 뒤 그 상태로 만든 새 입력을 접수해 보낸다. 보내지 못하면 보류로 남기고 제안한다.
    /// 크래시로 시작 때 폴더 상태를 잃어 수정 파일 목록은 확인 입력에 싣지 못한다.
    async fn resume_proven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        match self.continue_held(run.chat, Some(run.task)).await {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::warn!(run = run.id.0, error = %self.failure_line(&error), "failed to resume proven run, holding it");
                self.hold_unproven(run).await
            }
        }
    }

    /// 사용자가 `/continue`로 이을 때까지 멈춰 둔다. 제안은 그 채팅에 처음 TUI가 붙을 때 보낸다.
    async fn hold_unproven(&mut self, run: RunRecord) -> Result<(), EngineError> {
        self.suggest_resume(run.chat, run.task);
        Ok(())
    }

    /// 같은 작업은 한 번만 제안 목록에 넣는다.
    fn suggest_resume(&mut self, chat: ChatId, task: TaskId) {
        let tasks = self.notices.resume_suggested.entry(chat).or_default();
        if !tasks.contains(&task) {
            tasks.push(task);
        }
    }

    /// 기록 저장소에 남은 끊긴 하위 에이전트와 보류 작업을 메모리로 되살리고, 보류 작업마다 처음 붙는 TUI에 재개를 제안한다.
    /// 재개하거나 닫은 작업은 그때 기록에서 지워 남아 있지 않다. 보류 하나의 복구 실패는 경고로 남기고 나머지를 잇는다.
    async fn restore_recorded_holds(&mut self) -> Result<(), EngineError> {
        for row in self.store.interrupted_subagents().await? {
            self.flow
                .interrupted_watch
                .entry(row.agent)
                .or_default()
                .insert(row.subagent.clone());
            if !row.cleaned {
                self.flow
                    .interrupted_to_clean
                    .entry(row.agent)
                    .or_default()
                    .push(row.subagent);
            }
        }
        for hold in self.store.held_tasks().await? {
            let restored = self
                .restore_held_task(hold.chat, hold.agent, hold.task, hold.input)
                .await;
            match restored {
                Ok(()) => self.suggest_resume(hold.chat, hold.task),
                Err(error) => {
                    tracing::warn!(task = hold.task.0, error = %self.failure_line(&error), "failed to restore recorded held task");
                }
            }
        }
        Ok(())
    }

    /// 끊긴 실행의 작업을 보류로 되살리고 session도 보류로 둔 뒤 실행을 닫는다.
    /// 입력 없이 시작한 턴(`provider-wake`)은 확인 입력을 만들 원문이 없어 session만 보류하고 닫으며 거짓을 돌려준다.
    async fn hold_interrupted(&mut self, run: &RunRecord) -> Result<bool, EngineError> {
        let restored = match run.input {
            Some(input) => {
                self.restore_held_task(run.chat, run.agent, run.task, input)
                    .await?;
                self.store
                    .save_held_task(&StoredHold {
                        task: run.task,
                        chat: run.chat,
                        agent: run.agent,
                        input,
                    })
                    .await?;
                true
            }
            None => false,
        };
        let held = self.sessions.set_state(run.session, SessionState::Held);
        if held.is_err() {
            self.warn_failure("interrupted session was not held", held);
        } else {
            let persisted = self.persist_sessions(run.session).await;
            self.warn_failure("failed to record held session", persisted);
        }
        self.interrupt_subagents(run).await?;
        self.store.finish_run(run.id, RunEnd::Stopped).await?;
        Ok(restored)
    }

    /// 실행 중으로 남은 하위 에이전트를 기록에서 `끊김`으로 바꾼다. provider가 다시 실행하지 못하게 session을 다시 열 때
    /// 정리할 목록에 넣고, 다시 연 뒤 이 하위 에이전트의 이벤트가 오면 막을 감시 목록에도 넣는다. 다시 할지는 사용자가 정한다.
    async fn interrupt_subagents(&mut self, run: &RunRecord) -> Result<(), EngineError> {
        let mut running: Vec<SubagentId> = Vec::new();
        for event in self.store.run_events(run.id).await? {
            match event {
                ProviderEvent::SubagentStarted { subagent, .. } => running.push(subagent),
                ProviderEvent::SubagentEnded { subagent, .. }
                | ProviderEvent::SubagentInterrupted { subagent, .. } => {
                    running.retain(|id| *id != subagent);
                }
                _ => {}
            }
        }
        for subagent in running {
            let event = ProviderEvent::SubagentInterrupted {
                agent: run.agent,
                subagent: subagent.clone(),
            };
            self.store.append_event(run.id, run.chat, &event).await?;
            self.store
                .save_interrupted_subagent(run.chat, run.agent, &subagent)
                .await?;
            self.flow
                .interrupted_to_clean
                .entry(run.agent)
                .or_default()
                .push(subagent.clone());
            self.flow
                .interrupted_watch
                .entry(run.agent)
                .or_default()
                .insert(subagent);
        }
        Ok(())
    }

    /// 입력은 보낸 상태 그대로 두고 작업만 보류로 되살린다. 기록 저장소에는 쓰지 않는다.
    async fn restore_held_task(
        &mut self,
        chat: ChatId,
        agent: AgentId,
        task: TaskId,
        input: InputId,
    ) -> Result<(), EngineError> {
        let (stored, state) = self.store.stored_input(input).await?;
        self.ensure_chat_env(chat).await?;
        let write_scope = self.write_scope_of(chat, &stored.workdir);
        self.queue.restore_interrupted(
            QueuedInput {
                id: input,
                chat,
                text: stored.text,
                settings: stored.settings,
                permission: stored.permission,
                workdir: stored.workdir,
                write_scope,
                pinned_model: stored.pinned_model,
                skip_relation: stored.skip_relation,
                state,
                reason: None,
                task: Some(task),
            },
            agent,
            task,
        );
        self.flow.tasks.assign(task);
        self.flow.held.insert(
            task,
            HeldTask {
                agent: Some(agent),
                input: Some(input),
            },
        );
        Ok(())
    }

    /// 크래시 복구가 보류한 작업을 그 채팅에 처음 붙은 TUI에 알리고 `/continue`를 제안한다. 한 번만 보낸다.
    /// 그 사이 재개하거나 닫은 작업은 뺀다.
    pub(crate) async fn send_resume_suggestions(&mut self, chat: ChatId) {
        let Some(tasks) = self.notices.resume_suggested.remove(&chat) else {
            return;
        };
        let still_held: Vec<TaskId> = tasks
            .into_iter()
            .filter(|task| self.flow.held.contains_key(task))
            .collect();
        let mut labels = Vec::new();
        for task in still_held {
            if let Some(label) = self.flow.tasks.label(task) {
                labels.push(label);
            }
            self.notify_task(
                chat,
                task,
                saturn_protocol::state::TaskState::Held,
                None,
                None,
            )
            .await;
        }
        if !labels.is_empty() {
            self.notify_chat(chat, ChatNotice::ResumeSuggested { held: labels })
                .await;
        }
    }
}
