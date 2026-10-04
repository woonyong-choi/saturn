//! TUI, CLI → engine 요청과 engine → TUI 알림. 메서드 이름은 variant 이름, `params`는 필드.
//! 설계: docs/design/engine-lifecycle.md, docs/design/tui.md

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::event::ProviderEvent;
use crate::ids::{
    ChatId, ConstraintAskId, InputId, JudgmentId, LedgerSeq, Provider, SettingsRevision, TaskId,
    TaskLabel,
};
use crate::input::{InputAnswer, InputRequest};
use crate::state::{Disposition, InputState, QueueReason, TaskState};

/// engine과 클라이언트가 주고받는 메시지 판. 요청이나 알림의 모양을 호환되지 않게 바꿀 때 올린다.
pub const PROTOCOL_VERSION: u32 = 2;

/// engine이 에이전트 작업의 환경에 넣는 출입증 변수 이름. `saturn`이 이 값으로 `AttachChild`를 보낸다.
pub const PASS_ENV: &str = "SATURN_PASS";

/// engine이 에이전트 작업의 환경에 넣는 engine 소켓 경로 변수 이름. 없으면 기본 경로를 쓴다.
pub const SOCKET_ENV: &str = "SATURN_ENGINE_SOCKET";

/// `AttachChild`를 거절한 오류 번호(JSON-RPC 서버 오류 범위). 출입증이 없거나 회수됐거나 상한과 부모 권한을 넘었다.
pub const CHILD_REJECTED: i32 = -32002;

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
    /// 에이전트 작업 안의 `saturn`이 출입증(`pass`)으로 붙는다. engine은 출입증을 준 채팅의 하위 작업으로 새 채팅을 만들어
    /// 붙인다. 작업 폴더, 더한 폴더, 환경, 권한 규칙은 부모 채팅의 것을 물려받아 요청으로 바꿀 수 없다.
    /// `mode`는 부모 모드를 넘지 않는 모드이고 없으면 부모 모드다. 동시 상한이 차면 `ChildQueued`를 보내고 자리가 날 때까지
    /// 응답하지 않는다. 출입증이 없거나 회수됐거나 깊이 상한을 넘거나 부모 모드를 넘으면 거절한다. 응답과 알림 순서는 `Attach`와 같다.
    AttachChild {
        pass: String,
        mode: Option<String>,
    },
    /// 채팅의 provider session이 다룰 폴더를 더한다. `path`는 이미 있는 폴더의 절대 경로.
    /// 열려 있는 session에는 반영하지 못하고 다음 session부터 적용한다.
    AddDir {
        chat: ChatId,
        path: String,
    },
    /// 확장을 Saturn 확장 저장소에 설치한다. `source`는 로컬 폴더 경로나 git 저장소 주소다.
    /// 결과는 `chat`의 대화 기록에 한 줄로 남고, 설치와 제거는 다음 session부터 적용한다.
    InstallExtension {
        chat: ChatId,
        source: String,
    },
    /// 설치한 확장을 지운다. 없는 이름이면 이유를 `chat`의 대화 기록에 남긴다.
    RemoveExtension {
        chat: ChatId,
        name: String,
    },
    /// 설치한 확장 목록을 `ExtensionList` 알림으로 요청한 접속에 보낸다.
    ListExtensions,
    /// `before`(앞서 받은 결과의 `oldest`)보다 앞 기록 `limit`단위를 `QueryResult::History`로 돌려준다. 없으면 가장 최근부터.
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
    /// engine의 빌드 버전과 protocol 판을 `EngineVersion`으로 보낸다. `Attach` 전에도, router 키를 기다리는 동안에도 받는다.
    /// 클라이언트는 `Attach` 전에 이 값을 자기 값과 비교해 옛 engine을 교체한다.
    Version,
    /// engine을 업데이트하려고 끝낸다. 새 요청을 받지 않고, 실행 중인 작업은 끝내지 않은 채 provider 프로세스를 정리한 뒤
    /// 잠금을 풀고 끝난다. 끝나지 않은 실행은 다음 engine의 크래시 복구가 이어 받는다. 붙은 TUI에는 `Alert::EngineRestarting`을 보낸다.
    Shutdown,
    /// 연결을 끊는다. 마지막 TUI가 떨어지면 `OnExit`를 적용한다.
    /// 채팅을 옮길 때는 보내지 않고 같은 연결로 `Attach`를 다시 보낸다.
    Detach,
    /// TUI를 닫기 전에 보낸다. 닫은 뒤의 처리를 `QueryResult::ExitPlan`으로 돌려준다.
    PrepareExit {
        chat: ChatId,
    },
    /// `skip_relation`이 참이면 관계 판단 없이 대기(실행 중 `Tab`).
    /// `client_ref`는 TUI가 매긴 번호. `InputAccepted`로 입력 id와 짝지어 돌아온다.
    SubmitInput {
        chat: ChatId,
        client_ref: u64,
        text: String,
        skip_relation: bool,
    },
    /// router 판단 없이 `task`에 바로 끼워 넣는 입력. 허가를 거절하고 이어 쓴 말처럼 대상이 분명할 때 쓴다.
    /// `task`가 이미 끝났거나 없으면 `SubmitInput`처럼 판단을 받는다. `client_ref`는 `SubmitInput`과 같다.
    SubmitToTask {
        chat: ChatId,
        client_ref: u64,
        task: TaskId,
        text: String,
    },
    /// 아직 보내지 않은 입력을 새 작업으로 보낸다.
    RunAsNewTask {
        input: InputId,
    },
    SendNow {
        input: InputId,
    },
    /// 끼워 넣기를 받지 않은 충돌 입력(`QueueReason::ConfirmStop`)에 답한다. `stop`이 참이면 멈춘 뒤 그 입력을
    /// 실행하고, 거짓이면 대기열 맨 앞에서 현재 작업이 끝나길 기다린다.
    AnswerStopConfirm {
        input: InputId,
        stop: bool,
    },
    /// 보내기 전 입력만.
    CancelInput {
        input: InputId,
    },
    /// 채팅의 모든 작업과 보내지 않은 입력을 보류한다.
    Stop {
        chat: ChatId,
    },
    /// 모든 채팅의 작업과 보내지 않은 입력을 보류한다. `ask`에서 멈춤을 고른 종료에 쓴다.
    StopAll,
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
    /// 입력 요청에 답한다. `request_id`는 `InputRequested`의 값.
    AnswerInput {
        request_id: String,
        answer: InputAnswer,
    },
    AnswerFeedback {
        judgment: JudgmentId,
        correct: bool,
    },
    /// 제약 확인(`ConstraintAsked`)에 답한다. 이미 답했거나 대상이 바뀌어 닫힌 확인이면 거절한다.
    AnswerConstraintAsk {
        ask: ConstraintAskId,
        answer: ConstraintAskAnswer,
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
    /// 사용량을 `QueryResult::Usage`로 돌려준다. `Chat` 범위는 붙은 채팅이고, 붙은 채팅이 없으면 `folder`의 가장 최근 채팅이다.
    Usage {
        scope: UsageRange,
        folder: Option<String>,
    },
    /// 작업 목록을 `QueryResult::Tasks`로 돌려준다.
    ListTasks,
    /// `folder`에서 마지막 입력 접수가 가장 늦은 채팅(입력이 없으면 만든 시각)을 `QueryResult::LatestChat`으로 돌려준다.
    /// 붙지 않은 연결에서도 쓴다.
    LatestChat {
        folder: String,
    },
    /// `folder`의 채팅 목록을 `QueryResult::Chats`로 돌려준다. `folder`가 `None`이면 모든 폴더. 최근에 쓴 채팅이 앞이고
    /// 순서는 `LatestChat`과 같다. 붙지 않은 연결에서도 쓴다.
    ListChats {
        folder: Option<String>,
    },
    /// 채팅이 다시 바꿀 때까지 쓸 모델을 정하고 저장한다. 모델이 provider도 정한다. 붙은 모든 TUI에 `ModelPinned`로 알린다.
    SetModel {
        chat: ChatId,
        model: ModelChoice,
    },
    /// 기본 모델을 정해 사용자 설정(`model.default`)에 저장한다. 붙은 모든 TUI에 `ModelSettings`로 알린다.
    SetDefaultModel {
        chat: ChatId,
        model: ModelChoice,
    },
    /// 모델 선택 방식을 정해 사용자 설정(`model.mode`)에 저장한다. 붙은 모든 TUI에 `ModelSettings`로 알린다.
    SetModelMode {
        chat: ChatId,
        mode: ModelMode,
    },
    /// 고를 수 있는 모델 목록을 `QueryResult::Models`로 돌려준다. `provider`가 `None`이면 모든 provider.
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
    /// `QueryResult::RouterVersions`로 돌려준다.
    ListRouterVersions,
    UseRouterVersion {
        version: String,
    },
    /// `retention.max_age_days`보다 오래 쓰지 않은 채팅을 정리한다. `yes`가 거짓이면 아무것도 지우지 않고
    /// `QueryResult::PrunePreview`로, 참이면 지우고 `QueryResult::Pruned`로 돌려준다. 설정이 없으면 거절한다.
    /// `plan`은 미리보기가 돌려준 번호다. `yes`가 참이고 `plan`이 있으면 그 미리보기에 있던 채팅만 지운다.
    /// 모르거나 만료됐거나 이미 쓴 번호면 거절한다. `plan`이 없으면 `all`이 참일 때만 요청 순간의 기준으로 대상을 정해
    /// 지우고, `all`도 없으면 아무것도 지우지 않고 거절한다. 미리보기 확인을 모르는 옛 클라이언트의 `yes`가 지금 대상
    /// 전체의 삭제로 읽히지 않게 하기 위해서다.
    Prune {
        yes: bool,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        all: bool,
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

/// 등록을 묻는 확인의 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum ConstraintAskAnswer {
    /// 제약으로 등록한다.
    Yes,
    /// 등록하지 않는다.
    No,
}

/// TUI는 이것만으로 화면을 그린다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    /// `Version`의 답.
    EngineVersion {
        saturn_version: String,
        protocol_version: u32,
    },
    StartInfo {
        saturn_version: String,
        /// 옛 engine이 보낸 값에는 없어 0으로 읽는다.
        #[serde(default)]
        protocol_version: u32,
        /// 어댑터 레지스트리의 기본 순서대로.
        providers: Vec<ProviderInfo>,
        router: String,
        router_version: String,
        /// 채팅의 기본 폴더.
        folder: String,
        /// 더한 폴더. 기본 폴더는 들어 있지 않다.
        #[serde(default)]
        added_dirs: Vec<String>,
    },
    /// `AttachChild`가 상한 때문에 대기열에 섰다. `position`은 1부터 센 자리이고 자리가 나면 `Attach`와 같은 알림이 이어진다.
    ChildQueued {
        position: u32,
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
    /// `Attach` 직후 보내는 최근 기록. 항목은 실시간 알림과 같은 형식. `LoadHistory`의 답은 `QueryResult::History`.
    /// `oldest`는 이 묶음에서 가장 오래된 기록 위치로, 더 앞을 받으려면 `LoadHistory`의 `before`에 그대로 보낸다.
    /// 묶음이 비면 `None`. 더 앞 기록이 없으면 `has_more`가 거짓이니 더 요청하지 않는다.
    HistoryChunk {
        chat: ChatId,
        entries: Vec<Notification>,
        #[serde(default)]
        oldest: Option<LedgerSeq>,
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
    /// provider의 입력 요청. 허가 요청처럼 답이 올 때까지 두고 나중에 붙는 TUI도 받는다.
    InputRequested {
        task: TaskId,
        label: TaskLabel,
        provider: Provider,
        request_id: String,
        request: InputRequest,
        waiting: u32,
    },
    /// 다른 클라이언트가 먼저 답했거나 요청이 끝났다.
    InputResolved {
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
    /// 채팅의 고정 모델. 고정한 채팅에 붙을 때와 고정을 바꿀 때 보낸다.
    ModelPinned {
        chat: ChatId,
        model: ModelChoice,
    },
    /// 기본 모델과 모델 선택 방식. 채팅에 붙을 때와 설정이 바뀔 때 보낸다. `default`가 `None`이면 기본 모델을 아직 고르지 않았다.
    ModelSettings {
        chat: ChatId,
        default: Option<ModelChoice>,
        mode: ModelMode,
    },
    /// 채팅 이름이나 묶음이 바뀌었다. 같은 engine에 붙은 모든 TUI에 보낸다. 값은 저장한 결과이고
    /// 비어 있으면 `None`이다.
    ChatLabeled {
        chat: ChatId,
        name: Option<String>,
        group: Option<String>,
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
    /// 이 말을 앞으로 지킬 제약으로 등록할지 묻는다. 허가 요청처럼 답이 올 때까지 두고 나중에 붙는 TUI도 받는다.
    ConstraintAsked {
        ask: ConstraintAskId,
        chat: ChatId,
        /// 등록할 규칙. 사용자 원문에서 자른 글이고 한 입력에서 여러 건이면 줄바꿈으로 잇는다.
        rule: String,
    },
    /// 다른 클라이언트가 먼저 답했거나 대상이 바뀌어 확인이 닫혔다.
    ConstraintAskResolved {
        ask: ConstraintAskId,
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
        /// 병합한 설정의 `tui.keymap`. 검사 실패로 이전 번호를 쓰면 `None`이다.
        #[serde(default)]
        keymap: Option<String>,
        /// 병합한 설정의 `tui.screen`(`auto`, `full`, `plain`). 검사 실패로 이전 번호를 쓰면 `None`이다.
        #[serde(default)]
        screen: Option<String>,
    },
    Alert {
        alert: Alert,
    },
}

/// 조회 요청의 답. 응답의 `result`에 실려 요청을 보낸 접속에만 간다. 명령 요청의 `result`는 `null`.
/// 요청과 답: `LoadHistory`→`History`, `Usage`→`Usage`, `ListTasks`→`Tasks`, `LatestChat`→`LatestChat`,
/// `ListChats`→`Chats`, `ListModels`→`Models`, `ListRouterVersions`→`RouterVersions`,
/// `PrepareExit`→`ExitPlan`, `Prune`→`PrunePreview`(`yes`가 거짓)나 `Pruned`(`yes`가 참).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", content = "data")]
pub enum QueryResult {
    /// 항목은 실시간 알림과 같은 형식. `oldest`는 이 묶음에서 가장 오래된 기록 위치로, 더 앞을 받으려면
    /// `LoadHistory`의 `before`에 그대로 보낸다. 묶음이 비면 `None`.
    History {
        chat: ChatId,
        entries: Vec<Notification>,
        #[serde(default)]
        oldest: Option<LedgerSeq>,
        has_more: bool,
    },
    Usage {
        range: UsageRange,
        rows: Vec<UsageRow>,
    },
    Tasks {
        items: Vec<TaskListItem>,
    },
    /// 폴더에 채팅이 없으면 `None`.
    LatestChat {
        chat: Option<ChatId>,
    },
    Chats {
        chats: Vec<ChatListItem>,
    },
    /// provider 순서와 provider가 알려 준 순서를 지킨다.
    Models {
        models: Vec<ModelInfo>,
    },
    RouterVersions {
        current: String,
        versions: Vec<RouterVersionInfo>,
    },
    /// 닫은 뒤의 처리.
    ExitPlan {
        plan: ExitPlan,
    },
    /// `Prune`의 `yes`가 거짓일 때. 지울 채팅과 남길 채팅. 아무것도 지우지 않았다.
    PrunePreview {
        chats: Vec<ChatListItem>,
        skipped: Vec<PruneSkipped>,
        /// 지울 채팅의 입력, 실행, 이벤트, 사용량, session 행 수. 판단 기록과 설정 스냅샷은 세지 않는다.
        rows: u64,
        /// 이 미리보기의 번호. `Prune { yes: true, plan }`에 실어 보내면 이 목록의 채팅만 지운다. 한 번만 쓸 수 있다.
        plan: String,
    },
    /// `Prune`의 `yes`가 참일 때. 지운 채팅과 남긴 채팅.
    Pruned {
        chats: Vec<ChatListItem>,
        skipped: Vec<PruneSkipped>,
        rows: u64,
    },
    /// `ListExtensions`의 답. 설치한 순서대로.
    ExtensionList {
        extensions: Vec<ExtensionInfo>,
    },
}

/// 설치한 확장 하나. 부분마다 provider별 판정을 가진다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ExtensionInfo {
    pub name: String,
    /// 설치할 때 사용자가 준 원천 글자.
    pub source: String,
    /// 설치 시각(unix 밀리초).
    pub installed_at_ms: u64,
    pub parts: Vec<ExtensionPart>,
}

/// 확장을 나눈 부분 하나.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ExtensionPart {
    pub kind: ExtensionPartKind,
    pub name: String,
    /// 등록한 어댑터마다 하나. 어댑터 등록 순서대로.
    pub verdicts: Vec<(Provider, Injectability)>,
}

/// 확장 부분의 종류. 어댑터가 부분을 주입할 수 있는지 종류별로 답한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum ExtensionPartKind {
    /// `SKILL.md`가 든 폴더.
    Skill,
    /// MCP 서버 정의 하나.
    McpServer,
    /// 프롬프트 파일 하나.
    Command,
    /// 훅 정의 하나.
    Hook,
}

/// provider 하나에 부분을 주입할 수 있는지에 대한 어댑터의 답. `Unknown`은 `Unavailable`과 같게 알리되 어댑터가 바뀌면 다시 판정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub enum Injectability {
    Injectable,
    Unavailable,
    Unknown,
}

/// TUI를 닫을 때 할 일. `running`은 계속 처리될 작업 수다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum ExitPlan {
    /// 묻지 않고 닫는다.
    Close,
    /// 계속할지 멈출지 묻는다.
    Ask { running: u32 },
    /// 닫으면 작업이 계속된다는 한 줄을 보이고 닫는다.
    Notice { running: u32 },
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
    /// 읽기 전용으로 접수한 실행이 쓰기 허가를 요청해 접수 때 권한대로 거부했다. 쓰려면 새 입력으로 보내야 한다.
    ReadOnlyRunKept,
    /// 크래시 뒤 증명되지 않은 실행을 보류했다.
    ResumeSuggested {
        held: Vec<TaskLabel>,
    },
    /// provider의 MCP 서버를 쓸 수 없다. `reasons`는 서버마다 한 줄이다. 그 서버의 도구만 못 쓰고 입력은 막지 않는다.
    McpUnavailable {
        provider: crate::ids::Provider,
        reasons: Vec<String>,
    },
    /// 크래시로 끊긴 하위 에이전트의 이벤트가 provider에서 다시 왔다. 채팅의 작업을 바로 멈췄다.
    InterruptedSubagentReturned {
        provider: crate::ids::Provider,
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
    /// 확장을 설치했다. 부분마다 provider별 판정은 `extension`에 있다.
    ExtensionInstalled {
        extension: ExtensionInfo,
    },
    /// 확장을 지웠다. 열린 session은 그대로이고 다음 session부터 빠진다.
    ExtensionRemoved {
        name: String,
    },
    /// provider가 바뀌어 새 provider에 적용되지 않는 확장 부분이다. 바뀌기 전 provider에는 주입했고 새 provider는 받지
    /// 못하는 부분만 담는다. `ProviderSwitched` 바로 뒤에 보낸다.
    ExtensionPartsNotApplied {
        provider: crate::ids::Provider,
        /// 확장 이름, 부분 종류, 부분 이름.
        parts: Vec<(String, ExtensionPartKind, String)>,
    },
    /// 설치한 확장의 부분을 주입하지 못했다. 나머지 부분은 주입한 채 연결을 시작했다. `part`가 `None`이면 확장 전체의
    /// 원본이 확장 저장소에서 사라진 것이다. `reason`은 engine이나 어댑터가 낸 원문이라 번역하지 않는다.
    ExtensionInjectFailed {
        extension: String,
        part: Option<String>,
        provider: crate::ids::Provider,
        reason: String,
    },
    /// 설치하거나 지우지 못했다. 기존 설치는 바뀌지 않았다. `reason`은 engine이 낸 원문이라 번역하지 않는다.
    ExtensionFailed {
        name: Option<String>,
        reason: String,
    },
    /// 패킷의 고정 구역이 `P_hard`도 넘어 새 session으로 옮기지 못했다. 맥락 정리를 미루고 제약 목록을 보인다.
    ContextDeferred {
        constraints: Vec<String>,
    },
    /// 새 session의 패킷이 맥락 한도로 거절됐고, 경쟁 구역을 줄여 다시 보내도 들어가지 않거나 고정 구역만으로 넘쳐 보내지 않고 멈췄다.
    PacketOverflow,
    /// 제약을 등록했다. 규칙은 사용자 원문 그대로다. `unconfirmed`는 권한 모드 `full`이라 묻지 않고 등록했다는 뜻이다.
    ConstraintAdded {
        rule: String,
        unconfirmed: bool,
    },
    /// 제약을 해제했다. 입력을 취소해 그 입력의 제약을 함께 해제한 경우도 같다.
    ConstraintReleased {
        rule: String,
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
    /// 옛 이름으로 적힌 키를 새 이름으로 읽었다. `(옛 이름, 새 이름)`.
    RenamedKeys { keys: Vec<(String, String)> },
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
    SteerNotReady { provider: crate::ids::Provider },
    /// router 실패로 `[보내기]` 입력을 차례에 보낸다.
    RouterDownSendingInOrder,
    /// 시작할 때 기록 저장소 스키마를 이관했다. 첫 TUI에만 보낸다.
    SchemaMigrated { from: u32, to: u32 },
    /// 시작할 때 자동 정리가 오래된 채팅을 지웠다. 첫 TUI에만 보낸다.
    AutoPruned { chats: u32, rows: u64 },
    /// 시작할 때 자동 정리가 실패했다. 아무것도 지우지 않았다. 첫 TUI에만 보낸다.
    AutoPruneFailed,
    /// `Prune`을 보냈는데 `retention.max_age_days`가 없어 거절했다. 요청한 접속에만 보낸다.
    PruneNeedsRetention,
    /// 업데이트를 적용하려고 옛 engine을 끝내고 이 engine을 새로 시작했다. 첫 TUI에만 보낸다.
    EngineRestarted,
    /// 업데이트를 적용하려고 이 engine이 끝난다. 붙은 모든 TUI에 보내고 곧 연결을 끊는다. TUI는 다시 열어야 한다.
    EngineRestarting,
    /// 시작할 때 읽은 provider CLI 버전이 마지막으로 확인한 버전과 다르다. 첫 TUI에만 보낸다.
    ProviderUpdated {
        provider: crate::ids::Provider,
        from: String,
        to: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct CommandInfo {
    /// `/` 없이.
    pub name: String,
    pub description: String,
    pub is_skill: bool,
}

/// 붙을 때 알리는 provider 하나. 어댑터 설명자에서 온 값이다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProviderInfo {
    pub provider: Provider,
    /// 화면과 사용량에 보이는 이름.
    pub display_name: String,
    /// 확인하지 못했으면 빈 글자.
    pub version: String,
}

/// provider가 받는 모델 이름. 같은 이름이 두 provider에 있어도 provider로 구분한다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct ModelChoice {
    pub provider: Provider,
    pub model: String,
}

/// 새 작업의 모델을 누가 고르는지. 오토는 router, 매뉴얼은 사용자(기본 모델이나 `/model`로 고정한 모델).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelMode {
    Auto,
    Manual,
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
    /// 작업 행과 끝난 작업 행에만 있다. `None`이면 작업 없는 채팅 행.
    pub task: Option<TaskId>,
    /// 작업 글자. 끝난 작업(글자를 돌려줬다)과 채팅 행은 `None`.
    pub label: Option<TaskLabel>,
    /// 채팅 행은 `None`. `Done`과 `Failed`는 끝난 작업 행.
    pub state: Option<TaskState>,
    pub needs_permission: bool,
    /// 다른 Saturn이 실행 중이면 읽기 전용.
    pub busy_elsewhere: bool,
    /// subagent와 자식 채팅 수.
    pub children: u32,
    /// 채팅의 기본 폴더. TUI가 작업 목록의 폴더 범위를 가를 때 쓴다.
    #[serde(default)]
    pub folder: Option<String>,
    /// 이 행으로 갈 대기 입력을 접수 순서로. 갈 작업이 없는 새 대기 입력은 채팅 행에 붙는다.
    #[serde(default)]
    pub queued: Vec<InputId>,
    /// 그 작업 session이 지금 쓰는 모델. 모르면 `None`.
    #[serde(default)]
    pub model: Option<String>,
    /// 끝난 작업 행에서 마지막 실행이 끝난 시각(unix 밀리초).
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
}

/// 정리하지 않고 남긴 채팅과 그 이유.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct PruneSkipped {
    pub chat: ChatId,
    /// 이유가 여럿이면 모두.
    pub reasons: Vec<PruneSkipReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum PruneSkipReason {
    OpenInput,
    OpenRun,
    /// 멈춤 요청을 처리하는 중.
    PendingStop,
    /// 열린 session이 있다.
    ActiveSession,
    /// 보관했거나 보류한 session이 있다.
    WaitingSession,
    /// TUI가 붙어 있다.
    Attached,
    /// 미리본 뒤에 다시 쓰여 정리 기한 안으로 돌아왔다.
    UsedSincePreview,
}

/// 채팅 목록의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ChatListItem {
    pub chat: ChatId,
    /// 채팅의 기본 폴더.
    pub folder: String,
    /// `RenameChat`으로 붙인 이름. 붙이지 않았으면 `None`.
    #[serde(default)]
    pub name: Option<String>,
    /// 마지막 입력을 접수한 시각(unix 밀리초). 입력이 없으면 채팅을 만든 시각.
    pub last_active_ms: u64,
    /// 채팅의 첫 입력 원문. 입력이 없으면 `None`.
    pub preview: Option<String>,
    /// 지울(지운) 채팅의 기록 행 수. 정리 응답에서만 채우고 `ListChats`에서는 `None`.
    #[serde(default)]
    pub rows: Option<u64>,
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
