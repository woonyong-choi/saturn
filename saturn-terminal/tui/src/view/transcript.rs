//! 대화 기록 영역. 셀 글은 `TranscriptCell::lines`가 만들고 plain 출력도 같은 함수를 쓴다.
//! 설계: docs/design/tui.md

use std::path::PathBuf;
use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskLabel};
use saturn_protocol::rpc::{ChatNotice, ExtensionInfo};
use saturn_protocol::state::{Disposition, InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::shell::ShellOutput;
use crate::state::{InputUpdate, TaskView};
use crate::view::extensions;
use crate::view::start_screen::StartInfo;
use crate::view::status_board::activity_text;
use crate::view::{ERROR, MUTED, is_plain, wrap};

/// 초안 값.
pub(crate) const SHELL_PREVIEW_LINES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeliveryBadge {
    Delivering,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TranscriptCell {
    /// 첫 결과 뒤 시작 화면이 바뀐 맨 위 셀.
    Header(StartInfo),
    InputEcho {
        input: InputId,
        label: Option<TaskLabel>,
        text: String,
        badge: Option<DeliveryBadge>,
    },
    AgentText {
        label: Option<TaskLabel>,
        lines: Vec<String>,
    },
    /// `call_id`로 결과를 짝짓는다.
    Tool {
        label: Option<TaskLabel>,
        call_id: String,
        activity: Activity,
        output: String,
        /// 결과를 모른 채 작업이 끝났거나 멈췄다.
        is_interrupted: bool,
    },
    Result {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        tokens: Option<u64>,
    },
    Failed {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        cause: String,
    },
    /// 이름표는 보임 규칙과 관계없이 늘 붙인다.
    NeedsCheck { label: TaskLabel },
    /// `Stopped`, `StopUnconfirmed`는 상태판에 그리고 여기에는 넣지 않는다.
    Notice {
        label: Option<TaskLabel>,
        notice: ChatNotice,
    },
    /// 끼워 넣기가 아닌 판단의 머리 문구는 초안.
    Feedback {
        label: TaskLabel,
        disposition: Disposition,
        /// 고른 답의 자리(`FEEDBACK_ANSWERS`).
        selected: usize,
    },
    /// 고른 자리는 0이 `[실행]`, 1이 `[그대로]`.
    Correction { label: TaskLabel, selected: usize },
    /// 보류 닫기가 끝났다.
    HeldClosed { label: TaskLabel },
    /// 채점 후보가 모자라 `/train`을 실행하지 못했다.
    #[cfg_attr(not(test), expect(dead_code, reason = "#91 학습 실행 구현 전"))]
    TrainShort { graded: u32, need: u32 },
    /// `!` 셸 명령 결과.
    Shell(ShellOutput),
    /// 명령 해석 오류 같은 한 줄 경고, 원문 그대로.
    Warning(String),
    /// `/extensions`의 설치한 확장 목록.
    ExtensionList(Vec<ExtensionInfo>),
}

impl TranscriptCell {
    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 셀 글자 수
    // basis: estimate
    /// 전체 화면과 plain 출력이 같은 결과를 내도록 문구는 모두 여기서 만든다.
    pub(crate) fn lines(&self, lang: Lang, labels_visible: bool, expanded: bool) -> Vec<String> {
        if is_plain() {
            return self.plain_lines(lang, expanded);
        }
        self.base_lines(lang, labels_visible, expanded)
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 셀 글자 수
    // basis: estimate
    /// 단순 방식의 줄. 이름표를 늘 보이고 줄마다 말한 쪽을 앞에 적는다. 작업의 말은 `[A] `, Saturn 자신의 안내와 경고는
    /// `Saturn: `이다. 입력 에코(`>`)와 시작 머리는 그대로 둔다.
    pub(crate) fn plain_lines(&self, lang: Lang, expanded: bool) -> Vec<String> {
        let lines = self.base_lines(lang, true, expanded);
        let speaker = match self {
            Self::Header(_) | Self::InputEcho { .. } => return lines,
            Self::AgentText { label, .. }
            | Self::Tool { label, .. }
            | Self::Result { label, .. }
            | Self::Failed { label, .. }
            | Self::Notice { label, .. } => match label {
                Some(label) => format!("{} ", labels::format(*label)),
                None => SATURN_SPEAKER.to_owned(),
            },
            Self::NeedsCheck { label }
            | Self::Feedback { label, .. }
            | Self::Correction { label, .. }
            | Self::HeldClosed { label } => format!("{} ", labels::format(*label)),
            Self::TrainShort { .. }
            | Self::Shell(_)
            | Self::Warning(_)
            | Self::ExtensionList(_) => SATURN_SPEAKER.to_owned(),
        };
        lines
            .into_iter()
            .map(|line| {
                if line.starts_with(&speaker) {
                    line
                } else {
                    format!("{speaker}{line}")
                }
            })
            .collect()
    }

    fn base_lines(&self, lang: Lang, labels_visible: bool, expanded: bool) -> Vec<String> {
        let prefix = |label: Option<TaskLabel>| labels::prefix(label, labels_visible);
        match self {
            Self::Header(info) => info.lines(lang),
            Self::InputEcho {
                label, text, badge, ..
            } => echo_lines(lang, &prefix(*label), text, *badge),
            Self::AgentText { label, lines } => {
                let prefix = prefix(*label);
                lines.iter().map(|line| format!("{prefix}{line}")).collect()
            }
            Self::Tool {
                label,
                activity,
                output,
                is_interrupted,
                ..
            } => tool_lines(
                lang,
                &prefix(*label),
                activity,
                output,
                *is_interrupted,
                expanded,
            ),
            Self::Result {
                label,
                provider,
                elapsed,
                tokens,
            } => vec![result_line(
                lang,
                &prefix(*label),
                *provider,
                *elapsed,
                &tokens_text(lang, *tokens),
            )],
            Self::Failed {
                label,
                provider,
                elapsed,
                cause,
            } => vec![
                result_line(
                    lang,
                    &prefix(*label),
                    *provider,
                    *elapsed,
                    lang.tr(i18n::FAILED),
                ),
                cause.clone(),
            ],
            Self::NeedsCheck { label } => vec![format!(
                "{} {} · /continue {}",
                labels::format(*label),
                lang.tr(i18n::NEEDS_CHECK),
                label.0
            )],
            Self::Notice { label, notice } => notice_lines(lang, &prefix(*label), notice),
            Self::Feedback {
                label,
                disposition,
                selected,
            } => {
                if is_plain() {
                    return plain_feedback_lines(lang, *label, *disposition, *selected);
                }
                vec![feedback_line(lang, *label, *disposition, *selected)]
            }
            Self::Correction { label, selected } if is_plain() => {
                let head = format!(
                    "{} {}",
                    labels::format(*label),
                    lang.tr(i18n::CORRECTION_QUESTION)
                );
                vec![
                    head,
                    plain_choice(*selected == 0, '1', lang.tr(i18n::BUTTON_RUN)),
                    plain_choice(*selected == 1, '2', lang.tr(i18n::BUTTON_KEEP)),
                ]
            }
            Self::Correction { label, selected } => vec![format!(
                "{} {} {}  {}",
                labels::format(*label),
                lang.tr(i18n::CORRECTION_QUESTION),
                choice_text(*selected, 0, '1', lang.tr(i18n::BUTTON_RUN)),
                choice_text(*selected, 1, '2', lang.tr(i18n::BUTTON_KEEP))
            )],
            Self::HeldClosed { label } => vec![format!(
                "{} {}",
                labels::format(*label),
                lang.tr(i18n::HELD_CLOSED)
            )],
            Self::TrainShort { graded, need } => vec![train_short_line(lang, *graded, *need)],
            Self::Shell(output) => shell_lines(lang, output, expanded),
            Self::Warning(text) => vec![text.clone()],
            Self::ExtensionList(list) => extensions::list_lines(lang, list),
        }
    }

    fn line_style(&self, index: usize) -> Style {
        match self {
            Self::Failed { .. } if index > 0 => ERROR,
            Self::Tool {
                is_interrupted: true,
                ..
            } if index == 1 => ERROR,
            Self::Tool { .. } | Self::Shell(_) if index > 0 => MUTED,
            Self::InputEcho {
                text,
                badge: Some(DeliveryBadge::Rejected),
                ..
            } if index + 1 == text.lines().count().max(1) => ERROR,
            _ => Style::new(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Transcript {
    cells: Vec<TranscriptCell>,
    scroll_from_bottom: usize,
}

impl Transcript {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 셀 글자 수(위로 스크롤 중일 때만 줄 수를 센다)
    // basis: estimate
    /// 맨 아래를 보고 있으면 따라가고, 위를 보고 있으면 보던 자리를 지킨다.
    pub(crate) fn push(&mut self, cell: TranscriptCell) {
        if self.scroll_from_bottom > 0 {
            self.scroll_from_bottom += cell_rows(&cell);
        }
        self.cells.push(cell);
    }

    /// 머리 셀 뒤에 넣고, 보던 자리는 아래 기준이라 그대로다.
    pub(crate) fn prepend(&mut self, cells: Vec<TranscriptCell>) {
        let at = usize::from(matches!(
            self.cells.first(),
            Some(TranscriptCell::Header(_))
        ));
        self.cells.splice(at..at, cells);
    }

    pub(crate) fn set_header(&mut self, info: StartInfo) {
        match self.cells.first_mut() {
            Some(TranscriptCell::Header(header)) => *header = info,
            _ => self.cells.insert(0, TranscriptCell::Header(info)),
        }
    }

    /// 머리 셀이 있으면 더한 폴더에 넣는다. 이미 있으면 그대로 둔다.
    pub(crate) fn add_header_dir(&mut self, dir: PathBuf) {
        if let Some(TranscriptCell::Header(header)) = self.cells.first_mut()
            && !header.added_dirs.contains(&dir)
        {
            header.added_dirs.push(dir);
        }
    }

    pub(crate) fn remove_feedback(&mut self, label: TaskLabel) {
        self.cells.retain(
            |cell| !matches!(cell, TranscriptCell::Feedback { label: shown, .. } if *shown == label),
        );
    }

    pub(crate) fn remove_correction(&mut self, label: TaskLabel) {
        self.cells.retain(
            |cell| !matches!(cell, TranscriptCell::Correction { label: shown, .. } if *shown == label),
        );
    }

    // cost: time O(c), heap O(1), stack O(1)
    // vars: c = 셀 수
    // basis: estimate
    /// 그 이름표의 피드백 질문이나 바로잡기 제안이 고른 자리를 옮긴다.
    pub(crate) fn set_choice(&mut self, label: TaskLabel, index: usize) {
        for cell in self.cells.iter_mut().rev() {
            match cell {
                TranscriptCell::Feedback {
                    label: shown,
                    selected,
                    ..
                }
                | TranscriptCell::Correction {
                    label: shown,
                    selected,
                } if *shown == label => {
                    *selected = index;
                    return;
                }
                _ => {}
            }
        }
    }

    // cost: time O(c), heap O(1), stack O(1)
    // vars: c = 셀 수
    // basis: estimate
    pub(crate) fn set_badge(&mut self, input: InputId, badge: DeliveryBadge) {
        let echo = self.cells.iter_mut().rev().find_map(|cell| match cell {
            TranscriptCell::InputEcho {
                input: shown,
                badge,
                ..
            } if *shown == input => Some(badge),
            _ => None,
        });
        if let Some(slot) = echo {
            *slot = Some(badge);
        }
    }

    // cost: time O(c), heap O(1), stack O(1)
    // vars: c = 셀 수
    // basis: estimate
    /// 셀이 없으면 `false`.
    pub(crate) fn set_tool_output(&mut self, call_id: &str, text: String) -> bool {
        let tool = self.cells.iter_mut().rev().find_map(|cell| match cell {
            TranscriptCell::Tool {
                call_id: shown,
                output,
                ..
            } if shown == call_id => Some(output),
            _ => None,
        });
        match tool {
            Some(output) => {
                *output = text;
                true
            }
            None => false,
        }
    }

    // cost: time O(c), heap O(1), stack O(1)
    // vars: c = 셀 수
    // basis: estimate
    /// 셀이 없으면 `false`.
    pub(crate) fn set_tool_interrupted(&mut self, call_id: &str) -> bool {
        let tool = self.cells.iter_mut().rev().find_map(|cell| match cell {
            TranscriptCell::Tool {
                call_id: shown,
                is_interrupted,
                ..
            } if shown == call_id => Some(is_interrupted),
            _ => None,
        });
        match tool {
            Some(is_interrupted) => {
                *is_interrupted = true;
                true
            }
            None => false,
        }
    }

    // cost: time O(g), heap O(g), stack O(1)
    // vars: g = 대화 기록 글자 수
    // basis: estimate
    /// 맨 위에 닿아 이전 부분을 불러와야 하면 `true`.
    pub(crate) fn scroll_up(&mut self, rows: usize) -> bool {
        let total: usize = self.cells.iter().map(cell_rows).sum();
        let wanted = self.scroll_from_bottom + rows;
        self.scroll_from_bottom = wanted.min(total.saturating_sub(1));
        wanted >= total
    }

    pub(crate) fn scroll_down(&mut self, rows: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(rows);
    }

    /// 아래에서 올라온 줄 수.
    pub(crate) fn scroll_from_bottom(&self) -> usize {
        self.scroll_from_bottom
    }

    pub(crate) fn cells(&self) -> &[TranscriptCell] {
        &self.cells
    }
}

#[derive(Debug)]
pub(crate) struct TranscriptView<'a> {
    pub transcript: &'a Transcript,
    pub lang: Lang,
    pub labels_visible: bool,
}

impl TranscriptView<'_> {
    // cost: time O(g), heap O(g), stack O(1)
    // vars: g = 대화 기록 글자 수
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let rows = styled_rows(
            self.transcript.cells(),
            self.lang,
            self.labels_visible,
            false,
            area.width,
        );
        let height = usize::from(area.height);
        let end = rows
            .len()
            .saturating_sub(self.transcript.scroll_from_bottom());
        let start = end.saturating_sub(height);
        let lines: Vec<Line> = rows[start..end]
            .iter()
            .map(|(text, style)| Line::from(Span::styled(text.clone(), *style)))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }
}

/// 경과는 engine이 알린 값을 쓴다.
pub(crate) fn result_cell(task: &TaskView) -> TranscriptCell {
    if task.state == TaskState::Failed {
        TranscriptCell::Failed {
            label: Some(task.label),
            provider: task.provider,
            elapsed: task.reported_elapsed,
            cause: task.failure.clone().unwrap_or_default(),
        }
    } else {
        TranscriptCell::Result {
            label: Some(task.label),
            provider: task.provider,
            elapsed: task.reported_elapsed,
            tokens: task.tokens,
        }
    }
}

pub(crate) fn echo_cell(update: &InputUpdate) -> TranscriptCell {
    TranscriptCell::InputEcho {
        input: update.input,
        label: update.label,
        text: update.text.clone(),
        badge: delivery_badge(update.state),
    }
}

pub(crate) fn delivery_badge(state: InputState) -> Option<DeliveryBadge> {
    match state {
        InputState::Delivering => Some(DeliveryBadge::Delivering),
        InputState::Applied => Some(DeliveryBadge::Applied),
        InputState::Rejected => Some(DeliveryBadge::Rejected),
        _ => None,
    }
}

// cost: time O(c), heap O(c), stack O(1)
// vars: c = 셀 글 전체 글자 수
// basis: estimate
pub(crate) fn styled_rows(
    cells: &[TranscriptCell],
    lang: Lang,
    labels_visible: bool,
    expanded: bool,
    width: u16,
) -> Vec<(String, Style)> {
    let mut rows = Vec::new();
    for cell in cells {
        for (index, line) in cell
            .lines(lang, labels_visible, expanded)
            .iter()
            .enumerate()
        {
            let style = cell.line_style(index);
            rows.extend(
                wrap(line, usize::from(width))
                    .into_iter()
                    .map(|row| (row, style)),
            );
        }
    }
    rows
}

/// 접기 전 줄 수.
fn cell_rows(cell: &TranscriptCell) -> usize {
    cell.lines(Lang::Ko, true, false).len()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = text.len()
// basis: estimate
fn echo_lines(lang: Lang, prefix: &str, text: &str, badge: Option<DeliveryBadge>) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            if i == 0 {
                format!("> {prefix}{line}")
            } else {
                format!("  {line}")
            }
        })
        .collect();
    if lines.is_empty() {
        lines.push(format!("> {prefix}"));
    }
    if let Some(badge) = badge {
        let text = match badge {
            DeliveryBadge::Delivering => lang.tr(i18n::DELIVERING),
            DeliveryBadge::Applied => lang.tr(i18n::APPLIED),
            DeliveryBadge::Rejected => lang.tr(i18n::REJECTED),
        };
        if let Some(last) = lines.last_mut() {
            last.push_str(&format!(" · {text}"));
        }
    }
    lines
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = output.len()
// basis: estimate
fn tool_lines(
    lang: Lang,
    prefix: &str,
    activity: &Activity,
    output: &str,
    is_interrupted: bool,
    expanded: bool,
) -> Vec<String> {
    let mut lines = vec![format!("{prefix}• {}", activity_text(lang, activity))];
    if is_interrupted {
        lines.push(interrupted_line(lang));
    }
    if expanded {
        lines.extend(output.lines().map(|line| format!("  {line}")));
    }
    lines
}

/// 결과를 모르는 도구 실행 아래에 보이는 줄. plain 출력도 같은 줄을 쓴다.
pub(crate) fn interrupted_line(lang: Lang) -> String {
    format!("  {}", lang.tr(i18n::INTERRUPTED))
}

fn result_line(
    lang: Lang,
    prefix: &str,
    provider: Option<Provider>,
    elapsed: Duration,
    tail: &str,
) -> String {
    let elapsed = i18n::format_elapsed(lang, elapsed);
    match provider {
        Some(provider) => format!(
            "{prefix}{} · {elapsed} · {tail}",
            i18n::provider_name(provider)
        ),
        None => format!("{prefix}{elapsed} · {tail}"),
    }
}

pub(crate) fn tokens_text(lang: Lang, tokens: Option<u64>) -> String {
    match tokens {
        Some(tokens) => format!("{} {}", lang.tr(i18n::TOKEN), i18n::format_count(tokens)),
        None => lang.tr(i18n::TOKEN_UNREPORTED).to_string(),
    }
}

fn notice_lines(lang: Lang, prefix: &str, notice: &ChatNotice) -> Vec<String> {
    match notice {
        ChatNotice::Compacted => vec![format!("{prefix}{}", lang.tr(i18n::COMPACTED))],
        ChatNotice::ContextDeferred { constraints } => {
            let mut lines = vec![format!("{prefix}{}", lang.tr(i18n::CONTEXT_DEFERRED))];
            lines.extend(
                constraints
                    .iter()
                    .map(|constraint| format!("- {constraint}")),
            );
            lines
        }
        ChatNotice::PacketOverflow => vec![format!("{prefix}{}", lang.tr(i18n::PACKET_OVERFLOW))],
        ChatNotice::ProviderSwitched { from, to } => {
            let (from, to) = (i18n::provider_name(*from), i18n::provider_name(*to));
            vec![match lang {
                Lang::Ko => format!("{prefix}{from} → {to}{}", i18n::SWITCHED_SUFFIX),
                Lang::En => format!("{prefix}{} {from} → {to}", lang.tr(i18n::SWITCHED_SUFFIX)),
            }]
        }
        ChatNotice::ProviderRestarted { provider } => vec![format!(
            "{prefix}{}",
            lang.tr(i18n::PROVIDER_RESTARTED)
                .replace("{provider}", &i18n::provider_title(*provider))
        )],
        ChatNotice::PermissionsChanged => {
            vec![format!("{prefix}{}", lang.tr(i18n::PERMISSIONS_CHANGED))]
        }
        ChatNotice::ReadOnlyRunKept => {
            vec![format!("{prefix}{}", lang.tr(i18n::READ_ONLY_RUN_KEPT))]
        }
        ChatNotice::InterruptedSubagentReturned { provider } => vec![format!(
            "{prefix}{}",
            lang.tr(i18n::INTERRUPTED_SUBAGENT_RETURNED)
                .replace("{provider}", &i18n::provider_title(*provider))
        )],
        ChatNotice::McpUnavailable { provider, reasons } => {
            let mut lines = vec![format!(
                "{prefix}{}",
                lang.tr(i18n::MCP_UNAVAILABLE)
                    .replace("{provider}", &i18n::provider_title(*provider))
            )];
            lines.extend(reasons.iter().map(|reason| format!("- {reason}")));
            lines
        }
        ChatNotice::ResumeSuggested { held } => vec![format!(
            "{} {} · {}",
            held_labels(held),
            lang.tr(i18n::HELD_DONE),
            lang.tr(i18n::CONTINUE_HINT)
        )],
        ChatNotice::RequestSummary {
            provider_tokens,
            router_calls,
            router_tokens,
            elapsed_ms,
        } => vec![summary_line(
            lang,
            provider_tokens,
            *router_calls,
            *router_tokens,
            Duration::from_millis(*elapsed_ms),
        )],
        ChatNotice::FolderAdded {
            path,
            applies_from_next_session,
        } => {
            let mut line = format!("{prefix}{} · {path}", lang.tr(i18n::FOLDER_ADDED));
            if *applies_from_next_session {
                line.push_str(&format!(" · {}", lang.tr(i18n::FOLDER_NEXT_SESSION)));
            }
            vec![line]
        }
        ChatNotice::ConstraintAdded { rule, unconfirmed } => {
            let mut line = format!(
                "{prefix}{} · {}",
                lang.tr(i18n::CONSTRAINT_ADDED),
                one_line(rule)
            );
            if *unconfirmed {
                line.push_str(&format!(" · {}", lang.tr(i18n::CONSTRAINT_UNCONFIRMED)));
            }
            vec![line]
        }
        ChatNotice::ConstraintReleased { rule } => {
            vec![format!(
                "{prefix}{} · {}",
                lang.tr(i18n::CONSTRAINT_RELEASED),
                one_line(rule)
            )]
        }
        ChatNotice::ExtensionInstalled { .. }
        | ChatNotice::ExtensionRemoved { .. }
        | ChatNotice::ExtensionFailed { .. }
        | ChatNotice::ExtensionInjectFailed { .. }
        | ChatNotice::ExtensionPartsNotApplied { .. } => {
            extensions::notice_lines(lang, prefix, notice)
        }
        ChatNotice::Stopped { .. } | ChatNotice::StopUnconfirmed { .. } => Vec::new(),
    }
}

/// 제약 줄에 보이는 규칙의 최대 글자 수(초안). 넘으면 `…`로 줄인다.
const CONSTRAINT_LINE_CHARS: usize = 80;

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 규칙 글자 수
// basis: estimate
/// 줄바꿈과 겹친 공백을 한 칸으로 합쳐 한 줄로 만들고, 길면 줄인다.
fn one_line(rule: &str) -> String {
    let joined = rule.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= CONSTRAINT_LINE_CHARS {
        return joined;
    }
    let cut: String = joined.chars().take(CONSTRAINT_LINE_CHARS - 1).collect();
    format!("{cut}…")
}

// cost: time O(h), heap O(h), stack O(1)
// vars: h = 이름표 수
// basis: estimate
pub(crate) fn held_labels(held: &[TaskLabel]) -> String {
    held.iter()
        .map(|label| labels::format(*label))
        .collect::<Vec<_>>()
        .join(" ")
}

// cost: time O(p), heap O(p), stack O(1)
// vars: p = provider 수
// basis: estimate
fn summary_line(
    lang: Lang,
    provider_tokens: &[(Provider, u64)],
    router_calls: u32,
    router_tokens: u64,
    elapsed: Duration,
) -> String {
    let mut parts = vec![lang.tr(i18n::REQUEST_SUMMARY).to_string()];
    parts.extend(provider_tokens.iter().map(|(provider, tokens)| {
        format!(
            "{} {}",
            i18n::provider_name(*provider),
            tokens_text(lang, Some(*tokens))
        )
    }));
    let calls = match lang {
        Lang::Ko => format!("{router_calls}{}", i18n::TIMES_SUFFIX),
        Lang::En => format!("{router_calls} {}", lang.tr(i18n::TIMES_SUFFIX)),
    };
    parts.push(format!(
        "{} {calls} {}",
        lang.tr(i18n::ROUTER_CALLS),
        tokens_text(lang, Some(router_tokens))
    ));
    parts.push(i18n::format_elapsed(lang, elapsed));
    parts.join(" · ")
}

/// 단순 방식에서 말하는 쪽이 Saturn일 때의 줄 머리.
const SATURN_SPEAKER: &str = "Saturn: ";

/// 단순 방식의 선택지 한 줄. 번호는 누르는 키(`1`, `2`, `0`)이고 고른 줄에 `›`를 붙인다.
fn plain_choice(selected: bool, digit: char, text: &str) -> String {
    let marker = if selected { '›' } else { ' ' };
    format!("{marker} {digit}. {text}")
}

/// 피드백 질문을 질문 한 줄과 번호 목록으로 쓴다.
fn plain_feedback_lines(
    lang: Lang,
    label: TaskLabel,
    disposition: Disposition,
    selected: usize,
) -> Vec<String> {
    let full = feedback_line(lang, label, disposition, selected);
    let question = lang.tr(i18n::FEEDBACK_QUESTION);
    let head = full
        .split_once(question)
        .map_or(full.clone(), |(head, _)| format!("{head}{question}"));
    let choices = [
        ('1', i18n::FEEDBACK_RIGHT),
        ('2', i18n::FEEDBACK_WRONG),
        ('0', i18n::FEEDBACK_DISMISS),
    ];
    std::iter::once(head)
        .chain(
            choices.iter().enumerate().map(|(index, (digit, text))| {
                plain_choice(selected == index, *digit, lang.tr(text))
            }),
        )
        .collect()
}

/// 고른 선택지 앞에 `›`를 붙인다.
fn choice_text(selected: usize, index: usize, digit: char, text: &str) -> String {
    let marker = if selected == index { "›" } else { "" };
    format!("{marker}{digit} {text}")
}

fn feedback_line(
    lang: Lang,
    label: TaskLabel,
    disposition: Disposition,
    selected: usize,
) -> String {
    let label_text = labels::format(label);
    let head = match (disposition, lang) {
        (Disposition::Steer, Lang::Ko) => format!("{label_text}{}", i18n::FEEDBACK_STEERED),
        (Disposition::Steer, Lang::En) => {
            format!("{} {label_text}", lang.tr(i18n::FEEDBACK_STEERED))
        }
        (Disposition::NewTask, _) => format!("{label_text} {}", lang.tr(i18n::FEEDBACK_NEW_TASK)),
        (Disposition::Queue, _) => format!("{label_text} {}", lang.tr(i18n::FEEDBACK_QUEUED)),
    };
    let choices = [
        (1, i18n::FEEDBACK_RIGHT),
        (2, i18n::FEEDBACK_WRONG),
        (0, i18n::FEEDBACK_DISMISS),
    ];
    let choices: Vec<String> = choices
        .iter()
        .enumerate()
        .map(|(index, (digit, text))| {
            let digit = char::from_digit(*digit, 10).unwrap_or('0');
            choice_text(selected, index, digit, lang.tr(text))
        })
        .collect();
    format!(
        "{head} · {}  {}",
        lang.tr(i18n::FEEDBACK_QUESTION),
        choices.join("  ")
    )
}

fn train_short_line(lang: Lang, graded: u32, need: u32) -> String {
    let unit = lang.tr(i18n::COUNT_SUFFIX);
    let (graded, need) = (
        i18n::format_count(u64::from(graded)),
        i18n::format_count(u64::from(need)),
    );
    match lang {
        Lang::Ko => format!(
            "{} {graded} / {need}{unit} · {need}{unit}이 {}",
            i18n::TRAIN_SHORT,
            i18n::TRAIN_SHORT_SUFFIX
        ),
        Lang::En => format!(
            "{} {graded} / {need} · {need} {}",
            lang.tr(i18n::TRAIN_SHORT),
            lang.tr(i18n::TRAIN_SHORT_SUFFIX)
        ),
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = output.output.len()
// basis: estimate
fn shell_lines(lang: Lang, output: &ShellOutput, expanded: bool) -> Vec<String> {
    let mut lines = vec![format!("! {}", output.command)];
    let body: Vec<&str> = output.output.lines().collect();
    let shown = if expanded {
        body.len()
    } else {
        body.len().min(SHELL_PREVIEW_LINES)
    };
    lines.extend(body[..shown].iter().map(|line| format!("  {line}")));
    if shown < body.len() {
        lines.push("  …".to_string());
    }
    match output.status {
        Some(0) => {}
        Some(code) => lines.push(format!("  {} {code}", lang.tr(i18n::SHELL_EXIT))),
        None => lines.push(format!("  {} -", lang.tr(i18n::SHELL_EXIT))),
    }
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    #[test]
    fn plain_lines_name_the_speaker_on_every_line_and_saturn_speaks_for_notices() {
        let text = TranscriptCell::AgentText {
            label: Some(TaskLabel('A')),
            lines: vec!["첫 줄".to_string(), "둘째 줄".to_string()],
        };
        let failed = TranscriptCell::Failed {
            label: Some(TaskLabel('B')),
            provider: None,
            elapsed: Duration::from_secs(3),
            cause: "원인".to_string(),
        };
        let warning = TranscriptCell::Warning("경고".to_string());

        assert_eq!(
            text.plain_lines(Lang::Ko, false),
            ["[A] 첫 줄", "[A] 둘째 줄"]
        );
        assert_eq!(
            failed.plain_lines(Lang::Ko, false),
            ["[B] 3초 · 실패", "[B] 원인"]
        );
        assert_eq!(warning.plain_lines(Lang::Ko, false), ["Saturn: 경고"]);
    }

    #[test]
    fn plain_feedback_and_correction_questions_are_numbered_lists() {
        let feedback = TranscriptCell::Feedback {
            label: TaskLabel('B'),
            disposition: Disposition::NewTask,
            selected: 1,
        };
        let correction = TranscriptCell::Correction {
            label: TaskLabel('B'),
            selected: 0,
        };

        crate::view::set_plain(true);
        let feedback_lines = feedback.lines(Lang::Ko, false, false);
        let correction_lines = correction.lines(Lang::Ko, false, false);
        crate::view::set_plain(false);

        assert_eq!(
            feedback_lines,
            [
                "[B] 새 작업으로 보냄 · 판단이 맞았나요? (선택)",
                "[B]   1. 맞음",
                "[B] › 2. 틀림",
                "[B]   0. 닫기"
            ]
        );
        assert_eq!(
            correction_lines,
            [
                "[B] 바로 새 작업으로 실행할까요?",
                "[B] › 1. [실행]",
                "[B]   2. [그대로]"
            ]
        );
    }

    fn echo(text: &str) -> TranscriptCell {
        TranscriptCell::InputEcho {
            input: InputId(1),
            label: Some(TaskLabel('A')),
            text: text.to_string(),
            badge: None,
        }
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn rendered(transcript: &Transcript, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let view = TranscriptView {
            transcript,
            lang: Lang::Ko,
            labels_visible: true,
        };
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect()
    }

    #[test]
    fn lines_echo_hides_label_when_not_visible() {
        let cell = echo("버그 고쳐");

        assert_eq!(cell.lines(Lang::Ko, true, false), vec!["> [A] 버그 고쳐"]);
        assert_eq!(cell.lines(Lang::Ko, false, false), vec!["> 버그 고쳐"]);
    }

    #[test]
    fn lines_echo_appends_badge() {
        let cell = TranscriptCell::InputEcho {
            input: InputId(1),
            label: None,
            text: "a".to_string(),
            badge: Some(DeliveryBadge::Applied),
        };

        assert_eq!(cell.lines(Lang::Ko, true, false), vec!["> a · 반영됨"]);
    }

    #[test]
    fn lines_permission_notices_follow_language() {
        let restarted = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::ProviderRestarted {
                provider: Provider::from_static("codex"),
            },
        };
        let changed = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::PermissionsChanged,
        };

        assert_eq!(
            restarted.lines(Lang::Ko, true, false),
            vec!["Codex 다시 시작함 · 변경된 권한 설정을 적용했습니다"]
        );
        assert_eq!(
            restarted.lines(Lang::En, true, false),
            vec!["Restarted Codex · Applied the changed permission settings"]
        );
        assert_eq!(
            changed.lines(Lang::Ko, true, false),
            vec!["권한 설정 변경됨 · 다음 요청부터 적용됩니다"]
        );
        assert_eq!(
            changed.lines(Lang::En, true, false),
            vec!["Permission settings changed · Applies from your next request"]
        );
    }

    #[test]
    fn lines_mcp_unavailable_lists_each_reason() {
        let cell = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::McpUnavailable {
                provider: Provider::from_static("codex"),
                reasons: vec!["broken failed to start: no such file".to_string()],
            },
        };

        assert_eq!(
            cell.lines(Lang::Ko, true, false),
            vec![
                "Codex의 MCP 서버를 쓸 수 없습니다 · 그 서버의 도구만 빠지고 입력은 그대로 보냅니다",
                "- broken failed to start: no such file",
            ]
        );
        assert_eq!(
            cell.lines(Lang::En, true, false)[0],
            "Codex MCP servers unavailable · Only their tools are missing, your input is sent as is"
        );
    }

    #[test]
    fn lines_result_matches_design_text() {
        let cell = TranscriptCell::Result {
            label: Some(TaskLabel('A')),
            provider: Some(Provider::from_static("codex")),
            elapsed: Duration::from_secs(45),
            tokens: Some(3_210),
        };

        assert_eq!(
            cell.lines(Lang::Ko, true, false),
            vec!["[A] codex · 45초 · Token 3,210"]
        );
    }

    #[test]
    fn lines_failed_adds_cause_line() {
        let cell = TranscriptCell::Failed {
            label: Some(TaskLabel('A')),
            provider: Some(Provider::from_static("codex")),
            elapsed: Duration::from_secs(45),
            cause: "network".to_string(),
        };

        assert_eq!(
            cell.lines(Lang::Ko, true, false),
            vec!["[A] codex · 45초 · 실패", "network"]
        );
    }

    #[test]
    fn lines_notices_match_design_text() {
        let switched = TranscriptCell::Notice {
            label: Some(TaskLabel('A')),
            notice: ChatNotice::ProviderSwitched {
                from: Provider::from_static("codex"),
                to: Provider::from_static("claude"),
            },
        };
        let summary = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::RequestSummary {
                provider_tokens: vec![(Provider::from_static("codex"), 4_120)],
                router_calls: 3,
                router_tokens: 9_870,
                elapsed_ms: 151_000,
            },
        };

        assert_eq!(
            switched.lines(Lang::Ko, true, false),
            vec!["[A] codex → claude로 전환"]
        );
        assert_eq!(
            summary.lines(Lang::Ko, true, false),
            vec!["이번 요청 · codex Token 4,120 · 라우터 3회 Token 9,870 · 2분 31초"]
        );
    }

    #[test]
    fn lines_folder_added_mentions_the_next_session_only_when_one_is_open() {
        let added = |applies_from_next_session| TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::FolderAdded {
                path: "/shared/lib".to_owned(),
                applies_from_next_session,
            },
        };

        assert_eq!(
            added(true).lines(Lang::Ko, false, false),
            vec!["폴더 더함 · /shared/lib · 열린 session에는 다음 session부터 적용"]
        );
        assert_eq!(
            added(false).lines(Lang::Ko, false, false),
            vec!["폴더 더함 · /shared/lib"]
        );
    }

    #[test]
    fn lines_context_deferred_lists_the_constraints() {
        let deferred = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::ContextDeferred {
                constraints: vec!["never touch the vendor folder".to_owned()],
            },
        };

        assert_eq!(
            deferred.lines(Lang::Ko, false, false),
            vec![
                "고정 제약이 길어 맥락 정리를 미룹니다",
                "- never touch the vendor folder"
            ]
        );
    }

    #[test]
    fn lines_constraint_added_marks_unconfirmed_registration() {
        let added = |unconfirmed| TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::ConstraintAdded {
                rule: "에러 메시지는 영어로 통일해".to_owned(),
                unconfirmed,
            },
        };

        assert_eq!(
            added(false).lines(Lang::Ko, false, false),
            vec!["제약 등록됨 · 에러 메시지는 영어로 통일해"]
        );
        assert_eq!(
            added(true).lines(Lang::Ko, false, false),
            vec!["제약 등록됨 · 에러 메시지는 영어로 통일해 · 확인 없이"]
        );
        assert_eq!(
            added(true).lines(Lang::En, false, false),
            vec!["Constraint added · 에러 메시지는 영어로 통일해 · Without confirmation"]
        );
        let released = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::ConstraintReleased {
                rule: "에러 메시지는 영어로 통일해".to_owned(),
            },
        };
        assert_eq!(
            released.lines(Lang::Ko, false, false),
            vec!["제약 해제됨 · 에러 메시지는 영어로 통일해"]
        );
    }

    #[test]
    fn lines_constraint_rule_is_cut_to_one_line() {
        let long = format!("첫 줄\n둘째   줄 {}", "가".repeat(CONSTRAINT_LINE_CHARS));
        let added = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::ConstraintAdded {
                rule: long,
                unconfirmed: false,
            },
        };

        let lines = added.lines(Lang::Ko, false, false);

        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("제약 등록됨 · 첫 줄 둘째 줄 "));
        assert!(lines[0].ends_with('…'));
    }

    #[test]
    fn lines_packet_overflow_tells_the_user_how_to_retry() {
        let overflow = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::PacketOverflow,
        };

        assert_eq!(
            overflow.lines(Lang::Ko, false, false),
            vec!["맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요"]
        );
        assert_eq!(
            overflow.lines(Lang::En, false, false),
            vec!["Stopped over the context limit · Retry with /continue"]
        );
    }

    #[test]
    fn lines_feedback_and_needs_check_match_design_text() {
        let feedback = TranscriptCell::Feedback {
            label: TaskLabel('A'),
            disposition: Disposition::Steer,
            selected: 0,
        };
        let check = TranscriptCell::NeedsCheck {
            label: TaskLabel('A'),
        };

        assert_eq!(
            feedback.lines(Lang::Ko, false, false),
            vec!["[A]에 이어서 보냄 · 판단이 맞았나요? (선택)  ›1 맞음  2 틀림  0 닫기"]
        );
        assert_eq!(
            check.lines(Lang::Ko, false, false),
            vec!["[A] 결과 확인 필요 · /continue A"]
        );
    }

    #[test]
    fn lines_train_short_matches_design_text() {
        let cell = TranscriptCell::TrainShort {
            graded: 83,
            need: 200,
        };

        assert_eq!(
            cell.lines(Lang::Ko, true, false),
            vec!["채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다"]
        );
    }

    #[test]
    fn lines_tool_expands_only_when_asked() {
        let cell = TranscriptCell::Tool {
            label: None,
            call_id: "c1".to_string(),
            activity: Activity::ReadingFile,
            output: "line1\nline2".to_string(),
            is_interrupted: false,
        };

        assert_eq!(cell.lines(Lang::Ko, true, false), vec!["• 파일 읽는 중"]);
        assert_eq!(cell.lines(Lang::Ko, true, true).len(), 3);
    }

    #[test]
    fn interrupted_tool_shows_a_red_interrupted_line() {
        let mut transcript = Transcript::new();
        transcript.push(TranscriptCell::Tool {
            label: None,
            call_id: "c1".to_string(),
            activity: Activity::ReadingFile,
            output: String::new(),
            is_interrupted: false,
        });

        assert!(transcript.set_tool_interrupted("c1"));
        assert!(!transcript.set_tool_interrupted("missing"));

        let cell = &transcript.cells()[0];
        assert_eq!(
            cell.lines(Lang::Ko, true, false),
            vec!["• 파일 읽는 중", "  중단됨"]
        );
        assert_eq!(
            cell.lines(Lang::En, true, false),
            vec!["• Reading files", "  Interrupted"]
        );
        assert_eq!(cell.line_style(0), Style::new());
        assert_eq!(cell.line_style(1), ERROR);
    }

    #[test]
    fn rejected_input_shows_a_red_rejected_badge() {
        let mut transcript = Transcript::new();
        transcript.push(echo("a"));

        transcript.set_badge(InputId(1), DeliveryBadge::Rejected);

        let cell = &transcript.cells()[0];
        assert_eq!(cell.lines(Lang::Ko, true, false), vec!["> [A] a · 거절됨"]);
        assert_eq!(
            cell.lines(Lang::En, true, false),
            vec!["> [A] a · Rejected"]
        );
        assert_eq!(cell.line_style(0), ERROR);
        assert_eq!(
            delivery_badge(InputState::Rejected),
            Some(DeliveryBadge::Rejected)
        );
    }

    #[test]
    fn failed_cause_is_red() {
        let cell = TranscriptCell::Failed {
            label: None,
            provider: None,
            elapsed: Duration::from_secs(1),
            cause: "provider connection failed".to_string(),
        };

        assert_eq!(cell.line_style(0), Style::new());
        assert_eq!(cell.line_style(1), ERROR);
    }

    #[test]
    fn set_badge_updates_matching_echo() {
        let mut transcript = Transcript::new();
        transcript.push(echo("a"));

        transcript.set_badge(InputId(1), DeliveryBadge::Delivering);

        assert_eq!(
            transcript.cells()[0].lines(Lang::Ko, true, false),
            vec!["> [A] a · 전달 중"]
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn remove_feedback_drops_only_that_label() {
        let mut transcript = Transcript::new();
        for label in ['A', 'B'] {
            transcript.push(TranscriptCell::Feedback {
                label: TaskLabel(label),
                disposition: Disposition::Steer,
                selected: 0,
            });
        }

        transcript.remove_feedback(TaskLabel('A'));

        assert_eq!(transcript.cells().len(), 1);
    }

    #[test]
    fn scroll_up_reports_top() {
        let mut transcript = Transcript::new();
        transcript.push(echo("a"));
        transcript.push(echo("b"));

        assert!(!transcript.scroll_up(1));
        assert!(transcript.scroll_up(5));
        assert_eq!(transcript.scroll_from_bottom(), 1);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_latest_rows_at_bottom() {
        let mut transcript = Transcript::new();
        for text in ["one", "two", "three"] {
            transcript.push(echo(text));
        }

        let rows = rendered(&transcript, 20, 2);

        assert_eq!(rows, vec!["> [A] two", "> [A] three"]);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_scrolled_up_keeps_older_rows() {
        let mut transcript = Transcript::new();
        for text in ["one", "two", "three"] {
            transcript.push(echo(text));
        }
        transcript.scroll_up(1);

        let rows = rendered(&transcript, 20, 2);

        assert_eq!(rows, vec!["> [A] one", "> [A] two"]);
    }

    #[test]
    fn prepend_inserts_after_header() {
        let mut transcript = Transcript::new();
        transcript.push(echo("new"));
        transcript.set_header(StartInfo {
            saturn_version: "0.1.0".to_string(),
            providers: Vec::new(),
            router: None,
            router_version: None,
            folder: "/w".into(),
            added_dirs: Vec::new(),
        });

        transcript.prepend(vec![echo("old")]);

        assert!(matches!(transcript.cells()[0], TranscriptCell::Header(_)));
        assert_eq!(transcript.cells()[1], echo("old"));
    }
}
