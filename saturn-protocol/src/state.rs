//! 입력, 작업, session, 실행의 상태 값. 상태 전이 규칙은 `saturn-core`가 정한다.
//!
//! 설계: docs/design/input-handling.md(입력 전달 상태), docs/design/providers-and-sessions.md(session 상태),
//! docs/design/engine-lifecycle.md(효과 범위).

use serde::{Deserialize, Serialize};

/// 입력 전달 상태. `Applied`, `Rejected`, `Cancelled`는 끝 상태다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputState {
    /// judge 답을 기다린다. 다음: `Queued`, `Delivering`, `Held`, `Cancelled`.
    Judging,
    /// 보내기 전 대기열에 있다. 다음: `Delivering`, `Held`, `Cancelled`.
    Queued,
    /// 에이전트에 보냈다. 취소할 수 없다. 다음: `Applied`, `Rejected`.
    Delivering,
    /// 에이전트가 받았다.
    Applied,
    /// 에이전트가 받지 않았다.
    Rejected,
    /// 사용자가 멈춘 작업의 보내지 않은 입력. 다음: `Queued`, `Cancelled`.
    Held,
    /// 보내기 전에 취소했다.
    Cancelled,
}

/// 대기 입력이 기다리는 이유. 상태판 대기 줄의 문구를 고른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QueueReason {
    /// 실행 중인 작업 뒤(`대기 · A 다음`).
    AfterTask(crate::ids::TaskLabel),
    /// 같은 채팅의 앞 입력 판단 차례(`대기 · 판단 차례`). `[보내기]` 없이 `[취소]`만.
    JudgeOrder,
    /// judge 연결 회복(`대기 · 판단기 연결 기다림`).
    JudgeConnection,
    /// 다른 에이전트의 쓰기 끝(`대기 · 쓰기 차례`).
    WriteTurn,
    /// session 교체 끝(`대기 · 맥락 정리 뒤`).
    AfterCompaction,
    /// 모든 작업 끝. session을 바꾸는 명령만(`대기 · 모든 작업 뒤`).
    AfterAllTasks,
}

/// 작업 상태. 상태판 줄과 작업 목록 필터가 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskState {
    /// 에이전트가 실행 중이다.
    Running,
    /// 답은 나왔지만 subagent가 남았다(`answered-tree-running`).
    AnsweredTreeRunning,
    /// 허가 응답을 기다린다. 경과 시간이 멈춘다.
    AwaitingPermission,
    /// 보류됐다. `/continue`로 재개한다.
    Held,
    /// 결과를 확인해야 한다(결과 불명).
    NeedsCheck,
    /// 끝났다.
    Done,
    /// 실패했다.
    Failed,
}

/// Saturn session 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SessionState {
    /// provider 대화와 연결 중. 다음: `ClosedResumable`, `Held`, `Ended`.
    Open,
    /// 유예 뒤 닫고 provider session id를 보관했다. 다음: `Open`, `Ended`.
    ClosedResumable,
    /// 멈춤이나 증명되지 않은 크래시로 보류했다. 다음: `Open`, `Ended`.
    Held,
    /// 교체나 보류 종료로 끝났다.
    Ended,
}

/// 실행의 효과 범위. 크래시 뒤 자동 재개는 `Proven*`만 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectScope {
    /// 적용된 provider 설정을 기록했고 트리 전체에서 외부 효과가 불가능하다.
    ProvenByConfig,
    /// 트리 전체를 끊김 없이 관찰했고 모든 행동이 로컬 전용 목록 안이다.
    ProvenByObservation,
    /// 외부 효과 가능성을 배제하지 못했다.
    NetworkPossible,
    /// 관찰이 끊겼다(완료 신호 없는 흐름 끝, 읽는 중 종료, 끝이 없는 subagent).
    Unobserved,
}

impl EffectScope {
    /// 크래시 뒤 자동으로 이어 가도 되는지.
    pub fn allows_auto_resume(self) -> bool {
        matches!(self, Self::ProvenByConfig | Self::ProvenByObservation)
    }
}

/// 입력이 어떻게 처리될지. judge 결과와 `Tab` 입력이 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Disposition {
    /// 진행 중인 턴에 더한다(steer).
    Steer,
    /// 새 작업으로 시작한다.
    NewTask,
    /// 대기열에 둔다.
    Queue,
}

/// TUI를 닫을 때 engine이 할 일. 기본값은 `Background`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum OnExit {
    /// 접수된 입력을 계속 처리한다.
    #[default]
    Background,
    /// TODO(#70): 동작 미정
    Stop,
    /// TODO(#70): 동작 미정
    Ask,
}
