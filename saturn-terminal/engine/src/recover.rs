//! 크래시 복구: 끝나지 않은 실행을 보류로 되살리고, 효과 범위가 증명된 실행만 자동으로 이어 간다.
//! 설계: docs/design/engine-lifecycle.md#크래시-뒤-복구

use saturn_core::queue::QueuedInput;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, SubagentId, TaskId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::SessionState;

use crate::chat_env::ChatEnv;
use crate::stop::HeldTask;
use crate::store::{RunEnd, RunRecord};
use crate::{Engine, EngineError};

impl Engine {
    /// 시작 직후 한 번 부른다. 끝나지 않은 실행마다 보류로 되살린 뒤, 효과 범위가 증명된 실행은 파일 상태를 확인하게 하는
    /// 새 입력으로 이어 가고 나머지는 보류한 채 TUI가 붙으면 `/continue`를 제안한다.
    /// 크래시 전에 보낸 패킷은 어느 경우에도 다시 보내지 않는다.
    /// TODO(#66): 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 정리할지, 끊김 표시만 할지
    ///
    /// # Errors
    /// 끝나지 않은 실행을 읽지 못하면 `Store`. 실행 하나의 복구 실패는 경고로 남기고 나머지를 이어서 복구한다.
    pub(crate) async fn recover_after_crash(&mut self) -> Result<(), EngineError> {
        for run in self.latest_unfinished_runs().await? {
            let restored = self.hold_interrupted(&run).await;
            match restored {
                Ok(true) if run.effect_scope.allows_auto_resume() => {
                    self.resume_proven(run).await?;
                }
                Ok(true) => self.hold_unproven(run).await?,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(run = run.id.0, error = %self.failure_line(&error), "failed to recover interrupted run");
                }
            }
        }
        Ok(())
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
    /// TODO(#65): 수정 파일 목록을 실행 경계의 파일 상태 차이로 셀지, provider 이벤트로 셀지
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
        self.notices
            .resume_suggested
            .entry(run.chat)
            .or_default()
            .push(run.task);
        Ok(())
    }

    /// 끊긴 실행의 작업을 보류로 되살리고 session도 보류로 둔 뒤 실행을 닫는다.
    /// 입력 없이 시작한 턴(`provider-wake`)은 확인 입력을 만들 원문이 없어 session만 보류하고 닫으며 거짓을 돌려준다.
    async fn hold_interrupted(&mut self, run: &RunRecord) -> Result<bool, EngineError> {
        let restored = match run.input {
            Some(input) => {
                self.restore_held_task(run, input).await?;
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

    async fn restore_held_task(
        &mut self,
        run: &RunRecord,
        input: saturn_protocol::ids::InputId,
    ) -> Result<(), EngineError> {
        let (stored, state) = self.store.stored_input(input).await?;
        let chat = run.chat;
        self.load_chat_dirs(chat).await?;
        if self.chat_env(chat).is_none() {
            let workdir = self.store.chat_workdir(chat).await?;
            self.chats
                .insert(chat, ChatEnv::new(workdir, std::env::vars().collect()));
        }
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
                task: Some(run.task),
            },
            run.agent,
            run.task,
        );
        self.flow.tasks.assign(run.task);
        self.flow.held.insert(
            run.task,
            HeldTask {
                agent: Some(run.agent),
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
