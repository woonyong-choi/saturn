//! 작업 목록 화면(`/tasks`). 채팅을 가로지르는 작업을 필터와 묶음으로 본다.
//! 설계: docs/design/tui.md

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use saturn_protocol::ids::{ChatId, InputId, TaskId, TaskLabel};
use saturn_protocol::rpc::TaskListItem;
use saturn_protocol::state::TaskState;

use crate::chat_picker;
use crate::i18n::{self, Lang};
use crate::keys::Action;
use crate::labels;
use crate::view::{EMPHASIS, MUTED, SELECTED, WIDE_WIDTH, truncate, window_block};

/// 끝에서 처음으로 돈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TaskFilter {
    #[default]
    All,
    /// 허가 필요 작업을 포함한다.
    NeedsCheck,
    Running,
    Queued,
    Held,
    Done,
    /// 작업 없는 채팅 행.
    Chats,
}

impl TaskFilter {
    const ORDER: [Self; 7] = [
        Self::All,
        Self::NeedsCheck,
        Self::Running,
        Self::Queued,
        Self::Held,
        Self::Done,
        Self::Chats,
    ];

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn next(self) -> Self {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn prev(self) -> Self {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    pub(crate) fn text(self, lang: Lang) -> &'static str {
        lang.tr(match self {
            Self::All => i18n::FILTER_ALL,
            Self::NeedsCheck => i18n::FILTER_NEEDS_CHECK,
            Self::Running => i18n::FILTER_RUNNING,
            Self::Queued => i18n::FILTER_QUEUED,
            Self::Held => i18n::FILTER_HELD,
            Self::Done => i18n::FILTER_DONE,
            Self::Chats => i18n::FILTER_CHATS,
        })
    }

    /// `전체`는 끝난 작업과 대기 입력 없는 채팅 행을 빼고 보인다. 그 둘은 `끝남`과 `채팅`에서 본다.
    fn matches(self, row: &TaskRow) -> bool {
        match self {
            Self::All => !row.is_ended() && (!row.is_chat() || !row.queued.is_empty()),
            Self::NeedsCheck => row.needs_permission || row.state == Some(TaskState::NeedsCheck),
            Self::Running => row.state.is_some_and(labels::is_live),
            Self::Queued => !row.queued.is_empty(),
            Self::Held => row.state == Some(TaskState::Held),
            Self::Done => row.is_ended(),
            Self::Chats => row.is_chat(),
        }
    }
}

/// 작업 목록이 보이는 채팅의 폴더 범위. 기본은 현재 채팅의 폴더다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FolderScope {
    #[default]
    Current,
    All,
}

impl FolderScope {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    pub(crate) fn toggled(self) -> Self {
        match self {
            Self::Current => Self::All,
            Self::All => Self::Current,
        }
    }

    pub(crate) fn text(self, lang: Lang) -> &'static str {
        lang.tr(match self {
            Self::Current => i18n::TASKS_SCOPE_CURRENT,
            Self::All => i18n::TASKS_SCOPE_ALL,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskRow {
    /// 작업 없는 채팅 행이면 `None`.
    pub task: Option<TaskId>,
    /// 끝난 작업과 채팅 행은 `None`.
    pub label: Option<TaskLabel>,
    /// 채팅 행은 `None`. `Done`과 `Failed`는 끝난 작업.
    pub state: Option<TaskState>,
    pub needs_permission: bool,
    /// 이 행으로 갈 대기 입력, 접수 순서.
    pub queued: Vec<InputId>,
    pub model: Option<String>,
    pub children: u32,
    /// 끝난 작업이 끝난 시각(unix 밀리초).
    pub ended_at_ms: Option<u64>,
}

impl TaskRow {
    pub(crate) fn is_chat(&self) -> bool {
        self.task.is_none()
    }

    pub(crate) fn is_ended(&self) -> bool {
        matches!(self.state, Some(TaskState::Done | TaskState::Failed))
    }

    /// `d`와 `s`가 다루는 다음 차례 대기 입력.
    pub(crate) fn queued_input(&self) -> Option<InputId> {
        self.queued.first().copied()
    }
}

/// 채팅 번호와 작업 번호. 채팅 행은 작업 번호가 없다.
pub(crate) type RowKey = (ChatId, Option<TaskId>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChatGroup {
    pub group: String,
    pub chat: ChatId,
    pub name: String,
    pub folder: Option<PathBuf>,
    /// 다른 Saturn 프로세스가 실행 중이라 읽기 전용.
    pub busy_elsewhere: bool,
    pub tasks: Vec<TaskRow>,
}

impl ChatGroup {
    // cost: time O(n·c), heap O(n), stack O(1)
    // vars: n = items.len(), c = 채팅 수
    // basis: estimate
    pub(crate) fn from_items(items: Vec<TaskListItem>) -> Vec<Self> {
        let mut groups: Vec<Self> = Vec::new();
        for item in items {
            let row = TaskRow {
                task: item.task,
                label: item.label,
                state: item.state,
                needs_permission: item.needs_permission,
                queued: item.queued,
                model: item.model,
                children: item.children,
                ended_at_ms: item.ended_at_ms,
            };
            match groups.iter_mut().find(|g| g.chat == item.chat) {
                Some(group) => group.tasks.push(row),
                None => groups.push(Self {
                    group: item.group.unwrap_or_default(),
                    chat: item.chat,
                    name: item.chat_name,
                    folder: item.folder.map(PathBuf::from),
                    busy_elsewhere: item.busy_elsewhere,
                    tasks: vec![row],
                }),
            }
        }
        groups
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskListCommand {
    Open { chat: ChatId, task: Option<TaskId> },
    Continue { chat: ChatId, task: TaskId },
    CancelInput(InputId),
    CloseHeld { chat: ChatId, task: TaskId },
    SendNow(InputId),
    NewChat,
    Rename { chat: ChatId, name: String },
    Regroup { chat: ChatId, group: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskListInput {
    Search,
    Rename(ChatId),
    Regroup(ChatId),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct TaskList {
    pub groups: Vec<ChatGroup>,
    pub filter: TaskFilter,
    pub scope: FolderScope,
    /// 현재 채팅의 폴더. 모르면 폴더로 거르지 않는다.
    pub folder: Option<PathBuf>,
    pub query: Option<String>,
    /// 목록을 다시 받아도 유지한다.
    pub selected: Option<RowKey>,
    pub pending: Option<TaskListCommand>,
    pub editing: Option<(TaskListInput, String)>,
    pub help: bool,
    /// 한 칸으로 그리는 폭이라 `Enter`가 먼저 상세를 보이고, 상세가 보이는 동안 `Enter`가 채팅으로 이동한다.
    pub detail_on_enter: bool,
    /// 한 칸일 때 고른 행의 상세를 보이는 중이다.
    pub detail_open: bool,
}

impl TaskList {
    /// 기본 범위(현재 폴더)로 연다.
    pub(crate) fn for_folder(folder: Option<PathBuf>) -> Self {
        Self {
            folder,
            ..Self::default()
        }
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 범위를 바꾸고 선택은 보이는 행 안에서 다시 고른다.
    pub(crate) fn toggle_scope(&mut self) {
        self.scope = self.scope.toggled();
        let groups = std::mem::take(&mut self.groups);
        self.replace(groups);
    }

    // cost: time O(r), heap O(r), stack O(1)
    // vars: r = 행 수
    // basis: estimate
    /// 선택한 작업이 새 목록에 없으면 같은 자리의 행을 고른다.
    pub(crate) fn replace(&mut self, groups: Vec<ChatGroup>) {
        let old_index = self.selected_index();
        self.groups = groups;
        let rows = self.visible_keys();
        let kept = self.selected.filter(|key| rows.contains(key));
        self.selected = kept.or_else(|| {
            let index = old_index.unwrap_or(0).min(rows.len().saturating_sub(1));
            rows.get(index).copied()
        });
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    pub(crate) fn visible_rows(&self) -> Vec<(&ChatGroup, &TaskRow)> {
        self.groups
            .iter()
            .filter(|group| self.scope_matches(group))
            .filter(|group| self.query_matches(group))
            .flat_map(|group| group.tasks.iter().map(move |row| (group, row)))
            .filter(|(_, row)| self.filter.matches(row))
            .collect()
    }

    pub(crate) fn up(&mut self) {
        self.detail_open = false;
        let rows = self.visible_keys();
        let index = self.selected_index().unwrap_or(0).saturating_sub(1);
        self.selected = rows.get(index).copied();
    }

    pub(crate) fn down(&mut self) {
        self.detail_open = false;
        let rows = self.visible_keys();
        let index = match self.selected_index() {
            Some(index) => (index + 1).min(rows.len().saturating_sub(1)),
            None => 0,
        };
        self.selected = rows.get(index).copied();
    }

    pub(crate) fn set_filter(&mut self, filter: TaskFilter) {
        self.filter = filter;
        let groups = std::mem::take(&mut self.groups);
        self.replace(groups);
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 읽기 전용 채팅이면 `Open`, `NewChat`, 검색만 받는다.
    pub(crate) fn command(&mut self, action: &Action) -> Option<TaskListCommand> {
        match action {
            Action::Confirm => return self.confirm(),
            Action::NewChat => return Some(TaskListCommand::NewChat),
            Action::Search => {
                let query = self.query.clone().unwrap_or_default();
                self.editing = Some((TaskListInput::Search, query));
                return None;
            }
            Action::Help => {
                self.help = !self.help;
                return None;
            }
            _ => {}
        }
        let (group, row) = self.selected_row()?;
        if group.busy_elsewhere {
            return None;
        }
        let chat = group.chat;
        let held = row.task.filter(|_| row.state == Some(TaskState::Held));
        match action {
            Action::ContinueHeld => held.map(|task| TaskListCommand::Continue { chat, task }),
            Action::CancelOrCloseHeld => match (row.queued_input(), held) {
                (Some(input), _) => Some(TaskListCommand::CancelInput(input)),
                (None, Some(task)) => {
                    self.pending = Some(TaskListCommand::CloseHeld { chat, task });
                    None
                }
                (None, None) => None,
            },
            Action::SendQueued => row.queued_input().map(TaskListCommand::SendNow),
            Action::RenameChat => {
                self.editing = Some((TaskListInput::Rename(chat), group.name.clone()));
                None
            }
            Action::ChangeGroup => {
                self.editing = Some((TaskListInput::Regroup(chat), group.group.clone()));
                None
            }
            _ => None,
        }
    }

    /// 닫은 것이 없으면 `false`(화면 종료).
    pub(crate) fn cancel(&mut self) -> bool {
        if self.help {
            self.help = false;
        } else if self.pending.is_some() {
            self.pending = None;
        } else if self.editing.is_some() {
            self.editing = None;
        } else if self.detail_open {
            self.detail_open = false;
        } else {
            return false;
        }
        true
    }

    pub(crate) fn edit_push(&mut self, c: char) {
        if let Some((_, text)) = &mut self.editing {
            text.push(c);
        }
    }

    pub(crate) fn edit_pop(&mut self) {
        if let Some((_, text)) = &mut self.editing {
            text.pop();
        }
    }

    pub(crate) fn marker(state: TaskState) -> char {
        match state {
            TaskState::AwaitingPermission | TaskState::AwaitingInput => '!',
            TaskState::NeedsCheck => '?',
            _ => ' ',
        }
    }

    fn confirm(&mut self) -> Option<TaskListCommand> {
        if let Some(pending) = self.pending.take() {
            return Some(pending);
        }
        if let Some((input, text)) = self.editing.take() {
            return match input {
                TaskListInput::Search => {
                    self.query = (!text.is_empty()).then_some(text);
                    let groups = std::mem::take(&mut self.groups);
                    self.replace(groups);
                    None
                }
                TaskListInput::Rename(chat) => Some(TaskListCommand::Rename { chat, name: text }),
                TaskListInput::Regroup(chat) => {
                    Some(TaskListCommand::Regroup { chat, group: text })
                }
            };
        }
        let (group, row) = self.selected_row()?;
        if self.detail_on_enter && !self.detail_open {
            self.detail_open = true;
            return None;
        }
        Some(TaskListCommand::Open {
            chat: group.chat,
            task: row.task,
        })
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    fn selected_row(&self) -> Option<(&ChatGroup, &TaskRow)> {
        let selected = self.selected?;
        self.visible_rows()
            .into_iter()
            .find(|(group, row)| (group.chat, row.task) == selected)
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.visible_keys().iter().position(|key| *key == selected)
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    fn visible_keys(&self) -> Vec<RowKey> {
        self.visible_rows()
            .into_iter()
            .map(|(group, row)| (group.chat, row.task))
            .collect()
    }

    // cost: time O(f), heap O(1), stack O(1)
    // vars: f = 폴더 경로 길이
    // basis: estimate
    /// 모든 폴더 범위이거나 채팅이나 현재 폴더를 모르면 참이다. 모르는 채팅을 숨겨 확인이 필요한 작업을 놓치지 않기 위해서다.
    /// TODO(#161): 작업 목록 조회가 engine에 생기면 폴더 범위를 조회 조건으로도 보낸다. 지금은 받은 목록을 TUI가 거른다
    fn scope_matches(&self, group: &ChatGroup) -> bool {
        match (self.scope, &group.folder, &self.folder) {
            (FolderScope::Current, Some(chat), Some(current)) => chat == current,
            _ => true,
        }
    }

    // cost: time O(m·q), heap O(m), stack O(1)
    // vars: m = 채팅 이름·묶음·폴더 길이, q = 검색어 길이
    // basis: estimate
    /// 검색어가 없으면 참.
    fn query_matches(&self, group: &ChatGroup) -> bool {
        let Some(query) = &self.query else {
            return true;
        };
        let folder = group
            .folder
            .as_ref()
            .map(|f| f.display().to_string())
            .unwrap_or_default();
        group.name.contains(query.as_str())
            || group.group.contains(query.as_str())
            || folder.contains(query.as_str())
    }
}

#[derive(Debug)]
pub(crate) struct TaskListView<'a> {
    pub list: &'a TaskList,
    pub lang: Lang,
}

impl TaskListView<'_> {
    pub(crate) fn render(&self, frame: &mut Frame, area: Rect) {
        let block = window_block(self.lang.tr(i18n::TASKS_TITLE));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let wide = area.width > WIDE_WIDTH;
        let list_area = if wide {
            Rect::new(inner.x, inner.y, inner.width * 3 / 5, inner.height)
        } else {
            inner
        };
        let width = usize::from(list_area.width);
        let mut lines = vec![self.filter_line(), Line::from("")];
        lines.extend(self.row_lines(width));
        lines.push(Line::from(""));
        lines.extend(self.footer_lines(width));
        if wide {
            let detail_x = list_area.right() + 1;
            let detail_area = Rect::new(
                detail_x,
                inner.y,
                inner.right().saturating_sub(detail_x),
                inner.height,
            );
            let rule: Vec<Line> = (0..inner.height).map(|_| Line::from("│")).collect();
            frame.render_widget(
                Paragraph::new(rule),
                Rect::new(list_area.right(), inner.y, 1.min(inner.width), inner.height),
            );
            let detail: Vec<Line> = self
                .detail_lines()
                .into_iter()
                .map(|line| truncate(&line, usize::from(detail_area.width)))
                .map(Line::from)
                .collect();
            frame.render_widget(Paragraph::new(detail), detail_area);
        } else if self.list.detail_open {
            lines.extend(
                self.detail_lines()
                    .into_iter()
                    .map(|line| Line::from(truncate(&line, width))),
            );
        }
        frame.render_widget(Paragraph::new(lines), list_area);
    }

    // cost: time O(r·q), heap O(1), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 고른 행의 상세. 채팅 이름과 폴더, 상태, 모델, 하위 항목, 대기 입력 수.
    fn detail_lines(&self) -> Vec<String> {
        let lang = self.lang;
        let Some((group, row)) = self.list.selected_row() else {
            return Vec::new();
        };
        let mut lines = vec![self.chat_head(group)];
        lines.push(row_text(lang, row, chat_picker::now_ms()).trim().to_owned());
        if !row.is_chat() {
            let model = row
                .model
                .clone()
                .unwrap_or_else(|| lang.tr(i18n::MODEL_UNREPORTED).to_string());
            lines.push(format!("{}: {model}", lang.tr(i18n::TASKS_MODEL)));
            lines.push(format!(
                "{}: {}",
                lang.tr(i18n::TASKS_CHILDREN),
                row.children
            ));
        }
        lines
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn filter_line(&self) -> Line<'static> {
        let spans: Vec<Span> = TaskFilter::ORDER
            .iter()
            .flat_map(|filter| {
                let style = if *filter == self.list.filter {
                    SELECTED
                } else {
                    Style::new()
                };
                [
                    Span::styled(filter.text(self.lang).to_string(), style),
                    Span::raw("  "),
                ]
            })
            .chain([Span::styled(
                format!("· {}", self.list.scope.text(self.lang)),
                MUTED,
            )])
            .collect();
        Line::from(spans)
    }

    // cost: time O(r), heap O(r), stack O(1)
    // vars: r = 보이는 행 수
    // basis: estimate
    fn row_lines(&self, width: usize) -> Vec<Line<'static>> {
        let rows = self.list.visible_rows();
        if rows.is_empty() {
            return vec![Line::from(Span::styled(
                self.lang.tr(i18n::TASKS_EMPTY),
                MUTED,
            ))];
        }
        let mut lines = Vec::new();
        let mut current: Option<ChatId> = None;
        for (group, row) in rows {
            if current != Some(group.chat) {
                current = Some(group.chat);
                lines.push(Line::from(Span::styled(self.chat_head(group), EMPHASIS)));
            }
            let selected = self.list.selected == Some((group.chat, row.task));
            let style = if selected { SELECTED } else { Style::new() };
            let text = row_text(self.lang, row, chat_picker::now_ms());
            lines.push(Line::from(Span::styled(truncate(&text, width), style)));
        }
        lines
    }

    fn chat_head(&self, group: &ChatGroup) -> String {
        let mut head = if group.group.is_empty() {
            group.name.clone()
        } else {
            format!("{} › {}", group.group, group.name)
        };
        if let Some(folder) = &group.folder {
            head.push_str(&format!(" · {}", folder.display()));
        }
        if group.busy_elsewhere {
            head.push_str(&format!(" · {}", self.lang.tr(i18n::BUSY_ELSEWHERE)));
        }
        head
    }

    fn footer_lines(&self, width: usize) -> Vec<Line<'static>> {
        let lang = self.lang;
        let mut lines = Vec::new();
        if let Some(TaskListCommand::CloseHeld { .. }) = &self.list.pending {
            lines.push(Line::from(lang.tr(i18n::CLOSE_HELD_QUESTION)));
        }
        if let Some((input, text)) = &self.list.editing {
            let title = match input {
                TaskListInput::Search => i18n::TASKS_SEARCH,
                TaskListInput::Rename(_) => i18n::TASKS_RENAME,
                TaskListInput::Regroup(_) => i18n::TASKS_GROUP,
            };
            lines.push(Line::from(format!("{}: {text}", lang.tr(title))));
        }
        if self.list.help {
            lines.push(Line::from(Span::styled(
                truncate(lang.tr(i18n::TASKS_HELP), width),
                MUTED,
            )));
        }
        lines
    }
}

/// 작업 행은 `표시 [글자] 상태`, 끝난 작업 행은 `#작업 번호 결과 · 경과`, 채팅 행은 `작업 없음`.
/// 대기 입력이 있으면 끝에 ` · 대기 N`을 붙인다.
fn row_text(lang: Lang, row: &TaskRow, now_ms: u64) -> String {
    let mut text = match (row.task, row.label, row.state) {
        (Some(task), None, Some(state)) => {
            let age = row
                .ended_at_ms
                .map(|at| chat_picker::age(lang, now_ms.saturating_sub(at)));
            let result = lang.tr(state_key(state));
            match age {
                Some(age) => format!("    #{} {result} · {age}", task.0),
                None => format!("    #{} {result}", task.0),
            }
        }
        (_, label, Some(state)) => format!(
            "  {} {} {}",
            TaskList::marker(state),
            label.map(labels::format).unwrap_or_default(),
            lang.tr(state_key(state))
        ),
        (_, _, None) => format!("    {}", lang.tr(i18n::TASKS_EMPTY)),
    };
    if !row.queued.is_empty() {
        let count = lang
            .tr(i18n::TASKS_QUEUED_COUNT)
            .replace("{n}", &row.queued.len().to_string());
        text.push_str(&format!(" · {count}"));
    }
    text
}

fn state_key(state: TaskState) -> &'static str {
    match state {
        TaskState::Running | TaskState::AnsweredTreeRunning => i18n::FILTER_RUNNING,
        TaskState::AwaitingPermission => i18n::AWAITING_PERMISSION,
        TaskState::AwaitingInput => i18n::AWAITING_INPUT,
        TaskState::Held => i18n::FILTER_HELD,
        TaskState::NeedsCheck => i18n::FILTER_NEEDS_CHECK,
        TaskState::Done => i18n::FILTER_DONE,
        TaskState::Failed => i18n::FAILED,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn item(chat: u64, task: u64, label: char, state: TaskState) -> TaskListItem {
        TaskListItem {
            chat: ChatId(chat),
            chat_name: format!("chat{chat}"),
            group: None,
            task: Some(TaskId(task)),
            label: Some(TaskLabel(label)),
            state: Some(state),
            needs_permission: matches!(
                state,
                TaskState::AwaitingPermission | TaskState::AwaitingInput
            ),
            busy_elsewhere: chat == 9,
            children: 0,
            folder: None,
            queued: Vec::new(),
            model: None,
            ended_at_ms: None,
        }
    }

    /// 작업 없는 채팅 행.
    fn chat_item(chat: u64) -> TaskListItem {
        TaskListItem {
            task: None,
            label: None,
            state: None,
            needs_permission: false,
            ..item(chat, 0, 'A', TaskState::Running)
        }
    }

    /// 끝난 작업 행.
    fn ended_item(chat: u64, task: u64, state: TaskState) -> TaskListItem {
        TaskListItem {
            label: None,
            ended_at_ms: Some(1),
            ..item(chat, task, 'A', state)
        }
    }

    fn in_folder(folder: &str, item: TaskListItem) -> TaskListItem {
        TaskListItem {
            folder: Some(folder.to_owned()),
            ..item
        }
    }

    fn two_folders() -> TaskList {
        let mut list = TaskList::for_folder(Some(PathBuf::from("/work/a")));
        list.replace(ChatGroup::from_items(vec![
            in_folder("/work/a", item(1, 1, 'A', TaskState::Running)),
            in_folder("/work/b", item(2, 2, 'A', TaskState::Running)),
            in_folder("/work/a", item(3, 3, 'A', TaskState::Held)),
        ]));
        list
    }

    fn visible_chats(list: &TaskList) -> Vec<u64> {
        list.visible_rows()
            .iter()
            .map(|(group, _)| group.chat.0)
            .collect()
    }

    #[test]
    fn task_list_defaults_to_the_current_folder_and_the_key_widens_it() {
        let mut list = two_folders();

        let narrow = visible_chats(&list);
        list.toggle_scope();
        let wide = visible_chats(&list);
        list.toggle_scope();

        assert_eq!(narrow, vec![1, 3]);
        assert_eq!(wide, vec![1, 2, 3]);
        assert_eq!(list.scope, FolderScope::Current);
    }

    #[test]
    fn task_list_scope_keeps_the_selection_inside_the_visible_rows() {
        let mut list = two_folders();
        list.toggle_scope();
        list.selected = Some((ChatId(2), Some(TaskId(2))));

        list.toggle_scope();

        assert_eq!(list.selected, Some((ChatId(1), Some(TaskId(1)))));
    }

    #[test]
    fn task_list_does_not_hide_chats_whose_folder_is_unknown() {
        let mut list = TaskList::for_folder(Some(PathBuf::from("/work/a")));
        list.replace(ChatGroup::from_items(vec![
            item(1, 1, 'A', TaskState::Running),
            in_folder("/work/b", item(2, 2, 'A', TaskState::Running)),
        ]));
        let without_current = {
            let mut other = TaskList::for_folder(None);
            other.replace(list.groups.clone());
            visible_chats(&other)
        };

        assert_eq!(visible_chats(&list), vec![1]);
        assert_eq!(without_current, vec![1, 2]);
    }

    #[test]
    fn task_list_from_items_keeps_the_folder_for_the_head_line() {
        let list = two_folders();

        assert_eq!(list.groups[1].folder, Some(PathBuf::from("/work/b")));
    }

    #[test]
    fn task_list_scope_is_named_on_the_filter_line() {
        let mut list = two_folders();
        let backend = TestBackend::new(70, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut texts = Vec::new();
        for _ in 0..2 {
            terminal
                .draw(|frame| {
                    TaskListView {
                        list: &list,
                        lang: Lang::En,
                    }
                    .render(frame, frame.area());
                })
                .unwrap();
            let buffer = terminal.backend().buffer().clone();
            texts.push(
                (0..buffer.area.width)
                    .map(|x| buffer[(x, 1)].symbol().to_owned())
                    .collect::<String>(),
            );
            list.toggle_scope();
        }

        assert!(texts[0].contains("· This folder"));
        assert!(texts[1].contains("· All folders"));
    }

    fn list() -> TaskList {
        let mut list = TaskList::default();
        list.replace(ChatGroup::from_items(vec![
            item(1, 1, 'A', TaskState::Running),
            item(1, 2, 'B', TaskState::Held),
            item(2, 3, 'A', TaskState::Done),
            item(9, 4, 'A', TaskState::Held),
        ]));
        list
    }

    #[test]
    fn from_items_groups_by_chat_in_order() {
        let groups = list().groups;

        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].tasks.len(), 2);
    }

    #[test]
    fn filter_cycles_both_ways() {
        assert_eq!(TaskFilter::All.next(), TaskFilter::NeedsCheck);
        assert_eq!(TaskFilter::All.prev(), TaskFilter::Chats);
        assert_eq!(TaskFilter::Chats.next(), TaskFilter::All);
    }

    #[test]
    fn set_filter_limits_rows() {
        let mut list = list();

        list.set_filter(TaskFilter::Held);

        assert_eq!(list.visible_rows().len(), 2);
        assert_eq!(list.selected, Some((ChatId(1), Some(TaskId(2)))));
    }

    #[test]
    fn replace_keeps_selected_task() {
        let mut list = list();
        list.down();
        let selected = list.selected;

        list.replace(ChatGroup::from_items(vec![
            item(2, 3, 'A', TaskState::Done),
            item(1, 2, 'B', TaskState::Held),
        ]));

        assert_eq!(list.selected, selected);
    }

    #[test]
    fn close_held_needs_confirmation() {
        let mut list = list();
        list.down();

        let first = list.command(&Action::CancelOrCloseHeld);
        let confirmed = list.command(&Action::Confirm);

        assert_eq!(first, None);
        assert_eq!(
            confirmed,
            Some(TaskListCommand::CloseHeld {
                chat: ChatId(1),
                task: TaskId(2)
            })
        );
    }

    #[test]
    fn busy_chat_is_read_only() {
        let mut list = list();
        (0..3).for_each(|_| list.down());

        assert_eq!(list.command(&Action::ContinueHeld), None);
        assert_eq!(
            list.command(&Action::Confirm),
            Some(TaskListCommand::Open {
                chat: ChatId(9),
                task: Some(TaskId(4))
            })
        );
    }

    #[test]
    fn rename_opens_input_line_and_returns_command() {
        let mut list = list();

        list.command(&Action::RenameChat);
        list.edit_pop();
        list.edit_push('X');

        assert_eq!(
            list.command(&Action::Confirm),
            Some(TaskListCommand::Rename {
                chat: ChatId(1),
                name: "chatX".to_string()
            })
        );
    }

    #[test]
    fn marker_flags_permission_and_check() {
        assert_eq!(TaskList::marker(TaskState::AwaitingPermission), '!');
        assert_eq!(TaskList::marker(TaskState::NeedsCheck), '?');
        assert_eq!(TaskList::marker(TaskState::Done), ' ');
    }

    fn mixed() -> TaskList {
        let mut waiting = item(1, 1, 'A', TaskState::Running);
        waiting.queued = vec![InputId(11), InputId(12)];
        waiting.model = Some("opus".to_owned());
        let mut new_input = chat_item(3);
        new_input.queued = vec![InputId(13)];
        let mut list = TaskList::default();
        list.replace(ChatGroup::from_items(vec![
            waiting,
            ended_item(1, 2, TaskState::Done),
            ended_item(2, 3, TaskState::Failed),
            new_input,
            chat_item(4),
        ]));
        list
    }

    fn rows_in(list: &mut TaskList, filter: TaskFilter) -> Vec<(u64, Option<u64>)> {
        list.set_filter(filter);
        list.visible_rows()
            .iter()
            .map(|(group, row)| (group.chat.0, row.task.map(|task| task.0)))
            .collect()
    }

    #[test]
    fn filters_split_task_rows_ended_tasks_and_chat_rows() {
        let mut list = mixed();

        assert_eq!(
            rows_in(&mut list, TaskFilter::All),
            [(1, Some(1)), (3, None)]
        );
        assert_eq!(
            rows_in(&mut list, TaskFilter::Done),
            [(1, Some(2)), (2, Some(3))]
        );
        assert_eq!(
            rows_in(&mut list, TaskFilter::Chats),
            [(3, None), (4, None)]
        );
        assert_eq!(
            rows_in(&mut list, TaskFilter::Queued),
            [(1, Some(1)), (3, None)]
        );
        assert_eq!(rows_in(&mut list, TaskFilter::Running), [(1, Some(1))]);
    }

    #[test]
    fn chat_row_renames_regroups_and_opens_but_ignores_task_keys() {
        let mut list = mixed();
        list.set_filter(TaskFilter::Chats);
        list.down();

        list.command(&Action::RenameChat);
        list.edit_push('!');
        let renamed = list.command(&Action::Confirm);
        let open = list.command(&Action::Confirm);

        assert_eq!(list.command(&Action::ContinueHeld), None);
        assert_eq!(list.command(&Action::SendQueued), None);
        assert_eq!(
            renamed,
            Some(TaskListCommand::Rename {
                chat: ChatId(4),
                name: "chat4!".to_owned()
            })
        );
        assert_eq!(
            open,
            Some(TaskListCommand::Open {
                chat: ChatId(4),
                task: None
            })
        );
    }

    #[test]
    fn cancel_and_send_act_on_the_next_waiting_input_of_the_row() {
        let mut list = mixed();

        let send = list.command(&Action::SendQueued);
        let cancel = list.command(&Action::CancelOrCloseHeld);

        assert_eq!(send, Some(TaskListCommand::SendNow(InputId(11))));
        assert_eq!(cancel, Some(TaskListCommand::CancelInput(InputId(11))));
    }

    fn screen(list: &TaskList) -> String {
        screen_at(list, 70)
    }

    fn screen_at(list: &TaskList, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
        let view = TaskListView {
            list,
            lang: Lang::En,
        };
        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn render_shows_queue_count_model_ended_result_and_chat_rows() {
        let mut list = mixed();
        list.detail_open = true;
        let all = screen(&list);
        list.set_filter(TaskFilter::Done);
        let done = screen(&list);
        list.set_filter(TaskFilter::Chats);
        let chats = screen(&list);

        assert!(all.contains("[A] Running · Queued 2"), "{all}");
        assert!(all.contains("Model: opus"), "{all}");
        assert!(all.contains("No tasks · Queued 1"), "{all}");
        assert!(done.contains("#2 Done"), "{done}");
        assert!(done.contains("#3 Failed"), "{done}");
        assert!(chats.contains("chat4"), "{chats}");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_filters_rows_and_busy_chat() {
        let mut list = list();
        list.detail_open = true;
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        let view = TaskListView {
            list: &list,
            lang: Lang::En,
        };

        terminal
            .draw(|frame| view.render(frame, frame.area()))
            .unwrap();

        let content: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(content.contains("All"));
        assert!(content.contains("[B] Held"));
        assert!(content.contains("Running in another Saturn"));
        assert!(content.contains("Model not reported"));
    }

    /// 고른 행이 작업 A인 목록.
    fn selected_running() -> TaskList {
        let mut list = mixed();
        list.selected = Some((ChatId(1), Some(TaskId(1))));
        list
    }

    #[test]
    fn one_pane_hides_the_detail_until_enter_and_the_second_enter_opens_the_chat() {
        let mut list = selected_running();
        list.detail_on_enter = true;

        let hidden = screen_at(&list, 100);
        let first = list.command(&Action::Confirm);
        let shown = screen_at(&list, 100);
        let second = list.command(&Action::Confirm);

        assert!(!hidden.contains("Model: opus"), "{hidden}");
        assert_eq!(first, None);
        assert!(shown.contains("Model: opus"), "{shown}");
        assert_eq!(
            second,
            Some(TaskListCommand::Open {
                chat: ChatId(1),
                task: Some(TaskId(1))
            })
        );
    }

    #[test]
    fn one_pane_escape_closes_the_detail_before_the_screen_and_moving_closes_it() {
        let mut list = selected_running();
        list.detail_on_enter = true;
        list.command(&Action::Confirm);

        let closed_detail = list.cancel();
        list.command(&Action::Confirm);
        list.down();

        assert!(closed_detail);
        assert!(!list.detail_open);
        assert!(!list.cancel());
    }

    #[test]
    fn two_panes_show_the_detail_beside_the_list_and_enter_opens_at_once() {
        let mut list = selected_running();
        list.detail_on_enter = false;

        let wide = screen_at(&list, 130);
        let enter = list.command(&Action::Confirm);

        let model_row = wide
            .lines()
            .find(|row| row.contains("Model: opus"))
            .unwrap();
        let list_row = wide
            .lines()
            .find(|row| row.contains("[A] Running"))
            .unwrap();
        assert!(model_row.contains('│'), "{wide}");
        assert!(list_row.contains('│'), "{wide}");
        assert!(matches!(enter, Some(TaskListCommand::Open { .. })));
    }

    #[test]
    fn the_pane_switch_is_at_the_wide_width_constant() {
        let list = selected_running();

        let at_limit = screen_at(&list, WIDE_WIDTH);
        let over = screen_at(&list, WIDE_WIDTH + 1);

        assert!(!at_limit.contains("Model: opus"));
        assert!(over.contains("Model: opus"));
    }
}
