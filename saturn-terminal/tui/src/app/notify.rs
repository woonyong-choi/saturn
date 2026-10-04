//! engine 알림을 화면 상태에 반영한다.
//! 설계: docs/design/tui.md

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use saturn_protocol::envelope::RequestId;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{
    ChatId, ConstraintAskId, InputId, JudgmentId, LedgerSeq, SettingsRevision, TaskId, TaskLabel,
};
use saturn_protocol::rpc::{
    Alert, ChatNotice, ExitPlan, Notification, QueryResult, Request, SettingsWarning,
};
use saturn_protocol::state::{Disposition, InputState, QueueReason};

use super::{App, Effect, Window};
use crate::client::Rejection;
use crate::i18n;
use crate::keymap::Keymap;
use crate::state::{
    Change, ChatState, ContextSize, FeedbackPrompt, InputUpdate, TaskUpdate, TrainingProgress,
};
use crate::view::constraint_ask::ConstraintAsk;
use crate::view::exit_confirm::ExitConfirm;
use crate::view::folder_trust::{FolderTrust, TrustChoice};
use crate::view::live_area::LiveArea;
use crate::view::permission::PermissionRequest;
use crate::view::resume_prompt::ResumePrompt;
use crate::view::router_key_prompt::RouterKeyPrompt;
use crate::view::router_version::RouterVersionRow;
use crate::view::start_screen::StartInfo;
use crate::view::stop_confirm::StopConfirm;
use crate::view::task_list::ChatGroup;
use crate::view::train_confirm::{TrainChoice, TrainConfirm};
use crate::view::transcript::{TranscriptCell, delivery_badge, echo_cell, result_cell};
use crate::view::usage::UsageTable;

/// 거절 응답을 기다리며 원문을 들고 있는 입력 요청 수의 상한. 초안 값.
const SENT_INPUTS_KEPT: usize = 64;

impl App {
    fn on_start_info(&mut self, notification: &Notification) {
        self.start = StartInfo::from_notification(notification);
        if let Some(start) = &self.start {
            i18n::set_provider_names(
                start
                    .providers
                    .iter()
                    .map(|info| (info.provider, info.display_name.as_str())),
            );
        }
        self.chat_folder = self.start.as_ref().map(|start| start.folder.clone());
    }

    pub(super) fn on_notification(
        &mut self,
        notification: Notification,
        now: Instant,
    ) -> Vec<Effect> {
        if self.plain && needs_input(&notification) {
            self.bell = true;
        }
        match notification {
            Notification::StartInfo { .. } => self.on_start_info(&notification),
            Notification::InputAccepted { .. } => {}
            changed @ Notification::InputChanged { .. } => {
                if let Some((update, _)) = InputUpdate::of(changed) {
                    self.on_input_changed(update, now);
                }
            }
            Notification::TaskChanged {
                task,
                label,
                state,
                provider,
                elapsed_ms,
                failure,
            } => self.on_task_changed(
                TaskUpdate {
                    task,
                    label,
                    state,
                    provider,
                    elapsed: Duration::from_millis(elapsed_ms),
                    failure,
                },
                now,
            ),
            Notification::HistoryChunk {
                chat,
                entries,
                oldest,
                has_more,
            } => return self.on_history_chunk(chat, entries, oldest, has_more, now),
            Notification::PermissionRequested {
                task,
                label,
                provider,
                request_id,
                summary,
                reason,
                ..
            } => self.permissions.push(
                PermissionRequest {
                    request_id,
                    task,
                    label,
                    provider: Some(provider),
                    summary,
                    reason,
                },
                now,
            ),
            Notification::PermissionResolved { request_id } => {
                self.permissions.resolve(&request_id, now);
            }
            Notification::InputRequested {
                task,
                label,
                provider,
                request_id,
                request,
                ..
            } => self
                .inputs
                .push(request_id, task, label, Some(provider), request, now),
            Notification::InputResolved { request_id } => {
                self.inputs.resolve(&request_id, now);
            }
            Notification::ModelSettings {
                chat,
                default,
                mode,
            } => {
                if self.chat.chat == Some(chat) {
                    return self.on_model_settings(chat, default, mode);
                }
            }
            Notification::ChatLabeled { .. } => {
                if matches!(self.window, Some(Window::TaskList(_))) {
                    return vec![Effect::Send(Request::ListTasks)];
                }
            }
            other => self.on_window_notification(other, now),
        }
        Vec::new()
    }

    // cost: time O(e), heap O(e), stack O(1)
    // vars: e = 결과에 실린 행 수
    // basis: estimate
    /// 조회 요청의 결과를 그 결과를 기다리는 창에 반영한다. 창이 이미 닫혔으면 버린다.
    pub(super) fn on_result(&mut self, result: QueryResult, now: Instant) -> Vec<Effect> {
        match result {
            QueryResult::History {
                chat,
                entries,
                oldest,
                has_more,
            } => return self.on_history_chunk(chat, entries, oldest, has_more, now),
            QueryResult::ExitPlan { plan } => return self.on_exit_plan(plan),
            QueryResult::ExtensionList { extensions } => {
                self.push_cell(TranscriptCell::ExtensionList(extensions));
            }
            QueryResult::Tasks { items } => {
                if let Some(Window::TaskList(list)) = &mut self.window {
                    list.replace(ChatGroup::from_items(items));
                }
            }
            QueryResult::Usage { range, rows } => {
                if let Some(Window::Usage(screen)) = &mut self.window {
                    screen.table = Some(UsageTable { range, rows });
                }
            }
            QueryResult::PrunePreview {
                chats,
                skipped,
                rows,
                plan,
            } => {
                if let Some(Window::Prune(window)) = &mut self.window {
                    window.load(chats, &skipped, rows, plan);
                }
            }
            QueryResult::Pruned { chats, rows, .. } => self.on_pruned(chats.len(), rows),
            QueryResult::Models { models } => {
                if let Some(Window::Model(picker)) = &mut self.window {
                    picker.load(models);
                }
            }
            QueryResult::RouterVersions { current, versions } => {
                if let Some(Window::RouterVersion(screen)) = &mut self.window {
                    screen.rows = versions
                        .into_iter()
                        .map(|info| RouterVersionRow::from_info(info, &current))
                        .collect();
                    screen.selected = screen.selected.min(screen.rows.len().saturating_sub(1));
                }
            }
            // TUI는 채팅 목록과 폴더의 최근 채팅을 묻지 않는다. 그런 조회는 `cli`가 붙기 전에 쓴다
            QueryResult::LatestChat { .. } | QueryResult::Chats { .. } => {}
        }
        Vec::new()
    }

    /// 요청한 적 없는 계획은 무시한다.
    fn on_exit_plan(&mut self, plan: ExitPlan) -> Vec<Effect> {
        if !self.exit_requested {
            return Vec::new();
        }
        match plan {
            ExitPlan::Close => self.quit_now(),
            ExitPlan::Notice { running } => {
                self.exit_notice = Some(running);
                self.quit_now()
            }
            ExitPlan::Ask { running } => {
                self.exit_requested = false;
                self.exit_confirm = Some(ExitConfirm::new(running));
                Vec::new()
            }
        }
    }

    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 알림에 실린 행 수
    // basis: estimate
    fn on_window_notification(&mut self, notification: Notification, now: Instant) {
        match notification {
            Notification::FolderTrustRequested {
                path,
                fingerprint,
                applied,
                ignored,
                changed_lines,
            } => self.open_window(Window::FolderTrust(FolderTrust {
                path: PathBuf::from(path),
                fingerprint,
                applied,
                ignored,
                changed: changed_lines,
                selected: TrustChoice::Apply,
            })),
            Notification::RouterKeyRequired { reason } => {
                self.window = Some(Window::RouterKey(RouterKeyPrompt {
                    cause: reason,
                    input: Default::default(),
                }));
            }
            Notification::Commands { provider, commands } => {
                self.provider_commands.retain(|(p, _)| *p != provider);
                self.provider_commands
                    .extend(commands.into_iter().map(|info| (provider, info)));
            }
            Notification::ModelPinned { chat, model } => {
                if self.chat.chat == Some(chat) {
                    self.push_cell(TranscriptCell::Warning(self.model_pinned_notice(&model)));
                    self.chat.pinned_model = Some(model);
                }
            }
            other => self.on_progress_notification(other, now),
        }
    }

    fn on_progress_notification(&mut self, notification: Notification, now: Instant) {
        match notification {
            Notification::TrainPreview {
                candidates,
                grader,
                estimated_tokens,
                threshold_targets,
                retrain_model,
            } => self.open_window(Window::TrainConfirm(TrainConfirm {
                candidates,
                grading_model: grader,
                estimated_tokens,
                threshold_targets,
                fine_tune: retrain_model,
                reset_thresholds: self.train_reset,
                selected: TrainChoice::Run,
            })),
            Notification::TrainProgress {
                stage,
                labeled,
                elapsed_ms,
                tokens,
            } => {
                self.chat.training = Some(TrainingProgress {
                    stage,
                    graded: labeled,
                    elapsed: Duration::from_millis(elapsed_ms),
                    tokens,
                });
            }
            Notification::TaskEvent { task, event } => self.on_task_event(task, event, now),
            Notification::ChatNotice { chat, task, notice } => {
                self.on_chat_notice(chat, task, notice);
            }
            Notification::ConstraintAsked { ask, rule, .. } => {
                self.constraint_asks.push(ConstraintAsk::new(ask, rule));
                self.show_next_constraint_ask();
            }
            Notification::ConstraintAskResolved { ask } => self.on_constraint_ask_resolved(ask),
            Notification::FeedbackQuestion {
                judgment,
                input,
                label,
                disposition,
            } => self.on_feedback_question(judgment, input, label, disposition, now),
            Notification::ContextSize {
                chat,
                tokens,
                threshold,
            } => {
                self.learn_chat(chat);
                self.chat.context = Some(ContextSize { tokens, threshold });
            }
            Notification::SettingsApplied {
                revision,
                warning,
                keymap,
                screen,
            } => {
                self.on_settings_applied(revision, warning);
                self.on_settings_keymap(keymap);
                self.on_settings_screen(screen);
            }
            Notification::Alert { alert } => self.on_alert(alert),
            _ => {}
        }
    }

    fn on_input_changed(&mut self, update: InputUpdate, now: Instant) {
        let cell = echo_cell(&update);
        let (input, state) = (update.input, update.state);
        self.sync_stop_confirm(&update);
        match self.chat.apply_input(update, now) {
            Change::Echo { .. } => self.push_cell(cell),
            _ => {
                if let Some(badge) = delivery_badge(state) {
                    self.transcript.set_badge(input, badge);
                }
            }
        }
        self.drop_stale_correction();
    }

    /// 멈출지 묻는 입력이 오면 창을 띄우고, 그 입력이 답을 받아 다른 상태가 되면(다른 TUI가 먼저 답한 경우 포함) 창을 지운다.
    fn sync_stop_confirm(&mut self, update: &InputUpdate) {
        let is_asking =
            update.state == InputState::Queued && update.reason == Some(QueueReason::ConfirmStop);
        let shown = match &self.window {
            Some(Window::StopConfirm(confirm)) => Some(confirm.input),
            _ => None,
        };
        if is_asking && shown != Some(update.input) {
            let text = update.text.clone();
            self.open_window(Window::StopConfirm(StopConfirm::new(update.input, text)));
        } else if !is_asking && shown == Some(update.input) {
            self.window = None;
        }
    }

    /// 보류 재개 질문은 첫 기록을 모두 반영한 뒤 `on_history_chunk`가 띄운다.
    fn on_task_changed(&mut self, update: TaskUpdate, now: Instant) {
        let task = update.task;
        let change = self.chat.apply_task(update, now);
        for call_id in self.chat.take_interrupted_calls(task) {
            self.transcript.set_tool_interrupted(&call_id);
        }
        match change {
            Change::TaskFinished { task } => {
                let lines = self.live.finish(task);
                let Some(view) = self.chat.finish_task(task) else {
                    return;
                };
                if !lines.is_empty() {
                    self.push_cell(TranscriptCell::AgentText {
                        label: Some(view.label),
                        lines,
                    });
                }
                self.push_cell(result_cell(&view));
            }
            Change::TaskNeedsCheck { task } => {
                if let Some(view) = self.chat.tasks.get(&task) {
                    let label = view.label;
                    self.push_cell(TranscriptCell::NeedsCheck { label });
                }
            }
            _ => {}
        }
    }

    // cost: time O(e), heap O(e), stack O(d)
    // vars: e = 기록 항목 수, d = 접속 직후 묶음 반영의 호출 깊이(1)
    // basis: estimate
    /// 접속 직후 첫 묶음은 실시간 알림처럼 반영하고, 그 뒤 묶음은 대화 기록 셀로 바꿔 앞에 넣는다.
    fn on_history_chunk(
        &mut self,
        chat: ChatId,
        entries: Vec<Notification>,
        oldest: Option<LedgerSeq>,
        has_more: bool,
        now: Instant,
    ) -> Vec<Effect> {
        self.learn_chat(chat);
        self.history_before = oldest;
        self.history_has_more = has_more;
        self.history_loading = false;
        if self.history_loaded {
            self.transcript.prepend(history_cells(entries));
            return Vec::new();
        }
        self.history_loaded = true;
        let effects: Vec<Effect> = entries
            .into_iter()
            .flat_map(|entry| self.on_notification(entry, now))
            .collect();
        self.ask_resume();
        effects
    }

    // cost: time O(t log t), heap O(t), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 채팅을 열 때 한 번만 묻는다.
    fn ask_resume(&mut self) {
        if self.resume_asked {
            return;
        }
        self.resume_asked = true;
        let held: Vec<(TaskId, TaskLabel)> = self
            .chat
            .held_tasks()
            .iter()
            .map(|task| (task.id, task.label))
            .collect();
        if !held.is_empty() {
            self.open_window(Window::Resume(ResumePrompt::new(held)));
        }
    }

    fn on_task_event(&mut self, task: TaskId, event: ProviderEvent, now: Instant) {
        let change = self.chat.apply_event(task, event, now);
        let Some(view) = self.chat.tasks.get(&task) else {
            return;
        };
        let (label, provider) = (view.label, view.provider);
        match change {
            Change::Output { text, .. } => {
                self.live.push(task, label, &text);
            }
            Change::Tool {
                call_id,
                activity: Some(activity),
                ..
            } => self.push_cell(TranscriptCell::Tool {
                label: Some(label),
                call_id,
                activity,
                output: String::new(),
                is_interrupted: false,
            }),
            Change::Tool {
                call_id,
                output: Some(output),
                ..
            } => {
                self.transcript.set_tool_output(&call_id, output);
            }
            Change::PermissionRequested {
                request_id,
                summary,
                reason,
                ..
            } => self.permissions.push(
                PermissionRequest {
                    request_id,
                    task,
                    label,
                    provider,
                    summary,
                    reason,
                },
                now,
            ),
            _ => {}
        }
    }

    // cost: time O(log t + c), heap O(1), stack O(1)
    // vars: t = 작업 수, c = 대화 기록 셀 수(위로 스크롤 중 줄 수 계산)
    // basis: estimate
    fn on_chat_notice(&mut self, chat: ChatId, task: Option<TaskId>, notice: ChatNotice) {
        self.learn_chat(chat);
        let view = task.and_then(|task| self.chat.tasks.get_mut(&task));
        if let (Some(view), ChatNotice::ProviderSwitched { to, .. }) = (view, &notice) {
            view.provider = Some(*to);
        }
        let label = task.and_then(|task| self.chat.tasks.get(&task).map(|view| view.label));
        match notice {
            ChatNotice::Stopped { held } => self.chat.apply_stopped(held),
            ChatNotice::StopUnconfirmed { remaining } => {
                self.chat.apply_stop_unconfirmed(remaining);
            }
            ChatNotice::FolderAdded {
                path,
                applies_from_next_session,
            } => {
                let dir = PathBuf::from(&path);
                match &mut self.start {
                    Some(start) => start.added_dirs.push(dir),
                    None => self.transcript.add_header_dir(dir),
                }
                self.push_cell(TranscriptCell::Notice {
                    label,
                    notice: ChatNotice::FolderAdded {
                        path,
                        applies_from_next_session,
                    },
                });
            }
            notice => self.push_cell(TranscriptCell::Notice { label, notice }),
        }
    }

    /// 다른 TUI가 먼저 답했거나 대상이 바뀌어 닫힌 확인은 창에서 지우고 다음 확인을 띄운다.
    fn on_constraint_ask_resolved(&mut self, ask: ConstraintAskId) {
        self.constraint_asks.remove(ask);
        let is_shown =
            matches!(&self.window, Some(Window::ConstraintAsk(shown)) if shown.ask == ask);
        if is_shown {
            self.window = None;
        }
        self.show_next_constraint_ask();
    }

    fn on_feedback_question(
        &mut self,
        judgment: JudgmentId,
        input: InputId,
        label: TaskLabel,
        disposition: Disposition,
        now: Instant,
    ) {
        if let Some(previous) = self.chat.feedback.take() {
            self.transcript.remove_feedback(previous.label);
        }
        self.chat.feedback = Some(FeedbackPrompt {
            judgment,
            input,
            label,
            disposition,
            shown_at: now,
            selected: 0,
        });
        self.push_cell(TranscriptCell::Feedback {
            label,
            disposition,
            selected: 0,
        });
    }

    /// 설정의 `tui.screen`이 바뀌었을 때만 따른다. 옵션과 환경 변수가 정했으면 따르지 않고, 같은 값이 다시 오면
    /// `/plain`으로 바꾼 상태를 그대로 둔다. `plain`만 단순 방식이고 `auto`와 `full`은 전체 화면이다.
    fn on_settings_screen(&mut self, screen: Option<String>) {
        let Some(name) = screen else {
            return;
        };
        if self.settings_screen.as_deref() == Some(name.as_str()) {
            return;
        }
        if !self.plain_fixed {
            self.plain = name == "plain";
        }
        self.settings_screen = Some(name);
    }

    /// 설정의 `tui.keymap`이 바뀌었을 때만 따른다. 같은 값이 다시 오면 `/keymap`으로 고른 묶음을 지키지 않고 바꾸지도 않는다.
    fn on_settings_keymap(&mut self, keymap: Option<String>) {
        let Some(name) = keymap else {
            return;
        };
        if self.settings_keymap.as_deref() == Some(name.as_str()) {
            return;
        }
        if let Ok(loaded) = Keymap::load(&name) {
            self.keymap = loaded;
        }
        self.settings_keymap = Some(name);
    }

    fn on_settings_applied(
        &mut self,
        revision: SettingsRevision,
        warning: Option<SettingsWarning>,
    ) {
        self.chat.settings = Some((revision, warning));
    }

    /// 보낸 입력의 원문을 기억한다. 오래된 것은 접수됐다고 보고 버린다.
    pub(crate) fn note_sent(&mut self, id: RequestId, request: &Request) {
        let Request::SubmitInput { text, .. } = request else {
            return;
        };
        self.sent_inputs.insert(id, text.clone());
        while self.sent_inputs.len() > SENT_INPUTS_KEPT {
            self.sent_inputs.pop_first();
        }
    }

    /// 거절된 요청 하나만 알린다. 접수하지 못한 입력은 입력창이 비었을 때 되돌려 다시 고치게 하고, 자동으로 다시 보내지 않는다.
    pub(super) fn on_rejected(&mut self, rejection: Rejection) {
        let text = rejection.id.and_then(|id| self.sent_inputs.remove(&id));
        let template = if text.is_some() {
            i18n::INPUT_NOT_ACCEPTED
        } else {
            i18n::REQUEST_REJECTED
        };
        let line = self
            .lang
            .tr(template)
            .replace("{reason}", &rejection.message);
        self.push_cell(TranscriptCell::Warning(line));
        if let Some(text) = text
            && self.composer.is_empty()
        {
            self.set_draft(&text);
        }
    }

    fn on_alert(&mut self, alert: Alert) {
        if alert == Alert::PruneNeedsRetention {
            self.on_prune_needs_retention();
            return;
        }
        if alert == Alert::EngineRestarting {
            self.restarting = true;
        }
        self.chat.apply_alert(alert);
    }

    /// 지운 결과를 대화 기록에 한 줄 남기고 정리 창을 닫는다.
    fn on_pruned(&mut self, chats: usize, rows: u64) {
        if !matches!(self.window, Some(Window::Prune(_))) {
            return;
        }
        self.window = None;
        let line = self
            .lang
            .tr(i18n::CLI_PRUNE_DONE)
            .replace("{chats}", &chats.to_string())
            .replace("{rows}", &rows.to_string());
        self.push_cell(TranscriptCell::Warning(line));
    }

    /// 정리 기준 설정이 없다는 거절. 창을 닫고 설정 방법을 대화 기록에 남긴다.
    fn on_prune_needs_retention(&mut self) {
        if matches!(self.window, Some(Window::Prune(_))) {
            self.window = None;
        }
        let line = self.lang.tr(i18n::CLI_PRUNE_NO_RETENTION).to_owned();
        self.push_cell(TranscriptCell::Warning(line));
    }

    /// 새 채팅 id는 접속 직후 `HistoryChunk`가 처음 알린다.
    fn learn_chat(&mut self, chat: ChatId) {
        if self.chat.chat.is_none() {
            self.chat.chat = Some(chat);
        }
    }
}

// cost: time O(e), heap O(e), stack O(1)
// vars: e = entries.len()
// basis: estimate
/// 실시간 반영과 같은 문구가 나오도록 별도 상태에 차례로 반영한다.
fn history_cells(entries: Vec<Notification>) -> Vec<TranscriptCell> {
    let mut chat = ChatState::new();
    let mut live = LiveArea::new();
    let mut labels: BTreeMap<TaskId, TaskLabel> = BTreeMap::new();
    let mut cells = Vec::new();
    // 이전 기록에는 경과를 잴 기준 시각이 없다. 결과 머리줄은 engine이 알린 경과를 쓴다.
    let now = Instant::now();
    for entry in entries {
        match entry {
            Notification::InputChanged {
                input,
                text,
                label,
                state,
                disposition,
                reason,
                ..
            } => {
                let update = InputUpdate {
                    input,
                    text,
                    label,
                    state,
                    disposition,
                    reason,
                };
                let cell = echo_cell(&update);
                if let Change::Echo { .. } = chat.apply_input(update, now) {
                    cells.push(cell);
                }
            }
            Notification::TaskChanged {
                task,
                label,
                state,
                provider,
                elapsed_ms,
                failure,
            } => {
                labels.insert(task, label);
                let update = TaskUpdate {
                    task,
                    label,
                    state,
                    provider,
                    elapsed: Duration::from_millis(elapsed_ms),
                    failure,
                };
                cells.extend(finished_cells(&mut chat, &mut live, update, now));
            }
            Notification::TaskEvent { task, event } => {
                if let Change::Output { text, .. } = chat.apply_event(task, event, now) {
                    let label = labels.get(&task).copied().unwrap_or(TaskLabel('?'));
                    live.push(task, label, &text);
                }
            }
            Notification::ChatNotice { task, notice, .. }
                if !matches!(
                    notice,
                    ChatNotice::Stopped { .. } | ChatNotice::StopUnconfirmed { .. }
                ) =>
            {
                let label = task.and_then(|task| labels.get(&task).copied());
                cells.push(TranscriptCell::Notice { label, notice });
            }
            _ => {}
        }
    }
    cells
}

fn finished_cells(
    chat: &mut ChatState,
    live: &mut LiveArea,
    update: TaskUpdate,
    now: Instant,
) -> Vec<TranscriptCell> {
    let label = update.label;
    match chat.apply_task(update, now) {
        Change::TaskFinished { task } => {
            let lines = live.finish(task);
            let mut cells = Vec::new();
            if !lines.is_empty() {
                cells.push(TranscriptCell::AgentText {
                    label: Some(label),
                    lines,
                });
            }
            if let Some(view) = chat.finish_task(task) {
                cells.push(result_cell(&view));
            }
            cells
        }
        Change::TaskNeedsCheck { .. } => vec![TranscriptCell::NeedsCheck { label }],
        _ => Vec::new(),
    }
}

/// 사용자가 답해야 일이 진행되는 알림. 단순 방식은 이때 벨을 울린다.
fn needs_input(notification: &Notification) -> bool {
    match notification {
        Notification::PermissionRequested { .. }
        | Notification::InputRequested { .. }
        | Notification::FolderTrustRequested { .. }
        | Notification::RouterKeyRequired { .. }
        | Notification::ConstraintAsked { .. } => true,
        Notification::InputChanged { state, reason, .. } => {
            *state == InputState::Queued && *reason == Some(QueueReason::ConfirmStop)
        }
        _ => false,
    }
}
