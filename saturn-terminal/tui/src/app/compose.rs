//! 입력창, 팝업, 제출과 명령, `Ctrl+C`, 피드백 답.
//! 설계: docs/design/tui.md

use std::path::{Path, PathBuf};
use std::time::Instant;

use saturn_protocol::ids::{InputId, TaskId, TaskLabel};
use saturn_protocol::rpc::{ModelMode, Request, UsageRange};
use saturn_protocol::state::InputState;

use super::{App, Directed, Effect, Window};
use crate::commands::{
    self, CommandError, DEFAULT_PERMISSION_MODE, ExtensionsAction, PERMISSION_CYCLE,
    SATURN_COMMANDS, SlashCommand,
};
use crate::i18n::{self, Lang};
use crate::keymap::Keymap;
use crate::keys::Action;
use crate::state::CorrectionPrompt;
use crate::view::composer::Composer;
use crate::view::constraints;
use crate::view::model_picker::ModelPicker;
use crate::view::popup::{self, Popup, PopupItem, PopupKind, PopupSuppress};
use crate::view::prune_window::PruneWindow;
use crate::view::router_version::RouterVersionScreen;
use crate::view::task_list::TaskList;
use crate::view::transcript::TranscriptCell;
use crate::view::usage::{UsageScreen, usage_request};

impl App {
    // cost: time O(n + p), heap O(n + p), stack O(1)
    // vars: n = 초안 길이, p = 팝업 후보 수
    // basis: estimate
    pub(super) fn on_composer_action(&mut self, action: Action, now: Instant) -> Vec<Effect> {
        match action {
            Action::Insert(c) => self.edit(|composer| composer.insert(c)),
            Action::OpenPopup(kind) => {
                let c = match kind {
                    PopupKind::Command | PopupKind::Value => '/',
                    PopupKind::File => '@',
                    PopupKind::Skill => '$',
                };
                self.edit(|composer| composer.insert(c));
            }
            Action::ShowShortcuts => self.open_window(Window::Shortcuts),
            Action::Newline => self.edit(Composer::newline),
            Action::Backspace => self.edit(Composer::backspace),
            Action::CursorLeft => self.edit(Composer::left),
            Action::CursorRight => self.edit(Composer::right),
            Action::LineStart => self.composer.line_start(),
            Action::LineEnd => self.composer.line_end(),
            Action::KillToEnd => self.edit(Composer::kill_to_end),
            Action::KillToStart => self.edit(Composer::kill_to_start),
            Action::DeleteWordBack => self.edit(Composer::delete_word_back),
            Action::Yank => self.edit(Composer::yank),
            Action::Up => self.composer.cursor_up(),
            Action::Down => self.composer.cursor_down(),
            Action::HistoryPrev => {
                if let Some(text) = self.history.older().map(str::to_string) {
                    self.composer.set_text(&text, true);
                }
            }
            Action::HistoryNext => {
                let text = self.history.newer().map(str::to_string);
                self.composer
                    .set_text(text.as_deref().unwrap_or_default(), text.is_some());
            }
            Action::HistorySearch => self.composer.start_or_next_search(),
            Action::ClearSelection => self.close_popup(),
            Action::ExternalEditor => return vec![Effect::OpenEditor],
            Action::RecallLatestInput => return self.recall_latest_input(),
            Action::Interrupt => return self.interrupt(false),
            Action::InterruptQuit => return self.interrupt(true),
            Action::Quit => return self.quit_effects(),
            Action::StopWork => return self.stop_work(),
            Action::Rewind => self.push_cell(TranscriptCell::Warning(
                self.lang.tr(i18n::REWIND_NOT_READY).to_string(),
            )),
            Action::CompleteCommand => {
                if self.composer.is_empty() {
                    self.edit(|composer| composer.insert('/'));
                } else {
                    self.popup_suppress = PopupSuppress::default();
                    self.refresh_popup();
                }
            }
            Action::CyclePermissionMode => return self.cycle_permission_mode(None),
            Action::EnterBoard => self.enter_board(now),
            Action::OpenTaskList => return self.run_command(SlashCommand::Tasks),
            Action::Redraw => return vec![Effect::Redraw],
            Action::ShowFullTranscript => self.toggle_full_transcript(),
            Action::Suspend => return vec![Effect::Suspend],
            Action::Submit => return self.submit(false),
            Action::SubmitQueued => return self.submit(true),
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn edit(&mut self, change: impl FnOnce(&mut Composer)) {
        change(&mut self.composer);
        self.history.reset();
        self.refresh_popup();
    }

    // cost: time O(n + p), heap O(n + p), stack O(1)
    // vars: n = 초안 길이, p = 팝업 후보 수
    // basis: estimate
    pub(super) fn on_popup_action(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Up => self.popup.iter_mut().for_each(Popup::up),
            Action::Down => self.popup.iter_mut().for_each(Popup::down),
            Action::Confirm | Action::PopupComplete => self.apply_popup_selection(),
            Action::Close => self.close_popup(),
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn apply_popup_selection(&mut self) {
        let Some(popup) = self.popup.take() else {
            return;
        };
        let Some(item) = popup.selected().cloned() else {
            return;
        };
        match popup.kind {
            PopupKind::Command => self.composer.set_text(&format!("/{} ", item.value), false),
            PopupKind::Value | PopupKind::File | PopupKind::Skill => {
                self.composer.replace_token(&format!("{} ", item.value));
            }
        }
        self.refresh_popup();
    }

    /// 같은 토큰에서는 다시 띄우지 않는다.
    pub(super) fn close_popup(&mut self) {
        if self.popup.take().is_some()
            && let Some((_, _, key)) = self.popup_target()
        {
            self.popup_suppress.suppress(&key);
        }
    }

    // cost: time O(n + p·q), heap O(n + p), stack O(1)
    // vars: n = 초안 길이, p = 팝업 후보 수, q = 거르는 글 길이
    // basis: estimate
    pub(super) fn refresh_popup(&mut self) {
        let Some((kind, query, key)) = self.popup_target() else {
            self.popup = None;
            return;
        };
        if !self.popup_suppress.allows(&key) {
            self.popup = None;
            return;
        }
        let candidates = self.popup_candidates(kind);
        let mut popup = match self.popup.take() {
            Some(popup) if popup.kind == kind => popup,
            _ => Popup::open(kind, Vec::new()),
        };
        popup.filter(&query, &candidates);
        self.popup = (!popup.items.is_empty()).then_some(popup);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 초안 길이
    // basis: estimate
    /// 팝업 종류, 거르는 글, 억제 키.
    pub(super) fn popup_target(&self) -> Option<(PopupKind, String, String)> {
        let text = self.composer.text();
        if let Some(rest) = text.strip_prefix('/').filter(|rest| !rest.contains('\n')) {
            let value = SATURN_COMMANDS
                .iter()
                .filter(|spec| !spec.values.is_empty() || spec.takes_provider)
                .find_map(|spec| rest.strip_prefix(spec.path)?.strip_prefix(' '));
            return match value {
                Some(arg) if arg.contains(' ') => None,
                Some(arg) => Some((PopupKind::Value, arg.to_string(), text.clone())),
                None => Some((PopupKind::Command, rest.to_string(), text.clone())),
            };
        }
        let token = self.composer.popup_token()?;
        let kind = match token.chars().next() {
            Some('@') => PopupKind::File,
            Some('$') => PopupKind::Skill,
            _ => return None,
        };
        Some((kind, token.clone(), token))
    }

    // cost: time O(p), heap O(p), stack O(1), io f
    // vars: p = 후보 수, f = 처음 파일 목록을 모을 때 읽는 폴더 수
    // basis: estimate
    pub(super) fn popup_candidates(&mut self, kind: PopupKind) -> Vec<PopupItem> {
        let lang = self.lang;
        match kind {
            PopupKind::Command => SATURN_COMMANDS
                .iter()
                .map(|spec| PopupItem {
                    value: spec.path.to_string(),
                    description: lang.tr(spec.description).to_string(),
                    source: i18n::SOURCE_SATURN.to_string(),
                })
                .chain(self.provider_items(false))
                .collect(),
            PopupKind::Value => {
                let text = self.composer.text();
                SATURN_COMMANDS
                    .iter()
                    .find(|spec| text.starts_with(&format!("/{} ", spec.path)))
                    .map(|spec| {
                        if spec.takes_provider {
                            self.start
                                .iter()
                                .flat_map(|start| &start.providers)
                                .map(|info| value_item(info.provider.as_str()))
                                .collect()
                        } else {
                            spec.values.iter().map(|v| value_item(v)).collect()
                        }
                    })
                    .unwrap_or_default()
            }
            PopupKind::File => self
                .file_cache
                .get_or_insert_with(|| popup::file_candidates(&self.workdir))
                .clone(),
            PopupKind::Skill => self.provider_items(true).collect(),
        }
    }

    // cost: time O(p), heap O(p), stack O(1)
    // vars: p = provider 명령 수
    // basis: estimate
    /// `skills`가 참이면 스킬, 거짓이면 provider 명령.
    pub(super) fn provider_items(&self, skills: bool) -> impl Iterator<Item = PopupItem> + '_ {
        self.provider_commands
            .iter()
            .filter(move |(_, info)| info.is_skill == skills)
            .map(move |(provider, info)| PopupItem {
                value: if skills {
                    format!("${}", info.name)
                } else {
                    info.name.clone()
                },
                description: info.description.clone(),
                source: i18n::provider_name(*provider).to_string(),
            })
    }

    /// 채팅을 아직 모르면 보내지 않고 초안을 유지한다.
    pub(super) fn submit(&mut self, queued: bool) -> Vec<Effect> {
        let text = self.composer.text();
        if text.trim().is_empty() {
            return Vec::new();
        }
        if let Some(effects) = self.submit_directed(&text) {
            return effects;
        }
        if let Some(command) = self.composer.shell_command() {
            self.clear_draft();
            return vec![Effect::RecordHistory(text), Effect::RunShell(command)];
        }
        match commands::parse(&text) {
            Err(error) => {
                let warning = command_error_text(self.lang, &error);
                self.push_cell(TranscriptCell::Warning(warning));
                Vec::new()
            }
            Ok(Some(SlashCommand::Provider { line })) => self.submit_text(line, queued),
            Ok(Some(command)) => {
                self.clear_draft();
                let mut effects = vec![Effect::RecordHistory(text)];
                effects.extend(self.run_command(command));
                effects
            }
            Ok(None) => self.submit_text(text, queued),
        }
    }

    // cost: time O(n + a), heap O(n + a), stack O(1)
    // vars: n = 원문 길이, a = 첨부 길이 합
    // basis: estimate
    pub(super) fn submit_text(&mut self, text: String, queued: bool) -> Vec<Effect> {
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        self.clear_draft();
        let body = self.with_attachments(&text);
        self.next_client_ref += 1;
        vec![
            Effect::RecordHistory(text),
            Effect::Send(Request::SubmitInput {
                chat,
                client_ref: self.next_client_ref,
                text: body,
                skip_relation: queued && self.chat.is_running(),
            }),
        ]
    }

    /// 허가를 거절한 작업에 말을 이어 쓰도록 입력창에 `[A]에게: `를 채워 연다. 쓰던 초안이 있으면 건드리지 않는다.
    pub(super) fn open_directed_draft(&mut self, task: TaskId, label: TaskLabel) {
        if !self.composer.is_empty() {
            return;
        }
        let prefix = self
            .lang
            .tr(i18n::DIRECTED_PREFIX)
            .replace("{label}", &label.0.to_string());
        self.set_draft(&prefix);
        self.directed = Some(Directed { task, prefix });
    }

    // cost: time O(n + a), heap O(n + a), stack O(1)
    // vars: n = 원문 길이, a = 첨부 길이 합
    // basis: estimate
    /// 접두가 그대로 있고 뒤에 말이 있으면 router를 거치지 않고 그 작업에 보낸다. 접두만 있으면 아무것도 보내지 않고 초안을
    /// 남긴다. 접두를 고쳤거나 지웠으면 대상을 잊고 `None`을 돌려 보통 입력으로 처리한다.
    fn submit_directed(&mut self, text: &str) -> Option<Vec<Effect>> {
        let directed = self.directed.clone()?;
        let Some(rest) = text.strip_prefix(directed.prefix.as_str()) else {
            self.directed = None;
            return None;
        };
        let rest = rest.trim();
        let chat = self.chat.chat;
        let (true, Some(chat)) = (!rest.is_empty(), chat) else {
            return Some(Vec::new());
        };
        self.directed = None;
        self.clear_draft();
        let body = self.with_attachments(rest);
        self.next_client_ref += 1;
        Some(vec![
            Effect::RecordHistory(rest.to_owned()),
            Effect::Send(Request::SubmitToTask {
                chat,
                client_ref: self.next_client_ref,
                task: directed.task,
                text: body,
            }),
        ])
    }

    /// 메인 에이전트의 다음 입력에 붙일 셸 결과를 원문 뒤에 붙이고 비운다.
    fn with_attachments(&mut self, text: &str) -> String {
        let mut body = text.to_owned();
        for attachment in self.pending_attachments.drain(..) {
            body.push_str("\n\n");
            body.push_str(&attachment.to_attachment());
        }
        body
    }

    /// `/plain`. 엔진과 따로 떠 있어 그리는 방식만 바꾸면 된다.
    pub(super) fn toggle_plain(&mut self) {
        self.plain = !self.plain;
        let text = if self.plain {
            i18n::PLAIN_ON
        } else {
            i18n::PLAIN_OFF
        };
        self.push_cell(TranscriptCell::Warning(self.lang.tr(text).to_string()));
    }

    pub(super) fn clear_draft(&mut self) {
        self.directed = None;
        self.composer.take();
        self.history.reset();
        self.popup = None;
    }

    // cost: time O(i + t), heap O(1), stack O(1)
    // vars: i = 입력 수, t = 작업 수
    // basis: estimate
    /// `Provider`는 `submit`이 원문으로 보내므로 여기서는 무시한다.
    pub(super) fn run_command(&mut self, command: SlashCommand) -> Vec<Effect> {
        let chat = self.chat.chat;
        let request = match command {
            SlashCommand::Help => {
                self.open_window(Window::Shortcuts);
                None
            }
            SlashCommand::Record { on } => chat.map(|chat| Request::SetRecording { chat, on }),
            SlashCommand::Permissions { mode } => {
                self.chat.permission_mode = Some(mode);
                chat.map(|chat| Request::SetPermissionMode {
                    chat,
                    mode: mode.to_owned(),
                })
            }
            SlashCommand::AddDir { path } => chat.map(|chat| Request::AddDir {
                chat,
                path: absolute_path(&self.workdir, &path),
            }),
            SlashCommand::Extensions(action) => extensions_request(&self.workdir, chat, action),
            SlashCommand::Constraints(action) => return self.constraints_command(action),
            SlashCommand::Send { target } => self
                .chat
                .queued_by_label(target)
                .map(|input| Request::SendNow { input: input.id }),
            SlashCommand::Cancel { target } => self.cancel_target(target),
            SlashCommand::Continue { target } => self.continue_target(target),
            SlashCommand::Feedback { correct } => return self.answer_feedback(Some(correct)),
            SlashCommand::ReopenCorrection => {
                self.reopen_correction();
                None
            }
            SlashCommand::Mode { target } => return self.cycle_permission_mode(target),
            SlashCommand::Agents => {
                self.enter_board(Instant::now());
                None
            }
            SlashCommand::Stop => return self.stop_work(),
            SlashCommand::Rewind => return self.on_composer_action(Action::Rewind, Instant::now()),
            SlashCommand::Keymap { name } => {
                self.set_keymap(name);
                None
            }
            SlashCommand::Transcript => {
                self.toggle_full_transcript();
                None
            }
            SlashCommand::Redraw => return vec![Effect::Redraw],
            SlashCommand::Plain => {
                self.toggle_plain();
                None
            }
            SlashCommand::Suspend => return vec![Effect::Suspend],
            SlashCommand::Quit => return self.quit_effects(),
            SlashCommand::Tasks => {
                let folder = self.chat_folder.clone();
                self.open_window(Window::TaskList(TaskList::for_folder(folder)));
                Some(Request::ListTasks)
            }
            SlashCommand::Usage => {
                self.open_window(Window::Usage(UsageScreen::new(UsageRange::Chat)));
                Some(usage_request(UsageRange::Chat))
            }
            SlashCommand::Prune => {
                self.open_window(Window::Prune(PruneWindow::default()));
                Some(prune_preview())
            }
            SlashCommand::RouterVersion => {
                self.open_window(Window::RouterVersion(RouterVersionScreen::default()));
                Some(Request::ListRouterVersions)
            }
            SlashCommand::Model { provider } => {
                let mut picker = ModelPicker::new(provider, self.chat.pinned_model.clone());
                picker.default.clone_from(&self.chat.model_default);
                picker.mode = self.chat.model_mode.unwrap_or(ModelMode::Manual);
                self.open_window(Window::Model(picker));
                chat.map(|chat| Request::ListModels { chat, provider })
            }
            SlashCommand::Provider { .. } => None,
        };
        request.map(Effect::Send).into_iter().collect()
    }

    pub(super) fn cancel_target(&mut self, target: Option<TaskLabel>) -> Option<Request> {
        if let Some(input) = self.chat.queued_by_label(target) {
            return Some(Request::CancelInput { input: input.id });
        }
        let task = self.chat.held_by_label(target?)?.id;
        self.chat.close_held_confirm = Some(task);
        None
    }

    /// 이름표가 없으면 채팅의 보류 전부.
    pub(super) fn continue_target(&self, target: Option<TaskLabel>) -> Option<Request> {
        let chat = self.chat.chat?;
        let task = match target {
            Some(label) => Some(self.chat.held_by_label(label)?.id),
            None => None,
        };
        Some(Request::Continue { chat, task })
    }

    /// 실행 중인 작업을 모두 멈추고 보류한다. 멈출 것이 없으면 아무것도 하지 않는다.
    pub(super) fn stop_work(&mut self) -> Vec<Effect> {
        match self.chat.chat {
            Some(chat) if self.chat.is_running() => vec![Effect::Send(Request::Stop { chat })],
            _ => Vec::new(),
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `mode`가 없으면 `PERMISSION_CYCLE` 순서로 다음 모드를 고르고, 새 모드를 대화 기록에 한 줄 남긴다.
    pub(super) fn cycle_permission_mode(&mut self, mode: Option<&'static str>) -> Vec<Effect> {
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        let next = mode.unwrap_or_else(|| {
            let current = self.chat.permission_mode.unwrap_or(DEFAULT_PERMISSION_MODE);
            match PERMISSION_CYCLE.iter().position(|name| *name == current) {
                Some(at) => PERMISSION_CYCLE[(at + 1) % PERMISSION_CYCLE.len()],
                None => PERMISSION_CYCLE[0],
            }
        });
        self.chat.permission_mode = Some(next);
        let line = format!("{}: {next}", self.lang.tr(i18n::PERMISSION_MODE));
        self.push_cell(TranscriptCell::Warning(line));
        vec![Effect::Send(Request::SetPermissionMode {
            chat,
            mode: next.to_owned(),
        })]
    }

    /// `name`이 없으면 지금 키 묶음과 목록을 한 줄로 보인다. 이 TUI만 바꾸고 설정 파일은 고치지 않는다.
    pub(super) fn set_keymap(&mut self, name: Option<&'static str>) {
        let line = match name.map(Keymap::load) {
            None => format!(
                "{}: {} ({})",
                self.lang.tr(i18n::KEYMAP),
                self.keymap.name(),
                saturn_protocol::keymap::PRESET_NAMES.join(", ")
            ),
            Some(Ok(keymap)) => {
                self.keymap = keymap;
                format!("{}: {}", self.lang.tr(i18n::KEYMAP), self.keymap.name())
            }
            Some(Err(error)) => format!("{}: {error}", self.lang.tr(i18n::KEYMAP)),
        };
        self.push_cell(TranscriptCell::Warning(line));
    }

    /// 열린 창과 초안을 먼저 정리하고, 없으면 작업을 멈추고, 멈출 것이 없으면 종료한다.
    /// `quit_now`가 아니면 종료는 두 번째로 누를 때다(첫 번째는 안내 한 줄).
    pub(super) fn interrupt(&mut self, quit_now: bool) -> Vec<Effect> {
        if self.board_focus.take().is_some() {
            return Vec::new();
        }
        if self.window.as_ref().is_some_and(|w| !w.is_blocking()) {
            self.window = None;
            return Vec::new();
        }
        if self.popup.take().is_some() || self.composer.cancel_search() {
            return Vec::new();
        }
        if self.composer.clear() {
            self.directed = None;
            self.history.reset();
            return Vec::new();
        }
        match self.chat.chat {
            Some(chat) if self.chat.is_running() => vec![Effect::Send(Request::Stop { chat })],
            _ if quit_now || self.quit_armed => self.quit_effects(),
            _ => {
                self.quit_armed = true;
                self.push_cell(TranscriptCell::Warning(
                    self.lang.tr(i18n::QUIT_AGAIN).to_string(),
                ));
                Vec::new()
            }
        }
    }

    /// 원문을 모르는 입력은 취소만 한다.
    pub(super) fn recall_latest_input(&mut self) -> Vec<Effect> {
        let Some(input) = self.chat.latest_recallable() else {
            return Vec::new();
        };
        let (id, text) = (input.id, input.text.clone());
        if let Some(text) = text {
            self.composer.set_text(&text, false);
            self.refresh_popup();
        }
        vec![Effect::Send(Request::CancelInput { input: id })]
    }

    /// `Some(true)` 맞음, `Some(false)` 아님, `None` 닫기(요청 없음).
    pub(super) fn answer_feedback(&mut self, correct: Option<bool>) -> Vec<Effect> {
        let Some(feedback) = self.chat.feedback.take() else {
            return Vec::new();
        };
        self.transcript.remove_feedback(feedback.label);
        let Some(correct) = correct else {
            return Vec::new();
        };
        let unsent =
            self.chat.inputs.get(&feedback.input).is_some_and(|input| {
                matches!(input.state, InputState::Judging | InputState::Queued)
            });
        if !correct && unsent {
            self.offer_correction(feedback.input, feedback.label);
        }
        vec![Effect::Send(Request::AnswerFeedback {
            judgment: feedback.judgment,
            correct,
        })]
    }

    /// 피드백 질문의 선택지. `↑`, `↓`는 돌아가며 옮기고 `Enter`는 고른 답, `Esc`는 닫기다.
    pub(super) fn on_feedback_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(feedback) = self.chat.feedback.as_mut() else {
            return Vec::new();
        };
        match action {
            Action::Up | Action::Down => {
                feedback.selected = step(feedback.selected, FEEDBACK_ANSWERS.len(), action);
                let (label, selected) = (feedback.label, feedback.selected);
                self.transcript.set_choice(label, selected);
                Vec::new()
            }
            Action::Confirm => {
                let answer = FEEDBACK_ANSWERS[feedback.selected];
                self.answer_feedback(answer)
            }
            Action::Close => self.answer_feedback(None),
            _ => Vec::new(),
        }
    }

    /// 바로잡기 제안의 선택지. `↑`, `↓`는 `[실행]`과 `[그대로]`를 오가고 `Enter`는 고른 쪽, `Esc`는 닫기다.
    pub(super) fn on_correction_action(&mut self, action: Action) -> Vec<Effect> {
        let Some(correction) = self.chat.correction.as_mut() else {
            return Vec::new();
        };
        match action {
            Action::Up | Action::Down => {
                correction.selected = step(correction.selected, CORRECTION_CHOICES, action);
                let (label, selected) = (correction.label, correction.selected);
                self.transcript.set_choice(label, selected);
                Vec::new()
            }
            Action::Confirm => {
                let run = correction.selected == 0;
                self.finish_correction(run)
            }
            Action::Close => {
                self.close_correction();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn offer_correction(&mut self, input: InputId, label: TaskLabel) {
        if let Some(previous) = self.chat.correction.take() {
            self.transcript.remove_correction(previous.label);
        }
        self.chat.correction = Some(CorrectionPrompt {
            input,
            label,
            selected: 0,
            open: true,
        });
        self.push_cell(TranscriptCell::Correction { label, selected: 0 });
    }

    /// `run`이면 입력을 새 작업으로 보내라고 요청하고, 아니면 입력을 그대로 둔다.
    pub(super) fn finish_correction(&mut self, run: bool) -> Vec<Effect> {
        let Some(correction) = self.chat.correction.take() else {
            return Vec::new();
        };
        self.transcript.remove_correction(correction.label);
        if run {
            vec![Effect::Send(Request::RunAsNewTask {
                input: correction.input,
            })]
        } else {
            Vec::new()
        }
    }

    /// 제안은 남기고 화면에서만 접는다. 방향키는 입력창의 입력 기록으로 돌아간다.
    fn close_correction(&mut self) {
        if let Some(correction) = self.chat.correction.as_mut() {
            correction.open = false;
            let label = correction.label;
            self.transcript.remove_correction(label);
        }
    }

    /// `/feedback`을 인자 없이 실행하면 `Esc`로 닫은 바로잡기 제안을 다시 연다.
    fn reopen_correction(&mut self) {
        let Some(correction) = self.chat.correction.as_mut() else {
            return;
        };
        if correction.open {
            return;
        }
        correction.open = true;
        correction.selected = 0;
        let label = correction.label;
        self.push_cell(TranscriptCell::Correction { label, selected: 0 });
    }

    /// `/constraints`의 요청을 만든다. 채팅에 붙기 전이거나 번호가 목록과 맞지 않으면 대화 기록에 한 줄 남기고 보내지 않는다.
    fn constraints_command(
        &mut self,
        action: crate::constraints::ConstraintsAction,
    ) -> Vec<Effect> {
        let Some(chat) = self.chat.chat else {
            return Vec::new();
        };
        match self.constraints.requests(chat, action) {
            Ok(requests) => requests.into_iter().map(Effect::Send).collect(),
            Err(error) => {
                let line = constraints::error_line(self.lang, error);
                self.push_cell(TranscriptCell::Warning(line));
                Vec::new()
            }
        }
    }

    /// 입력이 이미 보내졌거나 취소됐으면 제안을 지운다.
    pub(super) fn drop_stale_correction(&mut self) {
        let Some(correction) = &self.chat.correction else {
            return;
        };
        let unsent = self
            .chat
            .inputs
            .get(&correction.input)
            .is_some_and(|input| matches!(input.state, InputState::Judging | InputState::Queued));
        if !unsent {
            let label = correction.label;
            self.chat.correction = None;
            self.transcript.remove_correction(label);
        }
    }
}

/// 선택지 수가 `count`일 때 `Up`은 앞, `Down`은 뒤로 돌아가며 옮긴다.
fn step(selected: usize, count: usize, action: Action) -> usize {
    match action {
        Action::Up => (selected + count - 1) % count,
        _ => (selected + 1) % count,
    }
}

/// 피드백 질문의 선택지 순서(`1` 맞음, `2` 틀림, `0` 닫기). `None`은 닫기다.
const FEEDBACK_ANSWERS: [Option<bool>; 3] = [Some(true), Some(false), None];
const CORRECTION_CHOICES: usize = 2;

/// `/extensions`의 요청. 목록 말고는 채팅에 붙어 있어야 한다.
fn extensions_request(
    workdir: &Path,
    chat: Option<saturn_protocol::ids::ChatId>,
    action: ExtensionsAction,
) -> Option<Request> {
    match action {
        ExtensionsAction::List => Some(Request::ListExtensions),
        ExtensionsAction::Install { source } => chat.map(|chat| Request::InstallExtension {
            chat,
            source: extension_source(workdir, &source),
        }),
        ExtensionsAction::Remove { name } => {
            chat.map(|chat| Request::RemoveExtension { chat, name })
        }
        ExtensionsAction::Move { provider, name } => {
            chat.map(|chat| Request::MoveDirectExtension {
                chat,
                provider,
                name,
            })
        }
    }
}

/// git 주소는 그대로, 폴더 경로는 `absolute_path`로 바꾼다. engine은 절대 경로와 git 주소만 받는다.
fn extension_source(workdir: &Path, source: &str) -> String {
    if source.contains("://") || source.starts_with("git@") {
        source.to_owned()
    } else {
        absolute_path(workdir, source)
    }
}

/// 상대 경로는 TUI의 현재 폴더 기준 절대 경로로 바꾸고, `~/`로 시작하면 홈 폴더 아래로 읽는다. engine은 절대 경로만 받는다.
fn absolute_path(workdir: &Path, path: &str) -> String {
    let home_relative = path
        .strip_prefix("~/")
        .zip(std::env::var_os("HOME"))
        .map(|(rest, home)| PathBuf::from(home).join(rest));
    home_relative
        .unwrap_or_else(|| workdir.join(path))
        .display()
        .to_string()
}

fn value_item(value: &str) -> PopupItem {
    PopupItem {
        value: value.to_string(),
        description: String::new(),
        source: i18n::SOURCE_SATURN.to_string(),
    }
}

fn command_error_text(lang: Lang, error: &CommandError) -> String {
    match error {
        CommandError::Unknown { name } => {
            format!("{}: /{name}", lang.tr(i18n::COMMAND_UNKNOWN))
        }
        CommandError::InvalidArgument { command, argument } => {
            format!("{}: /{command} {argument}", lang.tr(i18n::COMMAND_INVALID))
        }
    }
}

/// 정리 미리보기 요청. 확인 번호는 미리보기가 돌려준다.
fn prune_preview() -> Request {
    Request::Prune {
        yes: false,
        plan: None,
        all: false,
    }
}
