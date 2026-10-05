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

/// 작업이 끝났을 때 마지막 수정 뒤 검사가 통과했는지. `TaskState::Done`과 따로 두며 끝난 작업의 상태를 바꾸지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum EvidenceState {
    /// 설정한 검사 명령이 마지막 수정 뒤 실제로 종료 코드 0으로 끝났다.
    Verified,
    Unverified,
    /// 실행이 파일을 하나도 바꾸지 않았다.
    NotApplicable,
}

/// `Unverified`인 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum UnverifiedReason {
    /// 검사 명령을 설정하지 않았거나, 마지막 수정 뒤에 돌지 않았거나, 종료 코드를 알 수 없다.
    NotChecked,
    /// 마지막 수정 뒤에 돈 검사가 0이 아닌 코드로 끝났다.
    CheckFailed,
    /// 검사가 도는 사이에 수정이 있었다.
    EditedDuringCheck,
    /// 폴더가 커서 일부만 훑어 수정 목록이 모자랄 수 있다.
    PartialSnapshot,
    /// 이벤트에 없는 수정이 있고 셸 명령도 없어 수정과 검사의 순서를 알 수 없다.
    OrderUnknown,
    /// 하위 에이전트가 끝나지 않았거나 끊겼다.
    TreeNotIdle,
    /// 실행 시작 때 폴더 상태가 없어 수정 목록을 만들지 못했다.
    Unmeasured,
}

/// 끝난 실행의 완료 검사 근거. `events`는 근거가 된 검사 결과 이벤트의 기록 번호(채팅 안의 `events.seq`)다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct CompletionEvidence {
    pub state: EvidenceState,
    /// `Unverified`일 때만 있다.
    pub reason: Option<UnverifiedReason>,
    /// `Verified`일 때만 있다.
    #[serde(default)]
    pub events: Vec<u64>,
}
