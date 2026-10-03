//! 상태판. 줄은 매 프레임 `state::ChatState`에서 새로 만든다.
//! 설계: docs/design/tui.md

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, SettingsFault, SettingsLayer, SettingsWarning};
use saturn_protocol::state::QueueReason;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::state::{InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::state::{APPROVAL_PENDING_AFTER, ChatState, InputView, TaskView, TrainingProgress};
use crate::view::transcript::held_labels;
use crate::view::{EMPHASIS, text_width, truncate};

pub(crate) const COMMAND_PREVIEW_COLS: usize = 40;

/// TODO(#50): 칸 순서와 모델 이름 표기
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningLine {
    pub task: TaskId,
    pub label: TaskLabel,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub elapsed: Duration,
    pub activity: Option<Activity>,
    /// `activity`보다 앞선다.
    pub awaiting_permission: bool,
    /// `awaiting_permission` 다음으로 앞선다.
    pub awaiting_input: bool,
    /// 도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 없다. `awaiting_permission` 다음으로 앞선다.
    pub approval_pending: bool,
    /// 0이 아니면 하는 일 대신 보인다.
    pub subagents: usize,
    pub has_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatusLine {
    /// 살아 있는 작업마다 한 줄.
    Running(RunningLine),
    /// TODO(#53): 짧게 끝나는 판단의 판단 줄 표시 방식
    Judging {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// TODO(#55): `/stop`과 진행 중인 `/train`의 관계
    Training(TrainingProgress),
    Queued {
        input: InputId,
        label: Option<TaskLabel>,
        reason: QueueReason,
        text: Option<String>,
    },
    /// 보류 줄 맨 앞에 둔다.
    Stopped { held: Vec<TaskLabel> },
    HeldTask {
        task: TaskId,
        label: TaskLabel,
        provider: Option<Provider>,
    },
    HeldInput {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// 그 작업의 보류 줄 자리에 대신 그린다.
    CloseHeldConfirm { task: TaskId, label: TaskLabel },
    /// 같은 알림은 한 줄만.
    Alert(Alert),
    /// 멈춤 뒤 provider 프로세스 묶음 밖에 남은 프로세스 수.
    StopUnconfirmed { remaining: u32 },
    /// `[보내기]`를 눌렀지만 router가 실패해 차례에 보낸다.
    RouterUnavailableSend,
    /// `revision`은 적용된 설정 번호. 검사 실패면 계속 쓰는 이전 번호다.
    Settings {
        revision: u64,
        warning: SettingsWarning,
    },
}

impl StatusLine {
    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 줄 글자 수
    // basis: estimate
    /// `spinner`는 실행·판단·학습 줄의 머리 글자.
    pub(crate) fn text(&self, lang: Lang, labels_visible: bool, spinner: char) -> String {
        let prefix = |label: Option<TaskLabel>| labels::prefix(label, labels_visible);
        match self {
            Self::Running(line) => {
                format!(
                    "{spinner} {}{}",
                    prefix(Some(line.label)),
                    running_text(lang, line)
                )
            }
            Self::Judging { label, text, .. } => with_text(
                format!("{spinner} {}{}", prefix(*label), lang.tr(i18n::JUDGING)),
                text.as_deref(),
            ),
            Self::Training(progress) => training_text(lang, spinner, progress),
            Self::Queued {
                label,
                reason,
                text,
                ..
            } => with_text(
                format!(
                    "· {}{} · {}",
                    prefix(*label),
                    lang.tr(i18n::QUEUED),
                    queue_reason_text(lang, *reason)
                ),
                text.as_deref(),
            ),
            Self::Stopped { held } => format!(
                "‖ {} · {} {} · {}",
                lang.tr(i18n::STOPPED),
                held_labels(held),
                lang.tr(i18n::HELD_DONE),
                lang.tr(i18n::CONTINUE_HINT)
            ),
            Self::HeldTask {
                label, provider, ..
            } => {
                let head = format!("‖ {}{}", prefix(Some(*label)), lang.tr(i18n::HELD));
                let provider = provider.map(|p| format!(" · {}", i18n::provider_name(p)));
                format!(
                    "{head}{} · /continue {}",
                    provider.unwrap_or_default(),
                    label.0
                )
            }
            Self::HeldInput { label, text, .. } => with_text(
                format!("‖ {}{}", prefix(*label), lang.tr(i18n::HELD)),
                text.as_deref(),
            ),
            Self::CloseHeldConfirm { label, .. } => format!(
                "‖ {} {}",
                labels::format(*label),
                lang.tr(i18n::CLOSE_HELD_QUESTION)
            ),
            Self::Alert(alert) => alert_text(lang, alert),
            Self::StopUnconfirmed { remaining } => match lang {
                Lang::Ko => format!(
                    "{} · {remaining}{}",
                    i18n::STOP_UNCONFIRMED,
                    i18n::REMAINING_SUFFIX
                ),
                Lang::En => format!(
                    "{} · {remaining} {}",
                    lang.tr(i18n::STOP_UNCONFIRMED),
                    lang.tr(i18n::REMAINING_SUFFIX)
                ),
            },
            Self::RouterUnavailableSend => lang.tr(i18n::ROUTER_UNAVAILABLE_SEND).to_string(),
            Self::Settings { revision, warning } => settings_warning_text(lang, *revision, warning),
        }
    }

    pub(crate) fn buttons(&self) -> Vec<Button> {
        match self {
            Self::Queued {
                input,
                reason: QueueReason::RouterOrder,
                ..
            } => vec![Button::CancelInput(*input)],
            Self::Queued { input, .. } => vec![Button::Send(*input), Button::CancelInput(*input)],
            Self::HeldTask { task, .. } => {
                vec![Button::ContinueTask(*task), Button::CloseHeld(*task)]
            }
            Self::HeldInput { input, .. } => vec![
                Button::ContinueInput(*input),
                Button::CancelHeldInput(*input),
            ],
            _ => Vec::new(),
        }
    }
}

/// 누르면 같은 뜻의 명령과 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Button {
    Send(InputId),
    CancelInput(InputId),
    ContinueTask(TaskId),
    ContinueInput(InputId),
    CloseHeld(TaskId),
    CancelHeldInput(InputId),
}

impl Button {
    pub(crate) fn text(self, lang: Lang) -> &'static str {
        match self {
            Self::Send(_) => lang.tr(i18n::BUTTON_SEND),
            Self::ContinueTask(_) | Self::ContinueInput(_) => lang.tr(i18n::BUTTON_CONTINUE),
            Self::CancelInput(_) | Self::CloseHeld(_) | Self::CancelHeldInput(_) => {
                lang.tr(i18n::BUTTON_CANCEL)
            }
        }
    }
}

// cost: time O(t log t + i log i + a), heap O(t + i + a), stack O(1)
// vars: t = 작업 수, i = 입력 수, a = 알림 수
// basis: estimate
/// `Queued`인데 `reason`이 없으면 줄을 만들지 않는다(engine이 항상 싣는다).
pub(crate) fn build(state: &ChatState, now: Instant) -> Vec<StatusLine> {
    let mut lines: Vec<StatusLine> = Vec::new();
    let mut tasks: Vec<&TaskView> = state.tasks.values().collect();
    tasks.sort_by_key(|task| task.seq);
    let mut inputs: Vec<&InputView> = state.inputs.values().collect();
    inputs.sort_by_key(|input| input.seq);

    lines.extend(
        tasks
            .iter()
            .filter(|task| labels::is_live(task.state))
            .map(|task| StatusLine::Running(running_line(task, now))),
    );
    lines.extend(
        inputs
            .iter()
            .filter(|input| input.state == InputState::Judging)
            .map(|input| StatusLine::Judging {
                input: input.id,
                label: input.label,
                text: input.text.clone(),
            }),
    );
    lines.extend(state.training.clone().map(StatusLine::Training));
    lines.extend(inputs.iter().filter_map(|input| queued_line(input)));
    lines.extend(held_lines(state, &tasks, &inputs));
    lines.extend(alert_lines(state));
    lines
}

pub(crate) fn queue_reason_text(lang: Lang, reason: QueueReason) -> String {
    let key = match reason {
        QueueReason::AfterTask(label) => {
            return match lang {
                Lang::Ko => format!("{} {}", label.0, i18n::AFTER_TASK_SUFFIX),
                Lang::En => format!("{} {}", lang.tr(i18n::AFTER_TASK_SUFFIX), label.0),
            };
        }
        QueueReason::RouterOrder => i18n::ROUTER_ORDER,
        QueueReason::RouterConnection => i18n::ROUTER_CONNECTION,
        QueueReason::WriteTurn => i18n::WRITE_TURN,
        QueueReason::AfterCompaction => i18n::AFTER_COMPACTION,
        QueueReason::AfterAllTasks => i18n::AFTER_ALL_TASKS,
    };
    lang.tr(key).to_string()
}

pub(crate) fn activity_text(lang: Lang, activity: &Activity) -> String {
    let key = match activity {
        Activity::Thinking => i18n::THINKING,
        Activity::ReadingFile => i18n::READING_FILE,
        Activity::EditingFile => i18n::EDITING_FILE,
        Activity::RunningCommand { command } => {
            let first = command.lines().next().unwrap_or_default();
            return format!(
                "{} {}",
                lang.tr(i18n::RUNNING_COMMAND),
                truncate(first, COMMAND_PREVIEW_COLS)
            );
        }
        Activity::Compacting => i18n::COMPACTING,
        Activity::SwitchingProvider => i18n::SWITCHING_PROVIDER,
    };
    lang.tr(key).to_string()
}

pub(crate) fn settings_warning_text(
    lang: Lang,
    revision: u64,
    warning: &SettingsWarning,
) -> String {
    match warning {
        SettingsWarning::Fallback { layer, fault } => {
            let layer = match layer {
                SettingsLayer::Default => i18n::SETTINGS_LAYER_DEFAULT,
                SettingsLayer::User => i18n::SETTINGS_LAYER_USER,
                SettingsLayer::Folder => i18n::SETTINGS_LAYER_FOLDER,
                SettingsLayer::Chat => i18n::SETTINGS_LAYER_CHAT,
                SettingsLayer::Run => i18n::SETTINGS_LAYER_RUN,
            };
            let detail = match fault {
                SettingsFault::Parse { line, message } => lang
                    .tr(i18n::SETTINGS_PARSE_LINE)
                    .replace("{line}", &line.to_string())
                    .replace("{message}", message),
                SettingsFault::Invalid { key, reason } => format!("{key}: {reason}"),
            };
            lang.tr(i18n::SETTINGS_FALLBACK)
                .replace("{layer}", lang.tr(layer))
                .replace("{previous}", &revision.to_string())
                .replace("{detail}", &detail)
        }
        SettingsWarning::IgnoredFolderKeys { keys } => lang
            .tr(i18n::SETTINGS_IGNORED)
            .replace("{keys}", &keys.join(", ")),
    }
}

pub(crate) fn alert_text(lang: Lang, alert: &Alert) -> String {
    let key = match alert {
        Alert::RouterPaused => i18n::ROUTER_PAUSED,
        Alert::RouterDisconnected => i18n::ROUTER_DISCONNECTED,
        Alert::SteerNotReady { provider } => {
            return format!(
                "{} ({})",
                lang.tr(i18n::STEER_NOT_READY),
                i18n::provider_name(*provider)
            );
        }
        Alert::RouterDownSendingInOrder => i18n::ROUTER_UNAVAILABLE_SEND,
        Alert::SchemaMigrated { to, .. } => {
            return lang
                .tr(i18n::SCHEMA_MIGRATED)
                .replace("{to}", &to.to_string());
        }
    };
    lang.tr(key).to_string()
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 그릴 줄 수
// basis: estimate
/// 그리기와 마우스 클릭 판정이 같은 계산을 쓴다.
pub(crate) fn button_rects(lines: &[StatusLine], lang: Lang, area: Rect) -> Vec<(Rect, Button)> {
    let mut rects = Vec::new();
    for (row, line) in lines.iter().enumerate().take(usize::from(area.height)) {
        let buttons = line.buttons();
        let total = buttons_width(&buttons, lang);
        let y = area.y + row as u16;
        let mut x = area.right().saturating_sub(total).max(area.x);
        for button in buttons {
            let width = text_width(button.text(lang)) as u16;
            rects.push((Rect::new(x, y, width, 1), button));
            x += width + 1;
        }
    }
    rects
}

#[derive(Debug)]
pub(crate) struct StatusBoardView<'a> {
    pub lines: &'a [StatusLine],
    pub lang: Lang,
    pub labels_visible: bool,
    pub spinner: char,
}

impl StatusBoardView<'_> {
    // cost: time O(l·w), heap O(l·w), stack O(1)
    // vars: l = 그릴 줄 수, w = 칸 폭
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let rects = button_rects(self.lines, self.lang, area);
        let width = usize::from(area.width);
        let rows: Vec<Line> = self
            .lines
            .iter()
            .take(usize::from(area.height))
            .map(|line| {
                let buttons = buttons_width(&line.buttons(), self.lang);
                let room = if buttons == 0 {
                    width
                } else {
                    width.saturating_sub(usize::from(buttons) + 2)
                };
                let text = line.text(self.lang, self.labels_visible, self.spinner);
                Line::from(truncate(&text, room))
            })
            .collect();
        frame.render_widget(Paragraph::new(rows), area);
        for (rect, button) in rects {
            frame.render_widget(Span::styled(button.text(self.lang), EMPHASIS), rect);
        }
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 줄 글자 수
// basis: estimate
fn running_text(lang: Lang, line: &RunningLine) -> String {
    let doing = if line.awaiting_permission {
        Some(lang.tr(i18n::AWAITING_PERMISSION).to_string())
    } else if line.awaiting_input {
        Some(lang.tr(i18n::AWAITING_INPUT).to_string())
    } else if line.approval_pending {
        Some(approval_pending_text(lang, line.provider))
    } else if line.subagents > 0 {
        Some(subagents_text(lang, line.subagents))
    } else {
        line.activity.as_ref().map(|a| activity_text(lang, a))
    };
    if !line.has_output && doing.is_none() {
        return lang.tr(i18n::WORKING).to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    parts.extend(line.provider.map(|p| i18n::provider_name(p).to_string()));
    parts.extend(line.model.clone());
    parts.push(i18n::format_elapsed(lang, line.elapsed));
    parts.extend(doing);
    parts.join(" · ")
}

/// 문구 초안: `도구 사용 허가 준비 중 · codex`.
fn approval_pending_text(lang: Lang, provider: Option<Provider>) -> String {
    let text = lang.tr(i18n::APPROVAL_PENDING);
    match provider {
        Some(provider) => format!("{text} · {}", i18n::provider_name(provider)),
        None => text.to_string(),
    }
}

fn subagents_text(lang: Lang, count: usize) -> String {
    match lang {
        Lang::Ko => format!("{} {count}{}", i18n::SUBAGENTS, i18n::RUNNING_COUNT_SUFFIX),
        Lang::En => format!(
            "{} {} {count}",
            lang.tr(i18n::SUBAGENTS),
            lang.tr(i18n::RUNNING_COUNT_SUFFIX)
        ),
    }
}

fn training_text(lang: Lang, spinner: char, progress: &TrainingProgress) -> String {
    let graded = i18n::format_count(u64::from(progress.graded));
    let graded = match lang {
        Lang::Ko => format!("{} {graded}{}", i18n::USAGE_LABELS, i18n::COUNT_SUFFIX),
        Lang::En => format!("{} {graded}", lang.tr(i18n::USAGE_LABELS)),
    };
    format!(
        "{spinner} [{}] {} · {graded} · {} · {} {}",
        lang.tr(i18n::TRAINING),
        progress.stage,
        i18n::format_elapsed(lang, progress.elapsed),
        lang.tr(i18n::TOKEN),
        i18n::format_count(progress.tokens)
    )
}

fn with_text(head: String, text: Option<&str>) -> String {
    match text.and_then(|text| text.lines().next()) {
        Some(first) => format!("{head} · {first}"),
        None => head,
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn buttons_width(buttons: &[Button], lang: Lang) -> u16 {
    let widths: usize = buttons.iter().map(|b| text_width(b.text(lang))).sum();
    (widths + buttons.len().saturating_sub(1)) as u16
}

fn running_line(task: &TaskView, now: Instant) -> RunningLine {
    RunningLine {
        task: task.id,
        label: task.label,
        provider: task.provider,
        model: task.model.clone(),
        elapsed: task.stopwatch.elapsed(now),
        activity: task.activity.clone(),
        awaiting_permission: task.state == TaskState::AwaitingPermission,
        awaiting_input: task.state == TaskState::AwaitingInput,
        approval_pending: task.tool_started_at.is_some_and(|started| {
            now.saturating_duration_since(started) >= APPROVAL_PENDING_AFTER
        }),
        subagents: task.subagents.len(),
        has_output: task.has_output,
    }
}

fn queued_line(input: &InputView) -> Option<StatusLine> {
    if input.state != InputState::Queued {
        return None;
    }
    Some(StatusLine::Queued {
        input: input.id,
        label: input.label,
        reason: input.reason?,
        text: input.text.clone(),
    })
}

// cost: time O(t log t + i log i), heap O(t + i), stack O(1)
// vars: t = 보류 작업 수, i = 보류 입력 수
// basis: estimate
fn held_lines(state: &ChatState, tasks: &[&TaskView], inputs: &[&InputView]) -> Vec<StatusLine> {
    let mut held: Vec<(u64, StatusLine)> = Vec::new();
    for task in tasks
        .iter()
        .filter(|t| matches!(t.state, TaskState::Held | TaskState::NeedsCheck))
    {
        let line = if state.close_held_confirm == Some(task.id) {
            StatusLine::CloseHeldConfirm {
                task: task.id,
                label: task.label,
            }
        } else {
            StatusLine::HeldTask {
                task: task.id,
                label: task.label,
                provider: task.provider,
            }
        };
        held.push((task.seq, line));
    }
    for input in inputs.iter().filter(|i| i.state == InputState::Held) {
        let line = StatusLine::HeldInput {
            input: input.id,
            label: input.label,
            text: input.text.clone(),
        };
        held.push((input.seq, line));
    }
    held.sort_by_key(|(seq, _)| *seq);
    let stopped = state.stop.as_ref().map(|stop| StatusLine::Stopped {
        held: stop.held.clone(),
    });
    stopped
        .into_iter()
        .chain(held.into_iter().map(|(_, line)| line))
        .collect()
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 알림 수
// basis: estimate
fn alert_lines(state: &ChatState) -> Vec<StatusLine> {
    let mut lines: Vec<StatusLine> = state
        .alerts
        .iter()
        .map(|alert| match alert {
            Alert::RouterDownSendingInOrder => StatusLine::RouterUnavailableSend,
            other => StatusLine::Alert(other.clone()),
        })
        .collect();
    if let Some(remaining) = state.stop.as_ref().and_then(|stop| stop.unconfirmed) {
        lines.push(StatusLine::StopUnconfirmed { remaining });
    }
    if let Some((revision, Some(warning))) = &state.settings {
        lines.push(StatusLine::Settings {
            revision: revision.0,
            warning: warning.clone(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use saturn_protocol::event::{ProviderEvent, ToolDetail};
    use saturn_protocol::ids::{AgentId, SettingsRevision};

    use super::*;
    use crate::state::{InputUpdate, TaskUpdate};
    use crate::view::buffer_lines;

    fn task(state: &mut ChatState, id: u64, label: char, task_state: TaskState, now: Instant) {
        state.apply_task(
            TaskUpdate {
                task: TaskId(id),
                label: TaskLabel(label),
                state: task_state,
                provider: Some(Provider::Codex),
                elapsed: Duration::ZERO,
                failure: None,
            },
            now,
        );
    }

    fn input(state: &mut ChatState, id: u64, label: char, input_state: InputState, text: &str) {
        state.apply_input(InputUpdate {
            input: InputId(id),
            text: text.to_string(),
            label: Some(TaskLabel(label)),
            state: input_state,
            disposition: None,
            reason: (input_state == InputState::Queued)
                .then_some(QueueReason::AfterTask(TaskLabel('A'))),
        });
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn texts(lines: &[StatusLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.text(Lang::Ko, true, '⠙'))
            .collect()
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn alert_text_schema_migrated_shows_target_version() {
        let alert = Alert::SchemaMigrated { from: 1, to: 2 };

        assert_eq!(alert_text(Lang::Ko, &alert), "기록 저장소 v2로 옮김");
        assert_eq!(alert_text(Lang::En, &alert), "Record store migrated to v2");
    }

    #[test]
    fn build_orders_kinds_then_arrival() {
        let now = Instant::now();
        let mut state = ChatState::new();
        state.apply_alert(Alert::RouterPaused);
        input(
            &mut state,
            1,
            'C',
            InputState::Queued,
            "테스트도 같이 돌려줘",
        );
        task(&mut state, 1, 'E', TaskState::Held, now);
        input(
            &mut state,
            2,
            'D',
            InputState::Judging,
            "배포 스크립트 정리",
        );
        task(&mut state, 2, 'A', TaskState::Running, now);

        let lines = build(&state, now);

        assert_eq!(
            texts(&lines),
            vec![
                "⠙ [A] 작업 중",
                "⠙ [D] 판단 중 · 배포 스크립트 정리",
                "· [C] 대기 · A 다음 · 테스트도 같이 돌려줘",
                "‖ [E] 보류 · codex · /continue E",
                "자동 판단 일시 중단",
            ]
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn build_keeps_relative_order_when_line_removed() {
        let now = Instant::now();
        let mut state = ChatState::new();
        for (id, label) in [(1, 'A'), (2, 'B'), (3, 'C')] {
            task(&mut state, id, label, TaskState::Running, now);
        }
        let before = texts(&build(&state, now));

        task(&mut state, 2, 'B', TaskState::Done, now);
        state.finish_task(TaskId(2));
        let after = texts(&build(&state, now));

        assert_eq!(
            before,
            vec!["⠙ [A] 작업 중", "⠙ [B] 작업 중", "⠙ [C] 작업 중"]
        );
        assert_eq!(after, vec!["⠙ [A] 작업 중", "⠙ [C] 작업 중"]);
    }

    #[test]
    fn awaiting_input_shows_its_own_phrase() {
        let mut state = ChatState::new();
        let now = Instant::now();
        task(&mut state, 1, 'A', TaskState::AwaitingInput, now);

        let ko = texts(&build(&state, now)).join("\n");

        assert!(ko.contains("입력 기다림"), "{ko}");
        assert!(!ko.contains("허가 기다림"), "{ko}");
        assert_eq!(Lang::En.tr(i18n::AWAITING_INPUT), "Waiting for input");
    }

    #[test]
    fn running_line_with_output_shows_fields_and_subagents() {
        let line = StatusLine::Running(RunningLine {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Some(Provider::Claude),
            model: Some("opus".to_string()),
            elapsed: Duration::from_secs(60),
            activity: Some(Activity::Thinking),
            awaiting_permission: false,
            awaiting_input: false,
            approval_pending: false,
            subagents: 2,
            has_output: true,
        });

        assert_eq!(
            line.text(Lang::Ko, true, '⠙'),
            "⠙ [A] claude · opus · 1분 · 하위 에이전트 2개 실행 중"
        );
    }

    fn tool_call(task: TaskId) -> ProviderEvent {
        ProviderEvent::ToolCall {
            agent: AgentId(1),
            subagent: None,
            call_id: format!("call-{}", task.0),
            activity: Activity::RunningCommand {
                command: "touch a.txt".to_string(),
            },
            detail: ToolDetail::default(),
        }
    }

    fn running_texts(state: &ChatState, now: Instant) -> Vec<String> {
        texts(&build(state, now))
    }

    #[test]
    fn approval_pending_shows_after_three_seconds_without_events() {
        let start = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Running, start);
        state.apply_event(TaskId(1), tool_call(TaskId(1)), start);

        let early = running_texts(&state, start + Duration::from_millis(2_900));
        let late = running_texts(&state, start + Duration::from_secs(3));

        assert!(!early[0].contains("도구 사용 허가 준비 중"));
        assert!(late[0].ends_with("도구 사용 허가 준비 중 · codex"));
    }

    #[test]
    fn approval_pending_clears_when_permission_request_or_progress_arrives() {
        let start = Instant::now();
        let later = start + Duration::from_secs(4);
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Running, start);
        state.apply_event(TaskId(1), tool_call(TaskId(1)), start);
        assert!(running_texts(&state, later)[0].contains("도구 사용 허가 준비 중"));

        state.apply_event(
            TaskId(1),
            ProviderEvent::PermissionRequested {
                agent: AgentId(1),
                request_id: "r1".to_string(),
                summary: "touch a.txt".to_string(),
                reason: String::new(),
                call: None,
            },
            later,
        );
        task(&mut state, 1, 'A', TaskState::AwaitingPermission, later);
        let awaiting = running_texts(&state, later + Duration::from_secs(5));

        assert!(!awaiting[0].contains("도구 사용 허가 준비 중"));
        assert!(awaiting[0].contains("허가 기다림"));
        state.apply_event(TaskId(1), tool_call(TaskId(1)), later);
        state.apply_event(
            TaskId(1),
            ProviderEvent::ToolResult {
                agent: AgentId(1),
                subagent: None,
                call_id: "call-1".to_string(),
                output: String::new(),
                exit_code: Some(0),
            },
            later,
        );
        task(&mut state, 1, 'A', TaskState::Running, later);
        assert!(
            !running_texts(&state, later + Duration::from_secs(5))[0]
                .contains("도구 사용 허가 준비 중")
        );
    }

    #[test]
    fn approval_pending_text_names_the_provider_in_both_languages() {
        assert_eq!(
            approval_pending_text(Lang::Ko, Some(Provider::Codex)),
            "도구 사용 허가 준비 중 · codex"
        );
        assert_eq!(
            approval_pending_text(Lang::En, Some(Provider::Claude)),
            "Preparing tool permission · claude"
        );
        assert_eq!(
            approval_pending_text(Lang::En, None),
            "Preparing tool permission"
        );
    }

    #[test]
    fn stopped_and_close_confirm_lines_match_design() {
        let now = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Held, now);
        task(&mut state, 2, 'E', TaskState::Held, now);
        state.apply_stopped(vec![TaskLabel('A'), TaskLabel('C')]);
        state.close_held_confirm = Some(TaskId(2));

        let lines = texts(&build(&state, now));

        assert_eq!(lines[0], "‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서");
        assert_eq!(lines[2], "‖ [E] 보류를 닫을까요?");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn alert_lines_include_settings_error_and_unconfirmed_stop() {
        let now = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Held, now);
        state.apply_stopped(vec![TaskLabel('A')]);
        state.apply_stop_unconfirmed(2);
        state.settings = Some((
            SettingsRevision(12),
            Some(SettingsWarning::Fallback {
                layer: SettingsLayer::Folder,
                fault: SettingsFault::Parse {
                    line: 7,
                    message: "...".to_string(),
                },
            }),
        ));
        state.apply_alert(Alert::SteerNotReady {
            provider: Provider::Codex,
        });

        let lines = texts(&build(&state, now));

        assert!(lines.contains(&"바로 반영 준비 중 (codex)".to_string()));
        assert!(lines.contains(&"멈춤 확인 안 됨 · 2개 남음".to_string()));
        assert!(
            lines.contains(&"폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...".to_string())
        );
    }

    #[test]
    fn settings_warning_text_follows_language() {
        let fallback = SettingsWarning::Fallback {
            layer: SettingsLayer::User,
            fault: SettingsFault::Invalid {
                key: "router.endpoint".to_string(),
                reason: "not https".to_string(),
            },
        };
        let ignored = SettingsWarning::IgnoredFolderKeys {
            keys: vec!["router.endpoint".to_string(), "router.key".to_string()],
        };

        assert_eq!(
            settings_warning_text(Lang::Ko, 3, &fallback),
            "사용자 설정 오류 · 이전 설정 번호 3로 계속 · router.endpoint: not https"
        );
        assert_eq!(
            settings_warning_text(Lang::En, 3, &fallback),
            "User settings error · continuing with settings revision 3 · router.endpoint: not https"
        );
        assert_eq!(
            settings_warning_text(Lang::En, 3, &ignored),
            "Ignored folder settings items · router.endpoint, router.key"
        );
    }

    #[test]
    fn buttons_follow_line_kind() {
        let router_order = StatusLine::Queued {
            input: InputId(1),
            label: None,
            reason: QueueReason::RouterOrder,
            text: None,
        };
        let held = StatusLine::HeldTask {
            task: TaskId(1),
            label: TaskLabel('E'),
            provider: None,
        };

        assert_eq!(
            router_order.buttons(),
            vec![Button::CancelInput(InputId(1))]
        );
        assert_eq!(
            held.buttons(),
            vec![
                Button::ContinueTask(TaskId(1)),
                Button::CloseHeld(TaskId(1))
            ]
        );
    }

    #[test]
    fn queue_reason_and_activity_texts() {
        let command = Activity::RunningCommand {
            command: "x".repeat(60),
        };

        assert_eq!(
            queue_reason_text(Lang::En, QueueReason::AfterTask(TaskLabel('A'))),
            "after A"
        );
        assert_eq!(
            queue_reason_text(Lang::Ko, QueueReason::AfterAllTasks),
            "모든 작업 뒤"
        );
        let text = activity_text(Lang::Ko, &command);
        assert_eq!(text_width("명령 실행 중 "), 13);
        assert_eq!(text_width(&text), 13 + COMMAND_PREVIEW_COLS);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn button_rects_sit_at_line_end() {
        let lines = vec![StatusLine::Queued {
            input: InputId(1),
            label: None,
            reason: QueueReason::WriteTurn,
            text: None,
        }];

        let rects = button_rects(&lines, Lang::En, Rect::new(0, 5, 40, 1));

        assert_eq!(rects[0], (Rect::new(25, 5, 6, 1), Button::Send(InputId(1))));
        assert_eq!(
            rects[1],
            (Rect::new(32, 5, 8, 1), Button::CancelInput(InputId(1)))
        );
    }

    #[test]
    fn render_draws_text_and_buttons() {
        let lines = vec![StatusLine::Queued {
            input: InputId(1),
            label: Some(TaskLabel('C')),
            reason: QueueReason::AfterTask(TaskLabel('A')),
            text: Some("test".to_string()),
        }];
        let mut terminal = Terminal::new(TestBackend::new(50, 1)).unwrap();
        let view = StatusBoardView {
            lines: &lines,
            lang: Lang::Ko,
            labels_visible: true,
            spinner: '⠙',
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let row = &buffer_lines(terminal.backend().buffer())[0];
        assert!(row.starts_with("· [C] 대기 · A 다음 · test"));
        assert!(row.ends_with("[보내기] [취소]"));
    }
}
