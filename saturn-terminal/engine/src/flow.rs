//! 입력 흐름이 메모리에 두는 값과 TUI 알림. 정본은 기록 저장소와 `core` 대기열이다.
//! 설계: docs/design/input-handling.md

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use saturn_core::routers::RouterRequest;
use saturn_protocol::ids::{
    AgentId, ChatId, ChatRevision, InputId, JudgmentId, Provider, ProviderSessionId, RunId,
    SessionId, SettingsRevision, SubagentId, TaskId, TaskLabel,
};
use saturn_protocol::rpc::{Alert, CommandInfo, ModelInfo, Notification};
use saturn_protocol::state::{Disposition, TaskState};
use tokio::sync::mpsc;

use crate::Engine;
use crate::delivery::Parked;
use crate::events::PendingPermission;
use crate::inputs::PendingInput;
use crate::providers::ProviderMsg;
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
    /// provider 연결에서 받아 둔 모델 목록. `target_model` 후보이고 `/model` 목록과 같다. 목록을 받기 전이면 항목이 없다.
    pub(crate) models: HashMap<(ChatId, Provider), Vec<ModelInfo>>,
    /// 연결이 마지막으로 알린 명령 목록. 나중에 붙는 TUI에 그대로 보낸다.
    pub(crate) commands: HashMap<(ChatId, Provider), Vec<CommandInfo>>,
    /// 시작 때 읽은 provider CLI 버전. 읽지 못한 provider는 항목이 없다.
    pub(crate) cli_versions: HashMap<Provider, String>,
    pub(crate) router_tx: mpsc::UnboundedSender<RouterDone>,
    pub(crate) router_rx: mpsc::UnboundedReceiver<RouterDone>,
    /// 채팅의 가장 나중 판단이 정한 처리 방식. 다음 판단의 state에 넣는다.
    pub(crate) last_disposition: HashMap<ChatId, Disposition>,
    pub(crate) unrecorded: HashMap<InputId, Unrecorded>,
    /// 판단을 적용한 뒤 아직 보내기 판정을 거치지 않은 입력. 대기 사유가 정해진 뒤 한 번 알린다.
    pub(crate) applied: Vec<InputId>,
    /// 적용한 판단의 채팅과 `resume_held`. 판단이 없으면 `None`. `on_routed`가 접수 순서대로 보류 작업에 반영한다.
    pub(crate) resume_signals: Vec<(ChatId, Option<bool>)>,
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
    /// 답을 기다리는 허가 요청. 키는 engine이 발급해 TUI에 보낸 요청 ID이고 provider 요청 ID와 다르다.
    pub(crate) permissions: HashMap<String, PendingPermission>,
    /// 답을 기다리는 입력 요청. 키는 허가 요청과 같은 engine 요청 ID.
    pub(crate) inputs: HashMap<String, PendingInput>,
    /// 다음에 발급할 요청 번호. 허가 요청과 입력 요청이 함께 쓴다.
    next_request: u64,
    /// 에이전트가 가장 나중에 시작한 입력의 설정 번호. 허가 요청 판정이 그 번호의 규칙을 쓴다.
    pub(crate) settings_of: HashMap<AgentId, SettingsRevision>,
    /// 채팅의 연결을 시작할 때 쓴 규칙 지문. 규칙이 연결을 시작할 때 고정되는 어댑터만 항목이 있다. 연결이 없으면 항목도 없다.
    pub(crate) rules_of_connection: HashMap<(ChatId, Provider), String>,
    /// 바뀐 설정을 적용하려고 다시 시작할 연결. 채팅에 작업이 있으면 턴 끝에 시작한다.
    pub(crate) stale_connections: HashSet<(ChatId, Provider)>,
    /// 연결을 시작할 때 쓴 에이전트 질문 기능 값(켬이 참).
    pub(crate) questions_of_connection: HashMap<(ChatId, Provider), bool>,
    /// 설정 파일 감시가 채팅마다 마지막으로 본 바뀐 지문.
    pub(crate) watched_settings: crate::settings_watch::WatchedSettings,
    /// 보낸 뒤 결과를 모르는 작업.
    pub(crate) needs_check: HashMap<TaskId, NeedsCheck>,
    /// 사용자가 정한 다음 provider. 그 provider의 session이 열리면 지운다.
    pub(crate) switch_to: HashMap<ChatId, Provider>,
    pub(crate) stopping: HashMap<ChatId, StopProgress>,
    /// 멈춤 뒤에 실행하기로 한 충돌 입력. 멈춤이 끝나면 재개한다.
    pub(crate) run_after_stop: Vec<InputId>,
    /// 보류한 작업이 멈출 때 하던 일. 재개나 보류 종료 때 지운다.
    pub(crate) held: HashMap<TaskId, HeldTask>,
    /// 크래시 복구가 끊긴 것으로 기록한 하위 에이전트 중 provider에서 아직 정리하지 않은 것. 키는 메인 에이전트이고,
    /// session을 다시 열 때 provider에 넘기며 열리면 지운다.
    pub(crate) interrupted_to_clean: HashMap<AgentId, Vec<SubagentId>>,
    /// 끊긴 하위 에이전트 전부. 다시 연 뒤 이 하위 에이전트의 이벤트가 오면 provider가 다시 실행한 것으로 보고 막는다.
    pub(crate) interrupted_watch: HashMap<AgentId, HashSet<SubagentId>>,
    /// 이미 막고 알린 하위 에이전트. 같은 이벤트가 이어져도 한 번만 알린다.
    pub(crate) interrupted_blocked: HashSet<(AgentId, SubagentId)>,
    pub(crate) stop_tx: mpsc::UnboundedSender<StopDone>,
    pub(crate) stop_rx: mpsc::UnboundedReceiver<StopDone>,
    /// provider 응답을 기다리는 전달. 채팅마다 하나이고, 있는 동안 그 채팅의 다음 입력은 보내지 않는다.
    pub(crate) deliveries: HashMap<ChatId, Parked>,
    /// 연결 작업이 보내는 provider 이벤트와 요청 결과.
    pub(crate) provider_tx: mpsc::UnboundedSender<ProviderMsg>,
    pub(crate) provider_rx: mpsc::UnboundedReceiver<ProviderMsg>,
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
        let (provider_tx, provider_rx) = mpsc::unbounded_channel();
        Self {
            deliveries: HashMap::new(),
            provider_tx,
            provider_rx,
            last_run: HashMap::new(),
            last_task: HashMap::new(),
            context_tokens: HashMap::new(),
            packet_turns: HashMap::new(),
            permissions: HashMap::new(),
            inputs: HashMap::new(),
            next_request: 0,
            settings_of: HashMap::new(),
            rules_of_connection: HashMap::new(),
            stale_connections: HashSet::new(),
            questions_of_connection: HashMap::new(),
            watched_settings: HashMap::new(),
            needs_check: HashMap::new(),
            switch_to: HashMap::new(),
            stopping: HashMap::new(),
            run_after_stop: Vec::new(),
            held: HashMap::new(),
            interrupted_to_clean: HashMap::new(),
            interrupted_watch: HashMap::new(),
            interrupted_blocked: HashSet::new(),
            stop_tx,
            stop_rx,
            routed: HashMap::new(),
            judging: HashMap::new(),
            models: HashMap::new(),
            commands: HashMap::new(),
            cli_versions: HashMap::new(),
            router_tx,
            router_rx,
            last_disposition: HashMap::new(),
            unrecorded: HashMap::new(),
            applied: Vec::new(),
            resume_signals: Vec::new(),
            tasks: TaskBook::default(),
            live: HashMap::new(),
        }
    }
}

impl FlowState {
    // cost: time O(1), heap O(1), stack O(1), alloc 1
    // basis: estimate
    /// provider 요청 ID와 상관없이 engine이 사는 동안 겹치지 않는 요청 ID.
    pub(crate) fn issue_request_id(&mut self) -> String {
        self.next_request += 1;
        format!("saturn-{}", self.next_request)
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
