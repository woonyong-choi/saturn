//! 화면 없는 plain 출력. 파이프와 CI에서 전체 화면 대신 쓴다.
//!
//! 설계: docs/design/tui.md(화면 언어와 출력 방식, 요구사항 같은 명령 같은 결과).
//! 같은 `state::ChatState`와 같은 문구 함수(`TranscriptCell::lines`, `status_board` 문구)를 써서
//! 전체 화면 대화 기록과 같은 줄을 stdout에 한 줄씩 쓴다. 스피너, 버튼, 틱 갱신 줄(경과 시간)은 쓰지 않는다.
//! 창이 필요한 상호작용(허가 요청, 폴더 신뢰, 보류 재개 질문)의 plain 처리는 설계에 없어 허가 요청만 한 줄로 알린다.
//! 접속 직후의 최근 기록(`HistoryChunk`)은 쓰지 않는다. 파이프 출력에는 이번 입력의 결과만 남긴다.
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
use crate::view::transcript::{TranscriptCell, echo_cell, result_cell};

/// plain 출력기.
#[derive(Debug)]
pub struct PlainOutput<W: Write> {
    out: W,
    lang: Lang,
    chat: ChatState,
    finished: bool,
    /// 작업별로 줄바꿈 전 모델 글 조각.
    partial: BTreeMap<TaskId, String>,
    /// 이미 쓴 알림 줄.
    alerts_written: usize,
}

impl<W: Write> PlainOutput<W> {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// `out`(보통 stdout)에 쓰는 출력기.
    pub fn new(out: W, lang: Lang) -> Self {
        Self {
            out,
            lang,
            chat: ChatState::new(),
            finished: false,
            partial: BTreeMap::new(),
            alerts_written: 0,
        }
    }

    /// 제출했다. 원문은 engine이 `InputChanged`로 돌려주므로 에코에 쓰지 않고, 모든 작업이 끝날 때까지 기다린다.
    pub fn submitted(&mut self, text: String) {
        let _ = text;
        self.finished = false;
    }

    /// 붙은 채팅. 접속 직후 `HistoryChunk`가 알려 준다.
    pub fn chat(&self) -> Option<ChatId> {
        self.chat.chat
    }

    /// 알림 하나를 반영하고 대화 기록에 생길 셀과 같은 줄을 쓴다.
    /// 에코 `> [A] 원문`, 결과 머리줄, 실패 원인, 한 줄 알림, 이번 요청 합계, 멈춤 결과, 알림 줄(처음 한 번),
    /// 허가 요청(요청 내용과 이유 한 줄). 모델 글은 완성된 줄마다 바로 쓴다.
    ///
    /// # Errors
    /// 쓰기 실패.
    pub fn apply(&mut self, notification: Notification, now: Instant) -> std::io::Result<()> {
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
            Notification::ChatNotice { chat, notice, task } => {
                self.chat.chat.get_or_insert(chat);
                self.notice(task, notice)?;
            }
            Notification::SettingsApplied {
                revision,
                warning: Some(detail),
            } => {
                let line = StatusLine::SettingsError {
                    previous: revision.0,
                    detail,
                };
                self.line(&line.text(self.lang, false, ' '))?;
            }
            Notification::Alert { alert } => {
                self.chat.apply_alert(alert);
                self.write_new_alerts()?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 모든 작업이 끝났다(`ChatNotice::RequestSummary`를 쓴 뒤 참). 표준 입력이 끝났고 이것이 참이면 `run_plain`이 끝난다.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    // cost: time O(t + l), heap O(l), stack O(1), io l
    // vars: t = 작업 수, l = 쓸 줄 수
    // basis: estimate
    /// 작업 상태. 끝나면 남은 조각과 결과 머리줄, 결과 불명이면 확인 필요 줄.
    fn task_changed(&mut self, update: TaskUpdate, now: Instant) -> std::io::Result<()> {
        match self.chat.apply_task(update, now) {
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
    /// 작업 이벤트. 모델 글은 완성된 줄마다, 도구 호출은 줄인 도구 셀 한 줄.
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
    /// 한 줄 알림. 멈춤 결과와 남은 프로세스는 상태판 문구로, 이번 요청 합계 뒤에는 끝났다고 표시한다.
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
    /// 새로 생긴 알림 줄만 쓴다.
    fn write_new_alerts(&mut self) -> std::io::Result<()> {
        let fresh: Vec<String> = self.chat.alerts[self.alerts_written..]
            .iter()
            .map(|alert| alert_text(self.lang, alert))
            .collect();
        self.alerts_written = self.chat.alerts.len();
        fresh.iter().try_for_each(|text| self.line(text))
    }

    /// 모델 글 한 줄. 대화 기록 `AgentText` 셀과 같은 문구.
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
    /// 셀의 줄을 쓴다. 이름표 보임은 지금 상태로 정한다.
    fn cell(&mut self, cell: &TranscriptCell) -> std::io::Result<()> {
        let visible = self.chat.labels_visible();
        cell.lines(self.lang, visible, false)
            .iter()
            .try_for_each(|line| self.line(line))
    }

    /// 한 줄 쓰고 바로 비운다(파이프 버퍼에 머물지 않게).
    fn line(&mut self, text: &str) -> std::io::Result<()> {
        writeln!(self.out, "{text}")?;
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use saturn_protocol::event::{Activity, ProviderEvent};
    use saturn_protocol::ids::{AgentId, InputId, Provider};
    use saturn_protocol::state::{InputState, TaskState};

    use super::*;

    fn task(state: TaskState, elapsed_ms: u64) -> Notification {
        Notification::TaskChanged {
            task: TaskId(1),
            label: TaskLabel('A'),
            state,
            provider: Some(Provider::Codex),
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
                    provider_tokens: vec![(Provider::Codex, 4_120)],
                    judge_calls: 0,
                    judge_tokens: 0,
                    elapsed_ms: 45_000,
                },
            },
        ]);

        assert_eq!(
            text,
            "> 버그 고쳐 · 전달 중\n• 파일 읽는 중\n고쳤습니다\n끝\ncodex · 45초 · Token -\n\
             이번 요청 · codex Token 4,120 · 판단기 0회 Token 0 · 45초\n"
        );
        assert!(finished);
    }

    #[test]
    fn apply_writes_alert_once_and_stop_result() {
        let (text, finished) = output(vec![
            Notification::Alert {
                alert: saturn_protocol::rpc::Alert::JudgePaused,
            },
            Notification::Alert {
                alert: saturn_protocol::rpc::Alert::JudgePaused,
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
