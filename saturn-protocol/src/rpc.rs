//! TUI, CLI → engine 요청과 engine → TUI 알림. 메서드 이름은 variant 이름, `params`는 필드.
//! 설계: docs/design/engine-lifecycle.md, docs/design/tui.md
//! TODO(#46): 메서드 이름과 목록 확정

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::event::ProviderEvent;
use crate::ids::{
    ChatId, InputId, JudgmentId, LedgerSeq, Provider, SettingsRevision, TaskId, TaskLabel,
};
use crate::state::{Disposition, InputState, QueueReason, TaskState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    /// `chat`이 `None`이면 새 채팅.
    /// `overrides`(`-c key=value`)는 이 접속의 입력에만 적용한다.
    /// `workdir`는 폴더 설정 층 검색 위치이자 새 작업의 실행 위치.
    Attach {
        chat: Option<ChatId>,
        workdir: String,
        overrides: Vec<(String, String)>,
    },
    /// `before`보다 앞 기록 `limit`개.
    LoadHistory {
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    },
    RenameChat {
        chat: ChatId,
        name: String,
    },
    SetChatGroup {
        chat: ChatId,
        group: Option<String>,
    },
    /// 마지막 TUI가 떨어지면 `OnExit`를 적용한다.
    Detach,
    /// `skip_relation`이 참이면 관계 판단 없이 대기(실행 중 `Tab`).
    /// `client_ref`는 TUI가 매긴 번호. `InputAccepted`로 입력 id와 짝지어 돌아온다.
    SubmitInput {
        chat: ChatId,
        client_ref: u64,
        text: String,
        pinned_model: Option<String>,
        skip_relation: bool,
    },
    /// 아직 보내지 않은 입력을 새 작업으로 보낸다.
    RunAsNewTask {
        input: InputId,
    },
    SendNow {
        input: InputId,
    },
    /// 보내기 전 입력만.
    CancelInput {
        input: InputId,
    },
    /// 채팅의 모든 작업과 보내지 않은 입력을 보류한다.
    Stop {
        chat: ChatId,
    },
    /// `task`가 `None`이면 채팅의 보류 전부를 접수 순서로.
    Continue {
        chat: ChatId,
        task: Option<TaskId>,
    },
    ContinueInput {
        input: InputId,
    },
    /// 보내지 않은 입력은 취소하고 수정된 파일은 되돌리지 않는다.
    CloseHeld {
        chat: ChatId,
        task: TaskId,
    },
    AnswerPermission {
        request_id: String,
        answer: PermissionAnswer,
    },
    AnswerFeedback {
        judgment: JudgmentId,
        correct: bool,
    },
    /// 기록하지 않는다(judge 키).
    SubmitJudgeKey {
        key: String,
    },
    /// `apply`가 참이면 경로와 지문으로 신뢰를 기록한다.
    AnswerFolderTrust {
        path: String,
        fingerprint: String,
        apply: bool,
    },
    /// 끄면 그 채팅의 판단 기록을 저장하지 않는다.
    SetRecording {
        chat: ChatId,
        on: bool,
    },
    Usage {
        scope: UsageRange,
    },
    ListTasks,
    /// 채점 후보가 200건 미만이면 거절한다. `from`은 다시 학습할 judge 버전.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// 거짓이면 취소.
    ConfirmTrain {
        proceed: bool,
    },
    ListJudgeVersions,
    UseJudgeVersion {
        version: String,
    },
    /// `yes`가 거짓이면 미리보기만.
    Prune {
        yes: bool,
    },
    /// 채점하지 않은 기록도 내보낸다.
    ExportJudgments {
        path: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PermissionAnswer {
    Allow,
    /// 이 작업 동안 같은 명령 허용.
    AllowForTask,
    Deny,
    /// TODO(#56): 이어 받는 입력 처리 방식
    DenyAndRedirect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum UsageRange {
    Chat,
    Today,
    Week,
    All,
}

/// TUI는 이것만으로 화면을 그린다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    StartInfo {
        saturn_version: String,
        providers: Vec<(Provider, String)>,
        judge: String,
        judge_version: String,
        folder: String,
    },
    InputAccepted {
        client_ref: u64,
        input: InputId,
    },
    /// `text`는 다른 TUI가 보낸 입력에도 원문을 준다.
    InputChanged {
        input: InputId,
        text: String,
        label: Option<TaskLabel>,
        state: InputState,
        disposition: Option<Disposition>,
        reason: Option<QueueReason>,
    },
    /// `failure`는 실패 원인 한 줄.
    TaskChanged {
        task: TaskId,
        label: TaskLabel,
        state: TaskState,
        provider: Option<Provider>,
        elapsed_ms: u64,
        failure: Option<String>,
    },
    /// `LoadHistory`의 답. 항목은 실시간 알림과 같은 형식.
    HistoryChunk {
        chat: ChatId,
        entries: Vec<Notification>,
        has_more: bool,
    },
    PermissionRequested {
        task: TaskId,
        label: TaskLabel,
        provider: Provider,
        request_id: String,
        summary: String,
        reason: String,
        waiting: u32,
    },
    /// 다른 클라이언트가 먼저 답했다.
    PermissionResolved {
        request_id: String,
    },
    FolderTrustRequested {
        path: String,
        fingerprint: String,
        applied: Vec<String>,
        ignored: Vec<String>,
        changed_lines: Vec<String>,
    },
    JudgeKeyRequired {
        reason: String,
    },
    Commands {
        provider: Provider,
        commands: Vec<CommandInfo>,
    },
    TaskList {
        items: Vec<TaskListItem>,
    },
    Usage {
        range: UsageRange,
        rows: Vec<UsageRow>,
    },
    JudgeVersions {
        current: String,
        versions: Vec<JudgeVersionInfo>,
    },
    TrainPreview {
        candidates: u32,
        grader: String,
        estimated_tokens: u64,
        threshold_targets: Vec<String>,
        retrain_model: bool,
    },
    TrainProgress {
        stage: String,
        labeled: u32,
        elapsed_ms: u64,
        tokens: u64,
    },
    TaskEvent {
        task: TaskId,
        event: ProviderEvent,
    },
    ChatNotice {
        chat: ChatId,
        task: Option<TaskId>,
        notice: ChatNotice,
    },
    /// 8초 안에 답이 없으면 TUI가 지운다.
    FeedbackQuestion {
        judgment: JudgmentId,
        input: InputId,
        label: TaskLabel,
        disposition: Disposition,
    },
    /// 측정하지 못하면 `tokens`가 `None`.
    ContextSize {
        chat: ChatId,
        tokens: Option<u64>,
        threshold: u64,
    },
    /// 검사 실패면 이전 번호로 계속하고 `warning`을 채운다.
    SettingsApplied {
        revision: SettingsRevision,
        warning: Option<String>,
    },
    Alert {
        alert: Alert,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum ChatNotice {
    Compacted,
    ProviderSwitched {
        from: crate::ids::Provider,
        to: crate::ids::Provider,
    },
    Stopped {
        held: Vec<TaskLabel>,
    },
    /// 크래시 뒤 증명되지 않은 실행을 보류했다.
    ResumeSuggested {
        held: Vec<TaskLabel>,
    },
    /// provider 프로세스 묶음 밖에 남은 프로세스 수.
    StopUnconfirmed {
        remaining: u32,
    },
    /// 모든 작업이 끝난 순간의 합계.
    RequestSummary {
        provider_tokens: Vec<(crate::ids::Provider, u64)>,
        judge_calls: u32,
        judge_tokens: u64,
        elapsed_ms: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Alert {
    /// 질문별 대체 규칙을 적용 중.
    JudgePaused,
    /// 연속 3회 실패로 새 입력 접수를 멈췄다.
    IntakeStopped,
    /// 끼워 넣기 실측 전이라 대기로 처리한다.
    SteerNotReady {
        provider: crate::ids::Provider,
    },
    ChatBusyElsewhere {
        chat: ChatId,
    },
    /// judge 실패로 `[보내기]` 입력을 차례에 보낸다.
    JudgeDownSendingInOrder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct CommandInfo {
    /// `/` 없이.
    pub name: String,
    pub description: String,
    pub is_skill: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct TaskListItem {
    pub chat: ChatId,
    pub chat_name: String,
    pub group: Option<String>,
    pub task: TaskId,
    pub label: TaskLabel,
    pub state: TaskState,
    pub needs_permission: bool,
    /// 다른 Saturn이 실행 중이면 읽기 전용.
    pub busy_elsewhere: bool,
    /// subagent와 자식 채팅 수.
    pub children: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct UsageRow {
    /// 에이전트 이름이나 judge.
    pub who: String,
    /// 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론 순. 보고되지 않았으면 `None`.
    pub tokens: [Option<u64>; 5],
    pub judge_calls: u32,
    /// 단위: 마이크로 달러.
    pub estimated_cost_micros: Option<u64>,
    pub compactions: u32,
    pub labels: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct JudgeVersionInfo {
    pub version: String,
    pub judge: String,
    pub ece: Option<f64>,
    /// (질문, 목표 틀림 비율, 기준값, 최근 200건 틀림, 판단 수).
    pub questions: Vec<(String, f64, f64, u32, u32)>,
}
