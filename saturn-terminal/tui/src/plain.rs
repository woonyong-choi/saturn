//! 화면 없는 plain 출력. 파이프와 CI에서 전체 화면 대신 쓴다.
//! 설계: docs/design/tui.md. 켜는 조건은 터미널이 아닐 때, `--plain`, `NO_COLOR`, 설정 `tui.screen`이다

use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use saturn_protocol::ids::{ChatId, InputId, TaskId, TaskLabel};
use saturn_protocol::rpc::{ChatNotice, Notification};
use saturn_protocol::state::{Disposition, InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::state::{Change, ChatState, InputUpdate, TaskUpdate};
use crate::view::status_board::{StatusLine, alert_text};
use crate::view::transcript::{TranscriptCell, echo_cell, interrupted_line, result_cell};

#[derive(Debug)]
pub(crate) struct PlainOutput<W: Write> {
    out: W,
    lang: Lang,
    chat: ChatState,
    /// 아직 줄바꿈이 오지 않은 모델 글 조각.
    partial: BTreeMap<TaskId, String>,
    alerts_written: usize,
    /// engine가 키를 요청한 원인. 화면이 없어 묻지 않는다.
    key_required: Option<String>,
    /// 실패로 끝난 작업이 하나라도 있다.
    failed: bool,
    /// 입력이 필요한 줄 앞에 터미널 벨을 쓴다. 표준 출력이 터미널일 때만 켠다.
    bell: bool,
    /// 보냈지만 접수나 거절 응답을 아직 받지 못한 입력 수.
    unanswered: u64,
    /// 입력을 보낸 적이 있다.
    sent: bool,
    /// 이 접속이 접수시킨 입력 중 끝 상태가 아닌 것과 그 입력의 작업. 다른 접속의 입력은 넣지 않는다.
    pending: BTreeMap<InputId, Option<TaskId>>,
    /// 이 접속의 입력이 적용돼 끝나기를 기다리는 작업. 표시 글자(`TaskLabel`)는 작업 27개째부터 겹치므로 `TaskId`로 센다.
    owed: Vec<TaskId>,
}

impl<W: Write> PlainOutput<W> {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn new(out: W, lang: Lang) -> Self {
        Self {
            bell: false,
            out,
            lang,
            chat: ChatState::new(),
            partial: BTreeMap::new(),
            alerts_written: 0,
            key_required: None,
            failed: false,
            unanswered: 0,
            sent: false,
            pending: BTreeMap::new(),
            owed: Vec::new(),
        }
    }

    /// 입력이 필요한 줄(허가, 입력 요청) 앞에 벨을 쓴다.
    pub(crate) fn with_bell(mut self, bell: bool) -> Self {
        self.bell = bell;
        self
    }

    /// engine가 router 키를 요청했으면 그 원인. 호출자는 묻지 않고 안내하고 끝낸다.
    pub(crate) fn key_required(&self) -> Option<&str> {
        self.key_required.as_deref()
    }

    /// provider 작업이 실패로 끝난 적이 있으면 참. 호출자는 실패 종료 코드로 끝낸다.
    pub(crate) fn has_failed(&self) -> bool {
        self.failed
    }

    /// 원문은 engine이 `InputChanged`로 돌려주므로 여기서 쓰지 않는다.
    pub(crate) fn submitted(&mut self, text: String) {
        let _ = text;
        self.unanswered += 1;
        self.sent = true;
    }

    /// engine이 접수하지 않은 입력. 그 입력의 실행은 오지 않으므로 기다리는 응답 수에서 뺀다.
    ///
    /// # Errors
    /// 쓰기 실패.
    pub(crate) fn input_rejected(&mut self, message: &str) -> std::io::Result<()> {
        self.unanswered = self.unanswered.saturating_sub(1);
        let line = self
            .lang
            .tr(i18n::INPUT_NOT_ACCEPTED)
            .replace("{reason}", message);
        self.saturn_line(&line)
    }

    /// 접속 자체를 거절당한 줄처럼, 입력과 짝지을 수 없는 거절을 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패.
    pub(crate) fn request_rejected(&mut self, message: &str) -> std::io::Result<()> {
        let line = self
            .lang
            .tr(i18n::REQUEST_REJECTED)
            .replace("{reason}", message);
        self.saturn_line(&line)
    }

    /// 접속 직후 `HistoryChunk`가 알려 준다.
    pub(crate) fn chat(&self) -> Option<ChatId> {
        self.chat.chat
    }

    /// 접속 직후 최근 기록(`HistoryChunk`)은 쓰지 않고 이번 입력의 결과만 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패.
    pub(crate) fn apply(
        &mut self,
        notification: Notification,
        now: Instant,
    ) -> std::io::Result<()> {
        match notification {
            Notification::HistoryChunk { chat, .. } => {
                self.chat.chat.get_or_insert(chat);
            }
            Notification::InputAccepted { input, .. } => {
                self.unanswered = self.unanswered.saturating_sub(1);
                self.pending.insert(input, None);
            }
            changed @ Notification::InputChanged { .. } => {
                if let Some((update, task)) = InputUpdate::of(changed) {
                    self.track_input(update.input, update.state, task, update.disposition);
                    self.input_changed(update, now)?;
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
                let update = TaskUpdate {
                    task,
                    label,
                    state,
                    provider,
                    elapsed: Duration::from_millis(elapsed_ms),
                    failure,
                };
                self.track_task(task, state);
                self.task_changed(update, now)?;
            }
            Notification::TaskEvent { task, event } => {
                let change = self.chat.apply_event(task, event, now);
                self.task_event(task, change)?;
            }
            Notification::PermissionRequested {
                label,
                summary,
                reason,
                ..
            } => {
                let text = format!(
                    "{} {summary} · {}: {reason}",
                    labels::format(label),
                    self.lang.tr(i18n::PERMISSION_REASON)
                );
                self.ring()?;
                self.line(&text)?;
            }
            Notification::InputRequested { label, request, .. } => {
                let asked = request
                    .url
                    .clone()
                    .or_else(|| request.fields.first().map(|field| field.title.clone()))
                    .unwrap_or_else(|| request.message.clone());
                let text = format!(
                    "{} {}: {asked}",
                    labels::format(label),
                    self.lang.tr(i18n::INPUT_REQUESTED)
                );
                self.ring()?;
                self.line(&text)?;
            }
            Notification::ChatNotice { chat, notice, task } => {
                self.chat.chat.get_or_insert(chat);
                self.notice(task, notice)?;
            }
            Notification::SettingsApplied {
                revision,
                warning: Some(warning),
                ..
            } => {
                let line = StatusLine::Settings {
                    revision: revision.0,
                    warning,
                };
                self.saturn_line(&line.text(self.lang, false, ' '))?;
            }
            Notification::Alert { alert } => {
                self.chat.apply_alert(alert);
                self.write_new_alerts()?;
            }
            Notification::RouterKeyRequired { reason } => self.key_required = Some(reason),
            _ => {}
        }
        Ok(())
    }

    /// 보낸 입력이 모두 접수나 거절 응답을 받았고, 접수된 입력이 끝 상태가 됐으며, 그 입력이 시작하거나 끼워 넣어진
    /// 작업도 끝났으면 참. 다른 접속의 입력과 작업은 보지 않는다. 입력을 보낸 적이 없으면 거짓.
    pub(crate) fn is_finished(&self) -> bool {
        self.sent && self.unanswered == 0 && self.pending.is_empty() && self.owed.is_empty()
    }

    // cost: time O(t + o + l), heap O(l), stack O(1), io l
    // vars: t = 작업 수, o = 기다리는 작업 수, l = 쓸 줄 수
    // basis: estimate
    fn input_changed(&mut self, update: InputUpdate, now: Instant) -> std::io::Result<()> {
        let cell = echo_cell(&update);
        if let Change::Echo { .. } = self.chat.apply_input(update, now) {
            self.cell(&cell)?;
        }
        Ok(())
    }

    // cost: time O(t + o), heap O(1), stack O(1)
    // vars: t = 작업 수, o = 기다리는 작업 수
    // basis: estimate
    /// 이 접속이 접수시킨 입력의 상태를 따라간다. 끝 상태가 된 입력은 더 기다리지 않고, 적용된 입력의 작업은 끝나기를 기다린다.
    fn track_input(
        &mut self,
        input: InputId,
        state: InputState,
        task: Option<TaskId>,
        disposition: Option<Disposition>,
    ) {
        let Some(known) = self.pending.get_mut(&input) else {
            return;
        };
        if task.is_some() {
            *known = task;
        }
        let task = *known;
        match state {
            InputState::Rejected => self.failed = true,
            InputState::Applied => {
                // 끼워 넣을 작업이 이미 끝났다면 기다릴 끝이 오지 않는다
                let is_open = task.is_some_and(|task| {
                    self.chat
                        .tasks
                        .get(&task)
                        .is_some_and(|view| is_active(view.state))
                });
                if let Some(task) = task
                    && (disposition != Some(Disposition::Steer) || is_open)
                {
                    self.owed.push(task);
                }
            }
            _ => {}
        }
        if matches!(
            state,
            InputState::Applied | InputState::Rejected | InputState::Cancelled
        ) {
            self.pending.remove(&input);
        }
    }

    /// 이 접속의 입력이 기다리는 작업의 끝을 따라간다. 결과를 모르거나 멈춘 작업은 기다리지 않고 실패로 센다.
    fn track_task(&mut self, task: TaskId, state: TaskState) {
        let before = self.owed.len();
        self.owed.retain(|owed| *owed != task);
        let is_ours = self.owed.len() != before;
        match state {
            TaskState::Done => {}
            TaskState::Failed => self.failed |= is_ours,
            TaskState::NeedsCheck | TaskState::Held => {
                let before = self.pending.len();
                self.pending.retain(|_, pending| *pending != Some(task));
                self.failed |= is_ours || self.pending.len() != before;
            }
            _ => {
                if is_ours {
                    self.owed.push(task);
                }
            }
        }
    }

    // cost: time O(t + l), heap O(l), stack O(1), io l
    // vars: t = 작업 수, l = 쓸 줄 수
    // basis: estimate
    fn task_changed(&mut self, update: TaskUpdate, now: Instant) -> std::io::Result<()> {
        let task = update.task;
        let change = self.chat.apply_task(update, now);
        let speaker = self
            .chat
            .tasks
            .get(&task)
            .map(|view| format!("{} ", labels::format(view.label)))
            .unwrap_or_default();
        for _ in self.chat.take_interrupted_calls(task) {
            self.line(&format!("{speaker}{}", interrupted_line(self.lang)))?;
        }
        match change {
            Change::TaskFinished { task } => {
                let rest = self.partial.remove(&task).unwrap_or_default();
                let Some(view) = self.chat.tasks.get(&task) else {
                    return Ok(());
                };
                let label = view.label;
                if !rest.is_empty() {
                    self.agent_line(label, rest)?;
                }
                if let Some(view) = self.chat.finish_task(task) {
                    self.cell(&result_cell(&view))?;
                }
            }
            Change::TaskNeedsCheck { task } => {
                if let Some(label) = self.chat.tasks.get(&task).map(|view| view.label) {
                    self.cell(&TranscriptCell::NeedsCheck { label })?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    // cost: time O(n), heap O(n), stack O(1), io l
    // vars: n = 조각 글자 수, l = 완성된 줄 수
    // basis: estimate
    fn task_event(&mut self, task: TaskId, change: Change) -> std::io::Result<()> {
        let Some(label) = self.chat.tasks.get(&task).map(|view| view.label) else {
            return Ok(());
        };
        match change {
            Change::Output { text, .. } => {
                let buffer = self.partial.entry(task).or_default();
                buffer.push_str(&text);
                let mut lines = Vec::new();
                while let Some(end) = buffer.find('\n') {
                    let line: String = buffer.drain(..=end).collect();
                    lines.push(line.trim_end_matches(['\n', '\r']).to_string());
                }
                for line in lines {
                    self.agent_line(label, line)?;
                }
            }
            Change::Tool {
                call_id,
                activity: Some(activity),
                ..
            } => {
                let cell = TranscriptCell::Tool {
                    label: Some(label),
                    call_id,
                    activity,
                    output: String::new(),
                    is_interrupted: false,
                };
                self.cell(&cell)?;
            }
            _ => {}
        }
        Ok(())
    }

    // cost: time O(h), heap O(h), stack O(1), io 1
    // vars: h = 보류 이름표 수
    // basis: estimate
    fn notice(&mut self, task: Option<TaskId>, notice: ChatNotice) -> std::io::Result<()> {
        let label = task.and_then(|task| self.chat.tasks.get(&task).map(|view| view.label));
        let line = match notice {
            ChatNotice::Stopped { held } => {
                self.chat.apply_stopped(held.clone());
                Some(StatusLine::Stopped { held })
            }
            ChatNotice::StopUnconfirmed { remaining } => {
                Some(StatusLine::StopUnconfirmed { remaining })
            }
            notice => {
                self.cell(&TranscriptCell::Notice { label, notice })?;
                None
            }
        };
        match line {
            Some(line) => self.saturn_line(&line.text(self.lang, false, ' ')),
            None => Ok(()),
        }
    }

    // cost: time O(a), heap O(a), stack O(1), io a
    // vars: a = 새 알림 수
    // basis: estimate
    fn write_new_alerts(&mut self) -> std::io::Result<()> {
        let fresh: Vec<String> = self.chat.alerts[self.alerts_written..]
            .iter()
            .map(|alert| alert_text(self.lang, alert))
            .collect();
        self.alerts_written = self.chat.alerts.len();
        fresh.iter().try_for_each(|text| self.saturn_line(text))
    }

    /// 대화 기록 `AgentText` 셀과 같은 문구.
    fn agent_line(&mut self, label: TaskLabel, line: String) -> std::io::Result<()> {
        let cell = TranscriptCell::AgentText {
            label: Some(label),
            lines: vec![line],
        };
        self.cell(&cell)
    }

    // cost: time O(l), heap O(l), stack O(1), io l
    // vars: l = 셀 줄 수
    // basis: estimate
    /// 이름표 보임은 지금 상태로 정한다.
    fn cell(&mut self, cell: &TranscriptCell) -> std::io::Result<()> {
        cell.plain_lines(self.lang, false)
            .iter()
            .try_for_each(|line| self.line(line))
    }

    /// 말하는 쪽이 Saturn인 줄이다.
    fn saturn_line(&mut self, text: &str) -> std::io::Result<()> {
        self.line(&format!("Saturn: {text}"))
    }

    /// 입력이 필요하다는 뜻으로 터미널 벨을 울린다.
    fn ring(&mut self) -> std::io::Result<()> {
        if self.bell {
            self.out.write_all(b"\x07")?;
        }
        Ok(())
    }

    /// 파이프 버퍼에 머물지 않게 바로 비운다.
    fn line(&mut self, text: &str) -> std::io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.out.flush()
    }
}

/// 작업이 아직 끝나기를 기다릴 상태.
fn is_active(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Running
            | TaskState::AnsweredTreeRunning
            | TaskState::AwaitingPermission
            | TaskState::AwaitingInput
    )
}

#[cfg(test)]
mod tests {
    use saturn_protocol::event::{Activity, ProviderEvent, ToolDetail};
    use saturn_protocol::ids::{AgentId, InputId, Provider};
    use saturn_protocol::state::{InputState, TaskState};

    use super::*;

    fn task(state: TaskState, elapsed_ms: u64) -> Notification {
        Notification::TaskChanged {
            task: TaskId(1),
            label: TaskLabel('A'),
            state,
            provider: Some(Provider::from_static("codex")),
            elapsed_ms,
            failure: None,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn output(notifications: Vec<Notification>) -> (String, bool) {
        let mut plain = PlainOutput::new(Vec::new(), Lang::Ko);
        let now = Instant::now();
        for notification in notifications {
            plain.apply(notification, now).unwrap();
        }
        let finished = plain.is_finished();
        (String::from_utf8(plain.out).unwrap(), finished)
    }

    #[test]
    fn bell_precedes_the_permission_line_only_when_enabled() {
        let request = Notification::PermissionRequested {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Provider::from_static("codex"),
            request_id: "r1".to_string(),
            summary: "rm".to_string(),
            reason: "clean".to_string(),
            waiting: 0,
        };
        let mut quiet = PlainOutput::new(Vec::new(), Lang::Ko);
        let mut ringing = PlainOutput::new(Vec::new(), Lang::Ko).with_bell(true);

        quiet.apply(request.clone(), Instant::now()).unwrap();
        ringing.apply(request, Instant::now()).unwrap();

        let quiet = String::from_utf8(quiet.out).unwrap();
        let ringing = String::from_utf8(ringing.out).unwrap();
        assert_eq!(quiet, "[A] rm · 이유: clean\n");
        assert_eq!(ringing, "\x07[A] rm · 이유: clean\n");
    }

    #[test]
    fn apply_writes_echo_output_result_and_summary() {
        let (text, _) = output(vec![
            Notification::HistoryChunk {
                chat: ChatId(1),
                entries: Vec::new(),
                oldest: None,
                has_more: false,
            },
            Notification::InputChanged {
                input: InputId(1),
                text: "버그 고쳐".to_string(),
                label: Some(TaskLabel('A')),
                task: Some(TaskId(1)),
                state: InputState::Delivering,
                disposition: None,
                reason: None,
            },
            task(TaskState::Running, 0),
            Notification::TaskEvent {
                task: TaskId(1),
                event: ProviderEvent::ToolCall {
                    agent: AgentId(1),
                    subagent: None,
                    call_id: "c".to_string(),
                    activity: Activity::ReadingFile,
                    detail: ToolDetail::default(),
                },
            },
            Notification::TaskEvent {
                task: TaskId(1),
                event: ProviderEvent::Text {
                    agent: AgentId(1),
                    subagent: None,
                    text: "고쳤습니다\n끝".to_string(),
                },
            },
            task(TaskState::Done, 45_000),
            Notification::ChatNotice {
                chat: ChatId(1),
                task: None,
                notice: ChatNotice::RequestSummary {
                    provider_tokens: vec![(Provider::from_static("codex"), 4_120)],
                    router_calls: 0,
                    router_tokens: 0,
                    elapsed_ms: 45_000,
                },
            },
        ]);

        assert_eq!(
            text,
            "> [A] 버그 고쳐 · 전달 중\n[A] • 파일 읽는 중\n[A] 고쳤습니다\n[A] 끝\n[A] codex · 45초 · Token -\n\
             Saturn: 이번 요청 · codex Token 4,120 · 라우터 0회 Token 0 · 45초\n"
        );
    }

    #[test]
    fn interrupted_tool_without_result_is_marked_when_the_task_needs_a_check() {
        let (text, _) = output(vec![
            task(TaskState::Running, 0),
            Notification::TaskEvent {
                task: TaskId(1),
                event: ProviderEvent::ToolCall {
                    agent: AgentId(1),
                    subagent: None,
                    call_id: "c".to_string(),
                    activity: Activity::RunningCommand {
                        command: "cargo test".to_string(),
                    },
                    detail: ToolDetail::default(),
                },
            },
            task(TaskState::NeedsCheck, 0),
        ]);

        assert!(text.starts_with("[A] • "), "{text}");
        assert!(text.contains("\n[A]   중단됨\n"), "{text}");
    }

    #[test]
    fn router_key_request_is_kept_for_the_caller_and_writes_nothing() {
        let mut plain = PlainOutput::new(Vec::new(), Lang::Ko);

        plain
            .apply(
                Notification::RouterKeyRequired {
                    reason: "router rejected the key".to_owned(),
                },
                Instant::now(),
            )
            .unwrap();

        assert_eq!(plain.key_required(), Some("router rejected the key"));
        assert!(plain.out.is_empty());
    }

    #[test]
    fn apply_writes_alert_once_and_stop_result() {
        let (text, finished) = output(vec![
            Notification::Alert {
                alert: saturn_protocol::rpc::Alert::RouterPaused,
            },
            Notification::Alert {
                alert: saturn_protocol::rpc::Alert::RouterPaused,
            },
            Notification::ChatNotice {
                chat: ChatId(1),
                task: None,
                notice: ChatNotice::Stopped {
                    held: vec![TaskLabel('A')],
                },
            },
        ]);

        assert_eq!(
            text,
            "Saturn: 자동 판단 일시 중단\nSaturn: ‖ 멈춤 · [A] 보류됨 · /continue 로 이어서\n"
        );
        assert!(!finished);
    }

    #[test]
    fn plain_and_full_screen_cells_use_same_text() {
        let cell = TranscriptCell::NeedsCheck {
            label: TaskLabel('A'),
        };
        let (text, _) = output(vec![
            task(TaskState::Running, 0),
            task(TaskState::NeedsCheck, 0),
        ]);

        assert_eq!(text.trim_end(), cell.lines(Lang::Ko, true, false)[0]);
    }

    fn labeled_task(id: u64, label: char, state: TaskState) -> Notification {
        Notification::TaskChanged {
            task: TaskId(id),
            label: TaskLabel(label),
            state,
            provider: Some(Provider::from_static("codex")),
            elapsed_ms: 0,
            failure: None,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn accepted_and_applied(plain: &mut PlainOutput<Vec<u8>>, input: u64, label: char) {
        let now = Instant::now();
        plain.submitted("일".to_string());
        plain
            .apply(
                Notification::InputAccepted {
                    client_ref: input,
                    input: InputId(input),
                },
                now,
            )
            .unwrap();
        plain
            .apply(
                Notification::InputChanged {
                    input: InputId(input),
                    text: "일".to_string(),
                    label: Some(TaskLabel(label)),
                    task: Some(TaskId(input)),
                    state: InputState::Applied,
                    disposition: Some(Disposition::NewTask),
                    reason: None,
                },
                now,
            )
            .unwrap();
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn steered_onto(plain: &mut PlainOutput<Vec<u8>>, input: u64, task: u64, label: char) {
        let now = Instant::now();
        plain.submitted("일".to_string());
        plain
            .apply(
                Notification::InputAccepted {
                    client_ref: input,
                    input: InputId(input),
                },
                now,
            )
            .unwrap();
        plain
            .apply(
                Notification::InputChanged {
                    input: InputId(input),
                    text: "일".to_string(),
                    label: Some(TaskLabel(label)),
                    task: Some(TaskId(task)),
                    state: InputState::Applied,
                    disposition: Some(Disposition::Steer),
                    reason: None,
                },
                now,
            )
            .unwrap();
    }

    enum Event {
        Task(u64, char, TaskState),
        Applied(u64, char),
        Steered(u64, u64, char),
    }

    struct Step {
        event: Event,
        finished: Option<bool>,
        failed: Option<bool>,
    }

    fn step(event: Event, finished: Option<bool>, failed: Option<bool>) -> Step {
        Step {
            event,
            finished,
            failed,
        }
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 단계 수
    // basis: estimate
    fn run_steps(name: &str, steps: Vec<Step>) {
        let now = Instant::now();
        let mut plain = PlainOutput::new(Vec::new(), Lang::Ko);
        for (index, step) in steps.into_iter().enumerate() {
            match step.event {
                Event::Task(task, label, state) => {
                    plain.apply(labeled_task(task, label, state), now).unwrap();
                }
                Event::Applied(input, label) => accepted_and_applied(&mut plain, input, label),
                Event::Steered(input, task, label) => steered_onto(&mut plain, input, task, label),
            }
            if let Some(finished) = step.finished {
                assert_eq!(
                    plain.is_finished(),
                    finished,
                    "{name}: finished at step {index}"
                );
            }
            if let Some(failed) = step.failed {
                assert_eq!(plain.has_failed(), failed, "{name}: failed at step {index}");
            }
        }
    }

    #[test]
    fn another_task_neither_holds_the_end_nor_fails_this_connections_wait() {
        use Event::{Applied, Task};
        let cases = vec![
            (
                "another connection's task",
                vec![
                    step(Applied(1, 'A'), None, None),
                    step(Task(1, 'A', TaskState::Running), None, None),
                    step(Task(2, 'B', TaskState::Running), Some(false), None),
                    step(Task(1, 'A', TaskState::Done), None, None),
                    step(Task(2, 'B', TaskState::Failed), Some(true), Some(false)),
                ],
            ),
            (
                "a task sharing the label finishing first",
                vec![
                    step(Applied(1, 'Z'), None, None),
                    step(Task(1, 'Z', TaskState::Running), None, None),
                    step(Task(2, 'Z', TaskState::Running), None, None),
                    step(Task(2, 'Z', TaskState::Failed), Some(false), Some(false)),
                    step(Task(1, 'Z', TaskState::Done), Some(true), Some(false)),
                ],
            ),
        ];

        for (name, steps) in cases {
            run_steps(name, steps);
        }
    }

    #[test]
    fn a_steer_waits_for_the_task_it_joined_not_for_one_sharing_its_label() {
        use Event::{Steered, Task};
        let cases = vec![
            (
                "the joined task is still running",
                vec![
                    step(Task(1, 'Z', TaskState::Running), None, None),
                    step(Task(3, 'Z', TaskState::Running), None, None),
                    step(Steered(7, 1, 'Z'), None, None),
                    step(Task(3, 'Z', TaskState::Done), Some(false), None),
                    step(Task(1, 'Z', TaskState::Done), Some(true), None),
                ],
            ),
            (
                "the joined task already finished",
                vec![
                    step(Task(1, 'Z', TaskState::Done), None, None),
                    step(Task(3, 'Z', TaskState::Running), None, None),
                    step(Steered(7, 1, 'Z'), Some(true), None),
                ],
            ),
        ];

        for (name, steps) in cases {
            run_steps(name, steps);
        }
    }

    #[test]
    fn an_input_applied_before_its_task_starts_is_not_finished_yet() {
        let mut plain = PlainOutput::new(Vec::new(), Lang::Ko);

        accepted_and_applied(&mut plain, 1, 'A');

        assert!(!plain.is_finished());
    }
}
