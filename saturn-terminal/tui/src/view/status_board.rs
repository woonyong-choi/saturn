//! 상태판. 실행 줄, 판단 줄, 학습 줄, 대기 줄, 보류 줄, 알림 줄을 이 순서로 쌓는다.
//!
//! 설계: docs/design/tui.md(상태판 줄 순서, 상태 표시), docs/design/input-handling.md(대기와 취소, 멈춤과 보류).
//! - 같은 종류 안에서는 접수 순서(`seq`)를 따른다. 줄이 생기거나 사라져도 남은 줄끼리 상대 위치는 유지한다.
//! - 보류 줄을 뺀 나머지 줄은 그 항목이 끝나면 지운다.
//! - 판단 줄은 판단 방식과 관계없이 같은 문구를 쓰고 근거와 확률은 보이지 않는다.
//! - 대기 줄과 보류 줄의 버튼은 전체 화면에서 클릭할 수 있다(`buttons`로 칸을 구해 마우스 위치와 맞춘다).
//!
//! 줄은 매 프레임 `build`로 `state::ChatState`에서 새로 만든다.

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::QueueReason;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::state::{InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::state::{ChatState, InputView, TaskView, TrainingProgress};
use crate::view::transcript::held_labels;
use crate::view::{EMPHASIS, text_width, truncate};

/// 실행 줄 `명령 실행 중` 뒤에 보이는 명령 앞 칸 수.
pub const COMMAND_PREVIEW_COLS: usize = 40;

/// 줄 종류. 값 순서가 상태판 위→아래 순서다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LineKind {
    /// 실행 줄.
    Running,
    /// 판단 줄.
    Judging,
    /// 학습 줄.
    Training,
    /// 대기 줄.
    Queued,
    /// 보류 줄(멈춤 결과 줄과 보류 닫기 확인 포함).
    Held,
    /// 알림 줄.
    Alert,
}

/// 실행 줄.
/// 출력 전: `⠙ [A] 작업 중`. 출력 뒤: `⠙ [A] claude · opus · 1분 · 하위 에이전트 2개 실행 중`처럼
/// provider · 모델 · 경과 · 하는 일(subagent가 돌면 `하위 에이전트 N개 실행 중`).
/// TODO(#50): 칸 순서를 provider와 모델 먼저로 둘지, 하는 일 먼저로 둘지, 모델 이름을 보고된 그대로 쓸지 별칭으로 쓸지
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningLine {
    /// 작업.
    pub task: TaskId,
    /// 이름표.
    pub label: TaskLabel,
    /// 마지막으로 답한 provider.
    pub provider: Option<Provider>,
    /// 보고된 모델.
    pub model: Option<String>,
    /// 경과(허가 대기 동안 멈춘 값).
    pub elapsed: Duration,
    /// 하는 일. `None`이고 출력 전이면 `작업 중`.
    pub activity: Option<Activity>,
    /// 허가 응답 대기 중(`허가 기다림`). `activity`보다 앞선다.
    pub awaiting_permission: bool,
    /// 도는 provider subagent 수. 0이 아니면 하는 일 대신 `하위 에이전트 N개 실행 중`.
    pub subagents: usize,
    /// 모델 글이 한 번이라도 왔다.
    pub has_output: bool,
}

/// 상태판 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusLine {
    /// 실행 줄.
    Running(RunningLine),
    /// 판단 줄 `⠹ [D] 판단 중 · 배포 스크립트 정리`. 원문을 모르면 `· 원문` 칸을 뺀다.
    /// TODO(#53): 짧게 끝나는 판단의 판단 줄을 생략할지, 항상 그릴지, 지연 표시와 최소 표시 시간을 둘지
    Judging {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// 학습 줄 `⠼ [학습] 단계 · 채점 N건 · 경과 · Token N`.
    /// TODO(#55): `/stop`이 진행 중인 `/train`도 멈출지, 학습 전용 중지 명령을 둘지, 학습 줄에 중지 버튼을 둘지
    Training(TrainingProgress),
    /// 대기 줄 `· [C] 대기 · A 다음 · 테스트도 같이 돌려줘  [보내기] [취소]`.
    /// 이유 문구는 `queue_reason_text`. `JudgeOrder`면 `[보내기]` 없이 `[취소]`만.
    Queued {
        input: InputId,
        label: Option<TaskLabel>,
        reason: QueueReason,
        text: Option<String>,
    },
    /// 멈춤 결과 `‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서`. 보류 줄 맨 앞.
    Stopped { held: Vec<TaskLabel> },
    /// 보류 줄(작업) `‖ [E] 보류 · codex · /continue E  [이어서] [취소]`.
    HeldTask {
        task: TaskId,
        label: TaskLabel,
        provider: Option<Provider>,
    },
    /// 보류 줄(보내지 않은 입력) `‖ [C] 보류 · 원문  [이어서] [취소]`.
    HeldInput {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
    /// 보류 닫기 확인 `‖ [E] 보류를 닫을까요?`. 그 작업의 보류 줄 자리에 대신 그린다. `Enter` 종료, `Esc` 유지.
    CloseHeldConfirm { task: TaskId, label: TaskLabel },
    /// 알림 줄. 문구는 `alert_text`.
    Alert(Alert),
    /// 알림 줄 `멈춤 확인 안 됨 · N개 남음`.
    StopUnconfirmed { remaining: u32 },
    /// 알림 줄 `판단기 연결 없음 · 차례에 보냅니다`. `[보내기]`를 눌렀으나 judge 실패(`Alert::JudgeDownSendingInOrder`).
    JudgeUnavailableSend,
    /// 알림 줄 `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`(`SettingsApplied`의 경고).
    SettingsError { previous: u64, detail: String },
}

impl StatusLine {
    /// 줄 종류.
    pub fn kind(&self) -> LineKind {
        match self {
            Self::Running(_) => LineKind::Running,
            Self::Judging { .. } => LineKind::Judging,
            Self::Training(_) => LineKind::Training,
            Self::Queued { .. } => LineKind::Queued,
            Self::Stopped { .. }
            | Self::HeldTask { .. }
            | Self::HeldInput { .. }
            | Self::CloseHeldConfirm { .. } => LineKind::Held,
            Self::Alert(_)
            | Self::StopUnconfirmed { .. }
            | Self::JudgeUnavailableSend
            | Self::SettingsError { .. } => LineKind::Alert,
        }
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 줄 글자 수
    // basis: estimate
    /// 버튼을 뺀 줄 글. `spinner`는 실행·판단·학습 줄 머리 글자, 대기 줄은 `·`, 보류 줄은 `‖`.
    pub fn text(&self, lang: Lang, labels_visible: bool, spinner: char) -> String {
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
            Self::JudgeUnavailableSend => lang.tr(i18n::JUDGE_UNAVAILABLE_SEND).to_string(),
            Self::SettingsError { previous, detail } => match lang {
                Lang::Ko => format!(
                    "{} · {} {previous}{} · {detail}",
                    i18n::SETTINGS_ERROR,
                    i18n::SETTINGS_PREVIOUS,
                    i18n::SETTINGS_CONTINUE_SUFFIX
                ),
                Lang::En => format!(
                    "{} · {} {previous} · {detail}",
                    lang.tr(i18n::SETTINGS_ERROR),
                    lang.tr(i18n::SETTINGS_PREVIOUS)
                ),
            },
        }
    }

    /// 줄 끝 버튼. 대기 줄 `[보내기] [취소]`(`JudgeOrder`면 `[취소]`만), 보류 줄 `[이어서] [취소]`, 그 밖에는 없음.
    pub fn buttons(&self) -> Vec<Button> {
        match self {
            Self::Queued {
                input,
                reason: QueueReason::JudgeOrder,
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

/// 상태판 버튼. 누르면 같은 뜻의 명령과 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// `[보내기]` = `/send` → `Request::SendNow`.
    Send(InputId),
    /// 대기 줄 `[취소]` = `/cancel` → `Request::CancelInput`.
    CancelInput(InputId),
    /// 보류 줄 `[이어서]` = `/continue` → `Request::Continue { task: Some }`.
    ContinueTask(TaskId),
    /// 보류 입력 줄 `[이어서]` → `Request::ContinueInput`(그 입력만 대기열로 되돌린다).
    ContinueInput(InputId),
    /// 보류 줄 `[취소]` = `/cancel` → 보류 닫기 확인 줄을 띄운다.
    CloseHeld(TaskId),
    /// 보류 입력 줄 `[취소]` → `Request::CancelInput`.
    CancelHeldInput(InputId),
}

impl Button {
    /// 버튼 글(`[보내기]`, `[취소]`, `[이어서]`).
    pub fn text(self, lang: Lang) -> &'static str {
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
/// 상태에서 줄을 만든다. 종류 순서, 같은 종류 안 접수 순서로 정렬한다.
/// 끝난 작업·끝 상태 입력은 줄이 없다. `Queued`인데 `reason`이 없으면 줄을 만들지 않는다(engine이 항상 싣는다).
pub fn build(state: &ChatState, now: Instant) -> Vec<StatusLine> {
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

/// 대기 이유 문구: `AfterTask(A)` → `A 다음`, `JudgeOrder` → `판단 차례`, `JudgeConnection` → `판단기 연결 기다림`,
/// `WriteTurn` → `쓰기 차례`, `AfterCompaction` → `맥락 정리 뒤`, `AfterAllTasks` → `모든 작업 뒤`.
pub fn queue_reason_text(lang: Lang, reason: QueueReason) -> String {
    let key = match reason {
        QueueReason::AfterTask(label) => {
            return match lang {
                Lang::Ko => format!("{} {}", label.0, i18n::AFTER_TASK_SUFFIX),
                Lang::En => format!("{} {}", lang.tr(i18n::AFTER_TASK_SUFFIX), label.0),
            };
        }
        QueueReason::JudgeOrder => i18n::JUDGE_ORDER,
        QueueReason::JudgeConnection => i18n::JUDGE_CONNECTION,
        QueueReason::WriteTurn => i18n::WRITE_TURN,
        QueueReason::AfterCompaction => i18n::AFTER_COMPACTION,
        QueueReason::AfterAllTasks => i18n::AFTER_ALL_TASKS,
    };
    lang.tr(key).to_string()
}

/// 실행 줄 하는 일 문구: `Thinking` → `생각 중`, `ReadingFile` → `파일 읽는 중`, `EditingFile` → `파일 수정 중`,
/// `RunningCommand` → `명령 실행 중 ` + 명령 앞 40칸, `Compacting` → `맥락 정리 중`, `SwitchingProvider` → `공급자 전환 중`.
pub fn activity_text(lang: Lang, activity: &Activity) -> String {
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

/// 알림 문구: `JudgePaused` → `자동 판단 일시 중단`, `IntakeStopped` → `새 입력 접수 중단 · 판단기 연결을 확인하세요`,
/// `SteerNotReady(codex)` → `바로 반영: 준비 중 (codex)`, `ChatBusyElsewhere` → `다른 Saturn에서 실행 중`.
pub fn alert_text(lang: Lang, alert: &Alert) -> String {
    let key = match alert {
        Alert::JudgePaused => i18n::JUDGE_PAUSED,
        Alert::IntakeStopped => i18n::INTAKE_STOPPED,
        Alert::SteerNotReady { provider } => {
            return format!(
                "{} ({})",
                lang.tr(i18n::STEER_NOT_READY),
                i18n::provider_name(*provider)
            );
        }
        Alert::ChatBusyElsewhere { .. } => i18n::BUSY_ELSEWHERE,
        Alert::JudgeDownSendingInOrder => i18n::JUDGE_UNAVAILABLE_SEND,
    };
    lang.tr(key).to_string()
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 그릴 줄 수
// basis: estimate
/// 줄마다 버튼 칸. `area`는 상태판 칸, 줄 `i`는 `area.y + i`행, 버튼은 줄 끝에 두 칸 띄어 오른쪽에 붙인다.
/// 그리기와 마우스 클릭 판정이 같은 계산을 쓴다.
pub fn button_rects(lines: &[StatusLine], lang: Lang, area: Rect) -> Vec<(Rect, Button)> {
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

/// 상태판 그리기.
#[derive(Debug)]
pub struct StatusBoardView<'a> {
    /// `build`로 만든 줄.
    pub lines: &'a [StatusLine],
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
    /// 스피너 글자.
    pub spinner: char,
}

impl StatusBoardView<'_> {
    // cost: time O(l·w), heap O(l·w), stack O(1)
    // vars: l = 그릴 줄 수, w = 칸 폭
    // basis: estimate
    /// 줄을 위에서부터 그리고 버튼을 `button_rects` 자리에 그린다. subagent 줄은 부모 줄 아래 흐리게.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
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
/// 실행 줄 본문. 출력·하는 일·허가 대기·subagent가 모두 없으면 `작업 중`,
/// 아니면 provider · 모델 · 경과 · 하는 일(허가 대기 → subagent 수 → 도구 순).
fn running_text(lang: Lang, line: &RunningLine) -> String {
    let doing = if line.awaiting_permission {
        Some(lang.tr(i18n::AWAITING_PERMISSION).to_string())
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

/// `하위 에이전트 2개 실행 중`.
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

/// 학습 줄 `⠼ [학습] 단계 · 채점 83건 · 1분 · Token 9,870`.
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

/// 줄 끝에 ` · 원문 첫 줄`을 붙인다. 원문을 모르면 그대로.
fn with_text(head: String, text: Option<&str>) -> String {
    match text.and_then(|text| text.lines().next()) {
        Some(first) => format!("{head} · {first}"),
        None => head,
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 버튼 칸 전체 폭. 버튼 사이 한 칸.
fn buttons_width(buttons: &[Button], lang: Lang) -> u16 {
    let widths: usize = buttons.iter().map(|b| text_width(b.text(lang))).sum();
    (widths + buttons.len().saturating_sub(1)) as u16
}

/// 작업의 실행 줄.
fn running_line(task: &TaskView, now: Instant) -> RunningLine {
    RunningLine {
        task: task.id,
        label: task.label,
        provider: task.provider,
        model: task.model.clone(),
        elapsed: task.stopwatch.elapsed(now),
        activity: task.activity.clone(),
        awaiting_permission: task.state == TaskState::AwaitingPermission,
        subagents: task.subagents.len(),
        has_output: task.has_output,
    }
}

/// 대기 줄. 이유가 없으면 만들지 않는다.
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
/// 보류 줄: 멈춤 결과를 맨 앞에, 이어서 보류 작업과 보류 입력을 접수 순서로. 닫기 확인 중인 작업은 확인 줄로 바꾼다.
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
/// 알림 줄: engine 경고, 멈춤 뒤 남은 프로세스, 폴더 설정 오류.
fn alert_lines(state: &ChatState) -> Vec<StatusLine> {
    let mut lines: Vec<StatusLine> = state
        .alerts
        .iter()
        .map(|alert| match alert {
            Alert::JudgeDownSendingInOrder => StatusLine::JudgeUnavailableSend,
            other => StatusLine::Alert(other.clone()),
        })
        .collect();
    if let Some(remaining) = state.stop.as_ref().and_then(|stop| stop.unconfirmed) {
        lines.push(StatusLine::StopUnconfirmed { remaining });
    }
    if let Some((revision, Some(detail))) = &state.settings {
        lines.push(StatusLine::SettingsError {
            previous: revision.0,
            detail: detail.clone(),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use saturn_protocol::ids::SettingsRevision;

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

    #[test]
    fn build_orders_kinds_then_arrival() {
        let now = Instant::now();
        let mut state = ChatState::new();
        state.apply_alert(Alert::JudgePaused);
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
    fn running_line_with_output_shows_fields_and_subagents() {
        let line = StatusLine::Running(RunningLine {
            task: TaskId(1),
            label: TaskLabel('A'),
            provider: Some(Provider::Claude),
            model: Some("opus".to_string()),
            elapsed: Duration::from_secs(60),
            activity: Some(Activity::Thinking),
            awaiting_permission: false,
            subagents: 2,
            has_output: true,
        });

        assert_eq!(
            line.text(Lang::Ko, true, '⠙'),
            "⠙ [A] claude · opus · 1분 · 하위 에이전트 2개 실행 중"
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
        state.settings = Some((SettingsRevision(12), Some("줄 7: ...".to_string())));
        state.apply_alert(Alert::SteerNotReady {
            provider: Provider::Codex,
        });

        let lines = texts(&build(&state, now));

        assert!(lines.contains(&"바로 반영: 준비 중 (codex)".to_string()));
        assert!(lines.contains(&"멈춤 확인 안 됨 · 2개 남음".to_string()));
        assert!(
            lines.contains(&"폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...".to_string())
        );
    }

    #[test]
    fn buttons_follow_line_kind() {
        let judge_order = StatusLine::Queued {
            input: InputId(1),
            label: None,
            reason: QueueReason::JudgeOrder,
            text: None,
        };
        let held = StatusLine::HeldTask {
            task: TaskId(1),
            label: TaskLabel('E'),
            provider: None,
        };

        assert_eq!(judge_order.buttons(), vec![Button::CancelInput(InputId(1))]);
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
