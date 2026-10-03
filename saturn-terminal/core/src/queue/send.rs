use saturn_protocol::ids::{AgentId, ChatId, InputId, TaskId};
use saturn_protocol::state::{Disposition, InputState, QueueReason};

use super::gate::overlaps;
use super::{
    Entry, Permission, Queue, QueueError, QueuedInput, Route, SendAction, TaskPhase, TaskSlot,
};

impl Queue {
    // cost: time O(n·(t + h)), heap O(c), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수, c = 막힌 채팅 수
    // basis: estimate
    /// 끼워 넣기는 바로 보내고, 그 밖은 같은 채팅의 앞 입력이 기다리면 함께 기다린다.
    pub fn next_to_send(&mut self) -> Option<SendAction> {
        self.next_to_send_except(&[])
    }

    // cost: time O(n·(t + h)), heap O(c), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수, c = 막힌 채팅 수
    // basis: estimate
    /// `next_to_send`와 같되 `busy` 채팅의 입력은 내주지 않는다. 앞선 전달이 아직 끝나지 않은 채팅이라 순서를 지키려고
    /// 끼워 넣기도 기다린다. 다른 채팅의 입력은 막지 않는다.
    pub fn next_to_send_except(&mut self, busy: &[ChatId]) -> Option<SendAction> {
        let mut blocked_chats: Vec<ChatId> = Vec::new();
        for index in 0..self.inputs.len() {
            let entry = &self.inputs[index];
            if entry.input.state != InputState::Queued {
                continue;
            }
            let chat = entry.input.chat;
            if busy.contains(&chat) {
                continue;
            }
            if entry.is_dispatched {
                blocked_chats.push(chat);
                continue;
            }
            let route = self.route_for(entry);
            match route {
                Route::Steer { .. } => return self.commit(index, route),
                Route::Wait(reason) => {
                    let is_asking = entry.awaits_stop;
                    self.inputs[index].input.reason = if is_asking {
                        Some(QueueReason::ConfirmStop)
                    } else {
                        reason
                    };
                    blocked_chats.push(chat);
                }
                _ if blocked_chats.contains(&chat) => {}
                Route::NewTurn { .. } | Route::NewTask { .. } => {
                    return self.commit(index, route);
                }
            }
        }
        None
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 끼워 넣기 실측을 통과하지 않은 provider에 보낼 때 engine이 부른다. 충돌 입력이면 멈추고 실행할지 사용자에게
    /// 물어야 하므로 참을 돌려준다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Queued`가 아니면 `InvalidTransition`.
    pub fn defer_steer(&mut self, input: InputId) -> Result<bool, QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        if entry.input.state != InputState::Queued {
            return Err(QueueError::InvalidTransition {
                from: entry.input.state,
                to: InputState::Queued,
            });
        }
        if entry.disposition == Some(Disposition::Steer) {
            entry.disposition = Some(Disposition::Queue);
            entry.is_dispatched = false;
            entry.input.task = None;
            return Ok(self.ask_stop(index));
        }
        Ok(false)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 사용자가 대기 입력의 처리 방식을 바꾼다(바로 보내기는 `Steer`, 새 작업으로 보내기는 `NewTask`).
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Queued`가 아니면 `InvalidTransition`, 이미 내준 입력이면 `AlreadySent`.
    pub fn redirect(&mut self, input: InputId, disposition: Disposition) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        if entry.input.state != InputState::Queued {
            return Err(QueueError::InvalidTransition {
                from: entry.input.state,
                to: InputState::Queued,
            });
        }
        if entry.is_dispatched {
            return Err(QueueError::AlreadySent);
        }
        entry.disposition = Some(disposition);
        if disposition == Disposition::NewTask {
            entry.input.task = None;
        }
        let chat = entry.input.chat;
        self.bump(chat);
        Ok(())
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 사용자가 바로 보내기를 눌렀다. router를 거치지 않고 처리 방식을 끼워 넣기로 바꾸고, 같은 채팅의 대기 입력 중
    /// 맨 앞으로 옮긴다. 끼워 넣을 수 없는 상황이면 그 자리에서 다음 차례를 기다린다. 바꾸기 전 처리 방식을 돌려준다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Queued`가 아니면 `InvalidTransition`, 이미 내준 입력이면 `AlreadySent`.
    pub fn send_now(&mut self, input: InputId) -> Result<Option<Disposition>, QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        if entry.input.state != InputState::Queued {
            return Err(QueueError::InvalidTransition {
                from: entry.input.state,
                to: InputState::Queued,
            });
        }
        if entry.is_dispatched {
            return Err(QueueError::AlreadySent);
        }
        let previous = entry.disposition.replace(Disposition::Steer);
        entry.is_conflict = false;
        entry.awaits_stop = false;
        let chat = entry.input.chat;
        self.move_to_queue_front(index);
        self.bump(chat);
        Ok(previous)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// provider가 끼워 넣기를 거절했다(보내지 않음이 확정). 입력을 `Queued`로 되돌려 같은 채팅 대기열 맨 앞에 두고,
    /// 다시 끼워 넣지 않고 현재 작업이 끝난 뒤 다음 차례에 새 턴으로 가도록 처리 방식을 대기로 바꾼다. 충돌 입력이면
    /// 멈추고 실행할지 사용자에게 물어야 하므로 참을 돌려준다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Delivering`이 아니면 `InvalidTransition`.
    pub fn return_refused_steer(&mut self, input: InputId) -> Result<bool, QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        if entry.input.state != InputState::Delivering {
            return Err(QueueError::InvalidTransition {
                from: entry.input.state,
                to: InputState::Queued,
            });
        }
        entry.input.state = InputState::Queued;
        entry.input.reason = None;
        entry.input.task = None;
        entry.disposition = Some(Disposition::Queue);
        entry.is_dispatched = false;
        let chat = entry.input.chat;
        let index = self.move_to_queue_front(index);
        let is_asking = self.ask_stop(index);
        self.bump(chat);
        self.refresh_router_order(chat);
        Ok(is_asking)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 멈추고 실행할지 묻는 중인 입력이면 참.
    pub fn awaits_stop(&self, input: InputId) -> bool {
        self.inputs.iter().any(|entry| {
            entry.input.id == input
                && entry.awaits_stop
                && entry.input.state == InputState::Queued
                && !entry.is_dispatched
        })
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 사용자가 멈추지 않고 대기를 골랐다. 입력은 대기열 맨 앞에서 현재 작업이 끝나길 기다린다.
    ///
    /// # Errors
    /// 묻는 중이 아니면 `NotAwaitingStop`, 없는 입력이면 `NotFound`.
    pub fn keep_waiting(&mut self, input: InputId) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        if !self.awaits_stop(input) {
            return Err(QueueError::NotAwaitingStop(input));
        }
        let entry = &mut self.inputs[index];
        entry.awaits_stop = false;
        entry.input.reason = None;
        let chat = entry.input.chat;
        self.bump(chat);
        Ok(())
    }

    /// 충돌 입력을 대기로 돌렸으면 사용자에게 묻는 상태로 바꾸고 참을 돌려준다. 한 번만 묻는다.
    fn ask_stop(&mut self, index: usize) -> bool {
        let entry = &mut self.inputs[index];
        if !entry.is_conflict {
            return false;
        }
        entry.is_conflict = false;
        entry.awaits_stop = true;
        entry.input.reason = Some(QueueReason::ConfirmStop);
        true
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 입력을 같은 채팅의 가장 앞 대기 입력 앞으로 옮긴다. 같은 채팅의 다른 대기 입력이 없으면 제자리다. 옮긴 뒤의 위치를 돌려준다.
    fn move_to_queue_front(&mut self, index: usize) -> usize {
        let chat = self.inputs[index].input.chat;
        let Some(entry) = self.inputs.remove(index) else {
            return index;
        };
        let at = self
            .inputs
            .iter()
            .position(|other| other.input.chat == chat && other.input.state == InputState::Queued)
            .unwrap_or(index.min(self.inputs.len()));
        self.inputs.insert(at, entry);
        at
    }

    // cost: time O(t + h), heap O(1), stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 붙은 작업이 있으면 그 작업, 없으면 메인 작업이 대상이다.
    fn route_for(&self, entry: &Entry) -> Route {
        let input = &entry.input;
        let disposition = entry.disposition.unwrap_or(Disposition::Queue);
        // 새 작업 id는 그 작업을 시작한 입력 id와 같다.
        let new_task = Route::NewTask {
            task: TaskId(input.id.0),
            is_existing: false,
        };
        if disposition == Disposition::NewTask && input.task.is_none() {
            return self.check_write(input, None, new_task);
        }
        let target = match input.task {
            Some(id) => self
                .tasks
                .iter()
                .find(|slot| slot.id == id && slot.phase != TaskPhase::Closed),
            None => self.main_task(input.chat),
        };
        let Some(slot) = target else {
            return self.check_write(input, None, new_task);
        };
        match (slot.phase, slot.agent) {
            (TaskPhase::Running, Some(agent)) if disposition == Disposition::Steer => {
                // 쓰기 잠금 없이 도는 읽기 전용 실행에 쓰기 입력을 끼워 넣으면 그 입력이 잠금 없이 쓴다.
                if input.permission == Permission::Write && slot.permission == Permission::ReadOnly
                {
                    return Route::Wait(Some(QueueReason::WriteTurn));
                }
                Route::Steer {
                    agent,
                    task: slot.id,
                }
            }
            (TaskPhase::Idle, Some(agent)) => {
                let route = Route::NewTurn {
                    agent,
                    task: slot.id,
                };
                self.check_write(input, Some(agent), route)
            }
            (TaskPhase::Idle, None) => {
                let route = Route::NewTask {
                    task: slot.id,
                    is_existing: true,
                };
                self.check_write(input, None, route)
            }
            _ => Route::Wait(None),
        }
    }

    // cost: time O(t + h), heap O(1), stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 쓰기 범위가 겹치는 다른 쓰는 에이전트나 시작을 기다리는 쓰기 작업이 있으면 기다린다.
    fn check_write(&self, input: &QueuedInput, agent: Option<AgentId>, route: Route) -> Route {
        if input.permission == Permission::ReadOnly {
            return route;
        }
        let is_pending_writer = self.tasks.iter().any(|slot| {
            slot.phase == TaskPhase::Pending
                && slot.permission == Permission::Write
                && overlaps(&slot.write_scope, &input.write_scope)
        });
        if is_pending_writer || self.gate.is_held_by_other(&input.write_scope, agent) {
            return Route::Wait(Some(QueueReason::WriteTurn));
        }
        route
    }

    // cost: time O(t + h), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    fn commit(&mut self, index: usize, route: Route) -> Option<SendAction> {
        let input = self.inputs[index].input.clone();
        let action = match route {
            Route::Steer { agent, task } => {
                self.inputs[index].input.task = Some(task);
                SendAction::Steer {
                    input: input.id,
                    agent,
                }
            }
            Route::NewTurn { agent, task } => {
                if !self.run_turn(task, agent, &input) {
                    return None;
                }
                self.inputs[index].input.task = Some(task);
                SendAction::NewTurn {
                    input: input.id,
                    agent,
                }
            }
            Route::NewTask { task, is_existing } => {
                if !self.reserve_task(task, is_existing, &input) {
                    return None;
                }
                self.inputs[index].input.task = Some(task);
                SendAction::NewTask {
                    input: input.id,
                    task,
                }
            }
            Route::Wait(_) => return None,
        };
        self.inputs[index].is_dispatched = true;
        self.inputs[index].input.reason = None;
        self.inputs[index].awaits_stop = false;
        Some(action)
    }

    // cost: time O(t + h), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    fn run_turn(&mut self, task: TaskId, agent: AgentId, input: &QueuedInput) -> bool {
        let Some(index) = self.tasks.iter().position(|slot| slot.id == task) else {
            return false;
        };
        if input.permission == Permission::Write
            && !self.gate.try_acquire(&input.write_scope, agent)
        {
            return false;
        }
        let slot = &mut self.tasks[index];
        slot.phase = TaskPhase::Running;
        slot.permission = input.permission;
        slot.write_scope.clone_from(&input.write_scope);
        self.bump(input.chat);
        true
    }

    // cost: time O(t), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    fn reserve_task(&mut self, task: TaskId, is_existing: bool, input: &QueuedInput) -> bool {
        if is_existing {
            let Some(slot) = self.tasks.iter_mut().find(|slot| slot.id == task) else {
                return false;
            };
            slot.phase = TaskPhase::Pending;
            slot.permission = input.permission;
            slot.write_scope.clone_from(&input.write_scope);
        } else {
            let is_main = self.main_task(input.chat).is_none();
            self.tasks.push(TaskSlot {
                id: task,
                chat: input.chat,
                agent: None,
                permission: input.permission,
                write_scope: input.write_scope.clone(),
                is_main,
                was_interrupted: false,
                phase: TaskPhase::Pending,
            });
        }
        self.bump(input.chat);
        true
    }

    // cost: time O(t), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 새 작업 입력은 자기 작업, 그 밖은 메인 작업(없으면 이번 멈춤에서 처음 만든 작업)에 붙인다.
    pub(super) fn hold_target(
        &mut self,
        index: usize,
        fallback_main: &mut Option<TaskId>,
    ) -> TaskId {
        let entry = &self.inputs[index];
        let input = entry.input.clone();
        let is_new_task = entry.disposition == Some(Disposition::NewTask);
        let bound = input.task.filter(|id| {
            self.tasks
                .iter()
                .any(|slot| slot.id == *id && slot.phase != TaskPhase::Closed)
        });
        let task = match (bound, is_new_task) {
            (Some(task), _) => task,
            (None, true) => TaskId(input.id.0),
            (None, false) => *fallback_main.get_or_insert(TaskId(input.id.0)),
        };
        if let Some(slot) = self
            .tasks
            .iter_mut()
            .find(|slot| slot.id == task && slot.phase != TaskPhase::Closed)
        {
            slot.phase = TaskPhase::Held;
            return task;
        }
        self.tasks.push(TaskSlot {
            id: task,
            chat: input.chat,
            agent: None,
            permission: input.permission,
            write_scope: input.write_scope,
            is_main: !is_new_task,
            was_interrupted: false,
            phase: TaskPhase::Held,
        });
        task
    }
}
