//! 작업 목록 화면(`/tasks`). 채팅을 가로지르는 작업을 필터와 묶음으로 본다.
//!
//! 설계: docs/design/tui.md(영역 작업 목록 화면, 키 작업 목록 화면, 상태 표시 `!`, `?`, `다른 Saturn에서 실행 중`, `모델 미보고`).
//! - 행: 묶음, 채팅, 상태, 작업과 그 아래 subagent와 자식 채팅 수. 허가 필요 작업은 `!`, 결과 확인 필요 작업은 `?`.
//! - 다른 Saturn 프로세스가 실행 중인 채팅은 `다른 Saturn에서 실행 중`이고 읽기 전용(`c`, `d`, `s`, `r`, `g` 무시).
//! - engine 상태가 바뀌어 목록을 다시 받아도 선택한 작업을 유지한다.
//!
//! 목록은 `Request::ListTasks` → `Notification::TaskList`. 새 채팅은 `Request::Attach { chat: None }`,
//! 이름 변경은 `Request::RenameChat`, 묶음 변경은 `Request::SetChatGroup`.
//! protocol `TaskListItem`에 폴더, 대기 입력, 모델이 없어 그 칸은 비워 둔다(`Notification::TaskList`가 싣게 되면 채운다).

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use saturn_protocol::ids::{ChatId, InputId, TaskId, TaskLabel};
use saturn_protocol::rpc::TaskListItem;
use saturn_protocol::state::TaskState;

use crate::i18n::{self, Lang};
use crate::keys::Action;
use crate::labels;
use crate::view::{EMPHASIS, MUTED, SELECTED, truncate, window_block};

/// 필터. `Tab`은 다음, `Shift+Tab`은 이전, 끝에서 처음으로 돈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskFilter {
    /// `전체`.
    #[default]
    All,
    /// `확인 필요`(`NeedsCheck`, 허가 필요 포함).
    NeedsCheck,
    /// `실행 중`.
    Running,
    /// `대기`.
    Queued,
    /// `보류`.
    Held,
    /// `끝남`(`Done`, `Failed`).
    Done,
}

impl TaskFilter {
    /// 필터 순서.
    const ORDER: [Self; 6] = [
        Self::All,
        Self::NeedsCheck,
        Self::Running,
        Self::Queued,
        Self::Held,
        Self::Done,
    ];

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 다음 필터.
    pub fn next(self) -> Self {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 이전 필터.
    pub fn prev(self) -> Self {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// 필터 이름(`전체`, `확인 필요`, `실행 중`, `대기`, `보류`, `끝남`).
    pub fn text(self, lang: Lang) -> &'static str {
        lang.tr(match self {
            Self::All => i18n::FILTER_ALL,
            Self::NeedsCheck => i18n::FILTER_NEEDS_CHECK,
            Self::Running => i18n::FILTER_RUNNING,
            Self::Queued => i18n::FILTER_QUEUED,
            Self::Held => i18n::FILTER_HELD,
            Self::Done => i18n::FILTER_DONE,
        })
    }

    /// 행이 이 필터에 드는지.
    fn matches(self, row: &TaskRow) -> bool {
        match self {
            Self::All => true,
            Self::NeedsCheck => row.needs_permission || row.state == TaskState::NeedsCheck,
            Self::Running => labels::is_live(row.state),
            Self::Queued => row.queued_input.is_some(),
            Self::Held => row.state == TaskState::Held,
            Self::Done => matches!(row.state, TaskState::Done | TaskState::Failed),
        }
    }
}

/// 작업 목록 한 행의 작업.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    /// 작업.
    pub task: TaskId,
    /// 이름표.
    pub label: TaskLabel,
    /// 상태.
    pub state: TaskState,
    /// 허가가 필요하다(`!`).
    pub needs_permission: bool,
    /// 대기 입력이 있으면 그 입력(`s` 전송, `d` 취소 대상).
    pub queued_input: Option<InputId>,
    /// 보고된 모델. 없으면 상세에 `모델 미보고`.
    pub model: Option<String>,
    /// 그 아래 subagent와 자식 채팅 수.
    pub children: u32,
}

/// 작업 목록의 채팅 묶음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGroup {
    /// 묶음 이름. 없으면 빈 문자열.
    pub group: String,
    /// 채팅.
    pub chat: ChatId,
    /// 채팅 이름.
    pub name: String,
    /// 폴더. 모르면 `None`.
    pub folder: Option<PathBuf>,
    /// 다른 Saturn 프로세스가 실행 중(읽기 전용).
    pub busy_elsewhere: bool,
    /// 작업.
    pub tasks: Vec<TaskRow>,
}

impl ChatGroup {
    // cost: time O(n·c), heap O(n), stack O(1)
    // vars: n = items.len(), c = 채팅 수
    // basis: estimate
    /// `Notification::TaskList`의 행을 채팅별로 묶는다. 채팅 순서는 처음 나온 순서.
    pub fn from_items(items: Vec<TaskListItem>) -> Vec<Self> {
        let mut groups: Vec<Self> = Vec::new();
        for item in items {
            let row = TaskRow {
                task: item.task,
                label: item.label,
                state: item.state,
                needs_permission: item.needs_permission,
                queued_input: None,
                model: None,
                children: item.children,
            };
            match groups.iter_mut().find(|g| g.chat == item.chat) {
                Some(group) => group.tasks.push(row),
                None => groups.push(Self {
                    group: item.group.unwrap_or_default(),
                    chat: item.chat,
                    name: item.chat_name,
                    folder: None,
                    busy_elsewhere: item.busy_elsewhere,
                    tasks: vec![row],
                }),
            }
        }
        groups
    }
}

/// 작업 목록에서 고른 동작. `app::App`이 요청으로 바꾼다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskListCommand {
    /// `Enter` 그 채팅으로 이동해 해당 작업 결과로 스크롤.
    Open { chat: ChatId, task: TaskId },
    /// `c` 보류 작업 재개.
    Continue { chat: ChatId, task: TaskId },
    /// `d` 대기 취소.
    CancelInput(InputId),
    /// `d` 보류면 확인 한 줄 뒤 보류 종료.
    CloseHeld { chat: ChatId, task: TaskId },
    /// `s` 대기 입력 전송.
    SendNow(InputId),
    /// `n` 새 채팅.
    NewChat,
    /// `r` 채팅 이름 변경(입력 한 줄 뒤).
    Rename { chat: ChatId, name: String },
    /// `g` 묶음 변경(입력 한 줄 뒤).
    Regroup { chat: ChatId, group: String },
}

/// 입력 한 줄의 용도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskListInput {
    /// `f` 검색어.
    Search,
    /// `r` 채팅 이름.
    Rename(ChatId),
    /// `g` 묶음.
    Regroup(ChatId),
}

/// 작업 목록 화면 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskList {
    /// 받은 목록.
    pub groups: Vec<ChatGroup>,
    /// 필터.
    pub filter: TaskFilter,
    /// `f` 검색어. 채팅 이름, 묶음, 폴더에 포함되면 남긴다.
    pub query: Option<String>,
    /// 선택한 작업. 목록을 다시 받아도 유지한다.
    pub selected: Option<(ChatId, TaskId)>,
    /// 확인 한 줄(`d` 보류 종료) 대기 중.
    pub pending: Option<TaskListCommand>,
    /// 입력 한 줄(`r`, `g`, `f`)과 지금까지 친 글.
    pub editing: Option<(TaskListInput, String)>,
    /// `?` 도움말을 보이는 중.
    pub help: bool,
}

impl TaskList {
    // cost: time O(r), heap O(r), stack O(1)
    // vars: r = 행 수
    // basis: estimate
    /// 목록을 바꾼다. 선택한 작업이 새 목록에 있으면 유지하고, 없으면 같은 자리의 행을 고른다.
    pub fn replace(&mut self, groups: Vec<ChatGroup>) {
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
    /// 필터와 검색어를 거친 행(채팅, 작업).
    pub fn visible_rows(&self) -> Vec<(&ChatGroup, &TaskRow)> {
        self.groups
            .iter()
            .filter(|group| self.query_matches(group))
            .flat_map(|group| group.tasks.iter().map(move |row| (group, row)))
            .filter(|(_, row)| self.filter.matches(row))
            .collect()
    }

    /// `↑` 이동.
    pub fn up(&mut self) {
        let rows = self.visible_keys();
        let index = self.selected_index().unwrap_or(0).saturating_sub(1);
        self.selected = rows.get(index).copied();
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        let rows = self.visible_keys();
        let index = match self.selected_index() {
            Some(index) => (index + 1).min(rows.len().saturating_sub(1)),
            None => 0,
        };
        self.selected = rows.get(index).copied();
    }

    /// 필터를 바꾸고 선택을 새 목록에 맞춘다.
    pub fn set_filter(&mut self, filter: TaskFilter) {
        self.filter = filter;
        let groups = std::mem::take(&mut self.groups);
        self.replace(groups);
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 선택한 행의 키 동작. 읽기 전용 채팅이면 `Open`, `NewChat`, 검색만 받는다.
    /// `d`: 대기 입력이 있으면 `CancelInput`, 보류면 확인 한 줄을 띄우고 확인(`Enter`) 뒤 `CloseHeld`.
    /// `f`, `r`, `g`는 입력 한 줄을 열고 `Enter`에서 검색어를 적용하거나 명령을 돌려준다.
    pub fn command(&mut self, action: &Action) -> Option<TaskListCommand> {
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
        let (chat, task) = (group.chat, row.task);
        match action {
            Action::ContinueHeld if row.state == TaskState::Held => {
                Some(TaskListCommand::Continue { chat, task })
            }
            Action::CancelOrCloseHeld => match row.queued_input {
                Some(input) => Some(TaskListCommand::CancelInput(input)),
                None if row.state == TaskState::Held => {
                    self.pending = Some(TaskListCommand::CloseHeld { chat, task });
                    None
                }
                None => None,
            },
            Action::SendQueued => row.queued_input.map(TaskListCommand::SendNow),
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

    /// `Esc`: 도움말, 확인 한 줄, 입력 한 줄 중 떠 있는 것 하나를 닫는다. 닫은 것이 없으면 거짓(화면 종료).
    pub fn cancel(&mut self) -> bool {
        if self.help {
            self.help = false;
        } else if self.pending.is_some() {
            self.pending = None;
        } else if self.editing.is_some() {
            self.editing = None;
        } else {
            return false;
        }
        true
    }

    /// 입력 한 줄에 글자 하나.
    pub fn edit_push(&mut self, c: char) {
        if let Some((_, text)) = &mut self.editing {
            text.push(c);
        }
    }

    /// 입력 한 줄 끝 글자 지우기.
    pub fn edit_pop(&mut self) {
        if let Some((_, text)) = &mut self.editing {
            text.pop();
        }
    }

    /// 행 머리 표시. 허가 필요(`AwaitingPermission`) `!`, 결과 확인 필요(`NeedsCheck`) `?`, 그 밖에는 공백.
    pub fn marker(state: TaskState) -> char {
        match state {
            TaskState::AwaitingPermission => '!',
            TaskState::NeedsCheck => '?',
            _ => ' ',
        }
    }

    /// `Enter`: 확인 한 줄 확정, 입력 한 줄 확정, 그 밖에는 선택 작업 열기.
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
        Some(TaskListCommand::Open {
            chat: group.chat,
            task: row.task,
        })
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 선택한 행.
    fn selected_row(&self) -> Option<(&ChatGroup, &TaskRow)> {
        let selected = self.selected?;
        self.visible_rows()
            .into_iter()
            .find(|(group, row)| (group.chat, row.task) == selected)
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 선택한 행의 자리.
    fn selected_index(&self) -> Option<usize> {
        let selected = self.selected?;
        self.visible_keys().iter().position(|key| *key == selected)
    }

    // cost: time O(r·q), heap O(r), stack O(1)
    // vars: r = 행 수, q = 검색어 길이
    // basis: estimate
    /// 보이는 행의 (채팅, 작업).
    fn visible_keys(&self) -> Vec<(ChatId, TaskId)> {
        self.visible_rows()
            .into_iter()
            .map(|(group, row)| (group.chat, row.task))
            .collect()
    }

    // cost: time O(m·q), heap O(m), stack O(1)
    // vars: m = 채팅 이름·묶음·폴더 길이, q = 검색어 길이
    // basis: estimate
    /// 검색어가 채팅 이름, 묶음, 폴더에 들어 있는지. 검색어가 없으면 참.
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

/// 작업 목록 화면 그리기.
#[derive(Debug)]
pub struct TaskListView<'a> {
    /// 화면 상태.
    pub list: &'a TaskList,
    /// 화면 언어.
    pub lang: Lang,
}

impl TaskListView<'_> {
    /// 위에 필터 줄, 가운데 묶음 › 채팅 › 작업 행(하위 항목 수), 아래에 선택 작업 상세(모델 또는 `모델 미보고`),
    /// 확인·입력 한 줄. 읽기 전용 채팅은 `다른 Saturn에서 실행 중`. 도움말 중이면 키 안내를 끝 줄에 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = window_block(self.lang.tr(i18n::TASKS_TITLE));
        let inner = block.inner(area);
        frame.render_widget(Clear, area);
        frame.render_widget(block, area);
        let width = usize::from(inner.width);
        let mut lines = vec![self.filter_line(), Line::from("")];
        lines.extend(self.row_lines(width));
        lines.push(Line::from(""));
        lines.extend(self.footer_lines(width));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    /// 필터 줄. 지금 필터를 강조한다.
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
            .collect();
        Line::from(spans)
    }

    // cost: time O(r), heap O(r), stack O(1)
    // vars: r = 보이는 행 수
    // basis: estimate
    /// 묶음 › 채팅 머리와 작업 행.
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
            let text = format!(
                "  {} {} {}",
                TaskList::marker(row.state),
                labels::format(row.label),
                state_text(self.lang, row)
            );
            lines.push(Line::from(Span::styled(truncate(&text, width), style)));
        }
        lines
    }

    /// 채팅 머리 `묶음 › 채팅 이름 · 다른 Saturn에서 실행 중`.
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

    /// 상세, 확인 한 줄, 입력 한 줄, 도움말.
    fn footer_lines(&self, width: usize) -> Vec<Line<'static>> {
        let lang = self.lang;
        let mut lines = Vec::new();
        if let Some((_, row)) = self.list.selected_row() {
            let model = row
                .model
                .clone()
                .unwrap_or_else(|| lang.tr(i18n::MODEL_UNREPORTED).to_string());
            lines.push(Line::from(format!(
                "{}: {model} · {}: {}",
                lang.tr(i18n::TASKS_MODEL),
                lang.tr(i18n::TASKS_CHILDREN),
                row.children
            )));
        }
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

/// 행 상태 문구.
fn state_text(lang: Lang, row: &TaskRow) -> &'static str {
    lang.tr(match row.state {
        TaskState::Running | TaskState::AnsweredTreeRunning => i18n::FILTER_RUNNING,
        TaskState::AwaitingPermission => i18n::AWAITING_PERMISSION,
        TaskState::Held => i18n::FILTER_HELD,
        TaskState::NeedsCheck => i18n::FILTER_NEEDS_CHECK,
        TaskState::Done => i18n::FILTER_DONE,
        TaskState::Failed => i18n::FAILED,
    })
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
            task: TaskId(task),
            label: TaskLabel(label),
            state,
            needs_permission: state == TaskState::AwaitingPermission,
            busy_elsewhere: chat == 9,
            children: 0,
        }
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
        assert_eq!(TaskFilter::All.prev(), TaskFilter::Done);
        assert_eq!(TaskFilter::Done.next(), TaskFilter::All);
    }

    #[test]
    fn set_filter_limits_rows() {
        let mut list = list();

        list.set_filter(TaskFilter::Held);

        assert_eq!(list.visible_rows().len(), 2);
        assert_eq!(list.selected, Some((ChatId(1), TaskId(2))));
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
                task: TaskId(4)
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

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn render_shows_filters_rows_and_busy_chat() {
        let list = list();
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
        assert!(content.contains("all"));
        assert!(content.contains("[B] held"));
        assert!(content.contains("running in another Saturn"));
        assert!(content.contains("model not reported"));
    }
}
