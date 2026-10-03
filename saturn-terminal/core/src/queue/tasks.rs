use saturn_protocol::ids::{AgentId, ChatId, InputId, TaskId};
use saturn_protocol::state::InputState;

#[cfg(test)]
use super::WriteGate;
use super::{HELD_IGNORE_LIMIT, Permission, Queue, QueueError, TaskPhase, TaskSlot};

impl Queue {
    // cost: time O(t + n·t), heap O(t), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, t = 작업 수
    // basis: estimate
    /// 채팅의 보류 작업 전부를 id 순서로 돌려준다.
    pub fn stop(&mut self, chat: ChatId) -> Vec<TaskId> {
        let main = self.main_task(chat).map(|slot| slot.id);
        for slot in &mut self.tasks {
            if slot.chat == chat && matches!(slot.phase, TaskPhase::Running | TaskPhase::Pending) {
                slot.was_interrupted = slot.phase == TaskPhase::Running;
                slot.phase = TaskPhase::Held;
            }
        }
        let mut fallback_main = main;
        for index in 0..self.inputs.len() {
            let entry = &self.inputs[index];
            let is_unsent = matches!(entry.input.state, InputState::Judging | InputState::Queued);
            if entry.input.chat != chat || !is_unsent || entry.is_dispatched {
                continue;
            }
            let task = self.hold_target(index, &mut fallback_main);
            let entry = &mut self.inputs[index];
            entry.input.state = InputState::Held;
            entry.input.reason = None;
            entry.input.task = Some(task);
        }
        for entry in &mut self.inputs {
            if entry.input.chat == chat {
                entry.awaits_stop = false;
                entry.is_conflict = false;
            }
        }
        self.chats.entry(chat).or_default().held_ignored = 0;
        self.bump(chat);
        self.held_tasks(chat, None)
    }

    // cost: time O(n·t), heap O(t), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, t = 작업 수
    // basis: estimate
    /// 멈춤 때 실행 중이던 작업, 즉 engine이 새 입력을 만들어 보낼 작업을 돌려준다.
    pub fn resume(&mut self, chat: ChatId, task: Option<TaskId>) -> Vec<TaskId> {
        let resumed = self.held_tasks(chat, task);
        let mut interrupted = Vec::new();
        for slot in &mut self.tasks {
            if !resumed.contains(&slot.id) {
                continue;
            }
            slot.phase = TaskPhase::Idle;
            if slot.was_interrupted {
                interrupted.push(slot.id);
                slot.was_interrupted = false;
            }
        }
        for entry in &mut self.inputs {
            let is_target = match entry.input.task {
                Some(id) => resumed.contains(&id),
                None => task.is_none(),
            };
            if entry.input.chat == chat && entry.input.state == InputState::Held && is_target {
                entry.input.state = InputState::Queued;
            }
        }
        self.reset_ignored_if_clear(chat);
        self.bump(chat);
        interrupted
    }

    // cost: time O(t), heap O(t), stack O(1), alloc 1
    // vars: t = 작업 수
    // basis: estimate
    /// 보류 중 재개 뜻이 없는 입력이 `HELD_IGNORE_LIMIT`개 쌓이면 닫을 보류 작업을 돌려준다.
    pub fn note_resume_signal(&mut self, chat: ChatId, resume: bool) -> Option<Vec<TaskId>> {
        let held = self.held_tasks(chat, None);
        let state = self.chats.entry(chat).or_default();
        if held.is_empty() || resume {
            state.held_ignored = 0;
            return None;
        }
        state.held_ignored += 1;
        if state.held_ignored < HELD_IGNORE_LIMIT {
            return None;
        }
        state.held_ignored = 0;
        Some(held)
    }

    // cost: time O(n + t + h), heap O(k), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수, k = 취소한 입력 수
    // basis: estimate
    /// 기록과 수정된 파일은 그대로 두고, 취소한 입력을 접수 순서로 돌려준다.
    pub fn close_held(&mut self, task: TaskId) -> Vec<InputId> {
        let Some(slot) = self
            .tasks
            .iter_mut()
            .find(|slot| slot.id == task && slot.phase == TaskPhase::Held)
        else {
            return Vec::new();
        };
        slot.phase = TaskPhase::Closed;
        slot.was_interrupted = false;
        let chat = slot.chat;
        if let Some(agent) = slot.agent {
            self.gate.release(agent);
        }
        let mut cancelled = Vec::new();
        for entry in &mut self.inputs {
            if entry.input.task == Some(task) && entry.input.state == InputState::Held {
                entry.input.state = InputState::Cancelled;
                cancelled.push(entry.input.id);
            }
        }
        self.reset_ignored_if_clear(chat);
        self.bump(chat);
        cancelled
    }

    // cost: time O(t + h), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 이미 보류된 작업이면 에이전트만 기록하고 잠금은 잡지 않는다.
    ///
    /// # Errors
    /// 시작을 기다리는 작업이 아니면 `TaskNotFound`, 폴더를 다른 에이전트가 쥐고 있으면 `WriteConflict`.
    pub fn start_task(&mut self, task: TaskId, agent: AgentId) -> Result<(), QueueError> {
        let index = self
            .tasks
            .iter()
            .position(|slot| {
                slot.id == task
                    && slot.agent.is_none()
                    && matches!(slot.phase, TaskPhase::Pending | TaskPhase::Held)
            })
            .ok_or(QueueError::TaskNotFound(task))?;
        let slot = &self.tasks[index];
        if slot.phase == TaskPhase::Pending
            && slot.permission == Permission::Write
            && !self.gate.try_acquire(&slot.workdir, agent)
        {
            return Err(QueueError::WriteConflict(task));
        }
        let slot = &mut self.tasks[index];
        slot.agent = Some(agent);
        if slot.phase == TaskPhase::Held {
            return Ok(());
        }
        slot.phase = TaskPhase::Running;
        let chat = slot.chat;
        self.bump(chat);
        Ok(())
    }

    // cost: time O(t + h), heap O(1), stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 메인 에이전트는 쉬는 상태로, 보조 에이전트는 끝난 상태로 두고 보류된 작업은 그대로 둔다.
    pub fn finish_task(&mut self, agent: AgentId) {
        self.gate.release(agent);
        let Some(slot) = self
            .tasks
            .iter_mut()
            .rev()
            .find(|slot| slot.agent == Some(agent) && slot.phase == TaskPhase::Running)
        else {
            return;
        };
        slot.phase = if slot.is_main {
            TaskPhase::Idle
        } else {
            TaskPhase::Closed
        };
        let chat = slot.chat;
        self.bump(chat);
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 에이전트가 지금 실행 중인 작업의 접수 때 권한. 실행 중인 작업이 없으면 `None`.
    pub fn running_permission(&self, agent: AgentId) -> Option<Permission> {
        self.tasks
            .iter()
            .rev()
            .find(|slot| slot.agent == Some(agent) && slot.phase == TaskPhase::Running)
            .map(|slot| slot.permission)
    }

    #[cfg(test)]
    pub fn write_gate(&mut self) -> &mut WriteGate {
        &mut self.gate
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 모르는 작업이면 거짓. engine이 새 작업에 열 session의 역할을 정하는 데 쓴다.
    pub fn is_main_task(&self, task: TaskId) -> bool {
        self.tasks
            .iter()
            .any(|slot| slot.id == task && slot.is_main && slot.phase != TaskPhase::Closed)
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 시작하지 못한 새 작업을 닫아 쓰기 대기가 그 작업 때문에 막히지 않게 한다. 에이전트가 붙은 작업은 `finish_task`로 끝낸다.
    pub fn abandon_task(&mut self, task: TaskId) {
        let Some(slot) = self.tasks.iter_mut().find(|slot| {
            slot.id == task && slot.agent.is_none() && slot.phase == TaskPhase::Pending
        }) else {
            return;
        };
        slot.phase = TaskPhase::Closed;
        let chat = slot.chat;
        self.bump(chat);
    }

    // cost: time O(n + t + h), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 내줬지만 보내지 못한 입력을 보류한다. 입력이 붙은 작업은 보류로 두고 쓰기 잠금은 푼다.
    /// `Delivering`은 provider가 보내기 전에 거절해 보내지 않음이 확정된 입력이다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, 내준 `Queued` 입력이나 `Delivering` 입력이 아니면 `InvalidTransition`.
    pub fn hold_unsent(&mut self, input: InputId) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        let entry = &self.inputs[index];
        let is_handed_out = entry.input.state == InputState::Queued && entry.is_dispatched;
        if !is_handed_out && entry.input.state != InputState::Delivering {
            return Err(QueueError::InvalidTransition {
                from: entry.input.state,
                to: InputState::Held,
            });
        }
        let (task, chat) = (entry.input.task, entry.input.chat);
        if let Some(slot) = self.tasks.iter_mut().find(|slot| {
            Some(slot.id) == task && matches!(slot.phase, TaskPhase::Running | TaskPhase::Pending)
        }) {
            slot.phase = TaskPhase::Held;
            slot.was_interrupted = false;
            if let Some(agent) = slot.agent {
                self.gate.release(agent);
            }
        }
        let entry = &mut self.inputs[index];
        entry.input.state = InputState::Held;
        entry.input.reason = None;
        entry.is_dispatched = false;
        self.bump(chat);
        self.refresh_router_order(chat);
        Ok(())
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 보류되거나 끝난 메인은 메인으로 보지 않는다.
    pub(super) fn main_task(&self, chat: ChatId) -> Option<&TaskSlot> {
        self.tasks.iter().rev().find(|slot| {
            slot.chat == chat
                && slot.is_main
                && matches!(
                    slot.phase,
                    TaskPhase::Pending | TaskPhase::Running | TaskPhase::Idle
                )
        })
    }

    // cost: time O(t log t), heap O(t), stack O(1), alloc 1
    // vars: t = 작업 수
    // basis: estimate
    /// id 순서는 접수 순서와 같다.
    pub(super) fn held_tasks(&self, chat: ChatId, task: Option<TaskId>) -> Vec<TaskId> {
        let mut held: Vec<TaskId> = self
            .tasks
            .iter()
            .filter(|slot| slot.chat == chat && slot.phase == TaskPhase::Held)
            .filter(|slot| task.is_none_or(|id| slot.id == id))
            .map(|slot| slot.id)
            .collect();
        held.sort();
        held
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    fn reset_ignored_if_clear(&mut self, chat: ChatId) {
        let has_held = self
            .tasks
            .iter()
            .any(|slot| slot.chat == chat && slot.phase == TaskPhase::Held);
        if !has_held {
            self.chats.entry(chat).or_default().held_ignored = 0;
        }
    }
}
