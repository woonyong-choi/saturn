//! 채팅 대기열: 입력 접수, 판단 차례, 판단 적용(CAS), 전송 순서, 취소, 멈춤과 보류, 재개, 쓰기 규칙.
//!
//! 설계: docs/design/input-handling.md. engine은 입력을 기록 저장소에 접수(ACK)한 뒤 `accept`를 부른다.
//! 보내기 전에 확정된 실패만 다시 보내고, 보낸 뒤 결과가 불명이면 사용자 확인으로 넘긴다.

use std::collections::VecDeque;
use std::path::PathBuf;

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
    /// 붙은 작업.
    pub task: Option<TaskId>,
}

/// 입력을 어떻게 보낼지. engine이 provider 호출로 옮긴다.
#[derive(Debug, Clone)]
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
    /// 쓰기 잠금을 잡는다. 이미 쓰는 에이전트가 있으면 거짓(입력은 `WriteTurn` 대기).
    pub fn try_acquire(&mut self, workdir: &std::path::Path, agent: AgentId) -> bool {
        todo!("#76")
    }

    /// 트리 유휴가 된 에이전트의 잠금을 푼다.
    pub fn release(&mut self, agent: AgentId) {
        todo!("#76")
    }
}

/// 채팅별 대기열. 대기열은 session이 아니라 채팅에 둔다.
#[derive(Debug, Default)]
pub struct Queue {
    inputs: VecDeque<QueuedInput>,
    held_ignored: u32,
    gate: WriteGate,
}

impl Queue {
    /// 빈 대기열.
    pub fn new() -> Self {
        Self::default()
    }

    /// 기록 저장소에 접수된 입력을 받는다. 상태는 `Judging`, 모델 고정이나 `Tab`이면 그에 맞게 판단 질문이 줄어든다.
    pub fn accept(&mut self, input: QueuedInput) {
        todo!("#76")
    }

    /// 다음에 판단할 입력. 같은 채팅 입력은 접수 순서대로 하나씩 판단하고, 판단 시점의 채팅 revision을 함께 준다.
    pub fn next_to_judge(&self, chat: ChatId) -> Option<(InputId, ChatRevision)> {
        todo!("#76")
    }

    /// 판단 결과를 적용한다. 적용 직전 revision을 비교하고, 다르면 `RevisionConflict`(한 번 다시 판단, 또 다르면 대기).
    /// 관계 판단 확신도가 0.6 미만이면 대기로 보낸다.
    ///
    /// # Errors
    /// revision이 다르면 `RevisionConflict`, 없는 입력이면 `NotFound`.
    pub fn apply(
        &mut self,
        input: InputId,
        decision: &RouteDecision,
        current: ChatRevision,
    ) -> Result<Disposition, QueueError> {
        todo!("#76")
    }

    /// 보낼 입력을 고른다. 대기열 맨 앞부터, 접수 때 권한으로 쓰기 규칙을 판정한다.
    pub fn next_to_send(&mut self) -> Option<SendAction> {
        todo!("#76")
    }

    /// 상태를 옮긴다(`Delivering` → `Applied`/`Rejected` 등). 규칙 밖 전이는 무시하지 않고 오류.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub fn set_state(&mut self, input: InputId, state: InputState) -> Result<(), QueueError> {
        todo!("#76")
    }

    /// 보내기 전 입력을 취소한다.
    ///
    /// # Errors
    /// 이미 보냈으면 `AlreadySent`.
    pub fn cancel(&mut self, input: InputId) -> Result<(), QueueError> {
        todo!("#76")
    }

    /// 멈춤. 실행 중 작업과 보내지 않은 입력을 보류한다. 멈춘 작업은 자동으로 이어 가지 않는다. TODO(#36): 뒤집는 입력을 바로 멈출지
    pub fn stop(&mut self, chat: ChatId) -> Vec<TaskId> {
        todo!("#76")
    }

    /// 재개. 대상이 있으면 그 작업만, 없으면 채팅의 보류 전부를 접수 순서대로. 쓰기 규칙에 따라 하나씩 실행한다.
    /// 확인된 상태로 만든 새 입력을 보내고 같은 패킷은 다시 보내지 않는다.
    pub fn resume(&mut self, chat: ChatId, task: Option<TaskId>) -> Vec<TaskId> {
        todo!("#76")
    }

    /// 새 입력의 `resume_held` 판단을 반영한다. 재개 뜻이 없는 입력이 `HELD_IGNORE_LIMIT`개 쌓이면 보류 종료를 돌려준다.
    pub fn note_resume_signal(&mut self, chat: ChatId, resume: bool) -> Option<Vec<TaskId>> {
        todo!("#76")
    }

    /// 보류 종료. 보내지 않은 입력은 취소하고, 기록은 지우지 않고 수정된 파일은 되돌리지 않는다.
    pub fn close_held(&mut self, task: TaskId) -> Vec<InputId> {
        todo!("#76")
    }

    /// 쓰기 잠금.
    pub fn write_gate(&mut self) -> &mut WriteGate {
        &mut self.gate
    }
}
