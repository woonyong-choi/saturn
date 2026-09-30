//! TUI, CLI → engine 요청과 engine → TUI 알림. Unix 소켓 위 JSON-RPC 한 줄에 하나.
//!
//! 설계: docs/design/engine-lifecycle.md(TUI 접속), docs/design/tui.md(화면이 보내고 받는 것).
//! TODO(#46): 메서드 이름과 목록은 design 이슈 결정 뒤 확정. 지금 이름은 가칭
//! TODO(#75): 요청마다 `id`를 붙이는 JSON-RPC 2.0 봉투(`Envelope`)와 직렬화 테스트

use serde::{Deserialize, Serialize};

use crate::event::ProviderEvent;
use crate::ids::{ChatId, InputId, JudgmentId, SettingsRevision, TaskId, TaskLabel};
use crate::state::{Disposition, InputState, QueueReason, TaskState};

/// TUI나 CLI가 engine에 보내는 요청.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// 채팅에 붙는다. engine은 최근 기록부터 보내고 보관한 허가 요청을 먼저 보낸다.
    Attach { chat: Option<ChatId> },
    /// 채팅에서 떨어진다. 마지막 TUI가 떨어지면 `OnExit`를 적용한다.
    Detach,
    /// 입력 제출. `Enter`는 `send_now: true`, 실행 중 `Tab`은 관계 판단 없이 대기.
    SubmitInput { chat: ChatId, text: String, pinned_model: Option<String>, skip_relation: bool },
    /// 대기 입력을 지금 보낸다(`[보내기]`, `/send`).
    SendNow { input: InputId },
    /// 보내기 전 입력 취소(`[취소]`, `/cancel`, `Alt+↑`).
    CancelInput { input: InputId },
    /// 멈춤(`Ctrl+C`). 채팅의 모든 작업과 보내지 않은 입력을 보류한다.
    Stop { chat: ChatId },
    /// 보류 재개(`/continue`). `task`가 없으면 채팅의 보류 전부를 접수 순서로.
    Continue { chat: ChatId, task: Option<TaskId> },
    /// 보류 종료. 보내지 않은 입력은 취소하고 수정된 파일은 되돌리지 않는다.
    CloseHeld { chat: ChatId, task: TaskId },
    /// 허가 요청 답.
    AnswerPermission { request_id: String, answer: PermissionAnswer },
    /// 피드백 질문 답(`1` 맞아요, `2` 아니에요).
    AnswerFeedback { judgment: JudgmentId, correct: bool },
    /// 사용량 조회(`/usage`).
    Usage { scope: UsageRange },
    /// 작업 목록 조회(`/tasks`).
    ListTasks,
    /// 판단 모델 학습(`/train`). 채점 후보가 200건 미만이면 거절한다.
    Train { reset_thresholds: bool },
    /// 기록 정리. `yes`가 없으면 미리보기만 한다.
    Prune { yes: bool },
    /// 판단 기록 JSONL 내보내기. 채점하지 않은 기록도 내보낸다.
    ExportJudgments { path: String },
}

/// 허가 요청 창의 네 선택지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionAnswer {
    /// `y` 실행.
    Allow,
    /// `a` 이 작업 동안 같은 명령 허용.
    AllowForTask,
    /// `d` 실행하지 않고 계속.
    Deny,
    /// `Esc` 실행하지 않고 다르게 하라고 말하기. TODO(#56): 이어 받는 입력 처리 방식
    DenyAndRedirect,
}

/// `/usage` 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageRange {
    /// 현재 채팅.
    Chat,
    /// 오늘.
    Today,
    /// 이번 주.
    Week,
    /// 전체.
    All,
}

/// engine이 TUI에 보내는 알림. TUI는 이것만으로 화면을 그린다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Notification {
    /// 입력 에코와 상태 변화. `label`은 합쳐진 작업의 이름표다.
    InputChanged { input: InputId, label: Option<TaskLabel>, state: InputState, disposition: Option<Disposition>, reason: Option<QueueReason> },
    /// 작업 상태 변화. 상태판 줄을 고친다.
    TaskChanged { task: TaskId, label: TaskLabel, state: TaskState },
    /// 작업의 provider 이벤트. 작업별 출력 칸과 도구 셀을 그린다.
    TaskEvent { task: TaskId, event: ProviderEvent },
    /// 대화 기록 한 줄 알림(`맥락 정리 후 이어서 진행`, `codex → claude로 전환` 등).
    ChatNotice { chat: ChatId, task: Option<TaskId>, notice: ChatNotice },
    /// 피드백 질문. 8초 안에 답이 없으면 TUI가 지운다.
    FeedbackQuestion { judgment: JudgmentId, label: TaskLabel },
    /// 바닥줄 맥락 크기. 측정하지 못하면 `tokens`가 `None`(`맥락 미확인`).
    ContextSize { chat: ChatId, tokens: Option<u64>, threshold: u64 },
    /// 설정 적용 결과. 검사 실패면 이전 번호로 계속하고 경고한다.
    SettingsApplied { revision: SettingsRevision, warning: Option<String> },
    /// engine 경고 줄(`자동 판단 일시 중단`, `새 입력 접수 중단 · ...` 등).
    Alert { alert: Alert },
}

/// 대화 기록에 남는 한 줄 알림 종류.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatNotice {
    /// 맥락 정리 뒤 같은 작업 계속.
    Compacted,
    /// 작업의 provider 전환.
    ProviderSwitched { from: crate::ids::Provider, to: crate::ids::Provider },
    /// 멈춤 결과. 보류된 이름표 목록.
    Stopped { held: Vec<TaskLabel> },
    /// 멈춤 뒤 provider 프로세스 묶음 밖에 남은 프로세스 수.
    StopUnconfirmed { remaining: u32 },
    /// 모든 작업이 끝난 순간의 합계(`이번 요청 · ...`).
    RequestSummary { provider_tokens: Vec<(crate::ids::Provider, u64)>, judge_calls: u32, judge_tokens: u64, elapsed_ms: u64 },
}

/// 상태판 알림 줄.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alert {
    /// judge 호출 일시 실패. 질문별 대체 규칙을 적용 중이다.
    JudgePaused,
    /// judge 호출 연속 3회 실패. 새 입력 접수를 멈췄다.
    IntakeStopped,
    /// provider의 끼워 넣기 실측 전. 끼워 넣기를 대기로 처리한다.
    SteerNotReady { provider: crate::ids::Provider },
    /// 다른 Saturn 프로세스가 이 채팅을 실행 중이다(작업 목록에서 읽기 전용).
    ChatBusyElsewhere { chat: ChatId },
}
