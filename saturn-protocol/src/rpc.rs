//! TUI, CLI → engine 요청과 engine → TUI 알림. 메서드 이름은 variant 이름, `params`는 필드.
//! 설계: docs/design/engine-lifecycle.md, docs/design/tui.md

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::event::ProviderEvent;
use crate::ids::{
    ChatId, InputId, JudgmentId, LedgerSeq, Provider, SettingsRevision, TaskId, TaskLabel,
};
use crate::state::{Disposition, InputState, QueueReason, TaskState};

/// TUI가 `Attach`의 `env`에 담는 변수 이름. 이 밖의 변수는 보내지 않는다. 초안 목록.
pub const ATTACH_ENV_NAMES: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TMPDIR",
    "SSH_AUTH_SOCK",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];

#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    /// `chat`이 `None`이면 새 채팅.
    /// `overrides`(`-c key=value`)는 이 접속의 입력에만 적용한다.
    /// `workdir`는 폴더 설정 층 검색 위치이자 새 작업의 실행 위치.
    /// `env`는 이 TUI의 환경 변수(`ATTACH_ENV_NAMES`만)로, engine이 이 채팅의 provider 실행 환경으로 쓴다.
    /// `add_dirs`(`--add-dir`)는 절대 경로이고, 이 채팅에 폴더로 더한다(이미 있으면 그대로).
    Attach {
        chat: Option<ChatId>,
        workdir: String,
        env: Vec<(String, String)>,
        overrides: Vec<(String, String)>,
        #[serde(default)]
        add_dirs: Vec<String>,
    },
    /// 채팅의 provider session이 다룰 폴더를 더한다. `path`는 이미 있는 폴더의 절대 경로.
    /// 열려 있는 session에는 반영하지 못하고 다음 session부터 적용한다.
    AddDir {
        chat: ChatId,
        path: String,
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
        /// `/model`로 고른 모델. 있으면 provider도 이 모델이 정한다.
        pinned_model: Option<ModelChoice>,
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
    /// 기록하지 않는다(router 키).
    SubmitRouterKey {
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
    /// 채팅 층의 `permission.mode`를 바꾼다. 다음 허가 요청부터 새 모드로 판정한다.
    /// `mode`는 `ask`, `edit`, `read-only`, `full`.
    SetPermissionMode {
        chat: ChatId,
        mode: String,
    },
    /// `Chat` 범위는 붙은 채팅이고, 붙은 채팅이 없으면 `folder`의 가장 최근 채팅이다.
    Usage {
        scope: UsageRange,
        folder: Option<String>,
    },
    ListTasks,
    /// 고를 수 있는 모델 목록을 `Models`로 보낸다. `provider`가 `None`이면 모든 provider.
    /// `chat`은 Codex 목록을 받을 연결을 고른다.
    ListModels {
        chat: ChatId,
        provider: Option<Provider>,
    },
    /// 채점 후보가 200건 미만이면 거절한다. `from`은 다시 학습할 router 버전.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// 거짓이면 취소.
    ConfirmTrain {
        proceed: bool,
    },
    ListRouterVersions,
    UseRouterVersion {
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

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Request { .. }")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PermissionAnswer {
    /// 이 요청만 허용한다.
    AllowOnce,
    /// 같은 종류 호출을 앞으로도 허용한다. engine이 작업 폴더 단위로 저장한다.
    AllowAlways,
    /// `note`는 다르게 하라는 말. TODO(#56): 입력 방식이 정해지기 전에는 늘 `None`.
    Deny { note: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum UsageRange {
    Chat,
    /// 지금부터 24시간 전까지 모든 채팅.
    Day,
    /// 지금부터 7일 전까지 모든 채팅.
    Week,
}

/// TUI는 이것만으로 화면을 그린다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    StartInfo {
        saturn_version: String,
        providers: Vec<(Provider, String)>,
        router: String,
        router_version: String,
        /// 채팅의 기본 폴더.
        folder: String,
        /// 더한 폴더. 기본 폴더는 들어 있지 않다.
        #[serde(default)]
        added_dirs: Vec<String>,
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
    RouterKeyRequired {
        reason: String,
    },
    Commands {
        provider: Provider,
        commands: Vec<CommandInfo>,
    },
    TaskList {
        items: Vec<TaskListItem>,
    },
    /// `ListModels`의 답. provider 순서와 provider가 알려 준 순서를 지킨다.
    Models {
        models: Vec<ModelInfo>,
    },
    Usage {
        range: UsageRange,
        rows: Vec<UsageRow>,
    },
    RouterVersions {
        current: String,
        versions: Vec<RouterVersionInfo>,
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
        warning: Option<SettingsWarning>,
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
    /// 바뀐 권한 설정을 적용하려고 provider 연결을 다시 시작했다.
    ProviderRestarted {
        provider: crate::ids::Provider,
    },
    /// 권한 규칙이 바뀌었고 다음 요청부터 적용한다.
    PermissionsChanged,
    /// 크래시 뒤 증명되지 않은 실행을 보류했다.
    ResumeSuggested {
        held: Vec<TaskLabel>,
    },
    /// provider 프로세스 묶음 밖에 남은 프로세스 수.
    StopUnconfirmed {
        remaining: u32,
    },
    /// 폴더를 채팅에 더했다. 이미 열린 session에는 반영하지 못해 `applies_from_next_session`이면 다음 session부터 적용한다.
    FolderAdded {
        path: String,
        applies_from_next_session: bool,
    },
    /// 패킷의 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 못했다. 맥락 정리를 미루고 제약 목록을 보인다.
    ContextDeferred {
        constraints: Vec<String>,
    },
    /// 모든 작업이 끝난 순간의 합계.
    RequestSummary {
        provider_tokens: Vec<(crate::ids::Provider, u64)>,
        router_calls: u32,
        router_tokens: u64,
        elapsed_ms: u64,
    },
}

/// 설정을 적용하며 알릴 일. 문구는 TUI가 고른다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum SettingsWarning {
    /// 층의 검사가 실패해 `SettingsApplied`의 이전 번호로 계속한다.
    Fallback {
        layer: SettingsLayer,
        fault: SettingsFault,
    },
    /// 폴더 설정의 사용자 전용 키를 무시했다.
    IgnoredFolderKeys { keys: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum SettingsLayer {
    Default,
    User,
    Folder,
    Chat,
    Run,
}

/// 검사 실패 원인. `message`와 `reason`은 검사기가 낸 원문이라 번역하지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum SettingsFault {
    /// TOML을 읽지 못했다. `line`은 1부터.
    Parse {
        line: u32,
        message: String,
    },
    Invalid {
        key: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Alert {
    /// 질문별 대체 규칙을 적용 중.
    RouterPaused,
    /// 연속 3회 실패로 판단 모델 연결이 끊겼다. 입력은 계속 접수하고 현재 모델로 보낸다.
    RouterDisconnected,
    /// 끼워 넣기 실측 전이라 대기로 처리한다.
    SteerNotReady {
        provider: crate::ids::Provider,
    },
    ChatBusyElsewhere {
        chat: ChatId,
    },
    /// router 실패로 `[보내기]` 입력을 차례에 보낸다.
    RouterDownSendingInOrder,
    /// 시작할 때 기록 저장소 스키마를 이관했다. 첫 TUI에만 보낸다.
    SchemaMigrated {
        from: u32,
        to: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct CommandInfo {
    /// `/` 없이.
    pub name: String,
    pub description: String,
    pub is_skill: bool,
}

/// provider가 받는 모델 이름. 같은 이름이 두 provider에 있어도 provider로 구분한다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct ModelChoice {
    pub provider: Provider,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ModelInfo {
    pub choice: ModelChoice,
    /// 화면에 보일 이름. provider가 알려 주지 않으면 모델 이름과 같다.
    pub name: String,
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
    /// 채팅의 기본 폴더. TUI가 작업 목록의 폴더 범위를 가를 때 쓴다.
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct UsageRow {
    /// provider·모델이나 router. 예: `codex · gpt-5.6-terra`, `router · jev`.
    pub who: String,
    /// 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론 순. 보고되지 않았으면 `None`.
    pub tokens: [Option<u64>; 5],
    pub router_calls: u32,
    /// 단위: 마이크로 달러.
    pub estimated_cost_micros: Option<u64>,
    /// 기록에 없으면 `None`.
    pub compactions: Option<u32>,
    /// 기록에 없으면 `None`.
    pub labels: Option<u32>,
    /// 여러 턴의 합계인 행만 턴 수를 채운다. 한 턴이면 `None`.
    pub turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct RouterVersionInfo {
    pub version: String,
    pub router: String,
    pub ece: Option<f64>,
    /// (질문, 목표 틀림 비율, 기준값, 최근 200건 틀림, 판단 수).
    pub questions: Vec<(String, f64, f64, u32, u32)>,
}
