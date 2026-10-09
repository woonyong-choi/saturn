//! 상태판. 줄은 매 프레임 `state::ChatState`에서 새로 만든다.
//! 설계: docs/design/tui.md

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use saturn_protocol::event::Activity;
use saturn_protocol::ids::{InputId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::{Alert, SettingsFault, SettingsLayer, SettingsWarning};
use saturn_protocol::state::QueueReason;

use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use saturn_protocol::state::{InputState, TaskState};

use crate::i18n::{self, Lang};
use crate::labels;
use crate::state::{
    APPROVAL_PENDING_AFTER, ChatState, InputView, JUDGING_SHOW_AFTER, NO_RESPONSE_AFTER, TaskView,
};
use crate::view::transcript::{held_labels, tokens_text};
use crate::view::{EMPHASIS, MUTED, NARROW_WIDTH, text_width, truncate, wrap};

pub(crate) const COMMAND_PREVIEW_COLS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningLine {
    pub task: TaskId,
    pub label: TaskLabel,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub elapsed: Duration,
    pub activity: Option<Activity>,
    /// 마지막 도구 호출의 세부. 줄 안에 넣지 않고 아래에 한 단계 들여 보인다.
    pub detail: Vec<String>,
    pub detail_open: bool,
    /// 보고 전이면 `None`.
    pub tokens: Option<u64>,
    /// `activity`보다 앞선다.
    pub awaiting_permission: bool,
    /// `awaiting_permission` 다음으로 앞선다.
    pub awaiting_input: bool,
    /// 마지막 provider 이벤트 뒤 `NO_RESPONSE_AFTER` 이상 조용했을 때의 분. `awaiting_input` 다음으로 앞선다.
    pub no_response_minutes: Option<u64>,
    /// 도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 없다. `awaiting_permission` 다음으로 앞선다.
    pub approval_pending: bool,
    /// 0이 아니면 하는 일 대신 보인다.
    pub subagents: usize,
    pub has_output: bool,
}

/// 세부 줄의 머리와 이어지는 줄의 들여쓰기.
const DETAIL_HEAD: &str = "  └ ";
const DETAIL_CONT: &str = "    ";
/// 펼친 세부가 차지하는 최대 줄 수. 넘으면 마지막 줄을 `…`로 닫는다.
pub(crate) const DETAIL_MAX_ROWS: usize = 8;

impl RunningLine {
    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 세부 글자 수
    // basis: estimate
    /// 줄 아래에 그리는 세부. 접힌 세부는 첫 줄 하나이고 길면 끝을 `…`로 줄인다. 펼치면 모든 줄을 폭에 맞게 접는다.
    /// 세부가 없거나 폭이 0이면 비어 있다.
    pub(crate) fn detail_rows(&self, width: usize) -> Vec<String> {
        let Some(first) = self.detail.first() else {
            return Vec::new();
        };
        if width == 0 {
            return Vec::new();
        }
        if !self.detail_open {
            let more = if self.detail.len() > 1 { " …" } else { "" };
            return vec![truncate(&format!("{DETAIL_HEAD}{first}{more}"), width)];
        }
        let room = width.saturating_sub(text_width(DETAIL_HEAD)).max(1);
        let mut rows: Vec<String> = Vec::new();
        for line in &self.detail {
            for piece in wrap(line, room) {
                let head = if rows.is_empty() {
                    DETAIL_HEAD
                } else {
                    DETAIL_CONT
                };
                rows.push(truncate(&format!("{head}{piece}"), width));
            }
        }
        if rows.len() > DETAIL_MAX_ROWS {
            rows.truncate(DETAIL_MAX_ROWS);
            rows[DETAIL_MAX_ROWS - 1] = truncate(&format!("{DETAIL_CONT}…"), width);
        }
        rows
    }

    /// 접힌 세부가 줄었거나 더 있어서 펼칠 것이 있다.
    pub(crate) fn detail_expandable(&self, width: usize) -> bool {
        let Some(first) = self.detail.first() else {
            return false;
        };
        self.detail.len() > 1 || text_width(DETAIL_HEAD) + text_width(first) > width
    }

    /// 출력도 하는 일도 아직 없으면 `작업 중`만 보인다.
    fn is_bare(&self) -> bool {
        !self.has_output && self.doing(Lang::Ko).is_none()
    }

    /// 칸 `하는 일`. 허가, 입력, 무응답, 허가 준비, 하위 에이전트, 도구 순으로 앞선다.
    fn doing(&self, lang: Lang) -> Option<String> {
        if self.awaiting_permission {
            Some(lang.tr(i18n::AWAITING_PERMISSION).to_string())
        } else if self.awaiting_input {
            Some(lang.tr(i18n::AWAITING_INPUT).to_string())
        } else if let Some(minutes) = self.no_response_minutes {
            Some(
                lang.tr(i18n::NO_RESPONSE)
                    .replace("{minutes}", &minutes.to_string()),
            )
        } else if self.approval_pending {
            Some(approval_pending_text(lang, self.provider))
        } else if self.subagents > 0 {
            Some(subagents_text(lang, self.subagents))
        } else {
            self.activity.as_ref().map(|a| activity_label(lang, a))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatusLine {
    /// 살아 있는 작업마다 한 줄.
    Running(RunningLine),
    /// 판단이 `JUDGING_SHOW_AFTER`를 넘기면 그리고, 한 번 그렸으면 `JUDGING_MIN_SHOWN`은 그린다.
    Judging {
        input: InputId,
        label: Option<TaskLabel>,
        text: Option<String>,
    },
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
        self.text_at(lang, labels_visible, spinner, u16::MAX)
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 줄 글자 수
    // basis: estimate
    /// `width`가 `NARROW_WIDTH` 미만이면 실행 줄에서 토큰과 모델 이름부터 뺀다.
    pub(crate) fn text_at(
        &self,
        lang: Lang,
        labels_visible: bool,
        spinner: char,
        width: u16,
    ) -> String {
        let prefix = |label: Option<TaskLabel>| labels::prefix(label, labels_visible);
        match self {
            Self::Running(line) => {
                // 이름표와 칸 사이는 두 칸이다. 칸이 하나뿐인 `작업 중`은 한 칸이다
                let named = prefix(Some(line.label));
                let gap = if line.is_bare() || named.is_empty() {
                    ""
                } else {
                    " "
                };
                let compact = width < NARROW_WIDTH;
                format!(
                    "{spinner} {named}{gap}{}",
                    running_text(lang, line, compact)
                )
            }
            Self::Judging { label, text, .. } => with_text(
                format!("{spinner} {}{}", prefix(*label), lang.tr(i18n::JUDGING)),
                text.as_deref(),
            ),
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
    lines.extend(judging_lines(state, &inputs, now));
    lines.extend(inputs.iter().filter_map(|input| queued_line(input)));
    lines.extend(held_lines(state, &tasks, &inputs));
    lines.extend(alert_lines(state));
    lines
}

/// 상태판 줄의 종류. 나머지 개수를 종류별로 센다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    Running,
    Judging,
    Queued,
    Held,
    Alert,
}

impl LineKind {
    /// 나머지 개수를 붙이는 순서다.
    const ORDER: [Self; 5] = [
        Self::Running,
        Self::Judging,
        Self::Queued,
        Self::Held,
        Self::Alert,
    ];

    /// 보이는 줄과 같은 종류면 `개 더`를 붙인 문구다.
    fn count_text(self, lang: Lang, count: usize, same_kind: bool) -> String {
        let key = match (self, same_kind) {
            (Self::Running, true) => i18n::BOARD_MORE_RUNNING,
            (Self::Running, false) => i18n::BOARD_RUNNING,
            (Self::Judging, true) => i18n::BOARD_MORE_JUDGING,
            (Self::Judging, false) => i18n::BOARD_JUDGING,
            (Self::Queued, true) => i18n::BOARD_MORE_QUEUED,
            (Self::Queued, false) => i18n::BOARD_QUEUED,
            (Self::Held, true) => i18n::BOARD_MORE_HELD,
            (Self::Held, false) => i18n::BOARD_HELD,
            (Self::Alert, true) => i18n::BOARD_MORE_ALERTS,
            (Self::Alert, false) => i18n::BOARD_ALERTS,
        };
        lang.tr(key).replace("{n}", &count.to_string())
    }
}

impl StatusLine {
    fn kind(&self) -> LineKind {
        match self {
            Self::Running(_) => LineKind::Running,
            Self::Judging { .. } => LineKind::Judging,
            Self::Queued { .. } => LineKind::Queued,
            Self::Stopped { .. }
            | Self::HeldTask { .. }
            | Self::HeldInput { .. }
            | Self::CloseHeldConfirm { .. } => LineKind::Held,
            Self::Alert(_)
            | Self::StopUnconfirmed { .. }
            | Self::RouterUnavailableSend
            | Self::Settings { .. } => LineKind::Alert,
        }
    }
}

/// 상태판은 줄 하나만 둔다. 지금 보이는 줄과, 나머지를 종류별로 센 개수다. 전체는 작업 목록에서 본다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Board {
    pub line: StatusLine,
    /// 보이는 줄을 뺀 종류별 개수. 0인 종류는 싣지 않는다.
    pub others: Vec<(LineKind, usize)>,
}

impl Board {
    fn running(&self) -> Option<&RunningLine> {
        match &self.line {
            StatusLine::Running(line) => Some(line),
            _ => None,
        }
    }

    /// 실행 줄 아래에 그리는 세부 줄.
    pub(crate) fn detail_rows(&self, width: u16) -> Vec<String> {
        self.running()
            .map_or_else(Vec::new, |line| line.detail_rows(usize::from(width)))
    }

    /// 세부가 있는 실행 줄의 작업. 세부 줄을 누르거나 `Enter`로 접고 펼친다.
    pub(crate) fn detail_task(&self) -> Option<TaskId> {
        self.running()
            .filter(|line| !line.detail.is_empty())
            .map(|line| line.task)
    }

    /// 이 폭에서 접고 펼칠 것이 있다.
    pub(crate) fn detail_expandable(&self, width: u16) -> bool {
        self.running()
            .is_some_and(|line| line.detail_open || line.detail_expandable(usize::from(width)))
    }

    /// 줄 하나, 그 아래 세부 줄, 그 아래 그 줄의 버튼이 세로 목록으로 붙는다.
    pub(crate) fn height(&self, width: u16) -> u16 {
        let detail = self.detail_rows(width).len();
        let rows = 1 + detail + self.line.buttons().len();
        u16::try_from(rows).unwrap_or(u16::MAX)
    }

    pub(crate) fn buttons(&self) -> Vec<Button> {
        self.line.buttons()
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 줄 글자 수
    // basis: estimate
    /// 줄 글 뒤에 나머지 개수를 ` · 실행 2개 더 · 대기 3`처럼 붙인다.
    pub(crate) fn text(
        &self,
        lang: Lang,
        labels_visible: bool,
        spinner: char,
        width: u16,
    ) -> String {
        let mut text = self.line.text_at(lang, labels_visible, spinner, width);
        let shown = self.line.kind();
        for (kind, count) in &self.others {
            text.push_str(" · ");
            text.push_str(&kind.count_text(lang, *count, *kind == shown));
        }
        text
    }
}

// cost: time O(t log t + i log i + a), heap O(t + i + a), stack O(1)
// vars: t = 작업 수, i = 입력 수, a = 알림 수
// basis: estimate
pub(crate) fn board(state: &ChatState, now: Instant) -> Option<Board> {
    pick(build(state, now))
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 줄 수
// basis: estimate
/// 보류 닫기 확인이 있으면 그 줄이, 없으면 줄 순서의 맨 앞이 보인다. `멈춤` 줄은 보류 줄을 이미 이름으로 싣고 있어 보류 개수를 따로 세지 않는다.
pub(crate) fn pick(lines: Vec<StatusLine>) -> Option<Board> {
    let index = lines
        .iter()
        .position(|line| matches!(line, StatusLine::CloseHeldConfirm { .. }))
        .unwrap_or(0);
    let line = lines.get(index)?.clone();
    let hides_held = matches!(line, StatusLine::Stopped { .. });
    let others = LineKind::ORDER
        .into_iter()
        .filter_map(|kind| {
            let count = lines
                .iter()
                .enumerate()
                .filter(|(at, other)| *at != index && other.kind() == kind)
                .filter(|(_, other)| !matches!(other, StatusLine::Stopped { .. }))
                .count();
            let hidden = hides_held && kind == LineKind::Held;
            (count > 0 && !hidden).then_some((kind, count))
        })
        .collect();
    Some(Board { line, others })
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
        QueueReason::ConfirmStop => i18n::CONFIRM_STOP,
    };
    lang.tr(key).to_string()
}

/// 실행 줄의 하는 일. 명령 같은 세부는 줄 아래에 보이므로 넣지 않는다.
pub(crate) fn activity_label(lang: Lang, activity: &Activity) -> String {
    let key = match activity {
        Activity::Thinking => i18n::THINKING,
        Activity::ReadingFile => i18n::READING_FILE,
        Activity::EditingFile => i18n::EDITING_FILE,
        Activity::RunningCommand { .. } => i18n::RUNNING_COMMAND,
        Activity::Compacting => i18n::COMPACTING,
        Activity::SwitchingProvider => i18n::SWITCHING_PROVIDER,
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
        SettingsWarning::RenamedKeys { keys } => {
            let pairs: Vec<String> = keys
                .iter()
                .map(|(old, new)| format!("{old} → {new}"))
                .collect();
            lang.tr(i18n::SETTINGS_RENAMED)
                .replace("{keys}", &pairs.join(", "))
        }
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
        Alert::InputNotRecorded => i18n::INPUT_NOT_RECORDED,
        Alert::PruneNeedsRetention => i18n::CLI_PRUNE_NO_RETENTION,
        Alert::SchemaMigrated { to, .. } => {
            return lang
                .tr(i18n::SCHEMA_MIGRATED)
                .replace("{to}", &to.to_string());
        }
        Alert::ProviderUpdated { provider, from, to } => {
            return lang
                .tr(i18n::PROVIDER_UPDATED)
                .replace("{provider}", i18n::provider_name(*provider))
                .replace("{from}", from)
                .replace("{to}", to);
        }
        Alert::AutoPruned { chats, .. } => {
            return lang
                .tr(i18n::AUTO_PRUNED)
                .replace("{chats}", &chats.to_string());
        }
        Alert::AutoPruneFailed => i18n::AUTO_PRUNE_FAILED,
        Alert::EngineRestarted => i18n::ENGINE_RESTARTED,
        Alert::EngineRestarting => i18n::ENGINE_RESTARTING,
    };
    lang.tr(key).to_string()
}

/// 버튼은 줄 아래에 한 단계 들여 세로 목록으로 둔다. 폭이 좁아도 같다. 그리기와 마우스 클릭 판정이 같은 계산을 쓴다.
pub(crate) fn button_rects(board: Option<&Board>, lang: Lang, area: Rect) -> Vec<(Rect, Button)> {
    let Some(board) = board else {
        return Vec::new();
    };
    let indent = BUTTON_INDENT.min(area.width);
    let first = 1 + u16::try_from(board.detail_rows(area.width).len()).unwrap_or(u16::MAX - 1);
    board
        .buttons()
        .into_iter()
        .zip(first..)
        .map(|(button, row)| (button, area.y.saturating_add(row)))
        .filter(|(_, y)| *y < area.bottom())
        .map(|(button, y)| {
            let width = (text_width(button.text(lang)) as u16).min(area.width - indent);
            (Rect::new(area.x + indent, y, width, 1), button)
        })
        .collect()
}

// cost: time O(1), heap O(w), stack O(1)
// vars: w = 칸 폭
// basis: estimate
/// 세부 줄이 놓인 칸과 그 작업. 눌러서 접고 펼칠 때 클릭 판정이 쓴다.
pub(crate) fn detail_rect(board: Option<&Board>, area: Rect) -> Option<(Rect, TaskId)> {
    let board = board?;
    let task = board.detail_task()?;
    let rows = u16::try_from(board.detail_rows(area.width).len()).ok()?;
    let rect = Rect::new(area.x, area.y.saturating_add(1), area.width, rows).intersection(area);
    (rect.height > 0).then_some((rect, task))
}

/// 버튼 목록을 줄보다 들여 쓰는 칸 수.
const BUTTON_INDENT: u16 = 2;

#[derive(Debug)]
pub(crate) struct StatusBoardView<'a> {
    pub board: Option<&'a Board>,
    pub lang: Lang,
    pub labels_visible: bool,
    pub spinner: char,
    /// 상태판 버튼 고르기에서 고른 버튼. `›`를 붙이고 반전해서 그린다.
    pub focus: Option<Button>,
}

impl StatusBoardView<'_> {
    // cost: time O(w), heap O(w), stack O(1)
    // vars: w = 칸 폭
    // basis: estimate
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let Some(board) = self.board else {
            return;
        };
        let width = usize::from(area.width);
        let text = board.text(self.lang, self.labels_visible, self.spinner, area.width);
        let head = Rect::new(area.x, area.y, area.width, area.height.min(1));
        frame.render_widget(Paragraph::new(Line::from(truncate(&text, width))), head);
        if let Some((rect, _)) = detail_rect(Some(board), area) {
            let rows: Vec<Line> = board
                .detail_rows(area.width)
                .into_iter()
                .map(|row| Line::from(Span::styled(row, MUTED)))
                .collect();
            frame.render_widget(Paragraph::new(rows), rect);
        }
        for (rect, button) in button_rects(Some(board), self.lang, area) {
            let focused = self.focus == Some(button);
            let style = if focused {
                EMPHASIS.add_modifier(Modifier::REVERSED)
            } else {
                EMPHASIS
            };
            if focused && rect.x >= area.x + BUTTON_INDENT {
                let marker = Rect::new(rect.x - BUTTON_INDENT, rect.y, 1, 1);
                frame.render_widget(Span::raw("›"), marker);
            }
            frame.render_widget(Span::styled(button.text(self.lang), style), rect);
        }
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 줄 글자 수
// basis: estimate
fn running_text(lang: Lang, line: &RunningLine, compact: bool) -> String {
    let doing = line.doing(lang);
    if !line.has_output && doing.is_none() {
        return lang.tr(i18n::WORKING).to_string();
    }
    let who: Vec<String> = line
        .provider
        .map(|p| i18n::provider_name(p).to_string())
        .into_iter()
        .chain(line.model.clone().filter(|_| !compact))
        .collect();
    let mut columns: Vec<String> = Vec::new();
    if !who.is_empty() {
        columns.push(who.join(" · "));
    }
    columns.push(i18n::format_elapsed(lang, line.elapsed));
    columns.push(doing.unwrap_or_else(|| lang.tr(i18n::WORKING).to_string()));
    if !compact {
        columns.push(tokens_text(lang, line.tokens));
    }
    columns.join("  ")
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

fn with_text(head: String, text: Option<&str>) -> String {
    match text.and_then(|text| text.lines().next()) {
        Some(first) => format!("{head} · {first}"),
        None => head,
    }
}

fn running_line(task: &TaskView, now: Instant) -> RunningLine {
    RunningLine {
        task: task.id,
        label: task.label,
        provider: task.provider,
        model: task.model.clone(),
        elapsed: task.stopwatch.elapsed(now),
        activity: task.activity.clone(),
        detail: task.detail.clone(),
        detail_open: task.detail_open,
        tokens: task.tokens,
        awaiting_permission: task.state == TaskState::AwaitingPermission,
        awaiting_input: task.state == TaskState::AwaitingInput,
        no_response_minutes: no_response_minutes(task, now),
        approval_pending: task.tool_started_at.is_some_and(|started| {
            now.saturating_duration_since(started) >= APPROVAL_PENDING_AFTER
        }),
        subagents: task.subagents.len(),
        has_output: task.has_output,
    }
}

/// 실행 중(`Running`, `AnsweredTreeRunning`)일 때만 센다. 허가와 입력을 기다리는 동안은 무응답이 아니다.
fn no_response_minutes(task: &TaskView, now: Instant) -> Option<u64> {
    let is_working = matches!(
        task.state,
        TaskState::Running | TaskState::AnsweredTreeRunning
    );
    let silent = now.saturating_duration_since(task.last_event_at);
    (is_working && silent >= NO_RESPONSE_AFTER).then_some(silent.as_secs() / 60)
}

// cost: time O(i + j), heap O(j), stack O(1)
// vars: i = 입력 수, j = 판단 줄 수
// basis: estimate
/// 판단이 `JUDGING_SHOW_AFTER`를 넘긴 입력과, 끝났지만 최소 표시 시간이 남은 입력의 판단 줄. 접수 순서다.
fn judging_lines(state: &ChatState, inputs: &[&InputView], now: Instant) -> Vec<StatusLine> {
    let mut lines: Vec<(u64, StatusLine)> = inputs
        .iter()
        .filter(|input| input.state == InputState::Judging)
        .filter(|input| {
            input
                .judging_since
                .is_some_and(|since| now.saturating_duration_since(since) > JUDGING_SHOW_AFTER)
        })
        .map(|input| {
            let line = StatusLine::Judging {
                input: input.id,
                label: input.label,
                text: input.text.clone(),
            };
            (input.seq, line)
        })
        .collect();
    lines.extend(
        state
            .judging_tails
            .iter()
            .filter(|tail| tail.until > now)
            .map(|tail| {
                let line = StatusLine::Judging {
                    input: tail.input,
                    label: tail.label,
                    text: tail.text.clone(),
                };
                (tail.seq, line)
            }),
    );
    lines.sort_by_key(|(order, _)| *order);
    lines.into_iter().map(|(_, line)| line).collect()
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
                provider: Some(Provider::from_static("codex")),
                elapsed: Duration::ZERO,
                failure: None,
            },
            now,
        );
    }

    fn input(state: &mut ChatState, id: u64, label: char, input_state: InputState, text: &str) {
        input_at(state, id, label, input_state, text, Instant::now());
    }

    fn input_at(
        state: &mut ChatState,
        id: u64,
        label: char,
        input_state: InputState,
        text: &str,
        at: Instant,
    ) {
        state.apply_input(
            InputUpdate {
                input: InputId(id),
                text: text.to_string(),
                label: Some(TaskLabel(label)),
                state: input_state,
                disposition: None,
                reason: (input_state == InputState::Queued)
                    .then_some(QueueReason::AfterTask(TaskLabel('A'))),
            },
            at,
        );
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
    fn alert_text_shows_the_version_or_count_the_alert_carries() {
        // (사례, 경고, 한국어 문구, 영어 문구)
        let cases = [
            (
                "schema migrated shows target version",
                Alert::SchemaMigrated { from: 1, to: 2 },
                "기록 저장소 v2로 옮김",
                "Record store migrated to v2",
            ),
            (
                "provider updated shows both versions",
                Alert::ProviderUpdated {
                    provider: Provider::from_static("codex"),
                    from: "0.158.0".to_owned(),
                    to: "0.159.0".to_owned(),
                },
                "codex CLI가 0.158.0에서 0.159.0로 바뀜",
                "codex CLI changed from 0.158.0 to 0.159.0",
            ),
            (
                "auto prune shows deleted count",
                Alert::AutoPruned { chats: 3, rows: 40 },
                "오래된 채팅 3개를 지웠습니다",
                "Deleted 3 old chats",
            ),
            (
                "auto prune failure",
                Alert::AutoPruneFailed,
                "자동 정리에 실패했습니다 · 로그를 확인하세요",
                "Auto prune failed · Check the log",
            ),
        ];

        for (name, alert, korean, english) in cases {
            assert_eq!(alert_text(Lang::Ko, &alert), korean, "{name}");
            assert_eq!(alert_text(Lang::En, &alert), english, "{name}");
        }
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
        input_at(
            &mut state,
            2,
            'D',
            InputState::Judging,
            "배포 스크립트 정리",
            now,
        );
        task(&mut state, 2, 'A', TaskState::Running, now);

        let lines = build(&state, now + Duration::from_secs(1));

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

    fn judging_texts(state: &ChatState, now: Instant) -> Vec<String> {
        texts(&build(state, now))
            .into_iter()
            .filter(|line| line.contains("판단 중"))
            .collect()
    }

    #[test]
    fn judging_line_is_not_drawn_until_it_passes_the_show_delay() {
        let start = Instant::now();
        let mut state = ChatState::new();
        input_at(&mut state, 1, 'C', InputState::Judging, "테스트", start);

        let at = |ms| judging_texts(&state, start + Duration::from_millis(ms));

        assert!(at(0).is_empty());
        assert!(at(300).is_empty());
        assert_eq!(at(301), vec!["⠙ [C] 판단 중 · 테스트"]);
    }

    #[test]
    fn judging_that_ends_within_the_show_delay_never_draws_a_line() {
        let start = Instant::now();
        let mut state = ChatState::new();
        input_at(&mut state, 1, 'C', InputState::Judging, "테스트", start);
        let while_judging = judging_texts(&state, start + Duration::from_millis(250));

        input_at(
            &mut state,
            1,
            'C',
            InputState::Applied,
            "테스트",
            start + Duration::from_millis(250),
        );

        assert!(while_judging.is_empty());
        for ms in [250, 300, 400, 900] {
            assert!(judging_texts(&state, start + Duration::from_millis(ms)).is_empty());
        }
    }

    #[test]
    fn judging_drawn_once_stays_for_the_minimum_shown_time_after_it_ends() {
        let start = Instant::now();
        let mut state = ChatState::new();
        input_at(&mut state, 1, 'C', InputState::Judging, "테스트", start);

        input_at(
            &mut state,
            1,
            'C',
            InputState::Applied,
            "테스트",
            start + Duration::from_millis(400),
        );

        let at = |ms| judging_texts(&state, start + Duration::from_millis(ms));
        assert_eq!(at(400), vec!["⠙ [C] 판단 중 · 테스트"]);
        assert_eq!(at(799), vec!["⠙ [C] 판단 중 · 테스트"]);
        assert!(at(800).is_empty());
    }

    #[test]
    fn judging_that_ends_after_the_minimum_shown_time_stops_right_away() {
        let start = Instant::now();
        let mut state = ChatState::new();
        input_at(&mut state, 1, 'C', InputState::Judging, "테스트", start);

        input_at(
            &mut state,
            1,
            'C',
            InputState::Applied,
            "테스트",
            start + Duration::from_millis(2_000),
        );

        assert!(judging_texts(&state, start + Duration::from_millis(2_000)).is_empty());
    }

    #[test]
    fn judging_tail_keeps_its_place_among_judging_lines() {
        let start = Instant::now();
        let mut state = ChatState::new();
        input_at(&mut state, 1, 'C', InputState::Judging, "첫째", start);
        input_at(&mut state, 2, 'D', InputState::Judging, "둘째", start);

        input_at(
            &mut state,
            1,
            'C',
            InputState::Applied,
            "첫째",
            start + Duration::from_millis(400),
        );

        assert_eq!(
            judging_texts(&state, start + Duration::from_millis(500)),
            vec!["⠙ [C] 판단 중 · 첫째", "⠙ [D] 판단 중 · 둘째"]
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
            provider: Some(Provider::from_static("claude")),
            model: Some("opus".to_string()),
            elapsed: Duration::from_secs(60),
            activity: Some(Activity::Thinking),
            detail: Vec::new(),
            detail_open: false,
            tokens: None,
            awaiting_permission: false,
            awaiting_input: false,
            no_response_minutes: None,
            approval_pending: false,
            subagents: 2,
            has_output: true,
        });

        assert_eq!(
            line.text(Lang::Ko, true, '⠙'),
            "⠙ [A]  claude · opus  1분  하위 에이전트 2개 실행 중  Token -"
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
        assert!(late[0].contains("  도구 사용 허가 준비 중 · codex  Token -"));
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

    fn text_event() -> ProviderEvent {
        ProviderEvent::Text {
            agent: AgentId(1),
            subagent: None,
            text: "working".to_string(),
        }
    }

    // #38: 마지막 provider 이벤트 뒤 5분이 지나면 분 단위로 응답 없음을 보인다
    #[test]
    fn no_response_shows_in_minutes_after_the_threshold_without_events() {
        let start = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Running, start);

        let early = running_texts(&state, start + Duration::from_secs(299));
        let at = running_texts(&state, start + Duration::from_secs(300));
        let later = running_texts(&state, start + Duration::from_secs(7 * 60 + 10));

        assert!(!early[0].contains("응답 없음"));
        assert!(at[0].contains("  응답 없음 5분  Token -"), "{}", at[0]);
        assert!(
            later[0].contains("  응답 없음 7분  Token -"),
            "{}",
            later[0]
        );
    }

    #[test]
    fn no_response_text_is_translated() {
        assert_eq!(
            Lang::En.tr(i18n::NO_RESPONSE).replace("{minutes}", "5"),
            "No response 5m"
        );
    }

    // #38: 이벤트가 다시 오면 표시를 지우고 그 시각부터 다시 센다
    #[test]
    fn no_response_clears_when_an_event_arrives() {
        let start = Instant::now();
        let silent = start + Duration::from_secs(6 * 60);
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Running, start);
        assert!(running_texts(&state, silent)[0].contains("응답 없음 6분"));

        state.apply_event(TaskId(1), text_event(), silent);

        assert!(!running_texts(&state, silent)[0].contains("응답 없음"));
        assert!(!running_texts(&state, silent + Duration::from_secs(299))[0].contains("응답 없음"));
        assert!(
            running_texts(&state, silent + Duration::from_secs(300))[0].contains("응답 없음 5분")
        );
    }

    // #38: 허가나 입력 요청을 기다리는 동안은 무응답이 아니고, 답한 뒤 다시 센다
    #[test]
    fn no_response_does_not_show_while_waiting_for_permission_or_input() {
        for waiting in [TaskState::AwaitingPermission, TaskState::AwaitingInput] {
            let start = Instant::now();
            let long_after = start + Duration::from_secs(20 * 60);
            let mut state = ChatState::new();
            task(&mut state, 1, 'A', TaskState::Running, start);
            task(&mut state, 1, 'A', waiting, start + Duration::from_secs(60));

            let while_waiting = running_texts(&state, long_after);
            task(&mut state, 1, 'A', TaskState::Running, long_after);
            let just_answered = running_texts(&state, long_after + Duration::from_secs(1));

            assert!(!while_waiting[0].contains("응답 없음"), "{waiting:?}");
            assert!(!just_answered[0].contains("응답 없음"), "{waiting:?}");
        }
    }

    #[test]
    fn approval_pending_text_names_the_provider_in_both_languages() {
        assert_eq!(
            approval_pending_text(Lang::Ko, Some(Provider::from_static("codex"))),
            "도구 사용 허가 준비 중 · codex"
        );
        assert_eq!(
            approval_pending_text(Lang::En, Some(Provider::from_static("claude"))),
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
            provider: Provider::from_static("codex"),
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
    fn renamed_keys_warning_names_the_old_and_new_key() {
        let renamed = SettingsWarning::RenamedKeys {
            keys: vec![("on_exit".to_string(), "tui.on_exit".to_string())],
        };

        assert_eq!(
            settings_warning_text(Lang::Ko, 3, &renamed),
            "옛 설정 이름을 새 이름으로 읽음 · on_exit → tui.on_exit"
        );
        assert_eq!(
            settings_warning_text(Lang::En, 3, &renamed),
            "Old setting names read as the new names · on_exit → tui.on_exit"
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

    fn board_text(state: &ChatState, now: Instant, lang: Lang) -> Option<String> {
        board(state, now).map(|board| board.text(lang, true, '⠙', 120))
    }

    fn three_running_three_queued() -> (ChatState, Instant) {
        let now = Instant::now();
        let mut state = ChatState::new();
        for (id, label) in [(1, 'A'), (2, 'B'), (3, 'D')] {
            task(&mut state, id, label, TaskState::Running, now);
        }
        for id in 1..=3 {
            input(&mut state, id, 'C', InputState::Queued, "");
        }
        (state, now)
    }

    #[test]
    fn board_shows_the_first_running_task_and_counts_the_rest() {
        let (state, now) = three_running_three_queued();

        assert_eq!(
            board_text(&state, now, Lang::Ko).as_deref(),
            Some("⠙ [A] 작업 중 · 실행 2개 더 · 대기 3")
        );
        assert_eq!(
            board_text(&state, now, Lang::En).as_deref(),
            Some("⠙ [A] Working · 2 more running · Queued 3")
        );
    }

    #[test]
    fn board_counts_every_other_kind_in_a_fixed_order() {
        let (mut state, now) = three_running_three_queued();
        input_at(&mut state, 9, 'F', InputState::Judging, "", now);
        task(&mut state, 5, 'E', TaskState::Held, now);
        state.apply_alert(Alert::RouterPaused);

        let text = board_text(&state, now + Duration::from_secs(1), Lang::Ko).unwrap();

        assert_eq!(
            text,
            "⠙ [A] 작업 중 · 실행 2개 더 · 판단 1 · 대기 3 · 보류 1 · 알림 1"
        );
    }

    #[test]
    fn board_uses_more_wording_for_the_kind_that_is_shown() {
        let now = Instant::now();
        let mut state = ChatState::new();
        input(&mut state, 1, 'C', InputState::Queued, "첫째");
        input(&mut state, 2, 'D', InputState::Queued, "둘째");
        task(&mut state, 5, 'E', TaskState::Held, now);

        assert_eq!(
            board_text(&state, now, Lang::Ko).as_deref(),
            Some("· [C] 대기 · A 다음 · 첫째 · 대기 1개 더 · 보류 1")
        );
    }

    #[test]
    fn board_without_lines_is_empty() {
        assert_eq!(board(&ChatState::new(), Instant::now()), None);
    }

    #[test]
    fn board_does_not_count_held_tasks_a_stop_line_already_names() {
        let now = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Held, now);
        task(&mut state, 2, 'C', TaskState::Held, now);
        state.apply_stopped(vec![TaskLabel('A'), TaskLabel('C')]);

        assert_eq!(
            board_text(&state, now, Lang::Ko).as_deref(),
            Some("‖ 멈춤 · [A] [C] 보류됨 · /continue 로 이어서")
        );
    }

    #[test]
    fn board_puts_the_close_held_question_first() {
        let (mut state, now) = three_running_three_queued();
        task(&mut state, 5, 'E', TaskState::Held, now);
        state.close_held_confirm = Some(TaskId(5));

        let text = board_text(&state, now, Lang::Ko).unwrap();

        assert_eq!(text, "‖ [E] 보류를 닫을까요? · 실행 3 · 대기 3");
    }

    #[test]
    fn board_height_is_one_line_plus_the_buttons_of_that_line() {
        let (state, now) = three_running_three_queued();
        let running = board(&state, now).unwrap();
        let mut queued_only = ChatState::new();
        input(&mut queued_only, 1, 'C', InputState::Queued, "x");
        let queued = board(&queued_only, now).unwrap();

        assert_eq!(running.height(80), 1);
        assert_eq!(queued.height(80), 3);
    }

    fn editing_task(
        paths: &[&str],
        activity: Activity,
        tokens_report: Option<u64>,
    ) -> (ChatState, Instant) {
        let start = Instant::now();
        let mut state = ChatState::new();
        task(&mut state, 1, 'A', TaskState::Running, start);
        state.tasks.get_mut(&TaskId(1)).unwrap().model = Some("gpt-5.6-luna".to_owned());
        state.apply_event(
            TaskId(1),
            ProviderEvent::ToolCall {
                agent: AgentId(1),
                subagent: None,
                call_id: "c1".to_owned(),
                activity,
                detail: ToolDetail {
                    paths: paths.iter().map(|path| (*path).to_owned()).collect(),
                    ..ToolDetail::default()
                },
            },
            start,
        );
        let view = state.tasks.get_mut(&TaskId(1)).unwrap();
        view.tokens = tokens_report;
        view.tool_started_at = None;
        (state, start + Duration::from_secs(12))
    }

    #[test]
    fn running_line_columns_are_who_elapsed_doing_and_tokens() {
        let (state, now) = editing_task(&["src/main.rs"], Activity::EditingFile, Some(2_100));

        assert_eq!(
            board_text(&state, now, Lang::Ko).as_deref(),
            Some("⠙ [A]  codex · gpt-5.6-luna  12초  파일 수정 중  Token 2,100")
        );
        assert_eq!(
            board_text(&state, now, Lang::En).as_deref(),
            Some("⠙ [A]  codex · gpt-5.6-luna  12s  Editing files  Token 2,100")
        );
    }

    #[test]
    fn running_line_shows_a_dash_before_tokens_are_reported() {
        let (state, now) = editing_task(&["a.rs"], Activity::ReadingFile, None);

        let text = board_text(&state, now, Lang::Ko).unwrap();

        assert!(text.ends_with("파일 읽는 중  Token -"), "{text}");
    }

    #[test]
    fn running_line_keeps_the_command_out_of_the_line_and_under_it() {
        let (state, now) = editing_task(
            &[],
            Activity::RunningCommand {
                command: "cargo test --workspace".to_owned(),
            },
            None,
        );

        let board = board(&state, now).unwrap();

        let text = board.text(Lang::Ko, true, '⠙', 120);
        assert!(text.contains("명령 실행 중  Token -"), "{text}");
        assert!(!text.contains("cargo"));
        assert_eq!(board.detail_rows(60), vec!["  └ cargo test --workspace"]);
    }

    #[test]
    fn detail_shows_file_paths_one_level_down_and_adds_a_row_to_the_board() {
        let (state, now) = editing_task(&["src/main.rs"], Activity::EditingFile, None);

        let board = board(&state, now).unwrap();

        assert_eq!(board.detail_rows(40), vec!["  └ src/main.rs"]);
        assert_eq!(board.height(40), 2);
    }

    #[test]
    fn a_long_detail_is_cut_to_one_row_ending_in_an_ellipsis() {
        let (state, now) = editing_task(
            &["crates/very/long/path/that/keeps/going/main.rs"],
            Activity::EditingFile,
            None,
        );
        let board = board(&state, now).unwrap();

        let rows = board.detail_rows(20);

        assert_eq!(rows.len(), 1);
        assert_eq!(text_width(&rows[0]), 20);
        assert!(rows[0].ends_with('…'));
        assert!(board.detail_expandable(20));
        assert!(!board.detail_expandable(60));
    }

    #[test]
    fn a_multi_line_command_is_collapsed_to_its_first_line_with_a_mark() {
        let (state, now) = editing_task(
            &[],
            Activity::RunningCommand {
                command: "cd app\ncargo test".to_owned(),
            },
            None,
        );
        let board = board(&state, now).unwrap();

        assert_eq!(board.detail_rows(40), vec!["  └ cd app …"]);
        assert!(board.detail_expandable(40));
    }

    #[test]
    fn an_open_detail_shows_every_line_wrapped_to_the_width() {
        let (mut state, now) = editing_task(
            &[],
            Activity::RunningCommand {
                command: "cd app\ncargo test --workspace".to_owned(),
            },
            None,
        );
        state.tasks.get_mut(&TaskId(1)).unwrap().detail_open = true;
        let board = board(&state, now).unwrap();

        let rows = board.detail_rows(16);

        assert_eq!(
            rows,
            vec!["  └ cd app", "    cargo test -", "    -workspace"]
        );
        assert_eq!(board.height(16), 4);
    }

    #[test]
    fn an_open_detail_stops_at_the_row_limit_with_a_mark() {
        let paths: Vec<String> = (0..20).map(|n| format!("src/file{n}.rs")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let (mut state, now) = editing_task(&refs, Activity::EditingFile, None);
        state.tasks.get_mut(&TaskId(1)).unwrap().detail_open = true;

        let rows = board(&state, now).unwrap().detail_rows(40);

        assert_eq!(rows.len(), DETAIL_MAX_ROWS);
        assert_eq!(rows[DETAIL_MAX_ROWS - 1], "    …");
    }

    #[test]
    fn running_line_drops_the_model_and_tokens_below_the_narrow_width() {
        let (state, now) = editing_task(&["src/main.rs"], Activity::EditingFile, Some(2_100));
        let board = board(&state, now).unwrap();

        let narrow = board.text(Lang::Ko, true, '⠙', NARROW_WIDTH - 1);
        let edge = board.text(Lang::Ko, true, '⠙', NARROW_WIDTH);

        assert_eq!(narrow, "⠙ [A]  codex  12초  파일 수정 중");
        assert_eq!(
            edge,
            "⠙ [A]  codex · gpt-5.6-luna  12초  파일 수정 중  Token 2,100"
        );
    }

    #[test]
    fn a_line_wider_than_the_area_keeps_its_front_and_ends_in_an_ellipsis() {
        let (state, now) = editing_task(&["a.rs"], Activity::EditingFile, Some(2_100));
        let board = board(&state, now).unwrap();
        let view = StatusBoardView {
            board: Some(&board),
            lang: Lang::Ko,
            labels_visible: true,
            spinner: '⠙',
            focus: None,
        };
        let mut terminal = Terminal::new(TestBackend::new(24, 3)).unwrap();

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let rows = buffer_lines(terminal.backend().buffer());
        assert!(rows[0].starts_with("⠙ [A]  codex"), "{rows:?}");
        assert!(rows[0].ends_with('…'), "{rows:?}");
        assert!(text_width(&rows[0]) <= 24);
    }

    #[test]
    fn detail_rows_never_break_on_a_tiny_width() {
        let (state, now) = editing_task(&["src/main.rs"], Activity::EditingFile, None);
        let board = board(&state, now).unwrap();

        assert!(board.detail_rows(0).is_empty());
        for width in 1..6 {
            let rows = board.detail_rows(width);
            assert_eq!(rows.len(), 1);
            assert!(text_width(&rows[0]) <= usize::from(width));
        }
    }

    #[test]
    fn button_rects_stack_under_the_line() {
        let board = pick(vec![StatusLine::Queued {
            input: InputId(1),
            label: None,
            reason: QueueReason::WriteTurn,
            text: None,
        }]);

        let rects = button_rects(board.as_ref(), Lang::En, Rect::new(0, 5, 40, 3));

        assert_eq!(rects[0], (Rect::new(2, 6, 6, 1), Button::Send(InputId(1))));
        assert_eq!(
            rects[1],
            (Rect::new(2, 7, 8, 1), Button::CancelInput(InputId(1)))
        );
    }

    #[test]
    fn render_draws_text_and_buttons() {
        let board = pick(vec![StatusLine::Queued {
            input: InputId(1),
            label: Some(TaskLabel('C')),
            reason: QueueReason::AfterTask(TaskLabel('A')),
            text: Some("test".to_string()),
        }]);
        let mut terminal = Terminal::new(TestBackend::new(50, 3)).unwrap();
        let view = StatusBoardView {
            board: board.as_ref(),
            lang: Lang::Ko,
            labels_visible: true,
            spinner: '⠙',
            focus: None,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let rows = buffer_lines(terminal.backend().buffer());
        assert_eq!(rows[0], "· [C] 대기 · A 다음 · test");
        assert_eq!(rows[1], "  [보내기]");
        assert_eq!(rows[2], "  [취소]");
    }
}
