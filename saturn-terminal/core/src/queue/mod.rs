//! 채팅 대기열: 입력 접수, 판단 적용, 전송 순서, 멈춤과 재개, 쓰기 규칙.
//! 설계: docs/design/input-handling.md

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use saturn_protocol::ids::{AgentId, ChatId, ChatRevision, InputId, SettingsRevision, TaskId};
use saturn_protocol::state::{Disposition, InputState, QueueReason};

use crate::routers::RouteDecision;

mod gate;
mod send;
mod tasks;

pub use gate::WriteGate;

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
mod tests;
