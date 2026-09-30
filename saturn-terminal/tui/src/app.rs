//! 화면 상태와 이벤트 루프: 키 입력, engine 알림, 틱.
//!
//! 설계: docs/design/tui.md, docs/design/input-handling.md(대기와 취소, 멈춤과 보류), docs/design/engine-lifecycle.md(TUI 종료).
//! `App`은 입출력을 하지 않는다. 이벤트를 받아 상태를 고치고 할 일(`Effect`)을 돌려주면 `run_loop`가 실행한다.
//! 키는 `key_area`가 고른 영역에서 `keys::map`으로 `Action`이 되고 `on_action`이 처리한다.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEvent, MouseEvent};
use ratatui::Frame;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, ChatNotice, Notification, Request};
use saturn_protocol::state::{Disposition, InputState, QueueReason, TaskState};

use crate::TuiError;
use crate::client::EngineClient;
use crate::commands::SlashCommand;
use crate::history::InputHistory;
use crate::i18n::Lang;
use crate::keys::{Action, KeyArea, KeyContext};
use crate::shell::ShellOutput;
use crate::state::ChatState;
use crate::terminal::Screen;
use crate::view::composer::Composer;
use crate::view::folder_trust::FolderTrust;
use crate::view::full_transcript::FullTranscript;
use crate::view::judge_key_prompt::JudgeKeyPrompt;
use crate::view::judge_version::JudgeVersionScreen;
use crate::view::live_area::LiveArea;
use crate::view::permission::PermissionQueue;
use crate::view::popup::{Popup, PopupSuppress};
use crate::view::resume_prompt::ResumePrompt;
use crate::view::start_screen::StartInfo;
use crate::view::status_board::Button;
use crate::view::task_list::TaskList;
use crate::view::train_confirm::TrainConfirm;
use crate::view::transcript::Transcript;
use crate::view::usage::UsageScreen;

/// 틱 간격. 스피너, 경과 시간, 피드백 8초, 허가 1초 보호를 갱신한다. 초안 값이다(설계에 없음).
/// TODO(#92): 값 미정, 초안 100ms
pub const TICK: Duration = Duration::from_millis(100);
/// 피드백 질문이 답 없이 떠 있는 시간.
pub const FEEDBACK_TIMEOUT: Duration = Duration::from_secs(8);

/// 이벤트 루프가 받는 것.
#[derive(Debug)]
pub enum AppEvent {
    /// 키, 마우스, 붙여넣기, 화면 크기 변경.
    Terminal(Event),
    /// engine 알림.
    Engine(Notification),
    /// engine 연결이 끝났다(`EngineClient::next`가 `None`).
    EngineClosed,
    /// 틱.
    Tick,
    /// `!` 셸 명령이 끝났다.
    ShellDone(ShellOutput),
}

/// `App`이 돌려주는 할 일. `run_loop`가 순서대로 실행한다.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// engine에 요청을 보낸다.
    Send(Request),
    /// `Ctrl+Z` 화면 일시 중지.
    Suspend,
    /// `Ctrl+G` 외부 에디터로 초안 편집. 결과는 `App::set_draft`.
    OpenEditor,
    /// `!` 셸 명령을 작업 폴더에서 실행한다. 끝나면 `AppEvent::ShellDone`.
    RunShell(String),
    /// 입력 기록에 더한다.
    RecordHistory(String),
    /// `Detach`를 보내고 화면을 끝낸다.
    Quit,
}

/// 입력창 위에 덮는 창·화면. 한 번에 하나.
#[derive(Debug)]
pub enum Window {
    /// judge 키 입력 창.
    JudgeKey(JudgeKeyPrompt),
    /// 폴더 설정 신뢰 창.
    FolderTrust(FolderTrust),
    /// 보류 재개 질문.
    Resume(ResumePrompt),
    /// 작업 목록 화면.
    TaskList(TaskList),
    /// 전체 기록.
    FullTranscript(FullTranscript),
    /// 사용량 화면.
    Usage(UsageScreen),
    /// judge 버전 화면.
    JudgeVersion(JudgeVersionScreen),
    /// 학습 확인 창.
    TrainConfirm(TrainConfirm),
    /// `?` 단축키 안내. 아무 키나 누르면 닫힌다.
    Shortcuts,
}

/// 화면 전체 상태.
#[derive(Debug)]
pub struct App {
    /// 화면 언어.
    pub lang: Lang,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// engine 알림으로 채운 채팅 상태.
    pub chat: ChatState,
    /// 대화 기록.
    pub transcript: Transcript,
    /// 작업별 출력 칸.
    pub live: LiveArea,
    /// 입력창.
    pub composer: Composer,
    /// 팝업.
    pub popup: Option<Popup>,
    /// `Esc`로 닫은 팝업 억제.
    pub popup_suppress: PopupSuppress,
    /// 허가 요청 창 대기열. 떠 있으면 다른 창보다 먼저 키를 받는다(judge 키·폴더 신뢰 창 제외).
    pub permissions: PermissionQueue,
    /// 덮은 창.
    pub window: Option<Window>,
    /// 시작 화면. 첫 결과가 오면 대화 기록 머리 셀로 옮기고 `None`.
    pub start: Option<StartInfo>,
    /// 입력 기록.
    pub history: InputHistory,
    /// 보류 재개 질문을 이미 띄웠다(채팅을 열 때 한 번).
    pub resume_asked: bool,
    /// 다음 입력에 붙일 셸 명령 결과. 메인 에이전트의 다음 입력에 첨부하고 비운다.
    pub pending_attachments: Vec<ShellOutput>,
    /// 틱 수. 스피너 글자를 고른다.
    pub tick: u64,
    /// 종료 요청됨.
    pub quit: bool,
}

impl App {
    /// 새 화면. `chat`이 있으면 그 채팅에 붙을 준비를 한다(`attach_request`).
    pub fn new(lang: Lang, workdir: PathBuf, history: InputHistory, chat: Option<ChatId>) -> Self {
        todo!("#92")
    }

    /// 시작할 때 보낼 `Request::Attach`. engine은 최근 기록과 보관한 허가 요청을 먼저 보낸다.
    pub fn attach_request(&self) -> Request {
        todo!("#92")
    }

    /// 이벤트 하나를 처리하고 할 일을 돌려준다.
    pub fn handle(&mut self, event: AppEvent, now: Instant) -> Vec<Effect> {
        todo!("#92")
    }

    /// 키를 받는 영역. 우선순위: judge 키 입력 창 → 폴더 설정 신뢰 창 → 허가 요청 창 → 그 밖의 창 →
    /// 팝업 → 상태판(보류 닫기 확인 중) → 대화 기록(피드백 질문 중, 숫자 키만) → 입력창.
    pub fn key_area(&self) -> KeyArea {
        todo!("#92")
    }

    /// 매핑에 필요한 상태.
    pub fn key_context(&self) -> KeyContext {
        todo!("#92")
    }

    /// 키 하나. `keys::map`으로 동작을 얻어 `on_action`. 허가 요청 창이 1초 보호 중이면 무시한다.
    /// 피드백 질문 영역에서 숫자가 아닌 키는 입력창으로 넘긴다.
    fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Effect> {
        todo!("#92")
    }

    /// 마우스. 상태판 버튼(`status_board::button_rects`) 위 클릭이면 `on_button`, 휠은 대화 기록 스크롤.
    fn on_mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        todo!("#92")
    }

    /// 붙여넣기. 입력창에 넣고(1,000자 넘으면 요소) 팝업 토큰을 다시 거른다. 창이 떠 있으면 무시한다.
    fn on_paste(&mut self, text: String) {
        todo!("#92")
    }

    /// 동작 하나. 영역별 뜻:
    /// - `ShowFullTranscript`: 전체 기록 창을 열거나 닫는다. `Suspend`: `Effect::Suspend`.
    /// - `Submit`/`SubmitQueued`: `submit`. `Interrupt`: `interrupt`. `RecallLatestInput`: `recall_latest_input`.
    /// - `FeedbackAnswer`/`FeedbackDismiss`: `answer_feedback`. `ConfirmCloseHeld`: `Request::CloseHeld`, `KeepHeld`: 확인 해제.
    /// - `Permission(answer)`: `Request::AnswerPermission`. `Quit`: `Effect::Quit`.
    /// - 창 영역의 `Up`/`Down`/`Confirm`/`Close`는 떠 있는 창에 넘긴다.
    fn on_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        todo!("#92")
    }

    /// 상태판 버튼. `Send` → `SendNow`, `CancelInput`/`CancelHeldInput` → `CancelInput`,
    /// `ContinueTask` → `Continue { task: Some }`, `ContinueInput` → `Request::ContinueInput`, `CloseHeld` → 확인 줄.
    fn on_button(&mut self, button: Button) -> Vec<Effect> {
        todo!("#92")
    }

    /// `Enter`(`queued: false`)와 `Tab`(`queued: true`). 초안이 비면 아무것도 하지 않는다.
    /// - `!` 줄: `Effect::RunShell`.
    /// - `/` 줄: `commands::parse` 뒤 `run_command`. 해석 오류는 대화 기록 경고 한 줄, 초안 유지.
    /// - 그 밖: `SubmitInput { skip_relation: queued && 실행 중 }`. 유휴 `Tab`은 `Enter`와 같다.
    ///   셸 결과 첨부가 있으면 원문 뒤에 붙이고 비운다. 원문은 `chat.pending_texts`와 입력 기록에 넣는다.
    /// - 새 입력 접수 중단(`Alert::IntakeStopped`) 중이면 보내지 않고 초안을 유지한다.
    fn submit(&mut self, queued: bool) -> Vec<Effect> {
        todo!("#92")
    }

    /// 해석한 명령 실행. `Send`/`Cancel`/`Continue`는 이름표로 대상을 찾아 요청, `Feedback`은 `answer_feedback`,
    /// `Tasks`/`Usage`/`JudgeVersion`은 창을 열고 조회 요청, `Train`은 `Request::Train`(부족하면 engine이 거절),
    /// `Help`는 단축키 안내 창, `Provider`는 원문을 `SubmitInput`으로.
    fn run_command(&mut self, command: SlashCommand) -> Vec<Effect> {
        todo!("#92")
    }

    /// `Ctrl+C` 한 번에 하나: 창·전체 기록 닫기 → 기록 검색 취소 → 초안 삭제.
    /// 모두 해당 없으면 실행 중일 때 `Request::Stop`(모든 작업 멈춤과 보류), 유휴이면 `Effect::Quit`.
    fn interrupt(&mut self) -> Vec<Effect> {
        todo!("#92")
    }

    /// `Alt+↑`, `Shift+←`: 판단 중이거나 대기 중인 가장 최근 입력을 `CancelInput`하고 원문을 입력창에 넣는다.
    /// 원문을 모르는 입력(다른 TUI가 보낸 입력)은 취소만 한다.
    fn recall_latest_input(&mut self) -> Vec<Effect> {
        todo!("#92")
    }

    /// 피드백 답. `Some(true)` 맞아요, `Some(false)` 아니에요, `None` 닫기(요청 없음).
    /// 아니에요이고 그 입력이 아직 보내지지 않았으면(`Judging`, `Queued`) 바로잡기 제안 셀을 더한다.
    fn answer_feedback(&mut self, correct: Option<bool>) -> Vec<Effect> {
        todo!("#92")
    }

    /// engine 알림 하나. 변형별로 나눠 처리하고 첫 결과가 오면 시작 화면을 머리 셀로 바꾼다.
    fn on_notification(&mut self, notification: Notification, now: Instant) -> Vec<Effect> {
        todo!("#92")
    }

    /// `InputChanged`: `ChatState::apply_input`. `Change::Echo`면 에코 셀(`> [A] 원문`),
    /// `Delivering`/`Applied`면 에코 뒤 `전달 중`/`반영됨` 표시를 고친다.
    fn on_input_changed(
        &mut self,
        input: InputId,
        label: Option<TaskLabel>,
        state: InputState,
        disposition: Option<Disposition>,
        reason: Option<QueueReason>,
    ) -> Vec<Effect> {
        todo!("#92")
    }

    /// `TaskChanged`: `ChatState::apply_task`. 끝나면 작업별 출력 칸 내용을 대화 기록으로 옮기고 결과 머리줄
    /// (`[A] codex · 45초 · Token 3,210`, 실패면 `· 실패`와 원인 줄), `NeedsCheck`면 `[A] 결과 확인 필요 · /continue A`.
    /// 이 채팅을 연 뒤 처음 보는 보류 작업이 있고 아직 묻지 않았으면 보류 재개 질문을 띄운다.
    fn on_task_changed(
        &mut self,
        task: TaskId,
        label: TaskLabel,
        state: TaskState,
        now: Instant,
    ) -> Vec<Effect> {
        todo!("#92")
    }

    /// `TaskEvent`: `ChatState::apply_event`. 글은 작업별 출력 칸, 도구는 도구 셀, 허가 요청은 허가 요청 창 대기열.
    fn on_task_event(&mut self, task: TaskId, event: ProviderEvent, now: Instant) -> Vec<Effect> {
        todo!("#92")
    }

    /// `ChatNotice`: `Compacted`·`ProviderSwitched`·`RequestSummary`는 대화 기록 한 줄,
    /// `Stopped`·`StopUnconfirmed`는 상태판(`ChatState::stop`). `ProviderSwitched`는 작업 provider도 고친다.
    fn on_chat_notice(
        &mut self,
        chat: ChatId,
        task: Option<TaskId>,
        notice: ChatNotice,
    ) -> Vec<Effect> {
        todo!("#92")
    }

    /// `FeedbackQuestion`: 입력 에코 다음 줄에 질문 셀을 넣고 `now`부터 8초를 잰다. 이전 질문이 있으면 바꾼다.
    fn on_feedback_question(&mut self, judgment: JudgmentId, label: TaskLabel, now: Instant) {
        todo!("#92")
    }

    /// `SettingsApplied`: 경고가 있으면 알림 줄 `폴더 설정 오류 · 이전 설정 번호 N로 계속 · 경고`.
    fn on_settings_applied(&mut self, revision: SettingsRevision, warning: Option<String>) {
        todo!("#92")
    }

    /// `Alert`: `ChatState::apply_alert`.
    fn on_alert(&mut self, alert: Alert) {
        todo!("#92")
    }

    /// 틱: 스피너를 넘기고, 8초 지난 피드백 질문을 지운다.
    fn on_tick(&mut self, now: Instant) {
        todo!("#92")
    }

    /// 외부 에디터 결과로 초안을 바꾼다.
    pub fn set_draft(&mut self, text: &str) {
        todo!("#92")
    }

    /// 셸 명령 결과: 대화 기록에 셸 셀을 넣고 다음 입력 첨부 목록에 더한다.
    fn on_shell_done(&mut self, output: ShellOutput) {
        todo!("#92")
    }

    /// 화면 전체를 그린다. `view::layout`으로 영역을 나누고 시작 화면 또는 대화 기록, 작업별 출력 칸, 상태판,
    /// 팝업, 입력창, 바닥줄을 그린 뒤 창(`window`)과 허가 요청 창을 덮는다. 전체 기록은 화면 전체를 쓴다.
    pub fn render(&self, frame: &mut Frame, now: Instant) {
        todo!("#92")
    }
}

/// 이벤트 루프. 입력 이벤트 스레드, 틱 타이머, engine 알림을 `tokio::select!`로 기다리고,
/// 이벤트마다 `App::handle` → `Effect` 실행 → 다시 그리기. `Effect::Quit`이나 engine 연결 종료면
/// `Request::Detach`(연결이 살아 있으면)를 보내고 돌아온다. 화면 복원은 호출자(`crate::run`)가 한다.
///
/// # Errors
/// 그리기 실패, 요청 전송 실패(연결 끊김), 입력 기록 쓰기 실패.
pub async fn run_loop(
    app: &mut App,
    client: &mut EngineClient,
    screen: &mut Screen,
) -> Result<(), TuiError> {
    todo!("#92")
}

/// `Effect` 하나를 실행한다. `RunShell`은 tokio 작업으로 띄우고 결과를 `AppEvent::ShellDone`으로 보낸다.
///
/// # Errors
/// 요청 전송, 일시 중지, 외부 에디터, 입력 기록 쓰기 실패.
async fn apply_effect(
    effect: Effect,
    app: &mut App,
    client: &mut EngineClient,
    screen: &mut Screen,
    events: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) -> Result<(), TuiError> {
    todo!("#92")
}
