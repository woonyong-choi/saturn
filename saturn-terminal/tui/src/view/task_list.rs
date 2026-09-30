//! 작업 목록 화면(`/tasks`). 채팅을 가로지르는 작업을 필터와 묶음으로 본다.
//!
//! 설계: docs/design/tui.md(영역 작업 목록 화면, 키 작업 목록 화면, 상태 표시 `!`, `?`, `다른 Saturn에서 실행 중`, `모델 미보고`).
//! - 행: 묶음, 채팅, 폴더, 상태, 작업과 그 아래 subagent와 자식 채팅. 허가 필요 작업은 `!`, 결과 확인 필요 작업은 `?`.
//! - 다른 Saturn 프로세스가 실행 중인 채팅은 `다른 Saturn에서 실행 중`이고 읽기 전용(`c`, `d`, `s`, `r`, `g` 무시).
//! - engine 상태가 바뀌어 목록을 다시 받아도 선택한 작업을 유지한다.
//! TODO(#46): `Request::ListTasks`의 응답 알림이 protocol에 없다. 새 채팅, 이름 변경, 묶음 변경 요청도 없다

use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use saturn_protocol::ids::{ChatId, InputId, SubagentId, TaskId, TaskLabel};
use saturn_protocol::state::TaskState;

use crate::i18n::Lang;

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
    /// 다음 필터.
    pub fn next(self) -> Self {
        todo!("#92")
    }

    /// 이전 필터.
    pub fn prev(self) -> Self {
        todo!("#92")
    }

    /// 필터 이름(`전체`, `확인 필요`, `실행 중`, `대기`, `보류`, `끝남`).
    pub fn text(self, lang: Lang) -> &'static str {
        todo!("#92")
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
    /// 대기 입력이 있으면 그 입력(`s` 전송, `d` 취소 대상).
    pub queued_input: Option<InputId>,
    /// 보고된 모델. 없으면 상세에 `모델 미보고`.
    pub model: Option<String>,
    /// 그 아래 subagent.
    pub subagents: Vec<SubagentId>,
    /// 그 아래 자식 채팅.
    pub child_chats: Vec<ChatId>,
}

/// 작업 목록의 채팅 묶음.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGroup {
    /// 묶음 이름.
    pub group: String,
    /// 채팅.
    pub chat: ChatId,
    /// 채팅 이름.
    pub name: String,
    /// 폴더.
    pub folder: PathBuf,
    /// 다른 Saturn 프로세스가 실행 중(읽기 전용).
    pub busy_elsewhere: bool,
    /// 작업.
    pub tasks: Vec<TaskRow>,
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

/// 작업 목록 화면 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskList {
    /// 받은 목록.
    pub groups: Vec<ChatGroup>,
    /// 필터.
    pub filter: TaskFilter,
    /// `f` 검색어. 채팅 이름, 폴더, 작업 원문에 포함되면 남긴다.
    pub query: Option<String>,
    /// 선택한 작업. 목록을 다시 받아도 유지한다.
    pub selected: Option<(ChatId, TaskId)>,
    /// 확인 한 줄(`d` 보류 종료) 또는 입력 한 줄(`r`, `g`, `f`) 대기 중.
    pub pending: Option<TaskListCommand>,
    /// `?` 도움말을 보이는 중.
    pub help: bool,
}

impl TaskList {
    /// 목록을 바꾼다. 선택한 작업이 새 목록에 있으면 유지하고, 없으면 같은 자리의 행을 고른다.
    pub fn replace(&mut self, groups: Vec<ChatGroup>) {
        todo!("#92")
    }

    /// 필터와 검색어를 거친 행(채팅, 작업).
    pub fn visible_rows(&self) -> Vec<(&ChatGroup, &TaskRow)> {
        todo!("#92")
    }

    /// `↑` 이동.
    pub fn up(&mut self) {
        todo!("#92")
    }

    /// `↓` 이동.
    pub fn down(&mut self) {
        todo!("#92")
    }

    /// 선택한 행의 키 동작. 읽기 전용 채팅이면 `Open`만 돌려주고 나머지는 `None`.
    /// `d`: 대기 입력이 있으면 `CancelInput`, 보류면 확인 한 줄을 띄우고 확인 뒤 `CloseHeld`.
    pub fn command(&mut self, action: &crate::keys::Action) -> Option<TaskListCommand> {
        todo!("#92")
    }

    /// 행 머리 표시. 허가 필요(`AwaitingPermission`) `!`, 결과 확인 필요(`NeedsCheck`) `?`, 그 밖에는 공백.
    pub fn marker(state: TaskState) -> char {
        todo!("#92")
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
    /// 위에 필터 줄, 가운데 묶음 › 채팅(폴더, 상태) › 작업 › subagent·자식 채팅 트리, 오른쪽이나 아래에 선택 작업 상세
    /// (모델 또는 `모델 미보고`), 읽기 전용 채팅은 `다른 Saturn에서 실행 중`. 도움말 중이면 키 안내를 덮어 그린다.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        todo!("#92")
    }
}
