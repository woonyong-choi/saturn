//! 창과 화면의 키 동작.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ModelChoice, ModelMode, Request};

use std::time::Instant;

use super::{App, Effect, Window};
use crate::i18n;
use crate::keys::Action;
use crate::state::ChatState;
use crate::view::exit_confirm::ExitChoice;
use crate::view::folder_trust::TrustChoice;
use crate::view::input_request::InputQueue;
use crate::view::live_area::LiveArea;
use crate::view::model_picker::{ModelPicker, ModelPurpose};
use crate::view::permission::PermissionQueue;
use crate::view::resume_prompt::ResumeOutcome;
use crate::view::router_version::RouterVersionCommand;
use crate::view::status_board::{self, Button};
use crate::view::stop_confirm::StopChoice;
use crate::view::task_list::TaskListCommand;
use crate::view::train_confirm::TrainChoice;
use crate::view::transcript::{Transcript, TranscriptCell};
use crate::view::usage::usage_request;

impl App {
    pub(super) fn on_button(&mut self, button: Button) -> Vec<Effect> {
        let request = match button {
            Button::Send(input) => Request::SendNow { input },
            Button::CancelInput(input) | Button::CancelHeldInput(input) => {
                Request::CancelInput { input }
            }
            Button::ContinueTask(task) => {
                let Some(chat) = self.chat.chat else {
                    return Vec::new();
                };
                Request::Continue {
                    chat,
                    task: Some(task),
                }
            }
            Button::ContinueInput(input) => Request::ContinueInput { input },
            Button::CloseHeld(task) => {
                self.chat.close_held_confirm = Some(task);
                return Vec::new();
            }
        };
        vec![Effect::Send(request)]
    }

    // cost: time O(l), heap O(l), stack O(1)
    // vars: l = 상태판 줄 수
    // basis: estimate
    /// 버튼이 있는 줄마다 그 줄의 버튼들을 위에서 아래 순서로 모은다.
    fn button_rows(&self, now: Instant) -> Vec<Vec<Button>> {
        status_board::build(&self.chat, now)
            .iter()
            .map(status_board::StatusLine::buttons)
            .filter(|buttons| !buttons.is_empty())
            .collect()
    }

    /// 버튼이 없으면 들어가지 않는다.
    pub(super) fn enter_board(&mut self, now: Instant) {
        self.board_focus = self
            .button_rows(now)
            .first()
            .and_then(|buttons| buttons.first().copied());
    }

    /// `↑`, `↓`는 줄, `←`, `→`는 버튼, `Enter`는 실행, `Esc`는 돌아가기다. 고른 버튼이 사라졌으면 고르기를 끝낸다.
    pub(super) fn on_board_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        let rows = self.button_rows(now);
        let position = self.board_focus.and_then(|focus| {
            rows.iter().enumerate().find_map(|(row, buttons)| {
                buttons
                    .iter()
                    .position(|button| *button == focus)
                    .map(|col| (row, col))
            })
        });
        let Some((row, col)) = position else {
            self.board_focus = None;
            return Vec::new();
        };
        let target = match action {
            Action::Up => Some((row.saturating_sub(1), col)),
            Action::Down => Some(((row + 1).min(rows.len() - 1), col)),
            Action::CursorLeft => Some((row, col.saturating_sub(1))),
            Action::CursorRight => Some((row, col + 1)),
            _ => None,
        };
        if let Some((row, col)) = target {
            let buttons = &rows[row];
            self.board_focus = buttons.get(col.min(buttons.len() - 1)).copied();
            return Vec::new();
        }
        match action {
            Action::Confirm => {
                self.board_focus = None;
                let button = rows[row][col];
                self.on_button(button)
            }
            Action::Close => {
                self.board_focus = None;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn close_held(&mut self) -> Vec<Effect> {
        let Some(task) = self.chat.close_held_confirm.take() else {
            return Vec::new();
        };
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        if let Some(view) = self.chat.tasks.get(&task) {
            let label = view.label;
            self.push_cell(TranscriptCell::HeldClosed { label });
        }
        vec![Effect::Send(Request::CloseHeld { chat, task })]
    }

    pub(super) fn on_router_key_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::RouterKey(prompt)) = &mut self.window else {
            return Vec::new();
        };
        match action {
            Action::Insert(c) => prompt.input.push(c),
            Action::Backspace => prompt.input.pop(),
            Action::Confirm if !prompt.input.is_empty() => {
                let key = prompt.input.take();
                self.window = None;
                return vec![Effect::Send(Request::SubmitRouterKey { key })];
            }
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn on_trust_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::FolderTrust(trust)) = &mut self.window else {
            return Vec::new();
        };
        match action {
            Action::TrustApply => trust.selected = TrustChoice::Apply,
            Action::Up => trust.up(),
            Action::Down => trust.down(),
            Action::Confirm => {
                let apply = match trust.selected {
                    TrustChoice::Apply => true,
                    TrustChoice::Second => false,
                    TrustChoice::Quit => return self.quit_effects(),
                };
                let request = Request::AnswerFolderTrust {
                    path: trust.path.display().to_string(),
                    fingerprint: trust.fingerprint.clone(),
                    apply,
                };
                self.window = None;
                return vec![Effect::Send(request)];
            }
            _ => {}
        }
        Vec::new()
    }

    /// `계속`은 작업을 두고 닫고 터미널에 한 줄을 남긴다. `멈추기`는 모든 채팅을 멈춘 뒤 닫는다.
    pub(super) fn on_exit_confirm_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(confirm) = &mut self.exit_confirm else {
            return Vec::new();
        };
        match action {
            Action::Up => confirm.up(),
            Action::Down => confirm.down(),
            Action::Close => self.exit_confirm = None,
            Action::Confirm => {
                let running = confirm.running;
                let choice = confirm.selected;
                self.exit_confirm = None;
                return match choice {
                    ExitChoice::Continue => {
                        self.exit_notice = Some(running);
                        self.quit_now()
                    }
                    ExitChoice::Stop => {
                        let mut effects = vec![Effect::Send(Request::StopAll)];
                        effects.extend(self.quit_now());
                        effects
                    }
                };
            }
            _ => {}
        }
        Vec::new()
    }

    // cost: time O(h), heap O(h), stack O(1)
    // vars: h = 보류 작업 수
    // basis: estimate
    pub(super) fn on_resume_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::Resume(prompt)) = &mut self.window else {
            return Vec::new();
        };
        let outcome = match action {
            Action::Up => {
                prompt.up();
                return Vec::new();
            }
            Action::Down => {
                prompt.down();
                return Vec::new();
            }
            Action::Confirm => prompt.confirm(),
            _ => return Vec::new(),
        };
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        let requests = match outcome {
            ResumeOutcome::Pending => return Vec::new(),
            ResumeOutcome::ContinueAll => vec![Request::Continue { chat, task: None }],
            ResumeOutcome::Continue(tasks) => tasks
                .into_iter()
                .map(|task| Request::Continue {
                    chat,
                    task: Some(task),
                })
                .collect(),
            ResumeOutcome::Leave => Vec::new(),
        };
        self.window = None;
        requests.into_iter().map(Effect::Send).collect()
    }

    pub(super) fn on_task_list_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::TaskList(list)) = &mut self.window else {
            return Vec::new();
        };
        let command = match action {
            Action::Insert(c) => {
                list.edit_push(c);
                None
            }
            Action::Backspace => {
                list.edit_pop();
                None
            }
            Action::Up => {
                list.up();
                None
            }
            Action::Down => {
                list.down();
                None
            }
            Action::NextFilter => {
                list.set_filter(list.filter.next());
                None
            }
            Action::PrevFilter => {
                list.set_filter(list.filter.prev());
                None
            }
            Action::ToggleFolderScope => {
                list.toggle_scope();
                None
            }
            Action::Close => {
                if !list.cancel() {
                    self.window = None;
                }
                None
            }
            other => list.command(&other),
        };
        match command {
            Some(command) => self.on_task_list_command(command),
            None => Vec::new(),
        }
    }

    pub(super) fn on_task_list_command(&mut self, command: TaskListCommand) -> Vec<Effect> {
        let request = match command {
            TaskListCommand::Open { chat, .. } => {
                self.window = None;
                if self.chat.chat == Some(chat) {
                    return Vec::new();
                }
                return self.reattach(Some(chat));
            }
            TaskListCommand::NewChat => {
                self.window = None;
                return self.reattach(None);
            }
            TaskListCommand::Continue { chat, task } => Request::Continue {
                chat,
                task: Some(task),
            },
            TaskListCommand::CancelInput(input) => Request::CancelInput { input },
            TaskListCommand::CloseHeld { chat, task } => Request::CloseHeld { chat, task },
            TaskListCommand::SendNow(input) => Request::SendNow { input },
            // `engine`가 `ChatLabeled`로 알리므로 목록은 그 알림을 받아 다시 읽는다
            TaskListCommand::Rename { chat, name } => {
                return vec![Effect::Send(Request::RenameChat { chat, name })];
            }
            TaskListCommand::Regroup { chat, group } => {
                return vec![Effect::Send(Request::SetChatGroup {
                    chat,
                    group: (!group.is_empty()).then_some(group),
                })];
            }
        };
        vec![Effect::Send(request), Effect::Send(Request::ListTasks)]
    }

    /// 같은 연결로 `Attach`만 다시 보내 붙은 채팅을 바꾼다. `Detach`는 연결을 끊고 마지막 TUI 이탈로 세므로 보내지 않는다.
    /// 입력창, 입력 기록, provider 명령 목록은 채팅을 옮겨도 유지한다.
    pub(super) fn reattach(&mut self, chat: Option<ChatId>) -> Vec<Effect> {
        self.chat = ChatState::new();
        self.transcript = Transcript::new();
        self.live = LiveArea::new();
        self.popup = None;
        self.permissions = PermissionQueue::new();
        self.inputs = InputQueue::new();
        self.window = None;
        self.start = None;
        self.resume_asked = false;
        self.pending_attachments.clear();
        self.attach_chat = chat;
        self.history_loaded = false;
        self.history_loading = false;
        self.history_before = None;
        self.history_has_more = true;
        vec![Effect::Send(self.attach_request())]
    }

    pub(super) fn on_screen_action(&mut self, action: Action) -> Vec<Effect> {
        match (&mut self.window, action) {
            (Some(Window::FullTranscript(full)), Action::Up) => full.up(),
            (Some(Window::FullTranscript(full)), Action::Down) => full.down(),
            (Some(Window::Usage(usage)), Action::Confirm) => usage.toggle_detail(),
            (Some(Window::Usage(usage)), Action::UsageRange(range)) => {
                let range = usage.select(range);
                return vec![Effect::Send(usage_request(range))];
            }
            (Some(Window::RouterVersion(screen)), Action::Up) => screen.up(),
            (Some(Window::RouterVersion(screen)), Action::Down) => screen.down(),
            (Some(Window::RouterVersion(screen)), Action::Close) => {
                let cancelled = screen.cancel();
                if !cancelled {
                    self.window = None;
                }
            }
            (Some(Window::RouterVersion(screen)), other) => {
                return match screen.command(&other) {
                    Some(command) => vec![Effect::Send(router_version_request(command))],
                    None => Vec::new(),
                };
            }
            (_, Action::Close) => self.window = None,
            _ => {}
        }
        Vec::new()
    }

    /// `y`만 `Prune { yes: true }`를 보낸다. 지울 채팅이 없거나 이미 확정했으면 받지 않는다.
    pub(super) fn on_prune_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::Prune(window)) = &mut self.window else {
            return Vec::new();
        };
        match action {
            Action::Up => window.up(),
            Action::Down => window.down(),
            Action::Close => self.window = None,
            Action::PruneConfirm if window.can_confirm() => {
                window.is_deleting = true;
                return vec![Effect::Send(Request::Prune { yes: true })];
            }
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn on_train_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::TrainConfirm(confirm)) = &mut self.window else {
            return Vec::new();
        };
        let proceed = match action {
            Action::Up => {
                confirm.up();
                return Vec::new();
            }
            Action::Down => {
                confirm.down();
                return Vec::new();
            }
            Action::Confirm => confirm.selected == TrainChoice::Run,
            Action::Close => false,
            _ => return Vec::new(),
        };
        self.window = None;
        vec![Effect::Send(Request::ConfirmTrain { proceed })]
    }
}

impl App {
    /// `Esc`와 `Ctrl+C`는 작업을 멈추지 않는 `대기`와 같다. 답은 engine이 입력 상태로 알려 창을 지운다.
    pub(super) fn on_stop_confirm_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::StopConfirm(confirm)) = &mut self.window else {
            return Vec::new();
        };
        let stop = match action {
            Action::Up => {
                confirm.up();
                return Vec::new();
            }
            Action::Down => {
                confirm.down();
                return Vec::new();
            }
            Action::Confirm => confirm.selected == StopChoice::StopAndRun,
            Action::Close => false,
            _ => return Vec::new(),
        };
        let input = confirm.input;
        self.window = None;
        vec![Effect::Send(Request::AnswerStopConfirm { input, stop })]
    }
}

impl App {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `Enter`는 고른 모델을 이 채팅의 고정 모델로 두고 창을 닫는다. 목록이 오기 전의 `Enter`는 무시한다.
    pub(super) fn on_model_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(Window::Model(picker)) = &mut self.window else {
            return Vec::new();
        };
        match action {
            Action::Up => picker.up(),
            Action::Down => picker.down(),
            Action::Close => self.window = None,
            Action::Confirm => {
                if let (Some(choice), Some(chat)) =
                    (picker.selected_choice().cloned(), self.chat.chat)
                {
                    let first_default = picker.purpose == ModelPurpose::Default;
                    self.window = None;
                    return if first_default {
                        vec![Effect::Send(Request::SetDefaultModel {
                            chat,
                            model: choice,
                        })]
                    } else {
                        self.pin_model(chat, choice)
                    };
                }
            }
            Action::SetDefaultModel if picker.purpose == ModelPurpose::Pin => {
                if let (Some(choice), Some(chat)) =
                    (picker.selected_choice().cloned(), self.chat.chat)
                {
                    return vec![Effect::Send(Request::SetDefaultModel {
                        chat,
                        model: choice,
                    })];
                }
            }
            Action::ToggleModelMode if picker.purpose == ModelPurpose::Pin => {
                if let Some(chat) = self.chat.chat {
                    let mode = match picker.mode {
                        ModelMode::Auto => ModelMode::Manual,
                        ModelMode::Manual => ModelMode::Auto,
                    };
                    return vec![Effect::Send(Request::SetModelMode { chat, mode })];
                }
            }
            _ => {}
        }
        Vec::new()
    }

    /// engine가 알린 기본 모델과 선택 방식을 받는다. 이미 알던 값이 바뀌면 안내 한 줄을 남기고, 열린 `/model` 창에 반영한다.
    /// 기본 모델이 없으면 실행마다 한 번 처음 고르기 창을 열고 모든 provider의 목록을 요청한다.
    pub(super) fn on_model_settings(
        &mut self,
        chat: ChatId,
        default: Option<ModelChoice>,
        mode: ModelMode,
    ) -> Vec<Effect> {
        let known = self.chat.model_mode.is_some();
        if let (true, Some(model)) = (known && default != self.chat.model_default, &default) {
            let text = self
                .lang
                .tr(i18n::MODEL_DEFAULT_SET)
                .replace("{provider}", i18n::provider_name(model.provider))
                .replace("{model}", &model.model);
            self.push_cell(TranscriptCell::Warning(text));
        }
        if known && self.chat.model_mode != Some(mode) {
            let text = self
                .lang
                .tr(i18n::MODEL_MODE_SET)
                .replace("{mode}", self.lang.tr(i18n::model_mode_name(mode)));
            self.push_cell(TranscriptCell::Warning(text));
        }
        self.chat.model_default.clone_from(&default);
        self.chat.model_mode = Some(mode);
        if let Some(Window::Model(picker)) = &mut self.window {
            picker.default = default.clone();
            picker.mode = mode;
        }
        if default.is_none() && !self.default_model_asked {
            self.default_model_asked = true;
            self.open_window(Window::Model(ModelPicker::first_default(mode)));
            return vec![Effect::Send(Request::ListModels {
                chat,
                provider: None,
            })];
        }
        Vec::new()
    }

    /// 저장은 engine이 하고, 고정 상태와 안내 한 줄은 돌아오는 `ModelPinned`로 바뀐다.
    /// 채팅에 붙을 때 오는 `ModelPinned`도 같은 줄을 그려 다시 열어도 실시간과 같다.
    fn pin_model(&mut self, chat: ChatId, model: ModelChoice) -> Vec<Effect> {
        vec![Effect::Send(Request::SetModel { chat, model })]
    }

    pub(super) fn model_pinned_notice(&self, model: &ModelChoice) -> String {
        self.lang
            .tr(i18n::MODEL_PINNED)
            .replace("{provider}", i18n::provider_name(model.provider))
            .replace("{model}", &model.model)
    }
}

fn router_version_request(command: RouterVersionCommand) -> Request {
    match command {
        RouterVersionCommand::ResetThresholds => Request::Train {
            reset_thresholds: true,
            from: None,
        },
        RouterVersionCommand::TrainFrom(version) => Request::Train {
            reset_thresholds: false,
            from: Some(version),
        },
        RouterVersionCommand::Use(version) => Request::UseRouterVersion { version },
    }
}
