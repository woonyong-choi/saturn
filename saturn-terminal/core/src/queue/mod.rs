//! 채팅 대기열: 입력 접수, 판단 차례, 판단 적용(CAS), 전송 순서, 취소, 멈춤과 보류, 재개, 쓰기 규칙.
//!
//! 설계: docs/design/input-handling.md. engine은 입력을 기록 저장소에 접수(ACK)한 뒤 `accept`를 부른다.
//! 보내기 전에 확정된 실패만 다시 보내고, 보낸 뒤 결과가 불명이면 사용자 확인으로 넘긴다.
//!
//! 작업 수명: `next_to_send`가 `NewTask`를 주면 engine이 에이전트를 띄운 뒤 `start_task`를 부르고,
//! 그 에이전트 트리가 유휴가 되면 `finish_task`를 부른다. 새 작업 id는 그 작업을 시작한 입력 id와 같은 값이다.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use saturn_protocol::ids::{AgentId, ChatId, ChatRevision, InputId, SettingsRevision, TaskId};
use saturn_protocol::state::{Disposition, InputState, QueueReason};

use crate::judges::RouteDecision;

/// 한 채팅에서 재개 뜻이 없는 새 입력이 이만큼 쌓이면 보류를 종료한다.
pub const HELD_IGNORE_LIMIT: u32 = 3;

/// 대기열 규칙 위반.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// 없는 입력.
    #[error("input not found: {0:?}")]
    NotFound(InputId),
    /// 이미 보냈다. 취소는 보내기 전 입력에만 적용한다.
    #[error("input already sent")]
    AlreadySent,
    /// 판단 뒤 채팅 상태가 바뀌었다. 한 번 다시 판단하고 또 바뀌면 대기로 둔다.
    #[error("chat revision changed since judgment")]
    RevisionConflict,
    /// 입력 전달 상태표에 없는 전이.
    #[error("invalid input state transition: {from:?} -> {to:?}")]
    InvalidTransition {
        /// 지금 상태.
        from: InputState,
        /// 요청한 상태.
        to: InputState,
    },
    /// 시작을 기다리는 작업이 없다.
    #[error("task not found: {0:?}")]
    TaskNotFound(TaskId),
}

/// 에이전트 권한. 접수 때 고정해 쓰기 규칙을 결정론으로 판정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// 읽기 전용. 같은 폴더에서 병렬 실행.
    ReadOnly,
    /// 쓰기 가능. 같은 작업 폴더에서 한 번에 하나.
    Write,
}

/// 접수된 입력 하나.
#[derive(Debug, Clone)]
pub struct QueuedInput {
    /// 입력 id.
    pub id: InputId,
    /// 채팅.
    pub chat: ChatId,
    /// 원문.
    pub text: String,
    /// 접수 때 고정한 설정 번호.
    pub settings: SettingsRevision,
    /// 접수 때 고정한 권한.
    pub permission: Permission,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// 사용자가 고정한 모델. 있으면 judge 호출에서 모델 질문을 뺀다.
    pub pinned_model: Option<String>,
    /// `Tab`으로 관계 판단 없이 대기(보낼 때 judge 1회).
    pub skip_relation: bool,
    /// 상태.
    pub state: InputState,
    /// 대기 이유.
    pub reason: Option<QueueReason>,
    /// 붙은 작업. engine이 접수 때 정하면(재개 입력 등) 그 작업으로만 보낸다.
    pub task: Option<TaskId>,
}

/// 입력을 어떻게 보낼지. engine이 provider 호출로 옮긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendAction {
    /// 진행 중인 턴에 끼워 넣는다. 활성 턴 없음으로 실패하면 다시 판단하지 않고 `NewTurn`으로 보낸다.
    Steer { input: InputId, agent: AgentId },
    /// 같은 session의 새 턴.
    NewTurn { input: InputId, agent: AgentId },
    /// 새 작업(메인이나 보조 에이전트).
    NewTask { input: InputId, task: TaskId },
}

/// 쓰기 잠금. 트리 유휴일 때 푼다(subagent가 쓰는 중에 다음 쓰기가 시작되지 않게).
/// 파일 겹침 예측으로 병렬 쓰기를 허용하지 않는다. worktree 설정이 켜지고 git 저장소면 별도 worktree에서 병렬로 쓴다.
#[derive(Debug, Default)]
pub struct WriteGate {
    holders: Vec<(PathBuf, AgentId)>,
}

impl WriteGate {
    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    /// 쓰기 잠금을 잡는다. 이미 쓰는 에이전트가 있으면 거짓(입력은 `WriteTurn` 대기).
    /// 같은 에이전트가 이미 쥐고 있으면 참이다.
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
    /// 트리 유휴가 된 에이전트의 잠금을 푼다.
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

/// 대기열이 아는 작업의 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskPhase {
    /// `NewTask`를 내줬고 `start_task`를 기다린다.
    Pending,
    /// 에이전트 트리가 실행 중이다.
    Running,
    /// 메인 에이전트가 쉬고 있거나, 재개되어 다음 입력을 기다린다.
    Idle,
    /// 멈춤으로 보류됐다.
    Held,
    /// 끝났다(보조 에이전트 종료나 보류 종료).
    Closed,
}

/// 채팅 안 작업 하나.
#[derive(Debug, Clone)]
struct TaskSlot {
    id: TaskId,
    chat: ChatId,
    agent: Option<AgentId>,
    permission: Permission,
    workdir: PathBuf,
    is_main: bool,
    /// 멈춤 때 실행 중이었는지. 재개하면 engine이 확인된 상태로 만든 새 입력을 보낸다.
    was_interrupted: bool,
    phase: TaskPhase,
}

/// 대기열의 입력 하나와 판단 결과.
#[derive(Debug, Clone)]
struct Entry {
    input: QueuedInput,
    disposition: Option<Disposition>,
    /// `next_to_send`가 내줬고 engine이 아직 `Delivering`으로 바꾸지 않았다. 두 번 내주지 않는다.
    is_dispatched: bool,
}

/// 채팅별 상태.
#[derive(Debug, Clone, Copy, Default)]
struct ChatState {
    revision: u64,
    held_ignored: u32,
}

/// `next_to_send`가 입력 하나에 대해 정한 길.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Steer { agent: AgentId, task: TaskId },
    NewTurn { agent: AgentId, task: TaskId },
    NewTask { task: TaskId, is_existing: bool },
    Wait(Option<QueueReason>),
}

/// 채팅별 대기열. 대기열은 session이 아니라 채팅에 둔다. 한 값이 여러 채팅을 채팅 id로 나눠 담는다.
#[derive(Debug, Default)]
pub struct Queue {
    inputs: VecDeque<Entry>,
    tasks: Vec<TaskSlot>,
    chats: HashMap<ChatId, ChatState>,
    gate: WriteGate,
}

impl Queue {
    /// 빈 대기열.
    pub fn new() -> Self {
        Self::default()
    }

    /// 판단 적용 직전 비교에 쓰는 지금 채팅 revision. 작업 상태, 대기열 맨 앞, 마지막 판단이 바뀌면 오른다.
    pub fn revision(&self, chat: ChatId) -> ChatRevision {
        ChatRevision(self.chats.get(&chat).map_or(0, |state| state.revision))
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 입력 하나. 상태와 대기 이유를 기록 저장소에 옮길 때 쓴다.
    pub fn input(&self, input: InputId) -> Option<&QueuedInput> {
        self.inputs
            .iter()
            .find(|entry| entry.input.id == input)
            .map(|entry| &entry.input)
    }

    // cost: time O(n), heap O(1) amortized, stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 기록 저장소에 접수된 입력을 받는다. 상태는 `Judging`, 모델 고정이나 `Tab`이면 그에 맞게 판단 질문이 줄어든다.
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
        self.refresh_judge_order(chat);
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 다음에 판단할 입력. 같은 채팅 입력은 접수 순서대로 하나씩 판단하고, 판단 시점의 채팅 revision을 함께 준다.
    pub fn next_to_judge(&self, chat: ChatId) -> Option<(InputId, ChatRevision)> {
        self.inputs
            .iter()
            .find(|entry| entry.input.chat == chat && entry.input.state == InputState::Judging)
            .map(|entry| (entry.input.id, self.revision(chat)))
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 판단 결과를 적용한다. 적용 직전 revision을 비교하고, 다르면 `RevisionConflict`(한 번 다시 판단, 또 다르면 대기).
    /// 관계 판단 확신도가 0.6 미만이면 `decide_route`가 이미 대기로 정해 둔다. 적용하면 입력은 `Queued`가 된다.
    ///
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
        let chat = entry.input.chat;
        self.bump(chat);
        self.refresh_judge_order(chat);
        Ok(decision.disposition)
    }

    // cost: time O(n·(t + h)), heap O(c), stack O(1)
    // vars: n = 대기열 입력 수, t = 작업 수, h = 쓰기 잠금 수, c = 막힌 채팅 수
    // basis: estimate
    /// 보낼 입력을 고른다. 대기열 맨 앞부터, 접수 때 권한으로 쓰기 규칙을 판정한다.
    /// 끼워 넣기는 실행 중 턴에 바로 가고, 그 밖의 입력은 같은 채팅의 앞 입력이 기다리면 함께 기다린다.
    /// 내준 입력은 engine이 `Delivering`으로 바꿀 때까지 다시 내주지 않는다.
    pub fn next_to_send(&mut self) -> Option<SendAction> {
        let mut blocked_chats: Vec<ChatId> = Vec::new();
        for index in 0..self.inputs.len() {
            let entry = &self.inputs[index];
            if entry.input.state != InputState::Queued || entry.is_dispatched {
                continue;
            }
            let chat = entry.input.chat;
            let route = self.route_for(entry);
            match route {
                Route::Steer { .. } => return Some(self.commit(index, route)),
                _ if blocked_chats.contains(&chat) => {}
                Route::Wait(reason) => {
                    self.inputs[index].input.reason = reason;
                    blocked_chats.push(chat);
                }
                Route::NewTurn { .. } | Route::NewTask { .. } => {
                    return Some(self.commit(index, route));
                }
            }
        }
        None
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 끼워 넣기를 대기로 바꾼다. 끼워 넣기 실측을 통과하기 전의 provider에 보낼 때 engine이 부른다.
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
    /// 상태를 옮긴다(`Delivering` → `Applied`/`Rejected` 등). 규칙 밖 전이는 무시하지 않고 오류.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`, 입력 전달 상태표에 없는 전이면 `InvalidTransition`.
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
        self.refresh_judge_order(chat);
        Ok(())
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 대기열 입력 수
    // basis: estimate
    /// 보내기 전 입력을 취소한다. 이미 취소한 입력이면 아무것도 하지 않는다.
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
        self.refresh_judge_order(chat);
        Ok(())
    }

    // cost: time O(t + n·t), heap O(t), stack O(1), alloc 1
    // vars: n = 대기열 입력 수, t = 작업 수
    // basis: estimate
    /// 멈춤. 실행 중 작업과 보내지 않은 입력을 보류한다. 멈춘 작업은 자동으로 이어 가지 않는다. TODO(#36): 뒤집는 입력을 바로 멈출지
    /// 보내지 않은 입력은 붙은 작업, 없으면 메인 작업, 메인도 없으면 그 입력으로 만든 작업과 함께 보류한다.
    /// 돌려주는 값은 채팅의 보류 작업 전부, id 순서.
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
    /// 재개. 대상이 있으면 그 작업만, 없으면 채팅의 보류 전부를 접수 순서대로. 쓰기 규칙에 따라 하나씩 실행한다.
    /// 확인된 상태로 만든 새 입력을 보내고 같은 패킷은 다시 보내지 않는다.
    /// 보류 입력은 대기로 되돌린다. 돌려주는 값은 멈춤 때 실행 중이던 작업, 즉 engine이 새 입력을 만들어 보낼 작업이다.
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
    /// 새 입력의 `resume_held` 판단을 반영한다. 재개 뜻이 없는 입력이 `HELD_IGNORE_LIMIT`개 쌓이면 보류 종료를 돌려준다.
    /// 보류가 없으면 세지 않는다. 재개 뜻이 있으면 횟수를 0으로 되돌리고, 재개는 engine이 `resume`으로 한다.
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
    /// 보류 종료. 보내지 않은 입력은 취소하고, 기록은 지우지 않고 수정된 파일은 되돌리지 않는다.
    /// 보류가 아닌 작업이면 아무것도 하지 않는다. 돌려주는 값은 취소한 입력, 접수 순서.
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
    /// `NewTask`로 띄운 에이전트를 작업에 붙인다. 쓰기 권한이면 쓰기 잠금을 잡는다.
    /// 멈춤으로 이미 보류된 작업이면 에이전트만 기록하고 잠금은 잡지 않는다.
    ///
    /// # Errors
    /// 시작을 기다리는 작업이 아니면 `TaskNotFound`.
    ///
    /// # Panics
    /// 시작을 기다리는 쓰기 작업의 폴더를 다른 에이전트가 쥐고 있으면. `next_to_send`가 막으므로 일어나지 않는다.
    pub fn start_task(&mut self, task: TaskId, agent: AgentId) -> Result<(), QueueError> {
        let slot = self
            .tasks
            .iter_mut()
            .find(|slot| slot.id == task && slot.agent.is_none())
            .filter(|slot| matches!(slot.phase, TaskPhase::Pending | TaskPhase::Held))
            .ok_or(QueueError::TaskNotFound(task))?;
        slot.agent = Some(agent);
        if slot.phase == TaskPhase::Held {
            return Ok(());
        }
        slot.phase = TaskPhase::Running;
        let chat = slot.chat;
        if slot.permission == Permission::Write {
            let is_acquired = self.gate.try_acquire(&slot.workdir, agent);
            assert!(is_acquired, "pending write task should own its workdir");
        }
        self.bump(chat);
        Ok(())
    }

    // cost: time O(t + h), heap O(1), stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    /// 에이전트 트리가 유휴가 됐다. 쓰기 잠금을 풀고, 메인 에이전트는 쉬는 상태로, 보조 에이전트는 끝난 상태로 둔다.
    /// 보류된 작업은 보류 그대로 둔다.
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

    /// 쓰기 잠금.
    pub fn write_gate(&mut self) -> &mut WriteGate {
        &mut self.gate
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
    /// 판단을 기다리는 입력 중 맨 앞만 판단 중이고, 나머지는 `판단 차례` 대기로 보인다.
    fn refresh_judge_order(&mut self, chat: ChatId) {
        let mut is_first = true;
        for entry in &mut self.inputs {
            if entry.input.chat != chat || entry.input.state != InputState::Judging {
                continue;
            }
            entry.input.reason = if is_first {
                None
            } else {
                Some(QueueReason::JudgeOrder)
            };
            is_first = false;
        }
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 채팅의 메인 작업. 보류되거나 끝난 메인은 메인으로 보지 않는다.
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
    /// 대기 입력 하나의 길을 정한다. 붙은 작업이 있으면 그 작업, 없으면 메인 작업이 대상이다.
    fn route_for(&self, entry: &Entry) -> Route {
        let input = &entry.input;
        let disposition = entry.disposition.unwrap_or(Disposition::Queue);
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
    /// 쓰기 규칙. 쓰기 권한 입력은 같은 폴더에 다른 쓰는 에이전트나 시작을 기다리는 쓰기 작업이 있으면 기다린다.
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
    /// 정한 길을 대기열에 반영하고 engine에 줄 행동을 만든다.
    fn commit(&mut self, index: usize, route: Route) -> SendAction {
        let entry = &mut self.inputs[index];
        entry.is_dispatched = true;
        entry.input.reason = None;
        let input = entry.input.clone();
        match route {
            Route::Steer { agent, task } => {
                self.inputs[index].input.task = Some(task);
                SendAction::Steer {
                    input: input.id,
                    agent,
                }
            }
            Route::NewTurn { agent, task } => {
                self.inputs[index].input.task = Some(task);
                self.run_turn(task, agent, &input);
                SendAction::NewTurn {
                    input: input.id,
                    agent,
                }
            }
            Route::NewTask { task, is_existing } => {
                self.inputs[index].input.task = Some(task);
                self.reserve_task(task, is_existing, &input);
                SendAction::NewTask {
                    input: input.id,
                    task,
                }
            }
            Route::Wait(_) => unreachable!("waiting inputs should not be committed"),
        }
    }

    // cost: time O(t + h), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수, h = 쓰기 잠금 수
    // basis: estimate
    fn run_turn(&mut self, task: TaskId, agent: AgentId, input: &QueuedInput) {
        let slot = self
            .tasks
            .iter_mut()
            .find(|slot| slot.id == task)
            .expect("routed task should exist");
        slot.phase = TaskPhase::Running;
        slot.permission = input.permission;
        slot.workdir.clone_from(&input.workdir);
        if input.permission == Permission::Write {
            let is_acquired = self.gate.try_acquire(&input.workdir, agent);
            assert!(is_acquired, "write rule should be checked before the turn");
        }
        self.bump(input.chat);
    }

    // cost: time O(t), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    fn reserve_task(&mut self, task: TaskId, is_existing: bool, input: &QueuedInput) {
        if is_existing {
            let slot = self
                .tasks
                .iter_mut()
                .find(|slot| slot.id == task)
                .expect("routed task should exist");
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
    }

    // cost: time O(t), heap O(1) amortized, stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 멈춤 때 보내지 않은 입력 하나를 함께 보류할 작업을 정하고, 그 작업을 보류로 둔다.
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
    /// 채팅의 보류 작업, id 순서(접수 순서). `task`가 있으면 그 작업만.
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

/// 입력 전달 상태표의 전이인지.
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

    /// 접수하고 판단을 적용한다.
    fn accept_judged(queue: &mut Queue, id: u64, permission: Permission, disposition: Disposition) {
        queue.accept(input(id, permission));
        let revision = queue.revision(CHAT);
        queue
            .apply(InputId(id), &decision(revision, disposition), revision)
            .expect("apply should succeed");
    }

    /// 입력 하나를 새 작업으로 보내고 에이전트를 붙여 실행 중으로 만든다.
    fn start_running(queue: &mut Queue, id: u64, permission: Permission, agent: u64) -> TaskId {
        accept_judged(queue, id, permission, Disposition::NewTask);
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
    fn accept_sets_judging_and_later_inputs_wait_for_judge_order() {
        let mut queue = Queue::new();

        queue.accept(input(1, Permission::ReadOnly));
        queue.accept(input(2, Permission::ReadOnly));

        assert_eq!(state_of(&queue, 1), InputState::Judging);
        assert_eq!(queue.input(InputId(1)).unwrap().reason, None);
        assert_eq!(
            queue.input(InputId(2)).unwrap().reason,
            Some(QueueReason::JudgeOrder)
        );
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 입력 수
    // basis: estimate
    #[test]
    fn next_to_judge_follows_ack_order_one_at_a_time() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        queue.accept(input(2, Permission::ReadOnly));

        let first = queue.next_to_judge(CHAT).map(|(id, _)| id);
        let revision = queue.revision(CHAT);
        queue
            .apply(
                InputId(1),
                &decision(revision, Disposition::Queue),
                revision,
            )
            .unwrap();
        let second = queue.next_to_judge(CHAT).map(|(id, _)| id);

        assert_eq!(first, Some(InputId(1)));
        assert_eq!(second, Some(InputId(2)));
    }

    #[test]
    fn next_to_judge_other_chat_returns_none() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));

        assert_eq!(queue.next_to_judge(ChatId(9)), None);
    }

    #[test]
    fn apply_changed_revision_returns_conflict() {
        let mut queue = Queue::new();
        queue.accept(input(1, Permission::ReadOnly));
        let (_, judged_at) = queue.next_to_judge(CHAT).unwrap();
        start_running(&mut queue, 2, Permission::ReadOnly, 7);

        let result = queue.apply(
            InputId(1),
            &decision(judged_at, Disposition::Steer),
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

        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        assert!(queue.revision(CHAT) > before);
    }

    #[test]
    fn next_to_send_first_input_starts_main_task() {
        let mut queue = Queue::new();
        accept_judged(&mut queue, 1, Permission::Write, Disposition::Queue);

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
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        let first = queue.next_to_send();
        let second = queue.next_to_send();

        assert!(first.is_some());
        assert_eq!(second, None);
    }

    #[test]
    fn next_to_send_steer_goes_to_running_agent() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Steer);

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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Queue);
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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::NewTask);

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
        accept_judged(&mut queue, 1, Permission::Write, Disposition::NewTask);
        accept_judged(&mut queue, 2, Permission::Write, Disposition::NewTask);
        assert!(queue.next_to_send().is_some());

        let second = queue.next_to_send();

        assert_eq!(second, None);
    }

    #[test]
    fn next_to_send_read_only_runs_beside_writer() {
        let mut queue = Queue::new();
        start_running(&mut queue, 1, Permission::Write, 7);
        accept_judged(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);

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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Queue);
        accept_judged(&mut queue, 3, Permission::ReadOnly, Disposition::NewTask);

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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Steer);
        assert!(matches!(
            queue.next_to_send(),
            Some(SendAction::Steer { .. })
        ));

        queue.defer_steer(InputId(2)).unwrap();

        assert_eq!(queue.next_to_send(), None);
    }

    #[test]
    fn set_state_follows_transition_table() {
        let mut queue = Queue::new();
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        queue.set_state(InputId(1), InputState::Delivering).unwrap();
        queue.set_state(InputId(1), InputState::Applied).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Applied);
    }

    #[test]
    fn set_state_outside_table_returns_error() {
        let mut queue = Queue::new();
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
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
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);

        queue.cancel(InputId(1)).unwrap();

        assert_eq!(state_of(&queue, 1), InputState::Cancelled);
    }

    #[test]
    fn cancel_delivering_or_applied_returns_already_sent() {
        let mut queue = Queue::new();
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
        accept_judged(&mut queue, 2, Permission::ReadOnly, Disposition::NewTask);
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
        accept_judged(&mut queue, 1, Permission::ReadOnly, Disposition::Queue);
        assert!(queue.next_to_send().is_some());

        let result = queue.cancel(InputId(1));

        assert!(matches!(result, Err(QueueError::AlreadySent)));
    }

    #[test]
    fn stop_holds_running_task_and_unsent_inputs() {
        let mut queue = Queue::new();
        let task = start_running(&mut queue, 1, Permission::Write, 7);
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Queue);
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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Queue);
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
        accept_judged(&mut queue, 3, Permission::ReadOnly, Disposition::Queue);
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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::NewTask);
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
        accept_judged(&mut queue, 2, Permission::Write, Disposition::Queue);
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
    fn release_frees_workdir() {
        let mut gate = WriteGate::default();
        gate.try_acquire(Path::new("/work"), AgentId(1));

        gate.release(AgentId(1));

        assert!(gate.try_acquire(Path::new("/work"), AgentId(2)));
    }
}
