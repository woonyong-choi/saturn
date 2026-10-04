//! 작업 목록 조회: 채팅을 가로지르는 메인 작업을 이름, 묶음, 상태와 함께 보낸다.
//! 설계: docs/design/tui.md

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

use saturn_core::agents::TreeStatus;
use saturn_core::queue::{TaskInfo, TaskPhase};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{Notification, TaskListItem};
use saturn_protocol::state::TaskState;

use crate::rpc::ClientId;
use crate::store::RunEnd;
use crate::{Engine, EngineError};

/// 채팅 이름과 묶음, 폴더. 채팅마다 한 번만 읽는다.
struct ChatLabels {
    name: String,
    group: Option<String>,
    folder: String,
}

impl ChatLabels {
    /// 채팅 행의 모양. 작업 행은 여기서 작업 칸을 채운다.
    fn item(&self, chat: ChatId) -> TaskListItem {
        TaskListItem {
            chat,
            chat_name: self.name.clone(),
            group: self.group.clone(),
            task: None,
            label: None,
            state: None,
            needs_permission: false,
            busy_elsewhere: false,
            children: 0,
            folder: Some(self.folder.clone()),
            queued: Vec::new(),
            model: None,
            ended_at_ms: None,
        }
    }
}

impl Engine {
    /// 작업 행, 끝난 작업 행, 채팅 행을 채팅 번호, 작업 번호 순서로 보낸다. 채팅 행은 그 채팅의 작업 행 앞에 온다.
    ///
    /// # Errors
    /// 기록 저장소를 읽지 못하면 `Store`.
    pub(super) async fn send_task_list(&self, client: ClientId) -> Result<(), EngineError> {
        let mut tasks = self.queue.main_tasks();
        tasks.sort_by_key(|info| (info.chat, info.task));
        let mut chats: HashMap<ChatId, ChatLabels> = HashMap::new();
        let mut items = Vec::new();
        for info in &tasks {
            let (Some(label), Some(state)) =
                (self.flow.tasks.label(info.task), self.task_state(*info))
            else {
                continue;
            };
            let labels = self.labels_for(&mut chats, info.chat).await?;
            items.push(TaskListItem {
                task: Some(info.task),
                label: Some(label),
                state: Some(state),
                needs_permission: self.has_pending_permission(*info),
                children: self.children_of(*info),
                model: self.task_model(*info),
                ..labels.item(info.chat)
            });
        }
        self.add_ended_tasks(&mut items, &tasks, &mut chats).await?;
        self.add_waiting_inputs(&mut items, &mut chats).await?;
        self.add_chat_rows(&mut items, &mut chats).await?;
        items.sort_by_key(|item| (item.chat, item.task));
        self.send(client, Notification::TaskList { items }).await;
        Ok(())
    }

    /// 마지막 실행이 완료나 실패로 끝난 작업. 도는 작업, 시작을 기다리는 작업, 보류는 끝난 작업이 아니라 뺀다.
    /// 턴을 마치고 쉬는 작업은 마지막 실행이 끝났으므로 끝난 작업이다.
    async fn add_ended_tasks(
        &self,
        items: &mut Vec<TaskListItem>,
        alive: &[TaskInfo],
        chats: &mut HashMap<ChatId, ChatLabels>,
    ) -> Result<(), EngineError> {
        for ended in self.store.ended_tasks().await? {
            let is_unfinished = alive
                .iter()
                .any(|info| info.task == ended.task && info.phase != TaskPhase::Idle);
            if is_unfinished {
                continue;
            }
            let state = match ended.end {
                RunEnd::Completed => TaskState::Done,
                RunEnd::Failed => TaskState::Failed,
                RunEnd::Stopped => continue,
            };
            let labels = self.labels_for(chats, ended.chat).await?;
            items.push(TaskListItem {
                task: Some(ended.task),
                state: Some(state),
                ended_at_ms: Some(ended.ended_at_ms),
                ..labels.item(ended.chat)
            });
        }
        Ok(())
    }

    /// 갈 작업 행이 있으면 그 행에, 없으면 그 채팅의 채팅 행에 붙인다.
    async fn add_waiting_inputs(
        &self,
        items: &mut Vec<TaskListItem>,
        chats: &mut HashMap<ChatId, ChatLabels>,
    ) -> Result<(), EngineError> {
        for (chat, input, target) in self.queue.waiting_inputs() {
            let row = items
                .iter()
                .position(|item| item.chat == chat && target.is_some() && item.task == target);
            match row {
                Some(index) => items[index].queued.push(input),
                None => self.chat_row(items, chats, chat).await?.queued.push(input),
            }
        }
        Ok(())
    }

    /// 작업 행도 끝난 작업 행도 없는 채팅마다 채팅 행 하나.
    async fn add_chat_rows(
        &self,
        items: &mut Vec<TaskListItem>,
        chats: &mut HashMap<ChatId, ChatLabels>,
    ) -> Result<(), EngineError> {
        let with_rows: HashSet<ChatId> = items.iter().map(|item| item.chat).collect();
        for stored in self.store.list_chats(None).await? {
            if !with_rows.contains(&stored.chat) {
                self.chat_row(items, chats, stored.chat).await?;
            }
        }
        Ok(())
    }

    /// 그 채팅의 채팅 행. 없으면 만든다.
    async fn chat_row<'a>(
        &self,
        items: &'a mut Vec<TaskListItem>,
        chats: &mut HashMap<ChatId, ChatLabels>,
        chat: ChatId,
    ) -> Result<&'a mut TaskListItem, EngineError> {
        let index = match items
            .iter()
            .position(|item| item.chat == chat && item.task.is_none())
        {
            Some(index) => index,
            None => {
                let labels = self.labels_for(chats, chat).await?;
                items.push(labels.item(chat));
                items.len() - 1
            }
        };
        Ok(&mut items[index])
    }

    async fn labels_for<'a>(
        &self,
        chats: &'a mut HashMap<ChatId, ChatLabels>,
        chat: ChatId,
    ) -> Result<&'a ChatLabels, EngineError> {
        Ok(match chats.entry(chat) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(self.chat_labels_of(chat).await?),
        })
    }

    /// 그 작업의 session이 지금 쓰는 모델. session을 열지 않았거나 모델을 고르지 않았으면 `None`.
    fn task_model(&self, info: TaskInfo) -> Option<String> {
        let agent = info.agent?;
        let live = self.flow.live.values().find(|live| live.agent == agent)?;
        self.sessions.get(live.session)?.model.clone()
    }

    /// 이름이 없는 채팅은 `#채팅 번호`로 보인다. `--resume` 목록의 번호 표기와 같다.
    async fn chat_labels_of(&self, chat: ChatId) -> Result<ChatLabels, EngineError> {
        let (name, group) = self.store.chat_labels(chat).await?;
        let folder = self.store.chat_workdir(chat).await?;
        Ok(ChatLabels {
            name: name.unwrap_or_else(|| format!("#{}", chat.0)),
            group,
            folder: folder.display().to_string(),
        })
    }

    /// 시작을 기다리는 작업과 쉬는 작업은 보이지 않는다(`None`).
    fn task_state(&self, info: TaskInfo) -> Option<TaskState> {
        if self.flow.needs_check.contains_key(&info.task) {
            return Some(TaskState::NeedsCheck);
        }
        match info.phase {
            TaskPhase::Held => return Some(TaskState::Held),
            TaskPhase::Pending | TaskPhase::Idle | TaskPhase::Closed => return None,
            TaskPhase::Running => {}
        }
        if self.has_pending_permission(info) {
            return Some(TaskState::AwaitingPermission);
        }
        if self
            .flow
            .inputs
            .values()
            .any(|pending| pending.task == info.task)
        {
            return Some(TaskState::AwaitingInput);
        }
        let answered = info
            .agent
            .and_then(|agent| self.agents.status(agent))
            .is_some_and(|status| status == TreeStatus::AnsweredTreeRunning);
        Some(if answered {
            TaskState::AnsweredTreeRunning
        } else {
            TaskState::Running
        })
    }

    fn has_pending_permission(&self, info: TaskInfo) -> bool {
        self.flow
            .permissions
            .values()
            .any(|pending| pending.task == info.task)
    }

    /// 지금 도는 하위 에이전트 수. 자식 채팅은 아직 없다.
    fn children_of(&self, info: TaskInfo) -> u32 {
        let running = info
            .agent
            .map_or(0, |agent| self.agents.running_subagents(agent));
        u32::try_from(running).unwrap_or(u32::MAX)
    }
}
