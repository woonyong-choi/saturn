//! 화면 상태와 이벤트 루프. `App`은 입출력 없이 상태를 고치고 `Effect`를 돌려준다.
//! 설계: docs/design/tui.md

mod compose;
mod draw;
mod notify;
#[cfg(test)]
mod tests;
mod windows;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use saturn_protocol::ids::{ChatId, LedgerSeq, Provider};
use saturn_protocol::rpc::{CommandInfo, Notification, QueryResult, Request};
use tokio::sync::mpsc;

use crate::TuiError;
use crate::client::{ClientError, EngineClient, Incoming};
use crate::history::InputHistory;
use crate::i18n::{self, Lang};
use crate::keymap::{Keymap, Resolved};
use crate::keys::{Action, KeyArea, KeyContext};
use crate::shell::{self, ShellOutput};
use crate::state::ChatState;
use crate::terminal::{self, Screen};
use crate::view::composer::Composer;
use crate::view::constraint_ask::{ConstraintAsk, ConstraintAskQueue};
use crate::view::exit_confirm::ExitConfirm;
use crate::view::folder_trust::FolderTrust;
use crate::view::full_transcript::FullTranscript;
use crate::view::input_request::InputQueue;
use crate::view::live_area::LiveArea;
use crate::view::model_picker::ModelPicker;
use crate::view::permission::PermissionQueue;
use crate::view::popup::{Popup, PopupItem, PopupSuppress};
use crate::view::prune_window::PruneWindow;
use crate::view::resume_prompt::ResumePrompt;
use crate::view::router_key_prompt::RouterKeyPrompt;
use crate::view::router_version::RouterVersionScreen;
use crate::view::start_screen::StartInfo;
use crate::view::status_board::{self, Button};
use crate::view::stop_confirm::StopConfirm;
use crate::view::task_list::TaskList;
use crate::view::train_confirm::TrainConfirm;
use crate::view::transcript::{Transcript, TranscriptCell};
use crate::view::usage::UsageScreen;

/// 초안 값.
pub(crate) const TICK: Duration = Duration::from_millis(100);
pub(crate) const FEEDBACK_TIMEOUT: Duration = Duration::from_secs(8);
/// 초안 값.
pub(crate) const WHEEL_ROWS: usize = 3;
/// 초안 값.
pub(crate) const HISTORY_PAGE: u32 = 50;

#[derive(Debug)]
pub(crate) enum AppEvent {
    Terminal(Event),
    Engine(Notification),
    /// 조회 요청의 응답 `result`.
    Result(QueryResult),
    EngineClosed,
    Tick,
    ShellDone(ShellOutput),
}

/// `run_loop`가 순서대로 실행한다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    Send(Request),
    Suspend,
    /// 화면을 지우고 다시 그린다.
    Redraw,
    OpenEditor,
    RunShell(String),
    RecordHistory(String),
    Quit,
}

#[derive(Debug)]
pub(crate) enum Window {
    RouterKey(RouterKeyPrompt),
    FolderTrust(FolderTrust),
    Resume(ResumePrompt),
    TaskList(TaskList),
    FullTranscript(FullTranscript),
    Usage(UsageScreen),
    RouterVersion(RouterVersionScreen),
    TrainConfirm(TrainConfirm),
    StopConfirm(StopConfirm),
    ConstraintAsk(ConstraintAsk),
    Model(ModelPicker),
    Prune(PruneWindow),
    Shortcuts,
}

impl Window {
    /// 다른 창이 덮을 수 없는 창.
    fn is_blocking(&self) -> bool {
        matches!(self, Self::RouterKey(_) | Self::FolderTrust(_))
    }
}

#[derive(Debug)]
pub(crate) struct App {
    pub lang: Lang,
    pub workdir: PathBuf,
    /// `Attach`로 engine에 넘기는 이 TUI의 환경 변수.
    pub env: Vec<(String, String)>,
    /// `Attach`로 넘기는 실행 층(`-c key=value`).
    pub overrides: Vec<(String, String)>,
    /// `Attach`로 넘기는 `--add-dir` 절대 경로.
    pub add_dirs: Vec<String>,
    pub chat: ChatState,
    pub transcript: Transcript,
    pub live: LiveArea,
    pub composer: Composer,
    /// 키를 동작으로 바꾸는 해석기. 프리셋은 설정 `tui.keymap`과 `/keymap`이 정한다.
    pub keymap: Keymap,
    /// 마지막으로 받은 설정의 `tui.keymap`. 같은 값이 다시 오면 `/keymap`으로 고른 묶음을 바꾸지 않는다.
    settings_keymap: Option<String>,
    /// 상태판 버튼 고르기 중 고른 버튼. `None`이면 고르기 밖이다.
    pub board_focus: Option<Button>,
    /// `Ctrl+C`를 한 번 눌러 종료를 기다린다. 다른 동작이 오면 풀린다.
    quit_armed: bool,
    pub popup: Option<Popup>,
    pub popup_suppress: PopupSuppress,
    pub permissions: PermissionQueue,
    /// 답을 기다리는 제약 등록 확인. 도착 순서대로 한 번에 하나씩 창으로 띄운다.
    pub constraint_asks: ConstraintAskQueue,
    pub inputs: InputQueue,
    pub window: Option<Window>,
    /// 닫기 전 계속할지 멈출지 묻는 창. 다른 창 위에 덮는다.
    pub exit_confirm: Option<ExitConfirm>,
    /// 닫기 전에 engine의 `ExitPlan`을 기다리는 중이다.
    pub exit_requested: bool,
    /// 닫은 뒤 계속 실행될 작업 수. 있으면 터미널에 한 줄 남긴다.
    pub exit_notice: Option<u32>,
    /// engine이 업데이트로 끝난다고 알렸다. 연결이 끊겨도 오류로 끝내지 않고 다시 열라는 한 줄을 남긴다.
    pub restarting: bool,
    /// 첫 대화 기록 셀이 생기면 머리 셀로 옮기고 `None`.
    pub start: Option<StartInfo>,
    /// 채팅의 기본 폴더. 시작 화면이 머리 셀로 바뀐 뒤에도 남는다.
    pub chat_folder: Option<PathBuf>,
    pub history: InputHistory,
    /// 채팅을 열 때 한 번만 묻는다.
    pub resume_asked: bool,
    /// 기본 모델을 고르지 않았을 때 처음 고르기 창은 실행마다 한 번만 연다.
    pub default_model_asked: bool,
    /// 메인 에이전트의 다음 입력에 붙이고 비운다.
    pub pending_attachments: Vec<ShellOutput>,
    pub tick: u64,
    pub quit: bool,
    pub screen: Rect,
    pub provider_commands: Vec<(Provider, CommandInfo)>,
    attach_chat: Option<ChatId>,
    next_client_ref: u64,
    /// 첫 `HistoryChunk` 뒤의 묶음은 위로 스크롤한 이전 부분이다.
    history_loaded: bool,
    history_loading: bool,
    /// 받은 가장 오래된 기록 위치. 더 앞 기록을 요청할 때 `before`로 보낸다.
    history_before: Option<LedgerSeq>,
    history_has_more: bool,
    /// 마지막 `/train`이 `--reset-thresholds`였다.
    train_reset: bool,
    /// 파일 팝업을 처음 열 때 한 번 모은다.
    file_cache: Option<Vec<PopupItem>>,
}

impl App {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn new(
        lang: Lang,
        workdir: PathBuf,
        history: InputHistory,
        chat: Option<ChatId>,
    ) -> Self {
        Self {
            lang,
            workdir,
            env: Vec::new(),
            overrides: Vec::new(),
            add_dirs: Vec::new(),
            chat: ChatState::new(),
            transcript: Transcript::new(),
            live: LiveArea::new(),
            composer: Composer::new(),
            keymap: Keymap::saturn(),
            settings_keymap: None,
            board_focus: None,
            quit_armed: false,
            popup: None,
            popup_suppress: PopupSuppress::default(),
            permissions: PermissionQueue::new(),
            constraint_asks: ConstraintAskQueue::default(),
            inputs: InputQueue::new(),
            window: None,
            exit_confirm: None,
            exit_requested: false,
            exit_notice: None,
            restarting: false,
            start: None,
            chat_folder: None,
            history,
            resume_asked: false,
            default_model_asked: false,
            pending_attachments: Vec::new(),
            tick: 0,
            quit: false,
            screen: Rect::default(),
            provider_commands: Vec::new(),
            attach_chat: chat,
            next_client_ref: 0,
            history_loaded: false,
            history_loading: false,
            history_before: None,
            history_has_more: true,
            train_reset: false,
            file_cache: None,
        }
    }

    pub(crate) fn attach_request(&self) -> Request {
        Request::Attach {
            chat: self.attach_chat,
            workdir: self.workdir.display().to_string(),
            env: self.env.clone(),
            overrides: self.overrides.clone(),
            add_dirs: self.add_dirs.clone(),
        }
    }

    pub(crate) fn handle(&mut self, event: AppEvent, now: Instant) -> Vec<Effect> {
        match event {
            AppEvent::Terminal(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                self.on_key(key, now)
            }
            AppEvent::Terminal(Event::Mouse(mouse)) => self.on_mouse(mouse, now),
            AppEvent::Terminal(Event::Paste(text)) => {
                self.on_paste(text, now);
                Vec::new()
            }
            AppEvent::Terminal(Event::Resize(width, height)) => {
                self.screen = Rect::new(0, 0, width, height);
                Vec::new()
            }
            AppEvent::Terminal(_) => Vec::new(),
            AppEvent::Engine(notification) => self.on_notification(notification, now),
            AppEvent::Result(result) => self.on_result(result, now),
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

    pub(crate) fn key_area(&self) -> KeyArea {
        if self.exit_confirm.is_some() {
            return KeyArea::ExitConfirm;
        }
        match &self.window {
            Some(Window::RouterKey(_)) => return KeyArea::RouterKeyPrompt,
            Some(Window::FolderTrust(_)) => return KeyArea::FolderTrust,
            _ => {}
        }
        if !self.permissions.is_empty() {
            return KeyArea::Permission;
        }
        if !self.inputs.is_empty() {
            return KeyArea::Input;
        }
        match &self.window {
            Some(Window::Resume(_)) => return KeyArea::ResumePrompt,
            Some(Window::TaskList(list)) => {
                return if list.editing.is_some() {
                    KeyArea::TaskListEdit
                } else {
                    KeyArea::TaskList
                };
            }
            Some(Window::FullTranscript(_)) => return KeyArea::FullTranscript,
            Some(Window::Usage(_)) => return KeyArea::Usage,
            Some(Window::RouterVersion(_)) => return KeyArea::RouterVersion,
            Some(Window::TrainConfirm(_)) => return KeyArea::TrainConfirm,
            Some(Window::StopConfirm(_)) => return KeyArea::StopConfirm,
            Some(Window::ConstraintAsk(_)) => return KeyArea::ConstraintAsk,
            Some(Window::Model(_)) => return KeyArea::ModelPicker,
            Some(Window::Prune(_)) => return KeyArea::PruneWindow,
            _ => {}
        }
        if self.popup.is_some() {
            KeyArea::Popup
        } else if self.chat.close_held_confirm.is_some() {
            KeyArea::StatusBoard
        } else if self.board_focus.is_some() {
            KeyArea::BoardFocus
        } else if self.choice_has_keys() && self.correction_open() {
            KeyArea::Correction
        } else if self.choice_has_keys() && self.chat.feedback.is_some() {
            KeyArea::Transcript
        } else if self.composer.search().is_some() {
            KeyArea::Search
        } else {
            KeyArea::Composer
        }
    }

    /// 화면에 뜬 선택지는 입력창이 비어 있는 동안만 키를 가져간다. 초안을 쓰기 시작하면 숫자도 방향키도 입력창이 받는다.
    fn choice_has_keys(&self) -> bool {
        self.composer.is_empty() && self.composer.search().is_none()
    }

    fn correction_open(&self) -> bool {
        self.chat.correction.as_ref().is_some_and(|c| c.open)
    }

    pub(crate) fn key_context(&self) -> KeyContext {
        KeyContext {
            composer_empty: self.composer.is_empty(),
            at_line_start: self.composer.at_line_start(),
            at_word_start: self.composer.at_word_start(),
            history_browsable: self.composer.history_browsable(),
            running: self.chat.is_running(),
        }
    }

    pub(crate) fn set_draft(&mut self, text: &str) {
        self.composer.set_text(text, false);
        self.refresh_popup();
    }

    pub(super) fn on_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Effect> {
        if matches!(self.window, Some(Window::Shortcuts)) {
            self.window = None;
            return Vec::new();
        }
        let area = self.key_area();
        if area == KeyArea::Permission && !self.permissions.accepts_input(now) {
            return Vec::new();
        }
        let ctx = self.key_context();
        let fallback = match area {
            KeyArea::Popup | KeyArea::Transcript | KeyArea::Correction | KeyArea::Search => {
                Some(KeyArea::Composer)
            }
            KeyArea::TaskListEdit => Some(KeyArea::TaskList),
            _ => None,
        };
        let Some(Resolved { action, scope }) = self.keymap.resolve(area, fallback, key, ctx, now)
        else {
            return Vec::new();
        };
        if !matches!(action, Action::Interrupt | Action::InterruptQuit) {
            self.quit_armed = false;
        }
        if scope == KeyArea::Composer && area != KeyArea::Composer {
            self.on_composer_action(action, now)
        } else {
            self.on_action(action, now)
        }
    }

    /// 입력 요청 창의 동작. 보호 시간 중이면 받지 않는다.
    fn on_input_action(&mut self, action: &Action, now: Instant) -> Vec<Effect> {
        match self.inputs.on_action(action, now) {
            Some((request_id, answer)) => {
                vec![Effect::Send(Request::AnswerInput { request_id, answer })]
            }
            None => Vec::new(),
        }
    }

    /// 입력 기록 검색 중의 동작. 검색이 받지 않는 동작은 입력창이 받는다.
    fn on_search_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match action {
            Action::Insert(c) => self.composer.push_search_char(c),
            Action::Backspace => self.composer.pop_search_char(),
            Action::Confirm => {
                let found = self.search_result().map(str::to_string);
                self.composer.accept_search(found.as_deref());
            }
            Action::Close => {
                self.composer.cancel_search();
            }
            other => return self.on_composer_action(other, now),
        }
        Vec::new()
    }

    pub(super) fn search_result(&self) -> Option<&str> {
        let search = self.composer.search()?;
        self.history.search(&search.query, search.skip)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn on_mouse(&mut self, mouse: MouseEvent, now: Instant) -> Vec<Effect> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if self.window.is_none() && self.exit_confirm.is_none() =>
            {
                let board = status_board::board(&self.chat, now);
                let rows = board.as_ref().map_or(0, status_board::Board::height);
                let areas = self.areas(self.screen, rows);
                let point = Position::new(mouse.column, mouse.row);
                let hit = status_board::button_rects(board.as_ref(), self.lang, areas.status)
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
        // 첫 묶음이 오기 전이나 묶음이 비어 위치를 모르면 요청하지 않는다.
        let Some(before) = self.history_before else {
            return Vec::new();
        };
        self.history_loading = true;
        vec![Effect::Send(Request::LoadHistory {
            chat,
            before: Some(before),
            limit: HISTORY_PAGE,
        })]
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn on_paste(&mut self, text: String, now: Instant) {
        if let Some(Window::RouterKey(prompt)) = &mut self.window {
            text.chars()
                .filter(|c| !c.is_control())
                .for_each(|c| prompt.input.push(c));
            return;
        }
        if self.permissions.is_empty() && !self.inputs.is_empty() {
            self.inputs.paste(&text, now);
            return;
        }
        if self.window.is_some() || self.exit_confirm.is_some() || !self.permissions.is_empty() {
            return;
        }
        self.composer.paste(text);
        self.history.reset();
        self.refresh_popup();
    }

    pub(super) fn on_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match action {
            Action::ShowFullTranscript => {
                self.toggle_full_transcript();
                return Vec::new();
            }
            Action::Suspend => return vec![Effect::Suspend],
            Action::Quit => return self.quit_effects(),
            Action::Interrupt => return self.interrupt(false),
            Action::InterruptQuit => return self.interrupt(true),
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
            Action::CorrectionRun => return self.finish_correction(true),
            Action::CorrectionKeep => return self.finish_correction(false),
            Action::ConfirmCloseHeld => return self.close_held(),
            Action::KeepHeld => {
                self.chat.close_held_confirm = None;
                return Vec::new();
            }
            _ => {}
        }
        match self.key_area() {
            KeyArea::RouterKeyPrompt => self.on_router_key_action(action),
            KeyArea::FolderTrust => self.on_trust_action(action),
            KeyArea::ResumePrompt => self.on_resume_action(action),
            KeyArea::ExitConfirm => self.on_exit_confirm_action(action),
            KeyArea::TaskList | KeyArea::TaskListEdit => self.on_task_list_action(action),
            KeyArea::BoardFocus => self.on_board_action(action, now),
            KeyArea::Input => self.on_input_action(&action, now),
            KeyArea::Search => self.on_search_action(action, now),
            KeyArea::FullTranscript | KeyArea::Usage | KeyArea::RouterVersion => {
                self.on_screen_action(action)
            }
            KeyArea::TrainConfirm => self.on_train_action(action),
            KeyArea::StopConfirm => self.on_stop_confirm_action(action),
            KeyArea::ConstraintAsk => self.on_constraint_ask_action(action),
            KeyArea::ModelPicker => self.on_model_action(action),
            KeyArea::PruneWindow => self.on_prune_action(action),
            KeyArea::Transcript => self.on_feedback_action(action),
            KeyArea::Correction => self.on_correction_action(action),
            KeyArea::Popup => self.on_popup_action(action),
            _ => self.on_composer_action(action, now),
        }
    }

    pub(super) fn toggle_full_transcript(&mut self) {
        match &self.window {
            Some(Window::FullTranscript(_)) => self.window = None,
            Some(window) if window.is_blocking() => {}
            _ => self.window = Some(Window::FullTranscript(FullTranscript::default())),
        }
    }

    /// 채팅에 붙어 있으면 닫기 전에 engine에 닫은 뒤의 처리를 묻는다. 이미 묻는 중이면 더 기다리지 않고 닫는다.
    /// router 키 창은 engine이 요청을 받지 않는 때라 묻지 않는다.
    pub(super) fn quit_effects(&mut self) -> Vec<Effect> {
        let asks_engine =
            !self.exit_requested && !matches!(self.window, Some(Window::RouterKey(_)));
        match self.chat.chat {
            Some(chat) if asks_engine => {
                self.exit_requested = true;
                vec![Effect::Send(Request::PrepareExit { chat })]
            }
            _ => self.quit_now(),
        }
    }

    /// 닫은 뒤에도 작업이 계속되거나 engine이 업데이트로 끝났을 때 터미널에 남길 한 줄.
    pub(crate) fn exit_line(&self) -> Option<String> {
        if self.restarting {
            return Some(self.lang.tr(i18n::ENGINE_RESTARTING).to_owned());
        }
        self.exit_notice.map(|count| {
            self.lang
                .tr(i18n::EXIT_BACKGROUND)
                .replace("{count}", &count.to_string())
        })
    }

    pub(super) fn quit_now(&mut self) -> Vec<Effect> {
        self.quit = true;
        vec![Effect::Quit]
    }

    pub(super) fn open_window(&mut self, window: Window) {
        let blocked = self.window.as_ref().is_some_and(Window::is_blocking);
        if !blocked || window.is_blocking() {
            self.window = Some(window);
        }
    }

    pub(super) fn push_cell(&mut self, cell: TranscriptCell) {
        if let Some(info) = self.start.take() {
            self.transcript.set_header(info);
        }
        self.transcript.push(cell);
    }

    fn on_tick(&mut self, now: Instant) {
        self.tick = self.tick.wrapping_add(1);
        let expired = self.chat.feedback.as_ref().is_some_and(|feedback| {
            now.saturating_duration_since(feedback.shown_at) >= FEEDBACK_TIMEOUT
        });
        if expired {
            self.answer_feedback(None);
        }
    }

    fn on_shell_done(&mut self, output: ShellOutput) {
        self.push_cell(TranscriptCell::Shell(output.clone()));
        self.pending_attachments.push(output);
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 화면 복원은 호출자(`crate::run`)가 한다.
///
/// # Errors
/// 그리기 실패, 요청 전송 실패(연결 끊김), 일시 중지와 외부 에디터 실패.
pub(crate) async fn run_loop(
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
            incoming = client.next() => match incoming {
                Some(Incoming::Notification(notification)) => AppEvent::Engine(notification),
                Some(Incoming::Result(result)) => AppEvent::Result(result),
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
        if closed && app.restarting {
            return Ok(());
        }
        if closed {
            return Err(ClientError::Closed.into());
        }
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
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
        Effect::Redraw => screen.clear().map_err(terminal::TerminalError::Draw)?,
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
