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
use crate::view::input_request::InputQueue;
use crate::view::live_area::LiveArea;
use crate::view::model_picker::ModelPicker;
use crate::view::permission::PermissionQueue;
use crate::view::popup::{Popup, PopupItem, PopupSuppress};
use crate::view::resume_prompt::ResumePrompt;
use crate::view::router_key_prompt::RouterKeyPrompt;
use crate::view::router_version::RouterVersionScreen;
use crate::view::start_screen::StartInfo;
use crate::view::status_board;
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
    EngineClosed,
    Tick,
    ShellDone(ShellOutput),
}

/// `run_loop`가 순서대로 실행한다.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Effect {
    Send(Request),
    Suspend,
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
    Model(ModelPicker),
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
    pub popup: Option<Popup>,
    pub popup_suppress: PopupSuppress,
    pub permissions: PermissionQueue,
    pub inputs: InputQueue,
    pub window: Option<Window>,
    /// 첫 대화 기록 셀이 생기면 머리 셀로 옮기고 `None`.
    pub start: Option<StartInfo>,
    /// 채팅의 기본 폴더. 시작 화면이 머리 셀로 바뀐 뒤에도 남는다.
    pub chat_folder: Option<PathBuf>,
    pub history: InputHistory,
    /// 채팅을 열 때 한 번만 묻는다.
    pub resume_asked: bool,
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
            popup: None,
            popup_suppress: PopupSuppress::default(),
            permissions: PermissionQueue::new(),
            inputs: InputQueue::new(),
            window: None,
            start: None,
            chat_folder: None,
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
            Some(Window::TaskList(_)) => return KeyArea::TaskList,
            Some(Window::FullTranscript(_)) => return KeyArea::FullTranscript,
            Some(Window::Usage(_)) => return KeyArea::Usage,
            Some(Window::RouterVersion(_)) => return KeyArea::RouterVersion,
            Some(Window::TrainConfirm(_)) => return KeyArea::TrainConfirm,
            Some(Window::Model(_)) => return KeyArea::ModelPicker,
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
        if area == KeyArea::Input {
            return self.on_input_key(key, now);
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

    // cost: time O(f·o), heap O(f·o), stack O(1)
    // vars: f = 입력 요청의 칸 수, o = 선택지 수
    // basis: estimate
    /// 입력 요청 창은 전역 키와 `Ctrl+C`만 따로 읽고 나머지는 폼이 받는다.
    fn on_input_key(&mut self, key: KeyEvent, now: Instant) -> Vec<Effect> {
        if let Some(action) = keys::global(key) {
            return self.on_action(action, now);
        }
        if is_ctrl_c(key) {
            return self.interrupt();
        }
        match self.inputs.on_key(key, now) {
            Some((request_id, answer)) => {
                vec![Effect::Send(Request::AnswerInput { request_id, answer })]
            }
            None => Vec::new(),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 키를 받았으면 `true`.
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
    /// 검색이 처리하지 않는 키는 `None`으로 보통 매핑에 넘긴다.
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

    pub(super) fn search_result(&self) -> Option<&str> {
        let search = self.composer.search()?;
        self.history.search(&search.query, search.skip)
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
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
    /// TODO(#110): 기록 번호가 정해지면 `before`를 채운다
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
        if self.window.is_some() || !self.permissions.is_empty() {
            return;
        }
        self.composer.paste(text);
        self.history.reset();
        self.refresh_popup();
    }

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
            KeyArea::RouterKeyPrompt => self.on_router_key_action(action),
            KeyArea::FolderTrust => self.on_trust_action(action),
            KeyArea::ResumePrompt => self.on_resume_action(action),
            KeyArea::TaskList => self.on_task_list_action(action),
            KeyArea::FullTranscript | KeyArea::Usage | KeyArea::RouterVersion => {
                self.on_screen_action(action)
            }
            KeyArea::TrainConfirm => self.on_train_action(action),
            KeyArea::ModelPicker => self.on_model_action(action),
            KeyArea::Popup => self.on_popup_action(action),
            _ => self.on_composer_action(action),
        }
    }

    fn toggle_full_transcript(&mut self) {
        match &self.window {
            Some(Window::FullTranscript(_)) => self.window = None,
            Some(window) if window.is_blocking() => {}
            _ => self.window = Some(Window::FullTranscript(FullTranscript::default())),
        }
    }

    pub(super) fn quit_effects(&mut self) -> Vec<Effect> {
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

fn is_ctrl_c(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL
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
