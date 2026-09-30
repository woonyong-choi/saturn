//! 화면 상태 모델. engine 알림(`Notification`)만으로 채운다. 화면(`view`)과 plain 출력(`plain`)이 같은 모델을 읽는다.
//!
//! 설계: docs/design/tui.md(영역, 상태판 줄 순서, 이름표), docs/design/input-handling.md(입력 전달 상태, 대기와 취소, 멈춤과 보류).
//! 상태 전이 규칙은 engine(`saturn-core`)이 정한다. 여기서는 알린 값을 그대로 기록하고 전이를 검사하지 않는다.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use saturn_protocol::event::{Activity, ProviderEvent, UsageReport};
use saturn_protocol::ids::{
    ChatId, InputId, JudgmentId, Provider, SettingsRevision, SubagentId, TaskId, TaskLabel,
};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::{Disposition, InputState, QueueReason, TaskState};

/// 경과 시간. 허가를 기다리는 동안 멈춘다(`AwaitingPermission`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopwatch {
    started: Instant,
    paused_total: Duration,
    paused_at: Option<Instant>,
}

impl Stopwatch {
    /// `now`부터 잰다.
    pub fn start(now: Instant) -> Self {
        todo!("#92")
    }

    /// 멈춘다. 이미 멈췄으면 그대로.
    pub fn pause(&mut self, now: Instant) {
        todo!("#92")
    }

    /// 다시 잰다. 멈춘 동안은 경과에 넣지 않는다.
    pub fn resume(&mut self, now: Instant) {
        todo!("#92")
    }

    /// 멈춘 시간을 뺀 경과.
    pub fn elapsed(&self, now: Instant) -> Duration {
        todo!("#92")
    }
}

/// 작업 하나의 화면 상태. `Notification::TaskChanged`로 생기고 `TaskEvent`로 채운다.
#[derive(Debug, Clone)]
pub struct TaskView {
    /// 작업 id.
    pub id: TaskId,
    /// engine이 준 이름표.
    pub label: TaskLabel,
    /// 작업 상태.
    pub state: TaskState,
    /// 마지막으로 답한 provider. 결과 머리줄과 실행 줄에 쓴다.
    /// protocol `TaskChanged`에 `provider`가 있으나 `apply_task`가 받지 않아 지금은 `ChatNotice::ProviderSwitched`의 `to`로만 채운다.
    /// TODO(#92): `apply_task`가 `TaskChanged::provider`를 받게 보완
    pub provider: Option<Provider>,
    /// 사용량 보고의 모델 이름. 보고가 없으면 `None`(작업 상세 `모델 미보고`).
    pub model: Option<String>,
    /// 실행 줄의 하는 일. 도구 호출이 시작되면 바뀐다.
    pub activity: Option<Activity>,
    /// 도는 provider subagent. 부모 작업 줄 아래 흐리게 접어 `하위 에이전트 N개 실행 중`으로 보인다.
    pub subagents: Vec<SubagentId>,
    /// 모델 글이 한 번이라도 왔다. 없으면 실행 줄이 `작업 중`이다.
    pub has_output: bool,
    /// 이 작업 토큰 합계. 보고 전이면 `None`(`Token -`).
    pub tokens: Option<u64>,
    /// 경과 시간.
    pub stopwatch: Stopwatch,
    /// 이 화면이 처음 본 순서. 같은 종류 줄 안의 접수 순서로 쓴다.
    pub seq: u64,
}

/// 입력 하나의 화면 상태. `Notification::InputChanged`로 생기고 끝 상태가 되면 줄을 지운다.
#[derive(Debug, Clone)]
pub struct InputView {
    /// 입력 id.
    pub id: InputId,
    /// 판단 줄·대기 줄·에코의 이름표. 끼워 넣기면 합쳐진 작업의 이름표.
    pub label: Option<TaskLabel>,
    /// 원문. 판단 줄·대기 줄 끝과 에코에 쓴다. protocol `InputChanged`에 `text`가 있으나 `apply_input`이 받지 않아 지금은 이 TUI가 보낸 입력만 안다.
    /// TODO(#92): `apply_input`이 `InputChanged::text`를 받게 보완
    pub text: Option<String>,
    /// 전달 상태.
    pub state: InputState,
    /// 판단 결과. 판단 전이면 `None`.
    pub disposition: Option<Disposition>,
    /// 대기 이유. `Queued`일 때만 있다.
    pub reason: Option<QueueReason>,
    /// 에코를 이미 찍었다(판단이 끝난 순간 한 번).
    pub echoed: bool,
    /// 이 화면이 처음 본 순서. 같은 종류 줄 안의 접수 순서.
    pub seq: u64,
}

/// `/train` 진행. `Notification::TrainProgress`로 채운다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingProgress {
    /// 단계 이름(원문 그대로).
    pub stage: String,
    /// 채점한 건수.
    pub graded: u32,
    /// 경과 시간.
    pub elapsed: Duration,
    /// 쓴 토큰.
    pub tokens: u64,
}

/// 멈춤 결과(`ChatNotice::Stopped`). 보류 줄 맨 앞에 `‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서`로 보인다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopResult {
    /// 보류된 이름표.
    pub held: Vec<TaskLabel>,
    /// 멈춤 뒤 provider 프로세스 묶음 밖에 남은 프로세스 수(`멈춤 확인 안 됨 · N개 남음`). 없으면 `None`.
    pub unconfirmed: Option<u32>,
}

/// 맥락 크기(바닥줄 `맥락 38K/200K`, 측정 불가면 `맥락 미확인`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSize {
    /// 현재 활성 맥락 토큰. 측정하지 못하면 `None`.
    pub tokens: Option<u64>,
    /// Saturn 정리 기준.
    pub threshold: u64,
}

/// 떠 있는 피드백 질문. 8초 안에 답이 없으면 지운다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackPrompt {
    /// 판단 기록.
    pub judgment: JudgmentId,
    /// 판단한 입력이 붙은 작업 이름표.
    pub label: TaskLabel,
    /// 뜬 시각.
    pub shown_at: Instant,
}

/// 알림 하나를 반영한 결과. `app::App`이 대화 기록·작업별 출력 칸·창을 고칠 때 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// 판단이 끝나 입력 에코를 찍을 차례(`> [A] 원문`). 입력당 한 번.
    Echo { input: InputId },
    /// 모델 글 조각. 작업별 출력 칸에 완성된 줄 단위로 넣는다.
    Output { task: TaskId, text: String },
    /// 도구 호출 시작·결과. 대화 기록의 도구 셀.
    Tool {
        task: TaskId,
        call_id: String,
        activity: Option<Activity>,
        output: Option<String>,
    },
    /// 작업이 끝났다(`Done`, `Failed`). 작업별 출력 칸 내용을 대화 기록으로 옮기고 결과 머리줄을 찍는다.
    TaskFinished { task: TaskId },
    /// 결과 불명(`NeedsCheck`). `[A] 결과 확인 필요 · /continue A`.
    TaskNeedsCheck { task: TaskId },
    /// 허가 요청 도착. 허가 요청 창 대기열에 넣는다.
    PermissionRequested {
        task: TaskId,
        request_id: String,
        summary: String,
        reason: String,
    },
    /// 그 밖의 상태 변화. 다시 그리기만 한다.
    Redraw,
}

/// 채팅 하나의 화면 상태.
#[derive(Debug, Default)]
pub struct ChatState {
    /// 붙은 채팅. `Attach` 응답 전이면 `None`.
    pub chat: Option<ChatId>,
    /// 작업. 끝난 작업은 결과를 대화 기록으로 옮긴 뒤 지운다.
    pub tasks: BTreeMap<TaskId, TaskView>,
    /// 끝 상태가 아닌 입력.
    pub inputs: BTreeMap<InputId, InputView>,
    /// 상태판 알림 줄. 같은 값은 한 번만.
    pub alerts: Vec<Alert>,
    /// `/train` 진행 줄.
    pub training: Option<TrainingProgress>,
    /// 마지막 멈춤 결과. 보류 줄이 모두 사라지면 지운다.
    pub stop: Option<StopResult>,
    /// 보류 닫기 확인 중인 작업(`‖ [E] 보류를 닫을까요?`).
    pub close_held_confirm: Option<TaskId>,
    /// 떠 있는 피드백 질문.
    pub feedback: Option<FeedbackPrompt>,
    /// 바닥줄 맥락 크기.
    pub context: Option<ContextSize>,
    /// 적용된 설정 번호와 경고(`폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`).
    pub settings: Option<(SettingsRevision, Option<String>)>,
    /// 보낸 원문. 다음 `InputChanged`(`Judging`)의 새 입력에 붙인다. 지금은 순서로 짝짓는다.
    /// TODO(#92): `Notification::InputAccepted`의 `client_ref`로 짝짓게 보완
    pub pending_texts: std::collections::VecDeque<String>,
    next_seq: u64,
}

impl ChatState {
    /// 빈 상태.
    pub fn new() -> Self {
        Self::default()
    }

    /// `InputChanged` 반영. 처음 보는 입력이면 `pending_texts` 앞 원문을 붙인다.
    /// `Judging` → 다른 상태가 되고 아직 에코가 없으면 `Change::Echo`. 끝 상태(`Applied`, `Rejected`, `Cancelled`)면 입력을 지운다.
    /// `Queued`면 `reason`, 판단이 끝났으면 `disposition`을 기록한다.
    /// TODO(#60): provider가 끼워 넣기를 거절한 입력(`Rejected`)을 대기로 옮길지, 다시 판단할지, 물을지
    pub fn apply_input(
        &mut self,
        input: InputId,
        label: Option<TaskLabel>,
        state: InputState,
        disposition: Option<Disposition>,
        reason: Option<QueueReason>,
    ) -> Change {
        todo!("#92")
    }

    /// `TaskChanged` 반영. 처음 보면 `TaskView`를 만들고 경과를 잰다. `AwaitingPermission`이면 경과를 멈추고 벗어나면 다시 잰다.
    /// `Done`, `Failed`면 `Change::TaskFinished`, `NeedsCheck`면 `Change::TaskNeedsCheck`.
    pub fn apply_task(
        &mut self,
        task: TaskId,
        label: TaskLabel,
        state: TaskState,
        now: Instant,
    ) -> Change {
        todo!("#92")
    }

    /// `TaskEvent` 반영. `Text` → `Output`(has_output 참), `ToolCall` → 하는 일 갱신과 `Tool`, `ToolResult` → `Tool`,
    /// `SubagentStarted`/`Ended` → subagent 목록, `PermissionRequested` → `PermissionRequested`, `Usage` → `apply_usage`,
    /// `TurnCompleted`, `ContextSize`, `StreamLost` → `Redraw`. subagent의 `Text`는 부모 출력 칸에 넣지 않는다.
    pub fn apply_event(&mut self, task: TaskId, event: ProviderEvent, now: Instant) -> Change {
        todo!("#92")
    }

    /// 사용량 보고를 작업 토큰 합계에 더한다. 값이 `None`인 칸은 0으로 채우지 않고 건너뛴다.
    /// 범위가 `ThreadCumulative`면 같은 session 직전 누적을 빼야 하지만 TUI에는 session이 없어 engine 합계를 믿는다.
    fn apply_usage(&mut self, task: TaskId, report: &UsageReport) {
        todo!("#92")
    }

    /// `Alert` 반영. 같은 알림이 이미 있으면 더하지 않는다.
    pub fn apply_alert(&mut self, alert: Alert) {
        todo!("#92")
    }

    /// 살아 있는 작업 수(`labels::is_live`).
    pub fn live_tasks(&self) -> usize {
        todo!("#92")
    }

    /// 대기 줄 수(`Queued` 입력).
    pub fn queued_lines(&self) -> usize {
        todo!("#92")
    }

    /// 보류 줄 수(`Held` 작업과 `Held` 입력).
    pub fn held_lines(&self) -> usize {
        todo!("#92")
    }

    /// 이름표를 보일지(`labels::visible`).
    pub fn labels_visible(&self) -> bool {
        todo!("#92")
    }

    /// 살아 있는 작업이 있다. `Enter`, `Tab`, `Ctrl+C`의 뜻을 가른다.
    pub fn is_running(&self) -> bool {
        todo!("#92")
    }

    /// `Alt+↑`, `Shift+←` 대상: `Judging`이나 `Queued`인 입력 중 가장 최근 것.
    pub fn latest_recallable(&self) -> Option<&InputView> {
        todo!("#92")
    }

    /// 이름표로 대기 입력 찾기(`/send A`, `/cancel A`). 이름표가 없으면 가장 최근 `Queued` 입력.
    pub fn queued_by_label(&self, label: Option<TaskLabel>) -> Option<&InputView> {
        todo!("#92")
    }

    /// 이름표로 보류 작업 찾기(`/continue A`).
    pub fn held_by_label(&self, label: TaskLabel) -> Option<&TaskView> {
        todo!("#92")
    }

    /// 보류 작업을 이름표 순서로. 보류 재개 질문 목록.
    pub fn held_tasks(&self) -> Vec<&TaskView> {
        todo!("#92")
    }

    /// 다음 접수 순서 번호.
    fn next_seq(&mut self) -> u64 {
        todo!("#92")
    }
}
