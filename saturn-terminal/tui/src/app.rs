//! 화면 상태와 이벤트 루프: 키 입력, engine 알림, 틱.
//!
//! 설계: docs/design/tui.md, docs/design/input-handling.md(대기와 취소, 멈춤과 보류), docs/design/engine-lifecycle.md(TUI 종료).
//! `App`은 입출력을 하지 않는다. 이벤트를 받아 상태를 고치고 할 일(`Effect`)을 돌려주면 `run_loop`가 실행한다.
//! 키는 `key_area`가 고른 영역에서 `keys::map`으로 `Action`이 되고 `on_action`이 처리한다.
//! 입력창과 명령은 `app::compose`, 창 동작은 `app::windows`, engine 알림 처리는 `app::notify`, 그리기는 `app::draw`에 있다.

mod compose;
mod draw;
mod notify;
#[cfg(test)]
mod tests;
mod windows;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::{CommandInfo, Notification, Request};
use tokio::sync::mpsc;

use crate::TuiError;
use crate::client::{ClientError, EngineClient};
use crate::history::InputHistory;
use crate::i18n::{self, Lang};
use crate::keys::{self, Action, KeyArea, KeyContext};
use crate::shell::{self, ShellOutput};
use crate::state::ChatState;
use crate::terminal::{self, Screen};
use crate::view::composer::Composer;
use crate::view::folder_trust::FolderTrust;
use crate::view::full_transcript::FullTranscript;
use crate::view::judge_key_prompt::JudgeKeyPrompt;
use crate::view::judge_version::JudgeVersionScreen;
use crate::view::live_area::LiveArea;
use crate::view::permission::PermissionQueue;
use crate::view::popup::{Popup, PopupItem, PopupSuppress};
use crate::view::resume_prompt::ResumePrompt;
use crate::view::start_screen::StartInfo;
use crate::view::status_board;
use crate::view::task_list::TaskList;
use crate::view::train_confirm::TrainConfirm;
use crate::view::transcript::{Transcript, TranscriptCell};
use crate::view::usage::UsageScreen;

/// 틱 간격. 스피너, 경과 시간, 피드백 8초, 허가 1초 보호를 갱신한다. 100ms는 초안 값이다(설계에 없음, docs/design/tui.md 초안 값).
pub const TICK: Duration = Duration::from_millis(100);
/// 피드백 질문이 답 없이 떠 있는 시간.
pub const FEEDBACK_TIMEOUT: Duration = Duration::from_secs(8);
/// 마우스 휠 한 칸에 움직이는 대화 기록 줄 수. 초안 값이다(docs/design/tui.md 초안 값).
pub const WHEEL_ROWS: usize = 3;
/// 위로 스크롤해 맨 위에 닿았을 때 한 번에 불러오는 이전 기록 수. 초안 값이다(docs/design/tui.md 초안 값).
pub const HISTORY_PAGE: u32 = 50;

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

impl Window {
    /// 다른 창이 덮을 수 없는 창(judge 키 입력 창, 폴더 설정 신뢰 창).
    fn is_blocking(&self) -> bool {
        matches!(self, Self::JudgeKey(_) | Self::FolderTrust(_))
    }
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
    /// 시작 화면. 대화 기록에 첫 셀이 생기면 머리 셀로 옮기고 `None`.
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
    /// 화면 크기. 마우스 위치를 영역과 맞출 때 쓴다.
    pub screen: Rect,
    /// provider가 알려 준 명령과 스킬(`/`, `$` 팝업).
    pub provider_commands: Vec<(Provider, CommandInfo)>,
    /// 붙을 때 요청한 채팅. `Attach`에 싣는다.
    attach_chat: Option<ChatId>,
    /// 다음 `SubmitInput`의 `client_ref`.
    next_client_ref: u64,
    /// 첫 `HistoryChunk`(접속 직후 최근 기록)를 받았다. 그 뒤의 묶음은 위로 스크롤한 이전 부분이다.
    history_loaded: bool,
    /// 이전 기록을 요청하고 답을 기다린다.
    history_loading: bool,
    /// 더 불러올 이전 기록이 있다.
    history_has_more: bool,
    /// 마지막 `/train`이 `--reset-thresholds`였다(학습 확인 창 표시용).
    train_reset: bool,
    /// `@` 파일 목록 후보. 파일 팝업을 처음 열 때 한 번 모은다.
    file_cache: Option<Vec<PopupItem>>,
}

impl App {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 새 화면. `chat`이 있으면 그 채팅에 붙을 준비를 한다(`attach_request`).
    pub fn new(lang: Lang, workdir: PathBuf, history: InputHistory, chat: Option<ChatId>) -> Self {
        Self {
            lang,
            workdir,
            chat: ChatState::new(),
            transcript: Transcript::new(),
            live: LiveArea::new(),
            composer: Composer::new(),
            popup: None,
            popup_suppress: PopupSuppress::default(),
            permissions: PermissionQueue::new(),
            window: None,
            start: None,
            history,
            resume_asked: false,
            pending_attachments: Vec::new(),
            tick: 0,
            quit: false,
            screen: Rect::default(),
            provider_commands: Vec::new(),
            attach_chat: chat,
            next_client_ref: 0,
            history_loaded: false,
            history_loading: false,
            history_has_more: true,
            train_reset: false,
            file_cache: None,
        }
    }

    /// 시작할 때 보낼 `Request::Attach`. engine은 최근 기록과 보관한 허가 요청을 먼저 보낸다.
    pub fn attach_request(&self) -> Request {
        Request::Attach {
            chat: self.attach_chat,
            workdir: self.workdir.display().to_string(),
            overrides: Vec::new(),
        }
    }

    /// 이벤트 하나를 처리하고 할 일을 돌려준다.
    pub fn handle(&mut self, event: AppEvent, now: Instant) -> Vec<Effect> {
        match event {
            AppEvent::Terminal(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                self.on_key(key, now)
            }
            AppEvent::Terminal(Event::Mouse(mouse)) => self.on_mouse(mouse, now),
            AppEvent::Terminal(Event::Paste(text)) => {
                self.on_paste(text);
                Vec::new()
            }
            AppEvent::Terminal(Event::Resize(width, height)) => {
                self.screen = Rect::new(0, 0, width, height);
                Vec::new()
            }
            AppEvent::Terminal(_) => Vec::new(),
            AppEvent::Engine(notification) => self.on_notification(notification, now),
            AppEvent::EngineClosed => {
                self.quit = true;
                Vec::new()
            }
            AppEvent::Tick => {
                self.on_tick(now);
                Vec::new()
            }
            AppEvent::ShellDone(output) => {
                self.on_shell_done(output);
                Vec::new()
            }
        }
    }

    /// 키를 받는 영역. 우선순위: judge 키 입력 창 → 폴더 설정 신뢰 창 → 허가 요청 창 → 그 밖의 창 →
    /// 팝업 → 상태판(보류 닫기 확인 중) → 대화 기록(피드백 질문 중, 숫자 키만) → 입력창.
    pub fn key_area(&self) -> KeyArea {
        match &self.window {
            Some(Window::JudgeKey(_)) => return KeyArea::JudgeKeyPrompt,
            Some(Window::FolderTrust(_)) => return KeyArea::FolderTrust,
            _ => {}
        }
        if !self.permissions.is_empty() {
            return KeyArea::Permission;
        }
        match &self.window {
            Some(Window::Resume(_)) => return KeyArea::ResumePrompt,
            Some(Window::TaskList(_)) => return KeyArea::TaskList,
            Some(Window::FullTranscript(_)) => return KeyArea::FullTranscript,
            Some(Window::Usage(_)) => return KeyArea::Usage,
            Some(Window::JudgeVersion(_)) => return KeyArea::JudgeVersion,
            Some(Window::TrainConfirm(_)) => return KeyArea::TrainConfirm,
            _ => {}
        }
        if self.popup.is_some() {
            KeyArea::Popup
        } else if self.chat.close_held_confirm.is_some() {
            KeyArea::StatusBoard
        } else if self.chat.feedback.is_some() {
            KeyArea::Transcript
        } else {
            KeyArea::Composer
        }
    }

    /// 매핑에 필요한 상태.
    pub fn key_context(&self) -> KeyContext {
        KeyContext {
            composer_empty: self.composer.is_empty(),
            at_line_start: self.composer.at_line_start(),
            at_word_start: self.composer.at_word_start(),
            history_browsable: self.composer.history_browsable(),
            running: self.chat.is_running(),
        }
    }

    /// 외부 에디터 결과로 초안을 바꾼다.
    pub fn set_draft(&mut self, text: &str) {
        self.composer.set_text(text, false);
        self.refresh_popup();
    }

    /// 키 하나. `keys::map`으로 동작을 얻어 `on_action`. 허가 요청 창이 1초 보호 중이면 무시한다.
    /// 피드백 질문·팝업 영역에서 뜻이 없는 키는 입력창으로 넘기고, 창 화면의 `Ctrl+C`는 `interrupt`(뷰 해제)로 보낸다.
    /// 단축키 안내 창은 아무 키에나 닫힌다.
    /// 기록 검색 중이거나 작업 목록 입력 한 줄 중이면 글자를 그쪽에 넣는다.
    pub(super) fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Effect> {
        if matches!(self.window, Some(Window::Shortcuts)) {
            self.window = None;
            return Vec::new();
        }
        let area = self.key_area();
        if area == KeyArea::Permission && !self.permissions.accepts_input(now) {
            return Vec::new();
        }
        if self.edit_text_line(area, key) {
            return Vec::new();
        }
        if area == KeyArea::Composer
            && self.composer.search().is_some()
            && let Some(effects) = self.on_search_key(key)
        {
            return effects;
        }
        let ctx = self.key_context();
        match keys::map(area, key, ctx) {
            Some(action) => self.on_action(action, now),
            None if matches!(area, KeyArea::Popup | KeyArea::Transcript) => {
                match keys::map(KeyArea::Composer, key, ctx) {
                    Some(action) => self.on_composer_action(action),
                    None => Vec::new(),
                }
            }
            // 창 화면의 `Ctrl+C`는 입력창과 같이 뷰 해제부터 처리한다.
            None if is_ctrl_c(key) => self.interrupt(),
            None => Vec::new(),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 작업 목록 입력 한 줄의 글자와 지우기. 받았으면 참.
    fn edit_text_line(&mut self, area: KeyArea, key: KeyEvent) -> bool {
        let Some(Window::TaskList(list)) = &mut self.window else {
            return false;
        };
        if area != KeyArea::TaskList || list.editing.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                list.edit_push(c);
                true
            }
            KeyCode::Backspace => {
                list.edit_pop();
                true
            }
            _ => false,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `Ctrl+R` 기록 검색 중의 키. 글자는 검색어, `Enter`는 찾은 기록을 초안으로, `Esc`는 검색 취소.
    /// 그 밖의 키(`Ctrl+R`, `Ctrl+C` 등)는 `None`을 돌려 보통 매핑으로 넘긴다.
    fn on_search_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>> {
        let plain = (key.modifiers - KeyModifiers::SHIFT).is_empty();
        match key.code {
            KeyCode::Char(c) if plain => self.composer.push_search_char(c),
            KeyCode::Backspace => self.composer.pop_search_char(),
            KeyCode::Enter => {
                let found = self.search_result().map(str::to_string);
                self.composer.accept_search(found.as_deref());
            }
            KeyCode::Esc => {
                self.composer.cancel_search();
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// 지금 검색어로 찾은 기록.
    pub(super) fn search_result(&self) -> Option<&str> {
        let search = self.composer.search()?;
        self.history.search(&search.query, search.skip)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 마우스. 상태판 버튼(`status_board::button_rects`) 위 클릭이면 `on_button`, 휠은 대화 기록 스크롤.
    fn on_mouse(&mut self, mouse: MouseEvent, now: Instant) -> Vec<Effect> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if self.window.is_none() => {
                let lines = status_board::build(&self.chat, now);
                let areas = self.areas(self.screen, lines.len());
                let point = Position::new(mouse.column, mouse.row);
                let hit = status_board::button_rects(&lines, self.lang, areas.status)
                    .into_iter()
                    .find(|(rect, _)| rect.contains(point));
                match hit {
                    Some((_, button)) => self.on_button(button),
                    None => Vec::new(),
                }
            }
            MouseEventKind::ScrollUp => self.scroll_up(WHEEL_ROWS),
            MouseEventKind::ScrollDown => {
                match &mut self.window {
                    Some(Window::FullTranscript(full)) => (0..WHEEL_ROWS).for_each(|_| full.down()),
                    _ => self.transcript.scroll_down(WHEEL_ROWS),
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 위로 스크롤. 대화 기록 맨 위에 닿으면 이전 부분을 한 번 요청한다.
    /// TODO(#110): `HistoryChunk` 항목에 기록 번호가 없어 `before`를 비워 보낸다. 기준 위치가 정해지면 채운다
    fn scroll_up(&mut self, rows: usize) -> Vec<Effect> {
        if let Some(Window::FullTranscript(full)) = &mut self.window {
            (0..rows).for_each(|_| full.up());
            return Vec::new();
        }
        let at_top = self.transcript.scroll_up(rows);
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        if !at_top || self.history_loading || !self.history_has_more {
            return Vec::new();
        }
        self.history_loading = true;
        vec![Effect::Send(Request::LoadHistory {
            chat,
            before: None,
            limit: HISTORY_PAGE,
        })]
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 붙여넣기. 입력창에 넣고(1,000자 넘으면 요소) 팝업 토큰을 다시 거른다.
    /// judge 키 입력 창이면 줄바꿈을 뺀 글을 가린 입력칸에 넣고, 그 밖의 창이 떠 있으면 무시한다.
    fn on_paste(&mut self, text: String) {
        if let Some(Window::JudgeKey(prompt)) = &mut self.window {
            text.chars()
                .filter(|c| !c.is_control())
                .for_each(|c| prompt.input.push(c));
            return;
        }
        if self.window.is_some() || !self.permissions.is_empty() {
            return;
        }
        self.composer.paste(text);
        self.history.reset();
        self.refresh_popup();
    }

    /// 동작 하나. 영역별 뜻:
    /// - `ShowFullTranscript`: 전체 기록 창을 열거나 닫는다. `Suspend`: `Effect::Suspend`.
    /// - `Submit`/`SubmitQueued`: `submit`. `Interrupt`: `interrupt`. `RecallLatestInput`: `recall_latest_input`.
    /// - `FeedbackAnswer`/`FeedbackDismiss`: `answer_feedback`. `ConfirmCloseHeld`: `Request::CloseHeld`, `KeepHeld`: 확인 해제.
    /// - `Permission(answer)`: `Request::AnswerPermission`. `Quit`: `Effect::Quit`.
    /// - 창 영역의 `Up`/`Down`/`Confirm`/`Close`는 떠 있는 창에 넘긴다.
    fn on_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match action {
            Action::ShowFullTranscript => {
                self.toggle_full_transcript();
                return Vec::new();
            }
            Action::Suspend => return vec![Effect::Suspend],
            Action::Quit => return self.quit_effects(),
            Action::Permission(answer) => {
                return match self.permissions.answer(answer, now) {
                    Some((request_id, answer)) => {
                        vec![Effect::Send(Request::AnswerPermission {
                            request_id,
                            answer,
                        })]
                    }
                    None => Vec::new(),
                };
            }
            Action::FeedbackAnswer(correct) => return self.answer_feedback(Some(correct)),
            Action::FeedbackDismiss => return self.answer_feedback(None),
            Action::ConfirmCloseHeld => return self.close_held(),
            Action::KeepHeld => {
                self.chat.close_held_confirm = None;
                return Vec::new();
            }
            _ => {}
        }
        match self.key_area() {
            KeyArea::JudgeKeyPrompt => self.on_judge_key_action(action),
            KeyArea::FolderTrust => self.on_trust_action(action),
            KeyArea::ResumePrompt => self.on_resume_action(action),
            KeyArea::TaskList => self.on_task_list_action(action),
            KeyArea::FullTranscript | KeyArea::Usage | KeyArea::JudgeVersion => {
                self.on_screen_action(action)
            }
            KeyArea::TrainConfirm => self.on_train_action(action),
            KeyArea::Popup => self.on_popup_action(action),
            _ => self.on_composer_action(action),
        }
    }

    /// `Ctrl+T` 전체 기록을 열거나 닫는다. 다른 창이 떠 있으면 바꾸지 않는다(judge 키·폴더 신뢰 창).
    fn toggle_full_transcript(&mut self) {
        match &self.window {
            Some(Window::FullTranscript(_)) => self.window = None,
            Some(window) if window.is_blocking() => {}
            _ => self.window = Some(Window::FullTranscript(FullTranscript::default())),
        }
    }

    /// 종료. 다음 입력 기록 없이 `Detach` 뒤 끝낸다.
    pub(super) fn quit_effects(&mut self) -> Vec<Effect> {
        self.quit = true;
        vec![Effect::Quit]
    }

    /// 창을 연다. judge 키·폴더 신뢰 창 위에는 다른 창을 덮지 않는다.
    pub(super) fn open_window(&mut self, window: Window) {
        let blocked = self.window.as_ref().is_some_and(Window::is_blocking);
        if !blocked || window.is_blocking() {
            self.window = Some(window);
        }
    }

    /// 대화 기록에 셀을 더한다. 시작 화면이 떠 있으면 먼저 맨 위 머리 셀로 바꾼다.
    pub(super) fn push_cell(&mut self, cell: TranscriptCell) {
        if let Some(info) = self.start.take() {
            self.transcript.set_header(info);
        }
        self.transcript.push(cell);
    }

    /// 틱: 스피너를 넘기고, 8초 지난 피드백 질문을 지운다.
    fn on_tick(&mut self, now: Instant) {
        self.tick = self.tick.wrapping_add(1);
        let expired = self.chat.feedback.as_ref().is_some_and(|feedback| {
            now.saturating_duration_since(feedback.shown_at) >= FEEDBACK_TIMEOUT
        });
        if expired {
            self.answer_feedback(None);
        }
    }

    /// 셸 명령 결과: 대화 기록에 셸 셀을 넣고 다음 입력 첨부 목록에 더한다.
    fn on_shell_done(&mut self, output: ShellOutput) {
        self.push_cell(TranscriptCell::Shell(output.clone()));
        self.pending_attachments.push(output);
    }
}

/// `Ctrl+C`인지.
fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 이벤트 루프. 입력 이벤트 스레드, 틱 타이머, engine 알림을 `tokio::select!`로 기다리고,
/// 이벤트마다 `App::handle` → `Effect` 실행 → 다시 그리기. `Effect::Quit`이면 `Request::Detach`를 보내고 돌아오고,
/// engine 연결이 끝나면 `ClientError::Closed`로 돌아온다. 화면 복원은 호출자(`crate::run`)가 한다.
///
/// # Errors
/// 그리기 실패, 요청 전송 실패(연결 끊김), 일시 중지와 외부 에디터 실패.
pub async fn run_loop(
    app: &mut App,
    client: &mut EngineClient,
    screen: &mut Screen,
) -> Result<(), TuiError> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    // 받는 쪽(`rx`)이 이 함수와 함께 사라지면 스레드도 스스로 끝난다.
    let _reader = terminal::spawn_event_reader(tx.clone());
    let mut ticker = tokio::time::interval(TICK);
    let size = screen.size().map_err(terminal::TerminalError::Draw)?;
    app.screen = Rect::new(0, 0, size.width, size.height);
    client.send(app.attach_request()).await?;
    loop {
        screen
            .draw(|frame| app.render(frame, Instant::now()))
            .map_err(terminal::TerminalError::Draw)?;
        let event = tokio::select! {
            Some(event) = rx.recv() => event,
            notification = client.next() => match notification {
                Some(notification) => AppEvent::Engine(notification),
                None => AppEvent::EngineClosed,
            },
            _ = ticker.tick() => AppEvent::Tick,
        };
        let closed = matches!(event, AppEvent::EngineClosed);
        for effect in app.handle(event, Instant::now()) {
            if effect == Effect::Quit {
                client.send(Request::Detach).await?;
                return Ok(());
            }
            apply_effect(effect, app, client, screen, &tx).await?;
        }
        if closed {
            return Err(ClientError::Closed.into());
        }
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// `Effect` 하나를 실행한다. `RunShell`은 tokio 작업으로 띄우고 결과를 `AppEvent::ShellDone`으로 보낸다.
/// 입력 기록 쓰기 실패는 화면을 끝내지 않고 경고 로그만 남긴다.
///
/// # Errors
/// 요청 전송, 일시 중지, 외부 에디터 실패.
async fn apply_effect(
    effect: Effect,
    app: &mut App,
    client: &mut EngineClient,
    screen: &mut Screen,
    events: &mpsc::UnboundedSender<AppEvent>,
) -> Result<(), TuiError> {
    match effect {
        Effect::Send(request) => client.send(request).await?,
        Effect::Suspend => terminal::suspend(screen)?,
        Effect::OpenEditor => {
            let text = terminal::edit_external(screen, &app.composer.text())?;
            app.set_draft(&text);
        }
        Effect::RunShell(command) => {
            let events = events.clone();
            let workdir = app.workdir.clone();
            let failed = app.lang.tr(i18n::SHELL_FAILED);
            tokio::spawn(async move {
                let output = shell::run(&command, &workdir)
                    .await
                    .unwrap_or_else(|error| ShellOutput {
                        command: command.clone(),
                        status: None,
                        output: format!("{failed}: {error}"),
                    });
                // 이벤트 루프가 이미 끝났으면 결과를 보일 곳이 없다.
                let _ = events.send(AppEvent::ShellDone(output));
            });
        }
        Effect::RecordHistory(text) => {
            if let Err(error) = app.history.push(&text) {
                tracing::warn!(error = %error, "input history write failed");
            }
        }
        Effect::Quit => {}
    }
    Ok(())
}
