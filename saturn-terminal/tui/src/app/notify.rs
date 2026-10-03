//! engine 알림을 화면 상태에 반영한다.
//! 설계: docs/design/tui.md

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, ChatNotice, Notification, SettingsWarning};
use saturn_protocol::state::Disposition;

use super::{App, Effect, Window};
use crate::state::{
    Change, ChatState, ContextSize, FeedbackPrompt, InputUpdate, TaskUpdate, TrainingProgress,
};
use crate::view::folder_trust::{FolderTrust, TrustChoice};
use crate::view::live_area::LiveArea;
use crate::view::permission::PermissionRequest;
use crate::view::resume_prompt::ResumePrompt;
use crate::view::router_key_prompt::RouterKeyPrompt;
use crate::view::router_version::RouterVersionRow;
use crate::view::start_screen::StartInfo;
use crate::view::task_list::ChatGroup;
use crate::view::train_confirm::{TrainChoice, TrainConfirm};
use crate::view::transcript::{TranscriptCell, delivery_badge, echo_cell, result_cell};
use crate::view::usage::UsageTable;

impl App {
    pub(super) fn on_notification(
        &mut self,
        notification: Notification,
        now: Instant,
    ) -> Vec<Effect> {
        match notification {
            Notification::StartInfo { .. } => {
                self.start = StartInfo::from_notification(&notification);
                self.chat_folder = self.start.as_ref().map(|start| start.folder.clone());
            }
            Notification::InputAccepted { .. } => {}
            Notification::InputChanged {
                input,
                text,
                label,
                state,
                disposition,
                reason,
            } => self.on_input_changed(InputUpdate {
                input,
                text,
                label,
                state,
                disposition,
                reason,
            }),
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
                has_more,
            } => return self.on_history_chunk(chat, entries, has_more, now),
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
            other => self.on_window_notification(other, now),
        }
        Vec::new()
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
            Notification::TaskList { items } => {
                if let Some(Window::TaskList(list)) = &mut self.window {
                    list.replace(ChatGroup::from_items(items));
                }
            }
            Notification::Usage { range, rows } => {
                if let Some(Window::Usage(screen)) = &mut self.window {
                    screen.table = Some(UsageTable { range, rows });
                }
            }
            Notification::ModelPinned { chat, model } => {
                if self.chat.chat == Some(chat) {
                    self.chat.pinned_model = Some(model);
                }
            }
            Notification::Models { models } => {
                if let Some(Window::Model(picker)) = &mut self.window {
                    picker.load(models);
                }
            }
            Notification::RouterVersions { current, versions } => {
                if let Some(Window::RouterVersion(screen)) = &mut self.window {
                    screen.rows = versions
                        .into_iter()
                        .map(|info| RouterVersionRow::from_info(info, &current))
                        .collect();
                    screen.selected = screen.selected.min(screen.rows.len().saturating_sub(1));
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
            Notification::SettingsApplied { revision, warning } => {
                self.on_settings_applied(revision, warning);
            }
            Notification::Alert { alert } => self.on_alert(alert),
            _ => {}
        }
    }

    fn on_input_changed(&mut self, update: InputUpdate) {
        let cell = echo_cell(&update);
        let (input, state) = (update.input, update.state);
        match self.chat.apply_input(update) {
            Change::Echo { .. } => self.push_cell(cell),
            _ => {
                if let Some(badge) = delivery_badge(state) {
                    self.transcript.set_badge(input, badge);
                }
            }
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
        has_more: bool,
        now: Instant,
    ) -> Vec<Effect> {
        self.learn_chat(chat);
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
        });
        self.push_cell(TranscriptCell::Feedback { label, disposition });
    }

    fn on_settings_applied(
        &mut self,
        revision: SettingsRevision,
        warning: Option<SettingsWarning>,
    ) {
        self.chat.settings = Some((revision, warning));
    }

    fn on_alert(&mut self, alert: Alert) {
        self.chat.apply_alert(alert);
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
                if let Change::Echo { .. } = chat.apply_input(update) {
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
