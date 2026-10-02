//! 입력창, 팝업, 제출과 명령, `Ctrl+C`, 피드백 답.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::TaskLabel;
use saturn_protocol::rpc::Request;
use saturn_protocol::state::InputState;

use super::{App, Effect, Window};
use crate::commands::{self, CommandError, SATURN_COMMANDS, SlashCommand};
use crate::i18n::{self, Lang};
use crate::keys::Action;
use crate::view::composer::Composer;
use crate::view::judge_version::JudgeVersionScreen;
use crate::view::popup::{self, Popup, PopupItem, PopupKind};
use crate::view::task_list::TaskList;
use crate::view::transcript::TranscriptCell;
use crate::view::usage::UsageScreen;

impl App {
    // cost: time O(n + p), heap O(n + p), stack O(1)
    // vars: n = 초안 길이, p = 팝업 후보 수
    // basis: estimate
    pub(super) fn on_composer_action(&mut self, action: Action) -> Vec<Effect> {
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
            Action::KillToEnd => self.edit(Composer::kill_to_end),
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
            Action::Interrupt => return self.interrupt(),
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
                .filter(|spec| !spec.values.is_empty())
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
                    .map(|spec| spec.values.iter().map(|v| value_item(v)).collect())
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
        let mut body = text.clone();
        for attachment in self.pending_attachments.drain(..) {
            body.push_str("\n\n");
            body.push_str(&attachment.to_attachment());
        }
        self.next_client_ref += 1;
        vec![
            Effect::RecordHistory(text),
            Effect::Send(Request::SubmitInput {
                chat,
                client_ref: self.next_client_ref,
                text: body,
                pinned_model: None,
                skip_relation: queued && self.chat.is_running(),
            }),
        ]
    }

    pub(super) fn clear_draft(&mut self) {
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
            SlashCommand::Permissions { mode } => chat.map(|chat| Request::SetPermissionMode {
                chat,
                mode: mode.to_owned(),
            }),
            SlashCommand::Send { target } => self
                .chat
                .queued_by_label(target)
                .map(|input| Request::SendNow { input: input.id }),
            SlashCommand::Cancel { target } => self.cancel_target(target),
            SlashCommand::Continue { target } => self.continue_target(target),
            SlashCommand::Feedback { correct } => return self.answer_feedback(Some(correct)),
            SlashCommand::Tasks => {
                self.open_window(Window::TaskList(TaskList::default()));
                Some(Request::ListTasks)
            }
            SlashCommand::Usage { range } => {
                self.open_window(Window::Usage(UsageScreen::new(range)));
                Some(Request::Usage { scope: range })
            }
            SlashCommand::Train {
                reset_thresholds,
                from,
            } => {
                self.train_reset = reset_thresholds;
                Some(Request::Train {
                    reset_thresholds,
                    from,
                })
            }
            SlashCommand::JudgeVersion => {
                self.open_window(Window::JudgeVersion(JudgeVersionScreen::default()));
                Some(Request::ListJudgeVersions)
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

    pub(super) fn interrupt(&mut self) -> Vec<Effect> {
        if self.window.as_ref().is_some_and(|w| !w.is_blocking()) {
            self.window = None;
            return Vec::new();
        }
        if self.popup.take().is_some() || self.composer.cancel_search() {
            return Vec::new();
        }
        if self.composer.clear() {
            self.history.reset();
            return Vec::new();
        }
        match self.chat.chat {
            Some(chat) if self.chat.is_running() => vec![Effect::Send(Request::Stop { chat })],
            _ => self.quit_effects(),
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
            self.push_cell(TranscriptCell::Correction {
                label: feedback.label,
            });
        }
        vec![Effect::Send(Request::AnswerFeedback {
            judgment: feedback.judgment,
            correct,
        })]
    }
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
