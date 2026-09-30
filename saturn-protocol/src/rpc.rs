//! TUI, CLI → engine 요청과 engine → TUI 알림. Unix 소켓 위 JSON-RPC 한 줄에 하나.
//!
//! 설계: docs/design/engine-lifecycle.md(TUI 접속), docs/design/tui.md(화면이 보내고 받는 것).
//! TODO(#46): 메서드 이름과 목록은 design 이슈 결정 뒤 확정. 지금 이름은 가칭
//! TODO(#75): 요청마다 `id`를 붙이는 JSON-RPC 2.0 봉투(`Envelope`)와 직렬화 테스트

use serde::{Deserialize, Serialize};

use crate::event::ProviderEvent;
use crate::ids::{
    ChatId, InputId, JudgmentId, LedgerSeq, Provider, SettingsRevision, TaskId, TaskLabel,
};
use crate::state::{Disposition, InputState, QueueReason, TaskState};

/// TUI나 CLI가 engine에 보내는 요청.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Request {
    /// 채팅에 붙는다. `chat`이 없으면 새 채팅. engine은 `StartInfo`, 최근 기록, 보관한 허가 요청 순서로 보낸다.
    /// `overrides`는 실행 층(`-c key=value`)이다. 이 접속의 입력에만 적용한다.
    Attach {
        chat: Option<ChatId>,
        overrides: Vec<(String, String)>,
    },
    /// 대화 기록 이전 부분(위로 스크롤). `before`보다 앞 기록을 `limit`개.
    LoadHistory {
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    },
    /// 채팅 이름 변경(작업 목록 `r`).
    RenameChat { chat: ChatId, name: String },
    /// 채팅 묶음 변경(작업 목록 `g`).
    SetChatGroup { chat: ChatId, group: Option<String> },
    /// 채팅에서 떨어진다. 마지막 TUI가 떨어지면 `OnExit`를 적용한다.
    Detach,
    /// 입력 제출. `Enter`는 `send_now: true`, 실행 중 `Tab`은 관계 판단 없이 대기.
    /// `client_ref`는 TUI가 매긴 번호로, engine이 `InputAccepted`로 입력 id와 짝지어 돌려준다.
    SubmitInput {
        chat: ChatId,
        client_ref: u64,
        text: String,
        pinned_model: Option<String>,
        skip_relation: bool,
    },
    /// 피드백 `아니에요` 뒤 바로잡기 제안의 `[실행]`. 아직 보내지 않은 입력을 새 작업으로 보낸다.
    RunAsNewTask { input: InputId },
    /// 대기 입력을 지금 보낸다(`[보내기]`, `/send`).
    SendNow { input: InputId },
    /// 보내기 전 입력 취소(`[취소]`, `/cancel`, `Alt+↑`).
    CancelInput { input: InputId },
    /// 멈춤(`Ctrl+C`). 채팅의 모든 작업과 보내지 않은 입력을 보류한다.
    Stop { chat: ChatId },
    /// 보류 재개(`/continue`). `task`가 없으면 채팅의 보류 전부를 접수 순서로.
    Continue { chat: ChatId, task: Option<TaskId> },
    /// 보류된 입력 하나만 대기열로 되돌린다.
    ContinueInput { input: InputId },
    /// 보류 종료. 보내지 않은 입력은 취소하고 수정된 파일은 되돌리지 않는다.
    CloseHeld { chat: ChatId, task: TaskId },
    /// 허가 요청 답.
    AnswerPermission {
        request_id: String,
        answer: PermissionAnswer,
    },
    /// 피드백 질문 답(`1` 맞아요, `2` 아니에요).
    AnswerFeedback { judgment: JudgmentId, correct: bool },
    /// judge 키 전달(키 입력 창). engine은 받은 키로 다시 확인하고 저장한다. 이 메시지는 기록하지 않는다.
    SubmitJudgeKey { key: String },
    /// 폴더 설정 신뢰 답. `apply`가 참이면 경로와 지문으로 신뢰를 기록한다.
    AnswerFolderTrust {
        path: String,
        fingerprint: String,
        apply: bool,
    },
    /// 사용량 조회(`/usage`).
    Usage { scope: UsageRange },
    /// 작업 목록 조회(`/tasks`).
    ListTasks,
    /// 판단 모델 학습(`/train`). 채점 후보가 200건 미만이면 거절한다. `from`은 다시 학습할 judge 버전.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// 학습 확인 창의 답. 거짓이면 취소.
    ConfirmTrain { proceed: bool },
    /// judge 버전 목록 조회(`/judge version`).
    ListJudgeVersions,
    /// 고른 judge 버전 사용(`u`, `saturn judge version`).
    UseJudgeVersion { version: String },
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
    /// 접속 직후 시작 화면 정보.
    StartInfo {
        saturn_version: String,
        providers: Vec<(Provider, String)>,
        judge: String,
        judge_version: String,
        folder: String,
    },
    /// `SubmitInput`을 접수했다. `client_ref`로 보낸 입력과 짝짓는다.
    InputAccepted { client_ref: u64, input: InputId },
    /// 입력 에코와 상태 변화. `label`은 합쳐진 작업의 이름표, `text`는 다른 TUI가 보낸 입력에도 원문을 준다.
    InputChanged {
        input: InputId,
        text: String,
        label: Option<TaskLabel>,
        state: InputState,
        disposition: Option<Disposition>,
        reason: Option<QueueReason>,
    },
    /// 작업 상태 변화. 상태판 줄과 결과 머리줄을 고친다. `failure`는 실패 원인 한 줄.
    TaskChanged {
        task: TaskId,
        label: TaskLabel,
        state: TaskState,
        provider: Option<Provider>,
        elapsed_ms: u64,
        failure: Option<String>,
    },
    /// 대화 기록 이전 부분. `LoadHistory`의 답이고, 항목은 실시간 알림과 같은 형식이다.
    HistoryChunk {
        chat: ChatId,
        entries: Vec<Notification>,
        has_more: bool,
    },
    /// 허가 요청. 1초 입력 보호 뒤 접수 순서로 하나씩 창을 띄운다.
    PermissionRequested {
        task: TaskId,
        label: TaskLabel,
        provider: Provider,
        request_id: String,
        summary: String,
        reason: String,
        waiting: u32,
    },
    /// 다른 클라이언트가 먼저 답했다. 창을 지운다.
    PermissionResolved { request_id: String },
    /// 폴더 설정 신뢰 요청. 실행 중이면 다음 입력 접수 전에 묻는다.
    FolderTrustRequested {
        path: String,
        fingerprint: String,
        applied: Vec<String>,
        ignored: Vec<String>,
        changed_lines: Vec<String>,
    },
    /// judge 확인 실패. 키 입력 창을 띄운다.
    JudgeKeyRequired { reason: String },
    /// provider 명령과 스킬 목록(`/`, `$` 팝업).
    Commands {
        provider: Provider,
        commands: Vec<CommandInfo>,
    },
    /// 작업 목록(`/tasks`)의 답.
    TaskList { items: Vec<TaskListItem> },
    /// 사용량(`/usage`)의 답. 행은 에이전트별.
    Usage {
        range: UsageRange,
        rows: Vec<UsageRow>,
    },
    /// judge 버전 목록의 답.
    JudgeVersions {
        current: String,
        versions: Vec<JudgeVersionInfo>,
    },
    /// 학습 확인 창 값.
    TrainPreview {
        candidates: u32,
        grader: String,
        estimated_tokens: u64,
        threshold_targets: Vec<String>,
        retrain_model: bool,
    },
    /// 학습 진행 줄(`⠼ [학습]`).
    TrainProgress {
        stage: String,
        labeled: u32,
        elapsed_ms: u64,
        tokens: u64,
    },
    /// 작업의 provider 이벤트. 작업별 출력 칸과 도구 셀을 그린다.
    TaskEvent { task: TaskId, event: ProviderEvent },
    /// 대화 기록 한 줄 알림(`맥락 정리 후 이어서 진행`, `codex → claude로 전환` 등).
    ChatNotice {
        chat: ChatId,
        task: Option<TaskId>,
        notice: ChatNotice,
    },
    /// 피드백 질문. 8초 안에 답이 없으면 TUI가 지운다.
    /// `disposition`으로 문구를 고르고(`[A]에 이어서 보냈어요` 등), 입력이 아직 안 보내졌으면 `아니에요` 뒤 바로잡기 제안을 보인다.
    FeedbackQuestion {
        judgment: JudgmentId,
        input: InputId,
        label: TaskLabel,
        disposition: Disposition,
    },
    /// 바닥줄 맥락 크기. 측정하지 못하면 `tokens`가 `None`(`맥락 미확인`).
    ContextSize {
        chat: ChatId,
        tokens: Option<u64>,
        threshold: u64,
    },
    /// 설정 적용 결과. 검사 실패면 이전 번호로 계속하고 경고한다.
    SettingsApplied {
        revision: SettingsRevision,
        warning: Option<String>,
    },
    /// engine 경고 줄(`자동 판단 일시 중단`, `새 입력 접수 중단 · ...` 등).
    Alert { alert: Alert },
}

/// 대화 기록에 남는 한 줄 알림 종류.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatNotice {
    /// 맥락 정리 뒤 같은 작업 계속.
    Compacted,
    /// 작업의 provider 전환.
    ProviderSwitched {
        from: crate::ids::Provider,
        to: crate::ids::Provider,
    },
    /// 멈춤 결과. 보류된 이름표 목록.
    Stopped { held: Vec<TaskLabel> },
    /// 멈춤 뒤 provider 프로세스 묶음 밖에 남은 프로세스 수.
    StopUnconfirmed { remaining: u32 },
    /// 모든 작업이 끝난 순간의 합계(`이번 요청 · ...`).
    RequestSummary {
        provider_tokens: Vec<(crate::ids::Provider, u64)>,
        judge_calls: u32,
        judge_tokens: u64,
        elapsed_ms: u64,
    },
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
    /// `[보내기]`를 눌렀으나 judge가 실패해 차례에 보낸다.
    JudgeDownSendingInOrder,
}

/// provider 명령이나 스킬 하나.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInfo {
    /// 이름(`/` 없이).
    pub name: String,
    /// 설명.
    pub description: String,
    /// 스킬이면 참.
    pub is_skill: bool,
}

/// 작업 목록 한 행.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskListItem {
    /// 채팅.
    pub chat: ChatId,
    /// 채팅 이름.
    pub chat_name: String,
    /// 묶음.
    pub group: Option<String>,
    /// 작업.
    pub task: TaskId,
    /// 이름표.
    pub label: TaskLabel,
    /// 상태.
    pub state: TaskState,
    /// 허가가 필요하면 참(`!`).
    pub needs_permission: bool,
    /// 다른 Saturn이 실행 중이면 참(읽기 전용).
    pub busy_elsewhere: bool,
    /// subagent와 자식 채팅 수.
    pub children: u32,
}

/// 사용량 화면 한 행.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRow {
    /// 에이전트 이름이나 judge.
    pub who: String,
    /// 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론 토큰. 보고되지 않았으면 `None`.
    pub tokens: [Option<u64>; 5],
    /// judge 호출 수와 예상 비용(마이크로 달러).
    pub judge_calls: u32,
    /// 예상 비용.
    pub estimated_cost_micros: Option<u64>,
    /// 맥락 정리 횟수.
    pub compactions: u32,
    /// 채점 수.
    pub labels: u32,
}

/// judge 버전 한 행.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeVersionInfo {
    /// 버전 이름.
    pub version: String,
    /// judge와 보정값.
    pub judge: String,
    /// ECE.
    pub ece: Option<f64>,
    /// 질문별 목표 틀림 비율, 기준값, 최근 200건 틀림, 판단 수.
    pub questions: Vec<(String, f64, f64, u32, u32)>,
}
