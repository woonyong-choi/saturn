//! 입력 흐름이 메모리에 두는 값과 TUI 알림. 정본은 기록 저장소와 `core` 대기열이다.
//! 설계: docs/design/input-handling.md

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use saturn_core::routers::RouterRequest;
use saturn_protocol::ids::{
    AgentId, ChatId, ChatRevision, InputId, JudgmentId, Provider, ProviderSessionId, RunId,
    SessionId, SettingsRevision, TaskId, TaskLabel,
};
use saturn_protocol::rpc::{Alert, Notification};
use saturn_protocol::state::{Disposition, TaskState};
use tokio::sync::mpsc;

use crate::Engine;
use crate::events::PendingPermission;
use crate::routers::{RecordContext, RouterExchange};
use crate::stop::{HeldTask, StopDone, StopProgress};

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
pub(crate) struct Routed {
    pub(crate) judgment: Option<JudgmentId>,
    pub(crate) disposition: Disposition,
}

/// 적용 결과를 알기 전이라 아직 기록하지 않은 판단. revision이 어긋나면 `Superseded`로 쓴다.
pub(crate) struct Unrecorded {
    pub(crate) context: RecordContext,
    pub(crate) exchange: RouterExchange,
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

/// 돌고 있는 router 호출 하나(접수한 입력의 처리 방식 판단). 요청을 만들 때의 채팅 revision을 들고 있어 결과를
/// 적용할 때 비교한다. `retried`는 revision이 어긋나 다시 묻는 호출이면 참이다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouterJob {
    pub(crate) chat: ChatId,
    pub(crate) input: InputId,
    pub(crate) revision: ChatRevision,
    pub(crate) retried: bool,
}

/// 별도 작업이 engine 루프로 돌려주는 호출 결과.
pub(crate) struct RouterDone {
    pub(crate) job: RouterJob,
    pub(crate) request: RouterRequest,
    pub(crate) exchange: RouterExchange,
}

pub(crate) struct FlowState {
    pub(crate) routed: HashMap<InputId, Routed>,
    /// 채팅마다 판단 중인 접수 입력. 같은 채팅 입력은 하나씩만 판단한다.
    pub(crate) judging: HashMap<ChatId, InputId>,
    pub(crate) router_tx: mpsc::UnboundedSender<RouterDone>,
    pub(crate) router_rx: mpsc::UnboundedReceiver<RouterDone>,
    /// 채팅의 가장 나중 판단이 정한 처리 방식. 다음 판단의 state에 넣는다.
    pub(crate) last_disposition: HashMap<ChatId, Disposition>,
    pub(crate) unrecorded: HashMap<InputId, Unrecorded>,
    /// 판단을 적용한 뒤 아직 보내기 판정을 거치지 않은 입력. 대기 사유가 정해진 뒤 한 번 알린다.
    pub(crate) applied: Vec<InputId>,
    pub(crate) tasks: TaskBook,
    /// 키는 에이전트. provider를 연 뒤에만 들어간다.
    pub(crate) live: HashMap<AgentId, LiveSession>,
    /// 에이전트의 가장 나중 실행. 턴이 끝난 뒤 늦게 오는 사용량 같은 이벤트를 붙인다.
    pub(crate) last_run: HashMap<AgentId, RunId>,
    /// 입력 없이 시작한 턴(`provider-wake`)을 어느 작업 아래에 기록할지 정하는 데 쓴다.
    pub(crate) last_task: HashMap<AgentId, TaskId>,
    /// 가장 나중에 보고한 활성 맥락 `A`. 보고하지 않았으면 `None`.
    pub(crate) context_tokens: HashMap<AgentId, Option<u64>>,
    /// 패킷 턴 수. 그 턴의 완료는 작업 끝이 아니다.
    pub(crate) packet_turns: HashMap<AgentId, u32>,
    /// 답을 기다리는 허가 요청. 키는 `request_id`.
    pub(crate) permissions: HashMap<String, PendingPermission>,
    /// 에이전트가 가장 나중에 시작한 입력의 설정 번호. 허가 요청 판정이 그 번호의 규칙을 쓴다.
    pub(crate) settings_of: HashMap<AgentId, SettingsRevision>,
    /// 채팅의 Codex 연결을 시작할 때 쓴 규칙 지문. 연결이 없으면 항목도 없다.
    pub(crate) rules_of_connection: HashMap<ChatId, String>,
    /// 연결이 시작할 때 읽은 규칙과 최신 규칙이 달라 다음 턴 끝에 연결을 다시 시작할 채팅.
    pub(crate) rules_stale: HashSet<ChatId>,
    /// 보낸 뒤 결과를 모르는 작업.
    pub(crate) needs_check: HashMap<TaskId, NeedsCheck>,
    /// 사용자가 정한 다음 provider. 그 provider의 session이 열리면 지운다.
    pub(crate) switch_to: HashMap<ChatId, Provider>,
    pub(crate) stopping: HashMap<ChatId, StopProgress>,
    /// 보류한 작업이 멈출 때 하던 일. 재개나 보류 종료 때 지운다.
    pub(crate) held: HashMap<TaskId, HeldTask>,
    pub(crate) stop_tx: mpsc::UnboundedSender<StopDone>,
    pub(crate) stop_rx: mpsc::UnboundedReceiver<StopDone>,
}

/// 보낸 뒤 결과를 모르는 작업의 입력과 에이전트.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NeedsCheck {
    pub(crate) chat: ChatId,
    pub(crate) agent: AgentId,
    pub(crate) input: InputId,
}

impl Default for FlowState {
    fn default() -> Self {
        let (router_tx, router_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = mpsc::unbounded_channel();
        Self {
            last_run: HashMap::new(),
            last_task: HashMap::new(),
            context_tokens: HashMap::new(),
            packet_turns: HashMap::new(),
            permissions: HashMap::new(),
            settings_of: HashMap::new(),
            rules_of_connection: HashMap::new(),
            rules_stale: HashSet::new(),
            needs_check: HashMap::new(),
            switch_to: HashMap::new(),
            stopping: HashMap::new(),
            held: HashMap::new(),
            stop_tx,
            stop_rx,
            routed: HashMap::new(),
            judging: HashMap::new(),
            router_tx,
            router_rx,
            last_disposition: HashMap::new(),
            unrecorded: HashMap::new(),
            applied: Vec::new(),
            tasks: TaskBook::default(),
            live: HashMap::new(),
        }
    }
}

impl std::fmt::Debug for FlowState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FlowState")
            .field("routed", &self.routed.len())
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
