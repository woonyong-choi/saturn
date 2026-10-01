//! 대화 기록 영역. 입력 에코, 도구 셀, 결과 줄, 한 줄 알림, 피드백 질문, 이번 요청 합계를 쌓는다.
//!
//! 설계: docs/design/tui.md(영역 대화 기록, 피드백 질문, 상태 표시).
//! 갱신 시점: 판단 확정(에코), 작업 종료(작업별 출력 칸 내용과 결과 머리줄), 다시 실행할 때 기록 저장소에서 최근 부분부터 로드,
//! 위로 스크롤할 때 이전 부분 로드(`Request::LoadHistory` → `Notification::HistoryChunk`).
//! 각 셀의 글은 `TranscriptCell::lines`가 만들고 plain 출력도 같은 함수를 쓴다.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskLabel};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{Disposition, InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::shell::ShellOutput;
use crate::state::{InputUpdate, TaskView};
use crate::view::start_screen::StartInfo;
use crate::view::status_board::activity_text;
use crate::view::{MUTED, wrap};

/// 셸 명령 셀이 줄인 형태에서 보이는 출력 줄 수. 초안 값이다(docs/design/tui.md 초안 값).
pub const SHELL_PREVIEW_LINES: usize = 10;

/// 입력 에코 뒤에 붙는 전달 상태 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryBadge {
    /// `전달 중` 에이전트로 전달 중, 취소 불가.
    Delivering,
    /// `반영됨` 전달 완료, 취소 불가.
    Applied,
}

/// 대화 기록 한 칸.
#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptCell {
    /// 시작 화면이 첫 결과 뒤 바뀐 맨 위 머리 셀.
    Header(StartInfo),
    /// 판단이 끝난 입력의 에코 `> [A] 원문`. 끼워 넣은 입력은 합쳐진 작업의 이름표.
    InputEcho {
        input: InputId,
        label: Option<TaskLabel>,
        text: String,
        badge: Option<DeliveryBadge>,
    },
    /// 모델 글. 작업이 끝날 때 작업별 출력 칸에서 옮겨 온다.
    AgentText {
        label: Option<TaskLabel>,
        lines: Vec<String>,
    },
    /// 도구 셀. 기본은 한 줄로 줄이고 전체 기록(`Ctrl+T`)에서 펼친다. `call_id`로 결과를 짝짓는다.
    Tool {
        label: Option<TaskLabel>,
        call_id: String,
        activity: Activity,
        output: String,
    },
    /// 결과 머리줄 `[A] codex · 45초 · Token 3,210`. 토큰 보고 전이면 `Token -`.
    Result {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        tokens: Option<u64>,
    },
    /// 실패 `[A] codex · 45초 · 실패`, 다음 줄에 원인 한 줄.
    Failed {
        label: Option<TaskLabel>,
        provider: Option<Provider>,
        elapsed: Duration,
        cause: String,
    },
    /// 결과 불명 `[A] 결과 확인 필요 · /continue A`. 보류 줄과 함께 보인다. 이름표는 늘 붙인다.
    NeedsCheck { label: TaskLabel },
    /// `ChatNotice` 한 줄: `[A] 맥락 정리 후 이어서 진행`, `[A] codex → claude로 전환`,
    /// `이번 요청 · codex Token 4,120 · 판단기 3회 Token 9,870 · 2분 31초`, 크래시 뒤 보류 `[A] [C] 보류됨 · /continue 로 이어서`.
    /// `Stopped`, `StopUnconfirmed`는 상태판에 그리고 여기에는 넣지 않는다.
    Notice {
        label: Option<TaskLabel>,
        notice: ChatNotice,
    },
    /// 피드백 질문 `[A]에 이어서 보냈어요 · 판단이 맞았나요? (선택)  1 맞아요  2 아니에요  0 닫기`. 입력 에코 다음 줄에 뜬다.
    /// 끼워 넣기가 아닌 판단의 머리 문구는 초안이다(`[B] 새 작업으로 보냈어요`, `[C] 대기열에 넣었어요`).
    Feedback {
        label: TaskLabel,
        disposition: Disposition,
    },
    /// 바로잡기 제안 `[B] 바로 새 작업으로 실행할까요? [실행] [그대로]`. `2` 답 뒤 입력이 아직 보내지지 않았을 때.
    /// TODO(#109): `[실행]`(`Request::RunAsNewTask`)과 `[그대로]`를 고르는 키나 클릭. 지금은 그리기만 한다
    Correction { label: TaskLabel },
    /// 보류 종료 완료 `[E] 보류를 닫았습니다`.
    HeldClosed { label: TaskLabel },
    /// `/train` 불가 `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`.
    TrainShort { graded: u32, need: u32 },
    /// `!` 셸 명령 결과.
    Shell(ShellOutput),
    /// 한 줄 경고(명령 해석 오류 등). 원문 그대로.
    Warning(String),
}

impl TranscriptCell {
    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 셀 글자 수
    // basis: estimate
    /// 셀의 글 줄. `labels_visible`이 거짓이면 이름표를 빼고, `expanded`면 도구 셀 전체를 펼친다.
    /// 전체 화면과 plain 출력이 같은 결과를 내도록 문구는 모두 여기서 만든다.
    pub fn lines(&self, lang: Lang, labels_visible: bool, expanded: bool) -> Vec<String> {
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
                ..
            } => tool_lines(lang, &prefix(*label), activity, output, expanded),
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
            Self::Feedback { label, disposition } => {
                vec![feedback_line(lang, *label, *disposition)]
            }
            Self::Correction { label } => vec![format!(
                "{} {} {} {}",
                labels::format(*label),
                lang.tr(i18n::CORRECTION_QUESTION),
                lang.tr(i18n::BUTTON_RUN),
                lang.tr(i18n::BUTTON_KEEP)
            )],
            Self::HeldClosed { label } => vec![format!(
                "{} {}",
                labels::format(*label),
                lang.tr(i18n::HELD_CLOSED)
            )],
            Self::TrainShort { graded, need } => vec![train_short_line(lang, *graded, *need)],
            Self::Shell(output) => shell_lines(lang, output, expanded),
            Self::Warning(text) => vec![text.clone()],
        }
    }

    /// 줄 `index`의 글자 속성. 실패 원인 줄은 흐리게.
    fn line_style(&self, index: usize) -> Style {
        match self {
            Self::Failed { .. } if index > 0 => MUTED,
            Self::Tool { .. } | Self::Shell(_) if index > 0 => MUTED,
            _ => Style::new(),
        }
    }
}

/// 대화 기록 모음과 스크롤 위치.
#[derive(Debug, Default)]
pub struct Transcript {
    cells: Vec<TranscriptCell>,
    scroll_from_bottom: usize,
}

impl Transcript {
    /// 빈 기록.
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 셀 글자 수(위로 스크롤 중일 때만 줄 수를 센다)
    // basis: estimate
    /// 셀을 끝에 더한다. 맨 아래를 보고 있으면 계속 맨 아래를 따라가고, 위를 보고 있으면 보던 자리를 지킨다.
    pub fn push(&mut self, cell: TranscriptCell) {
        if self.scroll_from_bottom > 0 {
            self.scroll_from_bottom += cell_rows(&cell);
        }
        self.cells.push(cell);
    }

    /// 이전 부분(`HistoryChunk`)을 머리 셀 뒤, 기존 셀 앞에 넣는다. 보던 자리는 아래 기준이라 그대로다.
    pub fn prepend(&mut self, cells: Vec<TranscriptCell>) {
        let at = usize::from(matches!(
            self.cells.first(),
            Some(TranscriptCell::Header(_))
        ));
        self.cells.splice(at..at, cells);
    }

    /// 맨 위 머리 셀을 넣거나 바꾼다(시작 화면 전환).
    pub fn set_header(&mut self, info: StartInfo) {
        match self.cells.first_mut() {
            Some(TranscriptCell::Header(header)) => *header = info,
            _ => self.cells.insert(0, TranscriptCell::Header(info)),
        }
    }

    /// 떠 있는 피드백 질문 셀을 지운다(답, `0`, 8초 경과).
    pub fn remove_feedback(&mut self, label: TaskLabel) {
        self.cells.retain(
            |cell| !matches!(cell, TranscriptCell::Feedback { label: shown, .. } if *shown == label),
        );
    }

    // cost: time O(c), heap O(1), stack O(1)
    // vars: c = 셀 수
    // basis: estimate
    /// 입력 에코의 전달 상태 표시를 바꾼다. 에코가 없으면 아무것도 하지 않는다.
    pub fn set_badge(&mut self, input: InputId, badge: DeliveryBadge) {
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
    /// 도구 결과를 같은 `call_id`의 도구 셀에 붙인다. 셀이 없으면 거짓.
    pub fn set_tool_output(&mut self, call_id: &str, text: String) -> bool {
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

    // cost: time O(g), heap O(g), stack O(1)
    // vars: g = 대화 기록 글자 수
    // basis: estimate
    /// 위로 `rows`줄 스크롤. 맨 위에 닿으면 이전 부분 로드가 필요하다고 참을 돌려준다.
    pub fn scroll_up(&mut self, rows: usize) -> bool {
        let total: usize = self.cells.iter().map(cell_rows).sum();
        let wanted = self.scroll_from_bottom + rows;
        self.scroll_from_bottom = wanted.min(total.saturating_sub(1));
        wanted >= total
    }

    /// 아래로 `rows`줄 스크롤.
    pub fn scroll_down(&mut self, rows: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(rows);
    }

    /// 아래에서 올라온 줄 수.
    pub fn scroll_from_bottom(&self) -> usize {
        self.scroll_from_bottom
    }

    /// 모든 셀.
    pub fn cells(&self) -> &[TranscriptCell] {
        &self.cells
    }
}

/// 대화 기록 그리기.
#[derive(Debug)]
pub struct TranscriptView<'a> {
    /// 그릴 기록.
    pub transcript: &'a Transcript,
    /// 화면 언어.
    pub lang: Lang,
    /// 이름표를 보일지.
    pub labels_visible: bool,
}

impl TranscriptView<'_> {
    // cost: time O(g), heap O(g), stack O(1)
    // vars: g = 대화 기록 글자 수
    // basis: estimate
    /// 아래에서부터 칸 높이만큼 셀 줄을 그린다(도구 셀은 줄인 형태). 에코는 `>` 접두, 실패 원인과 도구 출력은 흐리게.
    /// 칸보다 긴 줄은 접어 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
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

/// 작업이 끝났을 때의 결과 머리줄 셀. 실패면 원인 줄이 붙는 `Failed`. 경과는 engine이 알린 값을 쓴다.
pub fn result_cell(task: &TaskView) -> TranscriptCell {
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

/// 입력 에코 셀. 전달 중이거나 반영된 입력이면 표시를 붙인다.
pub fn echo_cell(update: &InputUpdate) -> TranscriptCell {
    TranscriptCell::InputEcho {
        input: update.input,
        label: update.label,
        text: update.text.clone(),
        badge: delivery_badge(update.state),
    }
}

/// 전달 상태의 표시. `Delivering`, `Applied`만 있다.
pub fn delivery_badge(state: InputState) -> Option<DeliveryBadge> {
    match state {
        InputState::Delivering => Some(DeliveryBadge::Delivering),
        InputState::Applied => Some(DeliveryBadge::Applied),
        _ => None,
    }
}

// cost: time O(c), heap O(c), stack O(1)
// vars: c = 셀 글 전체 글자 수
// basis: estimate
/// 셀들을 폭 `width`로 접은 줄과 속성.
pub fn styled_rows(
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

/// 스크롤 계산에 쓰는 셀 줄 수(접기 전).
fn cell_rows(cell: &TranscriptCell) -> usize {
    cell.lines(Lang::Ko, true, false).len()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = text.len()
// basis: estimate
/// 에코 줄. 첫 줄 `> [A] 원문`, 이어지는 줄은 두 칸 들여 쓰고 전달 상태는 끝 줄 뒤에 붙인다.
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
/// 도구 셀. 줄인 형태는 `• 하는 일` 한 줄, 펼치면 출력 줄을 두 칸 들여 붙인다.
fn tool_lines(
    lang: Lang,
    prefix: &str,
    activity: &Activity,
    output: &str,
    expanded: bool,
) -> Vec<String> {
    let mut lines = vec![format!("{prefix}• {}", activity_text(lang, activity))];
    if expanded {
        lines.extend(output.lines().map(|line| format!("  {line}")));
    }
    lines
}

/// 결과 머리줄 `[A] codex · 45초 · 끝 칸`. provider를 모르면 그 칸을 뺀다.
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

/// 토큰 칸 `Token 3,210`, 보고 전이면 `Token -`.
pub fn tokens_text(lang: Lang, tokens: Option<u64>) -> String {
    match tokens {
        Some(tokens) => format!("{} {}", lang.tr(i18n::TOKEN), i18n::format_count(tokens)),
        None => lang.tr(i18n::TOKEN_UNREPORTED).to_string(),
    }
}

/// 한 줄 알림.
fn notice_lines(lang: Lang, prefix: &str, notice: &ChatNotice) -> Vec<String> {
    match notice {
        ChatNotice::Compacted => vec![format!("{prefix}{}", lang.tr(i18n::COMPACTED))],
        ChatNotice::ProviderSwitched { from, to } => {
            let (from, to) = (i18n::provider_name(*from), i18n::provider_name(*to));
            vec![match lang {
                Lang::Ko => format!("{prefix}{from} → {to}{}", i18n::SWITCHED_SUFFIX),
                Lang::En => format!("{prefix}{} {from} → {to}", lang.tr(i18n::SWITCHED_SUFFIX)),
            }]
        }
        ChatNotice::ResumeSuggested { held } => vec![format!(
            "{} {} · {}",
            held_labels(held),
            lang.tr(i18n::HELD_DONE),
            lang.tr(i18n::CONTINUE_HINT)
        )],
        ChatNotice::RequestSummary {
            provider_tokens,
            judge_calls,
            judge_tokens,
            elapsed_ms,
        } => vec![summary_line(
            lang,
            provider_tokens,
            *judge_calls,
            *judge_tokens,
            Duration::from_millis(*elapsed_ms),
        )],
        ChatNotice::Stopped { .. } | ChatNotice::StopUnconfirmed { .. } => Vec::new(),
    }
}

// cost: time O(h), heap O(h), stack O(1)
// vars: h = 이름표 수
// basis: estimate
/// 이름표 목록 `[A] [C]`.
pub fn held_labels(held: &[TaskLabel]) -> String {
    held.iter()
        .map(|label| labels::format(*label))
        .collect::<Vec<_>>()
        .join(" ")
}

// cost: time O(p), heap O(p), stack O(1)
// vars: p = provider 수
// basis: estimate
/// 이번 요청 합계 `이번 요청 · codex Token 4,120 · 판단기 3회 Token 9,870 · 2분 31초`.
fn summary_line(
    lang: Lang,
    provider_tokens: &[(Provider, u64)],
    judge_calls: u32,
    judge_tokens: u64,
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
        Lang::Ko => format!("{judge_calls}{}", i18n::TIMES_SUFFIX),
        Lang::En => format!("{judge_calls} {}", lang.tr(i18n::TIMES_SUFFIX)),
    };
    parts.push(format!(
        "{} {calls} {}",
        lang.tr(i18n::JUDGE_CALLS),
        tokens_text(lang, Some(judge_tokens))
    ));
    parts.push(i18n::format_elapsed(lang, elapsed));
    parts.join(" · ")
}

/// 피드백 질문 줄.
fn feedback_line(lang: Lang, label: TaskLabel, disposition: Disposition) -> String {
    let label_text = labels::format(label);
    let head = match (disposition, lang) {
        (Disposition::Steer, Lang::Ko) => format!("{label_text}{}", i18n::FEEDBACK_STEERED),
        (Disposition::Steer, Lang::En) => {
            format!("{} {label_text}", lang.tr(i18n::FEEDBACK_STEERED))
        }
        (Disposition::NewTask, _) => format!("{label_text} {}", lang.tr(i18n::FEEDBACK_NEW_TASK)),
        (Disposition::Queue, _) => format!("{label_text} {}", lang.tr(i18n::FEEDBACK_QUEUED)),
    };
    format!(
        "{head} · {}  {}",
        lang.tr(i18n::FEEDBACK_QUESTION),
        lang.tr(i18n::FEEDBACK_CHOICES)
    )
}

/// `/train` 불가 줄.
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
/// 셸 명령 셀. `! 명령`, 출력(줄인 형태는 앞 `SHELL_PREVIEW_LINES`줄과 `…`), 0이 아닌 종료 코드.
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
    fn lines_result_matches_design_text() {
        let cell = TranscriptCell::Result {
            label: Some(TaskLabel('A')),
            provider: Some(Provider::Codex),
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
            provider: Some(Provider::Codex),
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
                from: Provider::Codex,
                to: Provider::Claude,
            },
        };
        let summary = TranscriptCell::Notice {
            label: None,
            notice: ChatNotice::RequestSummary {
                provider_tokens: vec![(Provider::Codex, 4_120)],
                judge_calls: 3,
                judge_tokens: 9_870,
                elapsed_ms: 151_000,
            },
        };

        assert_eq!(
            switched.lines(Lang::Ko, true, false),
            vec!["[A] codex → claude로 전환"]
        );
        assert_eq!(
            summary.lines(Lang::Ko, true, false),
            vec!["이번 요청 · codex Token 4,120 · 판단기 3회 Token 9,870 · 2분 31초"]
        );
    }

    #[test]
    fn lines_feedback_and_needs_check_match_design_text() {
        let feedback = TranscriptCell::Feedback {
            label: TaskLabel('A'),
            disposition: Disposition::Steer,
        };
        let check = TranscriptCell::NeedsCheck {
            label: TaskLabel('A'),
        };

        assert_eq!(
            feedback.lines(Lang::Ko, false, false),
            vec!["[A]에 이어서 보냈어요 · 판단이 맞았나요? (선택)  1 맞아요  2 아니에요  0 닫기"]
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
        };

        assert_eq!(cell.lines(Lang::Ko, true, false), vec!["• 파일 읽는 중"]);
        assert_eq!(cell.lines(Lang::Ko, true, true).len(), 3);
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
            judge: None,
            judge_version: None,
            folder: "/w".into(),
        });

        transcript.prepend(vec![echo("old")]);

        assert!(matches!(transcript.cells()[0], TranscriptCell::Header(_)));
        assert_eq!(transcript.cells()[1], echo("old"));
    }
}
