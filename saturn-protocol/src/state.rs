//! 입력, 작업, session, 실행 상태 값. 전이 규칙은 `saturn-core`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `Applied`, `Rejected`, `Cancelled`는 끝 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum InputState {
    Judging,
    Queued,
    /// 취소할 수 없다.
    Delivering,
    Applied,
    Rejected,
    Held,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum QueueReason {
    AfterTask(crate::ids::TaskLabel),
    /// `[보내기]` 없이 `[취소]`만.
    RouterOrder,
    RouterConnection,
    WriteTurn,
    AfterCompaction,
    /// session을 바꾸는 명령만.
    AfterAllTasks,
    /// 반대 지시가 끼워 넣어지지 않아 멈추고 실행할지 사용자의 답을 기다린다. `[보내기]` 없이 확인 창만.
    ConfirmStop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum TaskState {
    Running,
    /// 답은 나왔지만 subagent가 남았다.
    AnsweredTreeRunning,
    /// 경과 시간이 멈춘다.
    AwaitingPermission,
    /// provider의 입력 요청에 답하기를 기다린다. 경과 시간이 멈춘다.
    AwaitingInput,
    Held,
    /// 결과 불명.
    NeedsCheck,
    Done,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum SessionState {
    Open,
    /// provider session id를 보관해 재개할 수 있다.
    ClosedResumable,
    Held,
    Ended,
}

/// 크래시 뒤 자동 재개는 `Proven*`만.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum EffectScope {
    /// 적용 설정상 트리 전체에서 외부 효과가 불가능하다.
    ProvenByConfig,
    /// 트리 전체를 끊김 없이 관찰했고 모든 행동이 로컬 전용이다.
    ProvenByObservation,
    NetworkPossible,
    /// 완료 신호 없는 흐름 끝, 읽는 중 종료, 끝나지 않은 subagent.
    Unobserved,
}

impl EffectScope {
    pub fn allows_auto_resume(self) -> bool {
        matches!(self, Self::ProvenByConfig | Self::ProvenByObservation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum Disposition {
    Steer,
    NewTask,
    Queue,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema, TS,
)]
pub enum OnExit {
    #[default]
    Background,
    /// 마지막 TUI가 떨어지면 모든 채팅의 작업을 멈춤과 같게 보류한다.
    Stop,
    /// TUI를 닫으려 할 때 계속할지 멈출지 묻는다.
    Ask,
}
