//! 화면 없는 plain 출력. 파이프와 CI에서 전체 화면 대신 쓴다.
//! 설계: docs/design/tui.md
//! TODO(#57): plain을 켜는 조건과 우선순위, 설정 키 이름

use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use saturn_protocol::ids::{ChatId, TaskId, TaskLabel};
use saturn_protocol::rpc::{ChatNotice, Notification};

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
    finished: bool,
    /// 아직 줄바꿈이 오지 않은 모델 글 조각.
    partial: BTreeMap<TaskId, String>,
    alerts_written: usize,
    /// engine가 키를 요청한 원인. 화면이 없어 묻지 않는다.
    key_required: Option<String>,
}

impl<W: Write> PlainOutput<W> {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn new(out: W, lang: Lang) -> Self {
        Self {
            out,
            lang,
            chat: ChatState::new(),
            finished: false,
            partial: BTreeMap::new(),
            alerts_written: 0,
            key_required: None,
        }
    }

    /// engine가 router 키를 요청했으면 그 원인. 호출자는 묻지 않고 안내하고 끝낸다.
    pub(crate) fn key_required(&self) -> Option<&str> {
        self.key_required.as_deref()
    }

    /// 원문은 engine이 `InputChanged`로 돌려주므로 여기서 쓰지 않는다.
    pub(crate) fn submitted(&mut self, text: String) {
        let _ = text;
        self.finished = false;
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
                if let Change::Echo { .. } = self.chat.apply_input(update) {
                    self.cell(&cell)?;
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
                self.line(&line.text(self.lang, false, ' '))?;
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

    /// `ChatNotice::RequestSummary`를 쓴 뒤 참.
    pub(crate) fn is_finished(&self) -> bool {
        self.finished
    }

    // cost: time O(t + l), heap O(l), stack O(1), io l
    // vars: t = 작업 수, l = 쓸 줄 수
    // basis: estimate
    fn task_changed(&mut self, update: TaskUpdate, now: Instant) -> std::io::Result<()> {
        let task = update.task;
        let change = self.chat.apply_task(update, now);
        for _ in self.chat.take_interrupted_calls(task) {
            self.line(&interrupted_line(self.lang))?;
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
                let summary = matches!(notice, ChatNotice::RequestSummary { .. });
                self.cell(&TranscriptCell::Notice { label, notice })?;
                self.finished |= summary;
                None
            }
        };
        match line {
            Some(line) => self.line(&line.text(self.lang, false, ' ')),
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
        fresh.iter().try_for_each(|text| self.line(text))
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
        let visible = self.chat.labels_visible();
        cell.lines(self.lang, visible, false)
            .iter()
            .try_for_each(|line| self.line(line))
    }

    /// 파이프 버퍼에 머물지 않게 바로 비운다.
    fn line(&mut self, text: &str) -> std::io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.out.flush()
    }
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
    fn apply_writes_echo_output_result_and_summary() {
        let (text, finished) = output(vec![
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
            "> 버그 고쳐 · 전달 중\n• 파일 읽는 중\n고쳤습니다\n끝\ncodex · 45초 · Token -\n\
             이번 요청 · codex Token 4,120 · 라우터 0회 Token 0 · 45초\n"
        );
        assert!(finished);
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

        assert!(text.starts_with("• "), "{text}");
        assert!(text.contains("\n  중단됨\n"), "{text}");
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
            "자동 판단 일시 중단\n‖ 멈춤 · [A] 보류됨 · /continue 로 이어서\n"
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
}
