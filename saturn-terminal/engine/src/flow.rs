//! 입력 흐름이 메모리에 두는 값과 TUI 알림. 정본은 기록 저장소와 `core` 대기열이다.
//! 설계: docs/design/input-handling.md

use std::collections::HashMap;
use std::time::Instant;

use saturn_protocol::ids::{
    AgentId, ChatId, InputId, JudgmentId, Provider, ProviderSessionId, SessionId, TaskId, TaskLabel,
};
use saturn_protocol::rpc::{Alert, Notification};
use saturn_protocol::state::{Disposition, TaskState};

use crate::Engine;
use crate::judges::{JudgeExchange, RecordContext};

/// 열려 있는 provider session 하나.
#[derive(Debug, Clone)]
pub(crate) struct LiveSession {
    pub(crate) agent: AgentId,
    pub(crate) session: SessionId,
    pub(crate) provider: Provider,
    pub(crate) provider_session: ProviderSessionId,
    /// 거짓이면 끼워 넣기를 대기로 바꾼다.
    pub(crate) steer_verified: bool,
}

/// 판단을 받아 적용한 입력. 사용자가 판단을 뒤집을 때 결과 신호를 알리는 데 쓴다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Judged {
    pub(crate) judgment: Option<JudgmentId>,
    pub(crate) disposition: Disposition,
}

/// 적용 결과를 알기 전이라 아직 기록하지 않은 판단. revision이 어긋나면 `Superseded`로 쓴다.
pub(crate) struct Unrecorded {
    pub(crate) context: RecordContext,
    pub(crate) exchange: JudgeExchange,
}

/// 작업 글자. `A`부터 쓰고 끝난 작업의 글자는 비어 있는 가장 앞 글자로 다시 쓴다.
/// 글자가 26개를 넘으면 마지막 글자를 함께 쓴다. 초안.
#[derive(Debug, Default)]
pub(crate) struct TaskBook {
    entries: HashMap<TaskId, (TaskLabel, Instant)>,
}

impl TaskBook {
    /// 이미 있으면 그 글자를 돌려준다.
    pub(crate) fn assign(&mut self, task: TaskId) -> TaskLabel {
        if let Some((label, _)) = self.entries.get(&task) {
            return *label;
        }
        let taken: Vec<char> = self.entries.values().map(|(label, _)| label.0).collect();
        let letter = ('A'..='Z')
            .find(|letter| !taken.contains(letter))
            .unwrap_or('Z');
        let label = TaskLabel(letter);
        self.entries.insert(task, (label, Instant::now()));
        label
    }

    pub(crate) fn label(&self, task: TaskId) -> Option<TaskLabel> {
        self.entries.get(&task).map(|(label, _)| *label)
    }

    pub(crate) fn elapsed_ms(&self, task: TaskId) -> u64 {
        self.entries.get(&task).map_or(0, |(_, since)| {
            u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
        })
    }

    pub(crate) fn release(&mut self, task: TaskId) {
        self.entries.remove(&task);
    }
}

#[derive(Default)]
pub(crate) struct FlowState {
    pub(crate) judged: HashMap<InputId, Judged>,
    /// 채팅의 가장 나중 판단이 정한 처리 방식. 다음 판단의 state에 넣는다.
    pub(crate) last_disposition: HashMap<ChatId, Disposition>,
    pub(crate) unrecorded: HashMap<InputId, Unrecorded>,
    /// 판단을 적용한 뒤 아직 보내기 판정을 거치지 않은 입력. 대기 사유가 정해진 뒤 한 번 알린다.
    pub(crate) applied: Vec<InputId>,
    pub(crate) tasks: TaskBook,
    /// 키는 에이전트. provider를 연 뒤에만 들어간다.
    pub(crate) live: HashMap<AgentId, LiveSession>,
}

impl std::fmt::Debug for FlowState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowState")
            .field("judged", &self.judged.len())
            .field("unrecorded", &self.unrecorded.len())
            .field("live", &self.live.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    /// 입력의 지금 상태를 그 채팅에 붙은 모든 TUI에 보낸다.
    pub(crate) async fn notify_input(&self, input: InputId) {
        let Some(record) = self.queue.input(input) else {
            return;
        };
        let label = record.task.and_then(|task| self.flow.tasks.label(task));
        let notification = Notification::InputChanged {
            input,
            text: record.text.clone(),
            label,
            state: record.state,
            disposition: self.queue.disposition(input),
            reason: record.reason,
        };
        self.rpc.broadcast(Some(record.chat), notification).await;
    }

    pub(crate) async fn notify_task(
        &self,
        chat: ChatId,
        task: TaskId,
        state: TaskState,
        provider: Option<Provider>,
        failure: Option<String>,
    ) {
        let Some(label) = self.flow.tasks.label(task) else {
            return;
        };
        let notification = Notification::TaskChanged {
            task,
            label,
            state,
            provider,
            elapsed_ms: self.flow.tasks.elapsed_ms(task),
            failure,
        };
        self.rpc.broadcast(Some(chat), notification).await;
    }

    pub(crate) async fn notify_alert(&self, chat: ChatId, alert: Alert) {
        self.rpc
            .broadcast(Some(chat), Notification::Alert { alert })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_reuse_the_first_free_letter() {
        let mut book = TaskBook::default();

        let a = book.assign(TaskId(1));
        let b = book.assign(TaskId(2));
        book.release(TaskId(1));
        let c = book.assign(TaskId(3));

        assert_eq!((a, b, c), (TaskLabel('A'), TaskLabel('B'), TaskLabel('A')));
        assert_eq!(book.assign(TaskId(2)), TaskLabel('B'));
        assert_eq!(book.label(TaskId(9)), None);
    }

    #[test]
    fn labels_share_the_last_letter_after_twenty_six() {
        let mut book = TaskBook::default();
        for id in 0..26 {
            book.assign(TaskId(id));
        }

        assert_eq!(book.assign(TaskId(100)), TaskLabel('Z'));
    }
}
