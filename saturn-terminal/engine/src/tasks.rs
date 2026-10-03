//! 작업 목록 조회: 채팅을 가로지르는 메인 작업을 이름, 묶음, 상태와 함께 보낸다.
//! 설계: docs/design/tui.md

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use saturn_core::agents::TreeStatus;
use saturn_core::queue::{TaskInfo, TaskPhase};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{Notification, TaskListItem};
use saturn_protocol::state::TaskState;

use crate::rpc::ClientId;
use crate::{Engine, EngineError};

/// 채팅 이름과 묶음, 폴더. 채팅마다 한 번만 읽는다.
struct ChatLabels {
    name: String,
    group: Option<String>,
    folder: String,
}

impl Engine {
    /// 작업 글자가 있는 메인 작업만 보낸다. 채팅 번호, 작업 번호 순서다.
    ///
    /// # Errors
    /// 기록 저장소를 읽지 못하면 `Store`.
    pub(super) async fn send_task_list(&self, client: ClientId) -> Result<(), EngineError> {
        let mut tasks = self.queue.main_tasks();
        tasks.sort_by_key(|info| (info.chat, info.task));
        let mut chats: HashMap<ChatId, ChatLabels> = HashMap::new();
        let mut items = Vec::new();
        for info in tasks {
            let (Some(label), Some(state)) =
                (self.flow.tasks.label(info.task), self.task_state(info))
            else {
                continue;
            };
            let chat = match chats.entry(info.chat) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(self.chat_labels_of(info.chat).await?),
            };
            items.push(TaskListItem {
                chat: info.chat,
                chat_name: chat.name.clone(),
                group: chat.group.clone(),
                task: info.task,
                label,
                state,
                needs_permission: self.has_pending_permission(info),
                busy_elsewhere: false,
                children: self.children_of(info),
                folder: Some(chat.folder.clone()),
            });
        }
        self.send(client, Notification::TaskList { items }).await;
        Ok(())
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
