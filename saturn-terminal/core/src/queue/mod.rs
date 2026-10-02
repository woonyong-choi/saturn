//! 채팅 대기열: 입력 접수, 판단 적용, 전송 순서, 멈춤과 재개, 쓰기 규칙.
//! 설계: docs/design/input-handling.md

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use saturn_protocol::ids::{AgentId, ChatId, ChatRevision, InputId, SettingsRevision, TaskId};
use saturn_protocol::state::{Disposition, InputState, QueueReason};

use crate::routers::RouteDecision;

/// 한 채팅에서 재개 뜻이 없는 새 입력이 이만큼 쌓이면 보류를 종료한다.
pub const HELD_IGNORE_LIMIT: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("input not found: {0:?}")]
    NotFound(InputId),
    /// 취소는 보내기 전 입력에만 적용한다.
    #[error("input already sent")]
    AlreadySent,
    /// 한 번 다시 판단하고 또 바뀌면 대기로 둔다.
    #[error("chat revision changed since judgment")]
    RevisionConflict,
    #[error("invalid input state transition: {from:?} -> {to:?}")]
    InvalidTransition { from: InputState, to: InputState },
    /// 시작을 기다리는 작업이 아니다.
    #[error("task not found: {0:?}")]
    TaskNotFound(TaskId),
    #[error("write task workdir is already held: {0:?}")]
    WriteConflict(TaskId),
}

/// 접수 때 고정해 쓰기 규칙을 결정론으로 판정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// 같은 폴더에서 병렬 실행.
    ReadOnly,
    /// 같은 작업 폴더에서 한 번에 하나.
    Write,
}

#[derive(Debug, Clone)]
pub struct QueuedInput {
    pub id: InputId,
    pub chat: ChatId,
    pub text: String,
    /// 접수 때 고정한다.
    pub settings: SettingsRevision,
    /// 접수 때 고정한다.
    pub permission: Permission,
    pub workdir: PathBuf,
    /// 사용자가 고정한 모델이나 router가 고른 모델. 있으면 그 모델로 보내고 router 호출에서 모델 질문을 뺀다.
    pub pinned_model: Option<String>,
    /// 관계 판단 없이 대기하고, 보낼 때 router를 한 번 부른다.
    pub skip_relation: bool,
    pub state: InputState,
    pub reason: Option<QueueReason>,
    /// 접수 때 정해져 있으면 그 작업으로만 보낸다.
    pub task: Option<TaskId>,
}

/// `Steer`가 활성 턴 없음으로 실패하면 다시 판단하지 않고 `NewTurn`으로 보낸다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendAction {
    Steer { input: InputId, agent: AgentId },
    NewTurn { input: InputId, agent: AgentId },
    NewTask { input: InputId, task: TaskId },
}

/// 트리가 유휴일 때 풀어 subagent가 쓰는 중에 다음 쓰기가 시작되지 않게 한다.
#[derive(Debug, Default)]
pub struct WriteGate {
    holders: Vec<(PathBuf, AgentId)>,
}

impl WriteGate {
    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    /// 다른 에이전트가 같은 폴더를 쥐고 있으면 거짓, 자기가 이미 쥐고 있으면 참.
    pub fn try_acquire(&mut self, workdir: &Path, agent: AgentId) -> bool {
        if self.is_held_by_other(workdir, Some(agent)) {
            return false;
        }
        let is_held_by_self = self
            .holders
            .iter()
            .any(|(path, holder)| path == workdir && *holder == agent);
        if !is_held_by_self {
            self.holders.push((workdir.to_path_buf(), agent));
        }
        true
    }

    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    pub fn release(&mut self, agent: AgentId) {
        self.holders.retain(|(_, holder)| *holder != agent);
    }

    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    fn is_held_by_other(&self, workdir: &Path, agent: Option<AgentId>) -> bool {
        self.holders
            .iter()
            .any(|(path, holder)| path == workdir && Some(*holder) != agent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskPhase {
    Pending,
    Running,
    Idle,
    Held,
    Closed,
}

#[derive(Debug, Clone)]
struct TaskSlot {
    id: TaskId,
    chat: ChatId,
    agent: Option<AgentId>,
    permission: Permission,
    workdir: PathBuf,
    is_main: bool,
    was_interrupted: bool,
    phase: TaskPhase,
}

#[derive(Debug, Clone)]
struct Entry {
    input: QueuedInput,
    disposition: Option<Disposition>,
    /// 내줬지만 아직 `Delivering`이 아닌 입력으로, 두 번 내주지 않는다.
    is_dispatched: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct ChatState {
    revision: u64,
    held_ignored: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Steer { agent: AgentId, task: TaskId },
    NewTurn { agent: AgentId, task: TaskId },
    NewTask { task: TaskId, is_existing: bool },
    Wait(Option<QueueReason>),
}

/// 대기열은 session이 아니라 채팅 단위로 둔다.
#[derive(Debug, Default)]
pub struct Queue {
    inputs: VecDeque<Entry>,
    tasks: Vec<TaskSlot>,
    chats: HashMap<ChatId, ChatState>,
    gate: WriteGate,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 작업 상태, 대기열 맨 앞, 마지막 판단이 바뀌면 오른다.
    pub fn revision(&self, chat: ChatId) -> ChatRevision {
        ChatRevision(self.chats.get(&chat).map_or(0, |state| state.revision))
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    pub fn input(&self, input: InputId) -> Option<&QueuedInput> {
        self.inputs
            .iter()
            .find(|entry| entry.input.id == input)
            .map(|entry| &entry.input)
    }

    // cost: time O(n), heap O(1) amortized, stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 기록 저장소에 접수(ACK)된 뒤에만 부른다.
    pub fn accept(&mut self, input: QueuedInput) {
        let chat = input.chat;
        self.chats.entry(chat).or_default();
        self.inputs.push_back(Entry {
            input: QueuedInput {
                state: InputState::Judging,
                reason: None,
                ..input
            },
            disposition: None,
            is_dispatched: false,
        });
        self.refresh_router_order(chat);
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 같은 채팅 입력은 접수 순서대로 하나씩 판단한다.
    pub fn next_to_route(&self, chat: ChatId) -> Option<(InputId, ChatRevision)> {
        self.inputs
            .iter()
            .find(|entry| entry.input.chat == chat && entry.input.state == InputState::Judging)
            .map(|entry| (entry.input.id, self.revision(chat)))
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// # Errors
    /// revision이 다르면 `RevisionConflict`, 없는 입력이면 `NotFound`, `Judging`이 아니면 `InvalidTransition`.
    pub fn apply(
        &mut self,
        input: InputId,
        decision: &RouteDecision,
        current: ChatRevision,
    ) -> Result<Disposition, QueueError> {
        let index = self.index_of(input)?;
        let from = self.inputs[index].input.state;
        if from != InputState::Judging {
            return Err(QueueError::InvalidTransition {
                from,
                to: InputState::Queued,
            });
        }
        if decision.revision != current {
            return Err(QueueError::RevisionConflict);
        }
        let entry = &mut self.inputs[index];
        entry.input.state = InputState::Queued;
        entry.input.reason = None;
        entry.disposition = Some(decision.disposition);
        // 모델 선택은 새 작업에만 쓴다. 이어 가기(대기)와 끼워 넣기는 현재 모델을 유지한다
        if decision.disposition == Disposition::NewTask && decision.model.is_some() {
            entry.input.pinned_model.clone_from(&decision.model);
        }
        let chat = entry.input.chat;
        self.bump(chat);
        self.refresh_router_order(chat);
        Ok(decision.disposition)
    }

    // cost: time O(n·(t + h)), heap O(c), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수, c = 막힌 채팅 수
    // basis: estimate
    /// 끼워 넣기는 바로 보내고, 그 밖은 같은 채팅의 앞 입력이 기다리면 함께 기다린다.
    pub fn next_to_send(&mut self) -> Option<SendAction> {
        let mut blocked_chats: Vec<ChatId> = Vec::new();
        for index in 0..self.inputs.len() {
            let entry = &self.inputs[index];
            if entry.input.state != InputState::Queued {
                continue;
            }
            let chat = entry.input.chat;
            if entry.is_dispatched {
                blocked_chats.push(chat);
                continue;
            }
            let route = self.route_for(entry);
            match route {
                Route::Steer { .. } => return self.commit(index, route),
                Route::Wait(reason) => {
                    self.inputs[index].input.reason = reason;
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
    /// 끼워 넣기 실측을 통과하지 않은 provider에 보낼 때 engine이 부른다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Queued`가 아니면 `InvalidTransition`.
    pub fn defer_steer(&mut self, input: InputId) -> Result<(), QueueError> {
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
        }
        Ok(())
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// # Errors
    /// 없는 입력이면 `NotFound`, 상태표에 없는 전이면 `InvalidTransition`.
    pub fn set_state(&mut self, input: InputId, state: InputState) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        let from = entry.input.state;
        if !can_move(from, state) {
            return Err(QueueError::InvalidTransition { from, to: state });
        }
        entry.input.state = state;
        entry.is_dispatched = false;
        if state != InputState::Queued {
            entry.input.reason = None;
        }
        let chat = entry.input.chat;
        if from == InputState::Queued || state == InputState::Queued {
            self.bump(chat);
        }
        self.refresh_router_order(chat);
        Ok(())
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 이미 취소한 입력이면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 이미 보냈거나 보내는 중이면 `AlreadySent`, 없는 입력이면 `NotFound`.
    pub fn cancel(&mut self, input: InputId) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        let entry = &mut self.inputs[index];
        let from = entry.input.state;
        match from {
            InputState::Cancelled => return Ok(()),
            InputState::Delivering | InputState::Applied | InputState::Rejected => {
                return Err(QueueError::AlreadySent);
            }
            InputState::Judging | InputState::Queued | InputState::Held => {}
        }
        if entry.is_dispatched {
            return Err(QueueError::AlreadySent);
        }
        entry.input.state = InputState::Cancelled;
        entry.input.reason = None;
        let chat = entry.input.chat;
        if from == InputState::Queued {
            self.bump(chat);
        }
        self.refresh_router_order(chat);
        Ok(())
    }

    // cost: time O(t + n·t), heap O(t), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, t = 작업 수
    // basis: estimate
    /// 채팅의 보류 작업 전부를 id 순서로 돌려준다.
    /// TODO(#36): 뒤집는 입력을 바로 멈출지
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
        let chat = entry.input.chat;
        self.move_to_queue_front(index);
        self.bump(chat);
        Ok(previous)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// provider가 끼워 넣기를 거절했다(보내지 않음이 확정). 입력을 `Queued`로 되돌려 같은 채팅 대기열 맨 앞에 두고,
    /// 다시 끼워 넣지 않고 현재 작업이 끝난 뒤 다음 차례에 새 턴으로 가도록 처리 방식을 대기로 바꾼다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, `Delivering`이 아니면 `InvalidTransition`.
    pub fn return_refused_steer(&mut self, input: InputId) -> Result<(), QueueError> {
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
        self.move_to_queue_front(index);
        self.bump(chat);
        self.refresh_router_order(chat);
        Ok(())
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 입력을 같은 채팅의 가장 앞 대기 입력 앞으로 옮긴다. 같은 채팅의 다른 대기 입력이 없으면 제자리다.
    fn move_to_queue_front(&mut self, index: usize) {
        let chat = self.inputs[index].input.chat;
        let Some(entry) = self.inputs.remove(index) else {
            return;
        };
        let at = self
            .inputs
            .iter()
            .position(|other| other.input.chat == chat && other.input.state == InputState::Queued)
            .unwrap_or(index.min(self.inputs.len()));
        self.inputs.insert(at, entry);
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 입력에 붙은 처리 방식. 판단 전이면 `None`.
    pub fn disposition(&self, input: InputId) -> Option<Disposition> {
        self.inputs
            .iter()
            .find(|entry| entry.input.id == input)
            .and_then(|entry| entry.disposition)
    }

    // cost: time O(n), heap O(k), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, k = 그 상태의 입력 수
    // basis: estimate
    /// 접수 순서로 돌려준다.
    pub fn inputs_in_state(&self, chat: ChatId, state: InputState) -> Vec<InputId> {
        self.inputs
            .iter()
            .filter(|entry| entry.input.chat == chat && entry.input.state == state)
            .map(|entry| entry.input.id)
            .collect()
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 보낼 차례를 기다리는 입력이 있으면 참. 다음 턴 경계에 합쳐질 입력이라 맥락 정리 판정을 미루는 데 쓴다.
    pub fn has_waiting(&self, chat: ChatId) -> bool {
        self.inputs
            .iter()
            .any(|entry| entry.input.chat == chat && entry.input.state == InputState::Queued)
    }

    // cost: time O(n + t + h), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 내줬지만 보내지 못한 입력을 보류한다. 입력이 붙은 작업은 보류로 두고 쓰기 잠금은 푼다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, 내준 `Queued` 입력이 아니면 `InvalidTransition`.
    pub fn hold_unsent(&mut self, input: InputId) -> Result<(), QueueError> {
        let index = self.index_of(input)?;
        let entry = &self.inputs[index];
        if entry.input.state != InputState::Queued || !entry.is_dispatched {
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

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    fn index_of(&self, input: InputId) -> Result<usize, QueueError> {
        self.inputs
            .iter()
            .position(|entry| entry.input.id == input)
            .ok_or(QueueError::NotFound(input))
    }

    fn bump(&mut self, chat: ChatId) {
        self.chats.entry(chat).or_default().revision += 1;
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 판단 대기 입력 중 맨 앞만 판단 중이고 나머지는 `RouterOrder` 대기다.
    fn refresh_router_order(&mut self, chat: ChatId) {
        let mut is_first = true;
        for entry in &mut self.inputs {
            if entry.input.chat != chat || entry.input.state != InputState::Judging {
                continue;
            }
            entry.input.reason = if is_first {
                None
            } else {
                Some(QueueReason::RouterOrder)
            };
            is_first = false;
        }
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 보류되거나 끝난 메인은 메인으로 보지 않는다.
    fn main_task(&self, chat: ChatId) -> Option<&TaskSlot> {
        self.tasks.iter().rev().find(|slot| {
            slot.chat == chat
                && slot.is_main
                && matches!(
                    slot.phase,
                    TaskPhase::Pending | TaskPhase::Running | TaskPhase::Idle
                )
        })
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
    /// 같은 폴더에 다른 쓰는 에이전트나 시작을 기다리는 쓰기 작업이 있으면 기다린다.
    fn check_write(&self, input: &QueuedInput, agent: Option<AgentId>, route: Route) -> Route {
        if input.permission == Permission::ReadOnly {
            return route;
        }
        let is_pending_writer = self.tasks.iter().any(|slot| {
            slot.phase == TaskPhase::Pending
                && slot.permission == Permission::Write
                && slot.workdir == input.workdir
        });
        if is_pending_writer || self.gate.is_held_by_other(&input.workdir, agent) {
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
        Some(action)
    }

    // cost: time O(t + h), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    fn run_turn(&mut self, task: TaskId, agent: AgentId, input: &QueuedInput) -> bool {
        let Some(index) = self.tasks.iter().position(|slot| slot.id == task) else {
            return false;
        };
        if input.permission == Permission::Write && !self.gate.try_acquire(&input.workdir, agent) {
            return false;
        }
        let slot = &mut self.tasks[index];
        slot.phase = TaskPhase::Running;
        slot.permission = input.permission;
        slot.workdir.clone_from(&input.workdir);
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
            slot.workdir.clone_from(&input.workdir);
        } else {
            let is_main = self.main_task(input.chat).is_none();
            self.tasks.push(TaskSlot {
                id: task,
                chat: input.chat,
                agent: None,
                permission: input.permission,
                workdir: input.workdir.clone(),
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
    fn hold_target(&mut self, index: usize, fallback_main: &mut Option<TaskId>) -> TaskId {
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
            workdir: input.workdir,
            is_main: !is_new_task,
            was_interrupted: false,
            phase: TaskPhase::Held,
        });
        task
    }

    // cost: time O(t log t), heap O(t), stack O(1), alloc 1
    // vars: t = 작업 수
    // basis: estimate
    /// id 순서는 접수 순서와 같다.
    fn held_tasks(&self, chat: ChatId, task: Option<TaskId>) -> Vec<TaskId> {
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

fn can_move(from: InputState, to: InputState) -> bool {
    use InputState::{Applied, Cancelled, Delivering, Held, Judging, Queued, Rejected};
    matches!(
        (from, to),
        (Judging, Queued | Delivering | Held | Cancelled)
            | (Queued, Delivering | Held | Cancelled)
            | (Delivering, Applied | Rejected)
            | (Held, Queued | Cancelled)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAT: ChatId = ChatId(1);

    fn input(id: u64, permission: Permission) -> QueuedInput {
        QueuedInput {
            id: InputId(id),
            chat: CHAT,
            text: format!("input {id}"),
            settings: SettingsRevision(1),
            permission,
            workdir: PathBuf::from("/work"),
            pinned_model: None,
            skip_relation: false,
            state: InputState::Queued,
            reason: None,
            task: None,
        }
    }

    fn decision(revision: ChatRevision, disposition: Disposition) -> RouteDecision {
        RouteDecision {
            revision,
            settings: SettingsRevision(1),
            disposition,
            keep_current: true,
            model: None,
            resume_held: false,
            fallbacks: Vec::new(),
        }
    }

    fn state_of(queue: &Queue, id: u64) -> InputState {
        queue.input(InputId(id)).expect("input should exist").state
    }

    fn accept_routed(queue: &mut Queue, id: u64, permission: Permission, disposition: Disposition) {
        queue.accept(input(id, permission));
        let revision = queue.revision(CHAT);
        queue
            .apply(InputId(id), &decision(revision, disposition), revision)
            .expect("apply should succeed");
    }

    fn start_running(queue: &mut Queue, id: u64, permission: Permission, agent: u64) -> TaskId {
        accept_routed(queue, id, permission, Disposition::NewTask);
        let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
            panic!("input should start a new task");
        };
        queue
            .set_state(InputId(id), InputState::Delivering)
            .unwrap();
        queue.start_task(task, AgentId(agent)).unwrap();
        task
    }

    #[test]
    fn accept_sets_judging_and_later_inputs_wait_for_router_order() {
        let mut queue = Queue::new();

        queue.accept(input(1, Permission::ReadOnly));
        queue.accept(input(2, Permission::ReadOnly));

        assert_eq!(state_of(&queue, 1), InputState::Judging);
        assert_eq!(queue.input(InputId(1)).unwrap().reason, None);
        assert_eq!(
            queue.input(InputId(2)).unwrap().reason,
            Some(QueueReason::RouterOrder)
        );
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 입력 수
    // basis: estimate
    #[test]
    fn next_to_route_follows_ack_order_one_at_a_time() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        queue.accept(input(2, Permission::ReadOnly));

        let first = queue.next_to_route(CHAT).map(|(id, _)| id);
        let revision = queue.revision(CHAT);
        queue
            .apply(
                InputId(1),
                &decision(revision, Disposition::Queue),
                revision,
            )
            .unwrap();
        let second = queue.next_to_route(CHAT).map(|(id, _)| id);

        assert_eq!(first, Some(InputId(1)));
        assert_eq!(second, Some(InputId(2)));
    }

    #[test]
    fn next_to_route_other_chat_returns_none() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));

        assert_eq!(queue.next_to_route(ChatId(9)), None);
    }

    #[test]
    fn apply_changed_revision_returns_conflict() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        let (_, routed_at) = queue.next_to_route(CHAT).unwrap();
        start_running(&mut queue, 2, Permission::ReadOnly, 7);

        let result = queue.apply(
            InputId(1),
            &decision(routed_at, Disposition::Steer),
            queue.revision(CHAT),
        );

        assert!(matches!(result, Err(QueueError::RevisionConflict)));
        assert_eq!(state_of(&queue, 1), InputState::Judging);
    }

    #[test]
    fn apply_second_conflict_can_fall_back_to_queue() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        let stale = decision(ChatRevision(99), Disposition::Steer);
        let current = queue.revision(CHAT);
        assert!(queue.apply(InputId(1), &stale, current).is_err());
        assert!(queue.apply(InputId(1), &stale, current).is_err());

        queue.set_state(InputId(1), InputState::Queued).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Queued);
    }

    #[test]
    fn apply_unknown_input_returns_not_found() {
        let mut queue = Queue::new();

        let result = queue.apply(
            InputId(5),
            &decision(ChatRevision(0), Disposition::Queue),
            ChatRevision(0),
        );

        assert!(matches!(result, Err(QueueError::NotFound(InputId(5)))));
    }

    #[test]
    fn apply_bumps_revision() {
        let mut queue = Queue::new();
        let before = queue.revision(CHAT);

        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        assert!(queue.revision(CHAT) > before);
    }

    #[test]
    fn next_to_send_first_input_starts_main_task() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);

        let action = queue.next_to_send();

        assert_eq!(
            action,
            Some(SendAction::NewTask {
                input: InputId(1),
                task: TaskId(1)
            })
        );
    }

    #[test]
    fn next_to_send_does_not_dispatch_twice() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        let first = queue.next_to_send();
        let second = queue.next_to_send();

        assert!(first.is_some());
        assert_eq!(second, None);
    }

    #[test]
    fn next_to_send_waits_for_dispatched_input_in_same_chat() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::NewTask);
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);

        assert!(queue.next_to_send().is_some());

        assert_eq!(queue.next_to_send(), None);
    }

    #[test]
    fn next_to_send_steer_goes_to_running_agent() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Steer);

        let action = queue.next_to_send();

        assert_eq!(
            action,
            Some(SendAction::Steer {
                input: InputId(2),
                agent: AgentId(7)
            })
        );
    }

    #[test]
    fn next_to_send_queue_waits_for_main_then_new_turn() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        assert_eq!(queue.next_to_send(), None);

        queue.finish_task(AgentId(7));
        let action = queue.next_to_send();

        assert_eq!(
            action,
            Some(SendAction::NewTurn {
                input: InputId(2),
                agent: AgentId(7)
            })
        );
    }

    #[test]
    fn next_to_send_second_writer_waits_for_tree_idle() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);

        let blocked = queue.next_to_send();
        let reason = queue.input(InputId(2)).unwrap().reason;
        queue.finish_task(AgentId(7));
        let released = queue.next_to_send();

        assert_eq!(blocked, None);
        assert_eq!(reason, Some(QueueReason::WriteTurn));
        assert_eq!(
            released,
            Some(SendAction::NewTask {
                input: InputId(2),
                task: TaskId(2)
            })
        );
    }

    #[test]
    fn next_to_send_pending_writer_blocks_other_writer() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
        assert!(queue.next_to_send().is_some());

        let second = queue.next_to_send();

        assert_eq!(second, None);
    }

    #[test]
    fn next_to_send_read_only_runs_beside_writer() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);

        let action = queue.next_to_send();

        assert_eq!(
            action,
            Some(SendAction::NewTask {
                input: InputId(2),
                task: TaskId(2)
            })
        );
    }

    #[test]
    fn next_to_send_keeps_order_within_chat() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::NewTask);

        let action = queue.next_to_send();

        assert_eq!(action, None);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 입력 수
    // basis: estimate
    #[test]
    fn finish_task_closes_auxiliary_agent() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::ReadOnly, 7);
        let aux = start_running(&mut queue, 2, Permission::ReadOnly, 8);

        queue.finish_task(AgentId(8));

        let slot = queue.tasks.iter().find(|slot| slot.id == aux).unwrap();
        assert_eq!(slot.phase, TaskPhase::Closed);
    }

    #[test]
    fn defer_steer_turns_steer_into_queue() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Steer);
        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::Steer { .. })
        ));

        queue.defer_steer(InputId(2)).unwrap();

        assert_eq!(queue.next_to_send(), None);
    }

    #[test]
    fn send_now_steers_a_running_task_ahead_of_earlier_waiting_inputs() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        accept_routed(&mut queue, 3, Permission::Write, Disposition::Queue);

        let previous = queue.send_now(InputId(3)).unwrap();

        assert_eq!(previous, Some(Disposition::Queue));
        assert_eq!(
            queue.inputs_in_state(CHAT, InputState::Queued),
            vec![InputId(3), InputId(2)]
        );
        assert_eq!(
            queue.next_to_send(),
            Some(SendAction::Steer {
                input: InputId(3),
                agent: AgentId(7)
            })
        );
    }

    #[test]
    fn send_now_that_cannot_steer_waits_first_in_line() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        accept_routed(&mut queue, 3, Permission::Write, Disposition::Queue);
        queue.send_now(InputId(3)).unwrap();
        let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
            panic!("input should steer");
        };
        queue.defer_steer(input).unwrap();

        queue.finish_task(AgentId(7));

        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::NewTurn {
                input: InputId(3),
                ..
            })
        ));
    }

    #[test]
    fn send_now_refuses_inputs_that_are_not_waiting() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::Write));
        accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
        queue.next_to_send();

        assert!(matches!(
            queue.send_now(InputId(1)),
            Err(QueueError::InvalidTransition { .. })
        ));
        assert!(matches!(
            queue.send_now(InputId(2)),
            Err(QueueError::AlreadySent)
        ));
        assert!(matches!(
            queue.send_now(InputId(9)),
            Err(QueueError::NotFound(InputId(9)))
        ));
    }

    #[test]
    fn refused_steer_returns_to_the_front_as_a_queued_input() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        accept_routed(&mut queue, 3, Permission::Write, Disposition::Steer);
        let Some(SendAction::Steer { input, .. }) = queue.next_to_send() else {
            panic!("input should steer");
        };
        queue.set_state(input, InputState::Delivering).unwrap();

        queue.return_refused_steer(input).unwrap();

        assert_eq!(state_of(&queue, 3), InputState::Queued);
        assert_eq!(queue.disposition(InputId(3)), Some(Disposition::Queue));
        assert_eq!(
            queue.inputs_in_state(CHAT, InputState::Queued),
            vec![InputId(3), InputId(2)]
        );
        assert_eq!(queue.next_to_send(), None);
        queue.finish_task(AgentId(7));
        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::NewTurn {
                input: InputId(3),
                ..
            })
        ));
    }

    #[test]
    fn refused_steer_needs_a_delivering_input() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);

        assert!(matches!(
            queue.return_refused_steer(InputId(1)),
            Err(QueueError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn set_state_follows_transition_table() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        queue.set_state(InputId(1), InputState::Delivering).unwrap();
        queue.set_state(InputId(1), InputState::Applied).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Applied);
    }

    #[test]
    fn set_state_outside_table_returns_error() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
        queue.set_state(InputId(1), InputState::Delivering).unwrap();

        let result = queue.set_state(InputId(1), InputState::Queued);

        assert!(matches!(
            result,
            Err(QueueError::InvalidTransition {
                from: InputState::Delivering,
                to: InputState::Queued
            })
        ));
    }

    #[test]
    fn set_state_unknown_input_returns_not_found() {
        let mut queue = Queue::new();

        let result = queue.set_state(InputId(3), InputState::Queued);

        assert!(matches!(result, Err(QueueError::NotFound(InputId(3)))));
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 입력 수
    // basis: estimate
    #[test]
    fn can_move_terminal_states_have_no_next() {
        let all = [
            InputState::Judging,
            InputState::Queued,
            InputState::Delivering,
            InputState::Applied,
            InputState::Rejected,
            InputState::Held,
            InputState::Cancelled,
        ];

        for terminal in [
            InputState::Applied,
            InputState::Rejected,
            InputState::Cancelled,
        ] {
            assert!(all.iter().all(|to| !can_move(terminal, *to)));
        }
    }

    #[test]
    fn cancel_queued_input_cancels() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        queue.cancel(InputId(1)).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Cancelled);
    }

    #[test]
    fn cancel_delivering_or_applied_returns_already_sent() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);
        queue.set_state(InputId(1), InputState::Delivering).unwrap();
        queue.set_state(InputId(2), InputState::Delivering).unwrap();
        queue.set_state(InputId(2), InputState::Applied).unwrap();

        let delivering = queue.cancel(InputId(1));
        let applied = queue.cancel(InputId(2));

        assert!(matches!(delivering, Err(QueueError::AlreadySent)));
        assert!(matches!(applied, Err(QueueError::AlreadySent)));
    }

    #[test]
    fn cancel_dispatched_input_returns_already_sent() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
        assert!(queue.next_to_send().is_some());

        let result = queue.cancel(InputId(1));

        assert!(matches!(result, Err(QueueError::AlreadySent)));
    }

    #[test]
    fn stop_holds_running_task_and_unsent_inputs() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        queue.accept(input(3, Permission::Write));

        let held = queue.stop(CHAT);

        assert_eq!(held, vec![task]);
        assert_eq!(state_of(&queue, 1), InputState::Delivering);
        assert_eq!(state_of(&queue, 2), InputState::Held);
        assert_eq!(state_of(&queue, 3), InputState::Held);
        assert_eq!(queue.input(InputId(2)).unwrap().task, Some(task));
    }

    #[test]
    fn stop_held_task_is_not_sent_without_resume() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        queue.stop(CHAT);

        queue.finish_task(AgentId(7));

        assert_eq!(queue.next_to_send(), None);
    }

    #[test]
    fn stop_without_task_holds_input_in_own_task() {
        let mut queue = Queue::new();
        queue.accept(input(4, Permission::Write));

        let held = queue.stop(CHAT);

        assert_eq!(held, vec![TaskId(4)]);
    }

    #[test]
    fn resume_all_requeues_in_ack_order_and_returns_interrupted() {
        let mut queue = Queue::new();
        let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
        let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);
        accept_routed(&mut queue, 3, Permission::ReadOnly, Disposition::Queue);
        queue.stop(CHAT);
        queue.finish_task(AgentId(7));
        queue.finish_task(AgentId(8));

        let resumed = queue.resume(CHAT, None);

        assert_eq!(resumed, vec![first, second]);
        assert_eq!(state_of(&queue, 3), InputState::Queued);
    }

    #[test]
    fn resume_target_only_resumes_that_task() {
        let mut queue = Queue::new();
        let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
        let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);
        queue.stop(CHAT);

        let resumed = queue.resume(CHAT, Some(second));

        assert_eq!(resumed, vec![second]);
        assert_eq!(queue.held_tasks(CHAT, None), vec![first]);
    }

    #[test]
    fn resume_writers_run_one_at_a_time() {
        let mut queue = Queue::new();
        let main = start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
        queue.stop(CHAT);
        queue.finish_task(AgentId(7));
        queue.resume(CHAT, None);
        let mut continued = input(3, Permission::Write);
        continued.task = Some(main);
        queue.accept(continued);
        queue.set_state(InputId(3), InputState::Queued).unwrap();

        let first = queue.next_to_send();
        let second = queue.next_to_send();

        assert_eq!(
            first,
            Some(SendAction::NewTask {
                input: InputId(2),
                task: TaskId(2)
            })
        );
        assert_eq!(second, None);
        assert_eq!(
            queue.input(InputId(3)).unwrap().reason,
            Some(QueueReason::WriteTurn)
        );
    }

    #[test]
    fn stop_input_bound_to_closed_task_holds_in_new_task() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::ReadOnly, 7);
        let aux = start_running(&mut queue, 2, Permission::ReadOnly, 8);
        queue.finish_task(AgentId(8));
        let mut late = input(3, Permission::ReadOnly);
        late.task = Some(aux);
        queue.accept(late);
        queue.stop(CHAT);

        queue.resume(CHAT, None);

        assert_eq!(state_of(&queue, 3), InputState::Queued);
    }

    #[test]
    fn note_resume_signal_closes_after_limit_without_resume() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::ReadOnly, 7);
        queue.stop(CHAT);

        let first = queue.note_resume_signal(CHAT, false);
        let second = queue.note_resume_signal(CHAT, false);
        let third = queue.note_resume_signal(CHAT, false);

        assert_eq!(first, None);
        assert_eq!(second, None);
        assert_eq!(third, Some(vec![task]));
    }

    #[test]
    fn note_resume_signal_resume_resets_count() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::ReadOnly, 7);
        queue.stop(CHAT);
        queue.note_resume_signal(CHAT, false);
        queue.note_resume_signal(CHAT, false);

        queue.note_resume_signal(CHAT, true);
        let after = queue.note_resume_signal(CHAT, false);

        assert_eq!(after, None);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 입력 수
    // basis: estimate
    #[test]
    fn note_resume_signal_without_held_returns_none() {
        let mut queue = Queue::new();

        let result = (0..HELD_IGNORE_LIMIT)
            .map(|_| queue.note_resume_signal(CHAT, false))
            .last()
            .flatten();

        assert_eq!(result, None);
    }

    #[test]
    fn close_held_cancels_unsent_inputs() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::Write, 7);
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        queue.stop(CHAT);

        let cancelled = queue.close_held(task);

        assert_eq!(cancelled, vec![InputId(2)]);
        assert_eq!(state_of(&queue, 1), InputState::Delivering);
        assert!(queue.held_tasks(CHAT, None).is_empty());
    }

    #[test]
    fn close_held_releases_write_gate() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::Write, 7);
        queue.stop(CHAT);

        queue.close_held(task);

        assert!(
            queue
                .write_gate()
                .try_acquire(Path::new("/work"), AgentId(8))
        );
    }

    #[test]
    fn start_task_unknown_task_returns_error() {
        let mut queue = Queue::new();

        let result = queue.start_task(TaskId(1), AgentId(1));

        assert!(matches!(result, Err(QueueError::TaskNotFound(TaskId(1)))));
    }

    #[test]
    fn start_task_write_conflict_keeps_task_pending() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
        assert!(queue.next_to_send().is_some());
        queue
            .write_gate()
            .try_acquire(Path::new("/work"), AgentId(9));

        let result = queue.start_task(TaskId(1), AgentId(1));

        assert!(matches!(result, Err(QueueError::WriteConflict(TaskId(1)))));
        assert_eq!(queue.tasks[0].phase, TaskPhase::Pending);
        assert_eq!(queue.tasks[0].agent, None);
    }

    #[test]
    fn try_acquire_other_agent_same_workdir_returns_false() {
        let mut gate = WriteGate::default();
        assert!(gate.try_acquire(Path::new("/work"), AgentId(1)));

        let same = gate.try_acquire(Path::new("/work"), AgentId(1));
        let other = gate.try_acquire(Path::new("/work"), AgentId(2));
        let elsewhere = gate.try_acquire(Path::new("/other"), AgentId(2));

        assert!(same);
        assert!(!other);
        assert!(elsewhere);
    }

    #[test]
    fn abandon_task_unblocks_write_inputs_waiting_on_pending_writer() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
        let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
            panic!("input should start a new task");
        };
        queue.set_state(InputId(1), InputState::Delivering).unwrap();
        accept_routed(&mut queue, 2, Permission::Write, Disposition::NewTask);
        assert_eq!(queue.next_to_send(), None);

        queue.abandon_task(task);

        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::NewTask { .. })
        ));
    }

    #[test]
    fn abandon_task_keeps_started_task() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::ReadOnly, 7);

        queue.abandon_task(task);

        assert!(queue.is_main_task(task));
        assert_eq!(queue.tasks[0].phase, TaskPhase::Running);
    }

    #[test]
    fn is_main_task_is_true_only_for_first_open_task() {
        let mut queue = Queue::new();
        let first = start_running(&mut queue, 1, Permission::ReadOnly, 7);
        let second = start_running(&mut queue, 2, Permission::ReadOnly, 8);

        assert!(queue.is_main_task(first));
        assert!(!queue.is_main_task(second));
        assert!(!queue.is_main_task(TaskId(99)));
    }

    #[test]
    fn redirect_changes_queued_input_disposition() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::ReadOnly, 7);
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);
        assert_eq!(queue.next_to_send(), None);

        queue.redirect(InputId(2), Disposition::Steer).unwrap();

        assert_eq!(queue.disposition(InputId(2)), Some(Disposition::Steer));
        assert_eq!(
            queue.next_to_send(),
            Some(SendAction::Steer {
                input: InputId(2),
                agent: AgentId(7)
            })
        );
    }

    #[test]
    fn redirect_new_task_starts_separate_task_even_while_running() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::ReadOnly, 7);
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);

        queue.redirect(InputId(2), Disposition::NewTask).unwrap();

        assert_eq!(
            queue.next_to_send(),
            Some(SendAction::NewTask {
                input: InputId(2),
                task: TaskId(2)
            })
        );
    }

    #[test]
    fn redirect_rejects_judging_dispatched_and_unknown_inputs() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);
        queue.next_to_send().unwrap();

        assert!(matches!(
            queue.redirect(InputId(1), Disposition::Steer),
            Err(QueueError::InvalidTransition { .. })
        ));
        assert!(matches!(
            queue.redirect(InputId(2), Disposition::Steer),
            Err(QueueError::AlreadySent)
        ));
        assert!(matches!(
            queue.redirect(InputId(9), Disposition::Steer),
            Err(QueueError::NotFound(InputId(9)))
        ));
    }

    #[test]
    fn hold_unsent_returns_a_dispatched_new_task_to_held_and_resume_sends_it_again() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::NewTask);
        let Some(SendAction::NewTask { task, .. }) = queue.next_to_send() else {
            panic!("input should start a new task");
        };

        queue.hold_unsent(InputId(1)).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Held);
        assert_eq!(queue.next_to_send(), None);
        assert_eq!(queue.resume(CHAT, None), Vec::<TaskId>::new());
        assert_eq!(state_of(&queue, 1), InputState::Queued);
        assert_eq!(
            queue.next_to_send(),
            Some(SendAction::NewTask {
                input: InputId(1),
                task
            })
        );
    }

    #[test]
    fn hold_unsent_on_an_idle_task_turn_frees_the_write_lock() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::Write, 7);
        queue.finish_task(AgentId(7));
        accept_routed(&mut queue, 2, Permission::Write, Disposition::Queue);
        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::NewTurn { .. })
        ));

        queue.hold_unsent(InputId(2)).unwrap();

        assert_eq!(state_of(&queue, 2), InputState::Held);
        assert!(
            queue
                .write_gate()
                .try_acquire(Path::new("/work"), AgentId(8))
        );
        assert_eq!(queue.resume(CHAT, Some(task)), Vec::<TaskId>::new());
        assert_eq!(state_of(&queue, 2), InputState::Queued);
    }

    #[test]
    fn hold_unsent_rejects_inputs_that_were_not_handed_out() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        accept_routed(&mut queue, 2, Permission::ReadOnly, Disposition::Queue);

        assert!(matches!(
            queue.hold_unsent(InputId(1)),
            Err(QueueError::InvalidTransition { .. })
        ));
        assert!(matches!(
            queue.hold_unsent(InputId(9)),
            Err(QueueError::NotFound(InputId(9)))
        ));
    }

    #[test]
    fn has_waiting_and_inputs_in_state_follow_the_chat() {
        let mut queue = Queue::new();
        accept_routed(&mut queue, 1, Permission::Write, Disposition::Queue);
        queue.accept(input(2, Permission::Write));

        assert!(queue.has_waiting(CHAT));
        assert!(!queue.has_waiting(ChatId(9)));
        assert_eq!(
            queue.inputs_in_state(CHAT, InputState::Judging),
            vec![InputId(2)]
        );
        queue.set_state(InputId(1), InputState::Cancelled).unwrap();
        assert!(!queue.has_waiting(CHAT));
    }

    #[test]
    fn release_frees_workdir() {
        let mut gate = WriteGate::default();
        gate.try_acquire(Path::new("/work"), AgentId(1));

        gate.release(AgentId(1));

        assert!(gate.try_acquire(Path::new("/work"), AgentId(2)));
    }
}
