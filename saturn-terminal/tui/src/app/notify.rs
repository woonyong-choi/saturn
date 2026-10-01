//! engine 알림을 화면 상태에 반영한다.
//!
//! 설계: docs/design/tui.md(영역의 갱신 시점), docs/design/engine-lifecycle.md(접속 직후 `StartInfo` → 최근 기록 → 보관한 허가 요청).
//! 접속 직후의 첫 `HistoryChunk`는 실시간 알림처럼 차례로 반영해 보류 작업과 대화 기록을 되살리고,
//! 그 뒤의 묶음(위로 스크롤한 이전 부분)은 대화 기록 셀로만 바꿔 앞에 넣는다.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, ChatNotice, Notification};
use saturn_protocol::state::Disposition;

use super::{App, Effect, Window};
use crate::state::{
    Change, ChatState, ContextSize, FeedbackPrompt, InputUpdate, TaskUpdate, TrainingProgress,
};
use crate::view::folder_trust::{FolderTrust, TrustChoice};
use crate::view::judge_key_prompt::JudgeKeyPrompt;
use crate::view::judge_version::JudgeVersionRow;
use crate::view::live_area::LiveArea;
use crate::view::permission::PermissionRequest;
use crate::view::resume_prompt::ResumePrompt;
use crate::view::start_screen::StartInfo;
use crate::view::task_list::ChatGroup;
use crate::view::train_confirm::{TrainChoice, TrainConfirm};
use crate::view::transcript::{TranscriptCell, delivery_badge, echo_cell, result_cell};
use crate::view::usage::UsageTable;

impl App {
    /// engine 알림 하나. 변형별로 나눠 처리한다. 대화 기록에 첫 셀이 생기면 시작 화면을 머리 셀로 바꾼다.
    pub(super) fn on_notification(
        &mut self,
        notification: Notification,
        now: Instant,
    ) -> Vec<Effect> {
        match notification {
            Notification::StartInfo { .. } => {
                self.start = StartInfo::from_notification(&notification);
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
            other => self.on_window_notification(other, now),
        }
        Vec::new()
    }

    // cost: time O(m), heap O(m), stack O(1)
    // vars: m = 알림에 실린 행 수
    // basis: estimate
    /// 창과 상태판, 바닥줄을 고치는 나머지 알림.
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
            Notification::JudgeKeyRequired { reason } => {
                self.window = Some(Window::JudgeKey(JudgeKeyPrompt {
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
            Notification::JudgeVersions { current, versions } => {
                if let Some(Window::JudgeVersion(screen)) = &mut self.window {
                    screen.rows = versions
                        .into_iter()
                        .map(|info| JudgeVersionRow::from_info(info, &current))
                        .collect();
                    screen.selected = screen.selected.min(screen.rows.len().saturating_sub(1));
                }
            }
            other => self.on_progress_notification(other, now),
        }
    }

    /// 학습, 작업 이벤트, 한 줄 알림, 피드백, 맥락 크기, 설정, 경고.
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

    /// `InputChanged`: `ChatState::apply_input`. `Change::Echo`면 에코 셀(`> [A] 원문`),
    /// `Delivering`/`Applied`면 에코 뒤 `전달 중`/`반영됨` 표시를 고친다.
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

    /// `TaskChanged`: `ChatState::apply_task`. 끝나면 작업별 출력 칸 내용을 대화 기록으로 옮기고 결과 머리줄
    /// (`[A] codex · 45초 · Token 3,210`, 실패면 `· 실패`와 원인 줄), `NeedsCheck`면 `[A] 결과 확인 필요 · /continue A`.
    /// 보류 재개 질문은 접속 직후 첫 기록을 모두 반영한 뒤 `on_history_chunk`가 띄운다.
    fn on_task_changed(&mut self, update: TaskUpdate, now: Instant) {
        match self.chat.apply_task(update, now) {
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
    /// `HistoryChunk`. 접속 직후 첫 묶음은 항목을 실시간 알림처럼 반영하고, 보류 작업이 있으면 보류 재개 질문을 한 번 띄운다.
    /// 그 뒤 묶음은 대화 기록 셀로 바꿔 앞에 넣는다.
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
    /// 보류 작업이 있으면 보류 재개 질문을 띄운다. 채팅을 열 때 한 번만.
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

    /// `TaskEvent`: `ChatState::apply_event`. 글은 작업별 출력 칸, 도구는 도구 셀, 허가 요청은 허가 요청 창 대기열.
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
    /// `ChatNotice`: `Compacted`·`ProviderSwitched`·`ResumeSuggested`·`RequestSummary`는 대화 기록 한 줄,
    /// `Stopped`·`StopUnconfirmed`는 상태판(`ChatState::stop`). `ProviderSwitched`는 작업 provider도 고친다.
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
            notice => self.push_cell(TranscriptCell::Notice { label, notice }),
        }
    }

    /// `FeedbackQuestion`: 입력 에코 다음 줄에 질문 셀을 넣고 `now`부터 8초를 잰다. 이전 질문이 있으면 바꾼다.
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

    /// `SettingsApplied`: 경고가 있으면 알림 줄 `폴더 설정 오류 · 이전 설정 번호 N로 계속 · 경고`.
    fn on_settings_applied(&mut self, revision: SettingsRevision, warning: Option<String>) {
        self.chat.settings = Some((revision, warning));
    }

    /// `Alert`: `ChatState::apply_alert`.
    fn on_alert(&mut self, alert: Alert) {
        self.chat.apply_alert(alert);
    }

    /// 아직 모르는 채팅 id를 알림에서 배운다(새 채팅은 접속 직후 `HistoryChunk`가 처음 알린다).
    fn learn_chat(&mut self, chat: ChatId) {
        if self.chat.chat.is_none() {
            self.chat.chat = Some(chat);
        }
    }
}

// cost: time O(e), heap O(e), stack O(1)
// vars: e = entries.len()
// basis: estimate
/// 이전 기록 항목을 대화 기록 셀로 바꾼다. 실시간 반영과 같은 문구가 나오도록 별도 상태로 차례로 반영한다.
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

/// 이전 기록의 작업 상태 하나. 끝났으면 모은 글과 결과 머리줄, 결과 불명이면 확인 필요 줄.
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
