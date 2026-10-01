//! 화면 상태 모델. engine 알림만으로 채우고 상태 전이는 검사하지 않는다.
//! 설계: docs/design/tui.md

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use saturn_protocol::event::{Activity, ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, JudgmentId, Provider, SettingsRevision, SubagentId, TaskId, TaskLabel,
};
use saturn_protocol::rpc::Alert;
use saturn_protocol::state::{Disposition, InputState, QueueReason, TaskState};

use crate::labels;

/// 허가를 기다리는 동안은 멈춘다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopwatch {
    started: Instant,
    paused_total: Duration,
    paused_at: Option<Instant>,
}

impl Stopwatch {
    pub fn start(now: Instant) -> Self {
        Self {
            started: now,
            paused_total: Duration::ZERO,
            paused_at: None,
        }
    }

    pub fn start_with(now: Instant, elapsed: Duration) -> Self {
        Self::start(now.checked_sub(elapsed).unwrap_or(now))
    }

    /// 이미 멈췄으면 그대로.
    pub fn pause(&mut self, now: Instant) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
    }

    pub fn resume(&mut self, now: Instant) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += now.saturating_duration_since(paused_at);
        }
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        let end = self.paused_at.unwrap_or(now);
        end.saturating_duration_since(self.started)
            .saturating_sub(self.paused_total)
    }
}

#[derive(Debug, Clone)]
pub struct TaskView {
    pub id: TaskId,
    pub label: TaskLabel,
    pub state: TaskState,
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub activity: Option<Activity>,
    pub subagents: Vec<SubagentId>,
    pub has_output: bool,
    pub tokens: Option<u64>,
    pub stopwatch: Stopwatch,
    pub reported_elapsed: Duration,
    pub failure: Option<String>,
    /// 같은 종류 줄 안의 접수 순서 정렬에 쓴다.
    pub seq: u64,
    /// `ThreadCumulative` 보고의 에이전트별 직전 누적이며 차이만 합계에 더한다.
    cumulative: BTreeMap<AgentId, u64>,
}

#[derive(Debug, Clone)]
pub struct InputView {
    pub id: InputId,
    /// 끼워 넣기면 합쳐진 작업의 이름표.
    pub label: Option<TaskLabel>,
    /// 빈 문자열이면 `None`.
    pub text: Option<String>,
    pub state: InputState,
    pub disposition: Option<Disposition>,
    /// `Queued`일 때만 있다.
    pub reason: Option<QueueReason>,
    pub echoed: bool,
    /// 같은 종류 줄 안의 접수 순서 정렬에 쓴다.
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputUpdate {
    pub input: InputId,
    /// 다른 TUI가 보낸 입력에도 온다.
    pub text: String,
    pub label: Option<TaskLabel>,
    pub state: InputState,
    pub disposition: Option<Disposition>,
    pub reason: Option<QueueReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskUpdate {
    pub task: TaskId,
    pub label: TaskLabel,
    pub state: TaskState,
    pub provider: Option<Provider>,
    pub elapsed: Duration,
    pub failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrainingProgress {
    pub stage: String,
    pub graded: u32,
    pub elapsed: Duration,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopResult {
    pub held: Vec<TaskLabel>,
    /// 멈춤 뒤 provider 프로세스 묶음 밖에 남은 프로세스 수.
    pub unconfirmed: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextSize {
    pub tokens: Option<u64>,
    /// Saturn이 맥락을 정리하는 기준 토큰.
    pub threshold: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackPrompt {
    pub judgment: JudgmentId,
    pub input: InputId,
    pub label: TaskLabel,
    pub disposition: Disposition,
    pub shown_at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// 입력당 한 번.
    Echo { input: InputId },
    /// 작업별 출력 칸에 완성된 줄 단위로 넣는다.
    Output { task: TaskId, text: String },
    Tool {
        task: TaskId,
        call_id: String,
        activity: Option<Activity>,
        output: Option<String>,
    },
    /// `Done` 또는 `Failed`.
    TaskFinished { task: TaskId },
    /// 처음 `NeedsCheck`가 됐을 때 한 번.
    TaskNeedsCheck { task: TaskId },
    PermissionRequested {
        task: TaskId,
        request_id: String,
        summary: String,
        reason: String,
    },
    /// 다시 그리기만 하면 되는 변화.
    Redraw,
}

#[derive(Debug, Default)]
pub struct ChatState {
    pub chat: Option<ChatId>,
    /// 끝난 작업은 결과를 대화 기록으로 옮긴 뒤 지운다.
    pub tasks: BTreeMap<TaskId, TaskView>,
    /// 끝 상태가 아닌 입력만 둔다.
    pub inputs: BTreeMap<InputId, InputView>,
    /// 같은 값은 한 번만.
    pub alerts: Vec<Alert>,
    pub training: Option<TrainingProgress>,
    /// 보류 줄이 모두 사라지면 지운다.
    pub stop: Option<StopResult>,
    pub close_held_confirm: Option<TaskId>,
    pub feedback: Option<FeedbackPrompt>,
    pub context: Option<ContextSize>,
    /// 적용된 설정 번호와 경고.
    pub settings: Option<(SettingsRevision, Option<String>)>,
    next_seq: u64,
}

impl ChatState {
    pub fn new() -> Self {
        Self::default()
    }

    // cost: time O(log i + h), heap O(n), stack O(1)
    // vars: i = 입력 수, h = 보류 줄 수, n = 원문 길이
    // basis: estimate
    /// TODO(#60): provider가 끼워 넣기를 거절한 입력(`Rejected`)을 대기로 옮길지, 다시 판단할지, 물을지
    pub fn apply_input(&mut self, update: InputUpdate) -> Change {
        let seq = self.next_seq();
        let view = self.inputs.entry(update.input).or_insert(InputView {
            id: update.input,
            label: None,
            text: None,
            state: update.state,
            disposition: None,
            reason: None,
            echoed: false,
            seq,
        });
        if !update.text.is_empty() {
            view.text = Some(update.text);
        }
        if update.label.is_some() {
            view.label = update.label;
        }
        if update.disposition.is_some() {
            view.disposition = update.disposition;
        }
        view.state = update.state;
        view.reason = update.reason.filter(|_| update.state == InputState::Queued);
        let echo = !view.echoed && is_judged(update.state);
        view.echoed |= echo;
        if is_final(update.state) {
            self.inputs.remove(&update.input);
            self.clear_stop_if_no_holds();
        }
        if echo {
            Change::Echo {
                input: update.input,
            }
        } else {
            Change::Redraw
        }
    }

    // cost: time O(log t + h), heap O(1), stack O(1)
    // vars: t = 작업 수, h = 보류 줄 수
    // basis: estimate
    pub fn apply_task(&mut self, update: TaskUpdate, now: Instant) -> Change {
        let seq = self.next_seq();
        let previous = self.tasks.get(&update.task).map(|view| view.state);
        let view = self.tasks.entry(update.task).or_insert_with(|| TaskView {
            id: update.task,
            label: update.label,
            state: update.state,
            provider: None,
            model: None,
            activity: None,
            subagents: Vec::new(),
            has_output: false,
            tokens: None,
            stopwatch: Stopwatch::start_with(now, update.elapsed),
            reported_elapsed: update.elapsed,
            failure: None,
            seq,
            cumulative: BTreeMap::new(),
        });
        view.label = update.label;
        view.state = update.state;
        view.reported_elapsed = update.elapsed;
        if update.provider.is_some() {
            view.provider = update.provider;
        }
        if update.failure.is_some() {
            view.failure = update.failure;
        }
        if update.state == TaskState::AwaitingPermission {
            view.stopwatch.pause(now);
        } else {
            view.stopwatch.resume(now);
        }
        let change = match update.state {
            TaskState::Done | TaskState::Failed => Change::TaskFinished { task: update.task },
            TaskState::NeedsCheck if previous != Some(TaskState::NeedsCheck) => {
                Change::TaskNeedsCheck { task: update.task }
            }
            _ => Change::Redraw,
        };
        if !matches!(update.state, TaskState::Held | TaskState::NeedsCheck)
            && self.close_held_confirm == Some(update.task)
        {
            self.close_held_confirm = None;
        }
        self.clear_stop_if_no_holds();
        change
    }

    /// 결과 머리줄을 찍은 뒤 부른다.
    pub fn finish_task(&mut self, task: TaskId) -> Option<TaskView> {
        let view = self.tasks.remove(&task);
        self.clear_stop_if_no_holds();
        view
    }

    // cost: time O(log t + s), heap O(n), stack O(1)
    // vars: t = 작업 수, s = subagent 수, n = 이벤트 글 길이
    // basis: estimate
    /// subagent의 글과 도구는 부모 출력 칸과 도구 셀에 넣지 않는다.
    pub fn apply_event(&mut self, task: TaskId, event: ProviderEvent, now: Instant) -> Change {
        let Some(view) = self.tasks.get_mut(&task) else {
            return Change::Redraw;
        };
        match event {
            ProviderEvent::Text {
                subagent: None,
                text,
                ..
            } => {
                view.has_output = true;
                Change::Output { task, text }
            }
            ProviderEvent::ToolCall {
                subagent: None,
                call_id,
                activity,
                ..
            } => {
                view.activity = Some(activity.clone());
                Change::Tool {
                    task,
                    call_id,
                    activity: Some(activity),
                    output: None,
                }
            }
            ProviderEvent::ToolResult {
                subagent: None,
                call_id,
                output,
                ..
            } => Change::Tool {
                task,
                call_id,
                activity: None,
                output: Some(output),
            },
            ProviderEvent::SubagentStarted { subagent, .. } => {
                if !view.subagents.contains(&subagent) {
                    view.subagents.push(subagent);
                }
                Change::Redraw
            }
            ProviderEvent::SubagentEnded { subagent, .. } => {
                view.subagents.retain(|s| *s != subagent);
                Change::Redraw
            }
            ProviderEvent::PermissionRequested {
                request_id,
                summary,
                reason,
                ..
            } => {
                view.stopwatch.pause(now);
                Change::PermissionRequested {
                    task,
                    request_id,
                    summary,
                    reason,
                }
            }
            ProviderEvent::Usage(report) => {
                self.apply_usage(task, &report);
                Change::Redraw
            }
            _ => Change::Redraw,
        }
    }

    /// 합계에서 캐시 읽기는 다시 쓴 토큰이라 뺀다(초안).
    fn apply_usage(&mut self, task: TaskId, report: &UsageReport) {
        let Some(view) = self.tasks.get_mut(&task) else {
            return;
        };
        if report.subagent.is_none() && report.model.is_some() {
            view.model.clone_from(&report.model);
        }
        let Some(value) = report_tokens(report) else {
            return;
        };
        let added = match report.scope {
            UsageScope::ThreadCumulative => {
                let previous = view.cumulative.insert(report.agent, value).unwrap_or(0);
                value.saturating_sub(previous)
            }
            UsageScope::MainTurn | UsageScope::TreeTotal => value,
        };
        view.tokens = Some(view.tokens.unwrap_or(0) + added);
    }

    // cost: time O(a), heap O(1), stack O(1)
    // vars: a = 알림 수
    // basis: estimate
    pub fn apply_alert(&mut self, alert: Alert) {
        if !self.alerts.contains(&alert) {
            self.alerts.push(alert);
        }
    }

    pub fn apply_stopped(&mut self, held: Vec<TaskLabel>) {
        self.stop = Some(StopResult {
            held,
            unconfirmed: None,
        });
    }

    pub fn apply_stop_unconfirmed(&mut self, remaining: u32) {
        let stop = self.stop.get_or_insert(StopResult {
            held: Vec::new(),
            unconfirmed: None,
        });
        stop.unconfirmed = Some(remaining);
    }

    // cost: time O(t), heap O(1), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    pub fn live_tasks(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| labels::is_live(task.state))
            .count()
    }

    // cost: time O(i), heap O(1), stack O(1)
    // vars: i = 입력 수
    // basis: estimate
    pub fn queued_lines(&self) -> usize {
        self.inputs
            .values()
            .filter(|input| input.state == InputState::Queued)
            .count()
    }

    // cost: time O(t + i), heap O(1), stack O(1)
    // vars: t = 작업 수, i = 입력 수
    // basis: estimate
    /// `NeedsCheck` 작업도 보류 줄로 센다.
    pub fn held_lines(&self) -> usize {
        let tasks = self
            .tasks
            .values()
            .filter(|task| is_held(task.state))
            .count();
        let inputs = self
            .inputs
            .values()
            .filter(|input| input.state == InputState::Held)
            .count();
        tasks + inputs
    }

    pub fn labels_visible(&self) -> bool {
        labels::visible(self.live_tasks(), self.queued_lines(), self.held_lines())
    }

    pub fn is_running(&self) -> bool {
        self.tasks.values().any(|task| labels::is_live(task.state))
    }

    // cost: time O(i), heap O(1), stack O(1)
    // vars: i = 입력 수
    // basis: estimate
    pub fn latest_recallable(&self) -> Option<&InputView> {
        self.inputs
            .values()
            .filter(|input| matches!(input.state, InputState::Judging | InputState::Queued))
            .max_by_key(|input| input.seq)
    }

    // cost: time O(i), heap O(1), stack O(1)
    // vars: i = 입력 수
    // basis: estimate
    /// 이름표가 없으면 가장 최근 `Queued` 입력.
    pub fn queued_by_label(&self, label: Option<TaskLabel>) -> Option<&InputView> {
        self.inputs
            .values()
            .filter(|input| input.state == InputState::Queued)
            .filter(|input| label.is_none() || input.label == label)
            .max_by_key(|input| input.seq)
    }

    pub fn held_by_label(&self, label: TaskLabel) -> Option<&TaskView> {
        self.tasks
            .values()
            .find(|task| task.label == label && is_held(task.state))
    }

    // cost: time O(t log t), heap O(t), stack O(1)
    // vars: t = 작업 수
    // basis: estimate
    /// 결과 불명 작업을 포함해 이름표 순서로.
    pub fn held_tasks(&self) -> Vec<&TaskView> {
        let mut held: Vec<&TaskView> = self
            .tasks
            .values()
            .filter(|task| is_held(task.state))
            .collect();
        held.sort_by_key(|task| task.label.0);
        held
    }

    fn next_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn clear_stop_if_no_holds(&mut self) {
        if self.held_lines() == 0 {
            self.stop = None;
        }
    }
}

fn is_judged(state: InputState) -> bool {
    matches!(
        state,
        InputState::Queued | InputState::Delivering | InputState::Applied | InputState::Held
    )
}

fn is_final(state: InputState) -> bool {
    matches!(
        state,
        InputState::Applied | InputState::Rejected | InputState::Cancelled
    )
}

fn is_held(state: TaskState) -> bool {
    matches!(state, TaskState::Held | TaskState::NeedsCheck)
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 합계 칸이 모두 `None`이면 `None`.
fn report_tokens(report: &UsageReport) -> Option<u64> {
    [
        report.input,
        report.cache_write,
        report.output,
        report.reasoning,
    ]
    .into_iter()
    .flatten()
    .reduce(|a, b| a + b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: u64, state: InputState) -> InputUpdate {
        InputUpdate {
            input: InputId(id),
            text: format!("입력 {id}"),
            label: Some(TaskLabel('C')),
            state,
            disposition: None,
            reason: (state == InputState::Queued).then_some(QueueReason::JudgeOrder),
        }
    }

    fn task(id: u64, label: char, state: TaskState) -> TaskUpdate {
        TaskUpdate {
            task: TaskId(id),
            label: TaskLabel(label),
            state,
            provider: Some(Provider::Codex),
            elapsed: Duration::from_secs(5),
            failure: None,
        }
    }

    fn usage(scope: UsageScope, input: Option<u64>, output: Option<u64>) -> UsageReport {
        UsageReport {
            agent: AgentId(1),
            subagent: None,
            model: Some("gpt".to_string()),
            scope,
            input,
            cache_read: Some(1_000),
            cache_write: None,
            output,
            reasoning: None,
        }
    }

    #[test]
    fn stopwatch_pause_excludes_paused_time() {
        let start = Instant::now();
        let mut watch = Stopwatch::start(start);

        watch.pause(start + Duration::from_secs(3));
        watch.resume(start + Duration::from_secs(10));

        assert_eq!(
            watch.elapsed(start + Duration::from_secs(12)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn stopwatch_paused_elapsed_stays_fixed() {
        let start = Instant::now();
        let mut watch = Stopwatch::start(start);

        watch.pause(start + Duration::from_secs(2));

        assert_eq!(
            watch.elapsed(start + Duration::from_secs(9)),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn apply_input_judging_then_queued_echoes_once() {
        let mut state = ChatState::new();

        let first = state.apply_input(input(1, InputState::Judging));
        let second = state.apply_input(input(1, InputState::Queued));
        let third = state.apply_input(input(1, InputState::Queued));

        assert_eq!(first, Change::Redraw);
        assert_eq!(second, Change::Echo { input: InputId(1) });
        assert_eq!(third, Change::Redraw);
        assert_eq!(state.inputs[&InputId(1)].text.as_deref(), Some("입력 1"));
    }

    #[test]
    fn apply_input_final_state_removes_line() {
        let mut state = ChatState::new();
        state.apply_input(input(1, InputState::Judging));

        let change = state.apply_input(input(1, InputState::Cancelled));

        assert_eq!(change, Change::Redraw);
        assert!(state.inputs.is_empty());
    }

    #[test]
    fn apply_input_first_seen_applied_echoes_and_removes() {
        let mut state = ChatState::new();

        let change = state.apply_input(input(1, InputState::Applied));

        assert_eq!(change, Change::Echo { input: InputId(1) });
        assert!(state.inputs.is_empty());
    }

    #[test]
    fn apply_input_reason_kept_only_while_queued() {
        let mut state = ChatState::new();
        state.apply_input(input(1, InputState::Queued));

        state.apply_input(InputUpdate {
            reason: Some(QueueReason::WriteTurn),
            ..input(1, InputState::Held)
        });

        assert_eq!(state.inputs[&InputId(1)].reason, None);
    }

    #[test]
    fn apply_task_done_returns_finished_and_keeps_view_until_finish() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        let change = state.apply_task(task(1, 'A', TaskState::Done), now);

        assert_eq!(change, Change::TaskFinished { task: TaskId(1) });
        assert!(state.finish_task(TaskId(1)).is_some());
        assert!(state.tasks.is_empty());
    }

    #[test]
    fn apply_task_needs_check_reports_once() {
        let mut state = ChatState::new();
        let now = Instant::now();

        let first = state.apply_task(task(1, 'A', TaskState::NeedsCheck), now);
        let second = state.apply_task(task(1, 'A', TaskState::NeedsCheck), now);

        assert_eq!(first, Change::TaskNeedsCheck { task: TaskId(1) });
        assert_eq!(second, Change::Redraw);
    }

    #[test]
    fn apply_task_awaiting_permission_pauses_stopwatch() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        state.apply_task(task(1, 'A', TaskState::AwaitingPermission), now);
        let later = now + Duration::from_secs(30);

        assert_eq!(
            state.tasks[&TaskId(1)].stopwatch.elapsed(later),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn apply_task_provider_is_recorded() {
        let mut state = ChatState::new();

        state.apply_task(task(1, 'A', TaskState::Running), Instant::now());

        assert_eq!(state.tasks[&TaskId(1)].provider, Some(Provider::Codex));
    }

    #[test]
    fn apply_event_subagent_text_is_not_output() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        let change = state.apply_event(
            TaskId(1),
            ProviderEvent::Text {
                agent: AgentId(1),
                subagent: Some(SubagentId("s".to_string())),
                text: "x".to_string(),
            },
            now,
        );

        assert_eq!(change, Change::Redraw);
        assert!(!state.tasks[&TaskId(1)].has_output);
    }

    #[test]
    fn apply_event_main_text_is_output() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        let change = state.apply_event(
            TaskId(1),
            ProviderEvent::Text {
                agent: AgentId(1),
                subagent: None,
                text: "hi\n".to_string(),
            },
            now,
        );

        assert_eq!(
            change,
            Change::Output {
                task: TaskId(1),
                text: "hi\n".to_string()
            }
        );
        assert!(state.tasks[&TaskId(1)].has_output);
    }

    #[test]
    fn apply_usage_skips_cache_read_and_missing_values() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        let report = usage(UsageScope::MainTurn, Some(100), Some(20));
        state.apply_event(TaskId(1), ProviderEvent::Usage(report), now);

        assert_eq!(state.tasks[&TaskId(1)].tokens, Some(120));
        assert_eq!(state.tasks[&TaskId(1)].model.as_deref(), Some("gpt"));
    }

    #[test]
    fn apply_usage_all_missing_keeps_unreported() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        let report = usage(UsageScope::MainTurn, None, None);
        state.apply_event(TaskId(1), ProviderEvent::Usage(report), now);

        assert_eq!(state.tasks[&TaskId(1)].tokens, None);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn apply_usage_cumulative_adds_difference() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);

        for total in [100, 250] {
            let report = usage(UsageScope::ThreadCumulative, Some(total), None);
            state.apply_event(TaskId(1), ProviderEvent::Usage(report), now);
        }

        assert_eq!(state.tasks[&TaskId(1)].tokens, Some(250));
    }

    #[test]
    fn labels_visible_follows_live_queue_and_hold() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Running), now);
        let alone = state.labels_visible();

        state.apply_input(input(2, InputState::Queued));

        assert!(!alone);
        assert!(state.labels_visible());
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn latest_recallable_picks_most_recent_judging_or_queued() {
        let mut state = ChatState::new();
        state.apply_input(input(1, InputState::Queued));
        state.apply_input(input(2, InputState::Judging));
        state.apply_input(input(3, InputState::Delivering));

        assert_eq!(state.latest_recallable().map(|i| i.id), Some(InputId(2)));
    }

    #[test]
    fn queued_by_label_filters_by_label() {
        let mut state = ChatState::new();
        state.apply_input(input(1, InputState::Queued));

        assert!(state.queued_by_label(Some(TaskLabel('C'))).is_some());
        assert!(state.queued_by_label(Some(TaskLabel('D'))).is_none());
        assert!(state.queued_by_label(None).is_some());
    }

    #[test]
    fn stop_result_cleared_when_last_hold_ends() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'A', TaskState::Held), now);
        state.apply_stopped(vec![TaskLabel('A')]);

        state.apply_task(task(1, 'A', TaskState::Running), now);

        assert_eq!(state.stop, None);
    }

    #[test]
    fn apply_alert_ignores_duplicates() {
        let mut state = ChatState::new();

        state.apply_alert(Alert::JudgePaused);
        state.apply_alert(Alert::JudgePaused);

        assert_eq!(state.alerts, vec![Alert::JudgePaused]);
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn held_tasks_sorted_by_label() {
        let mut state = ChatState::new();
        let now = Instant::now();
        state.apply_task(task(1, 'C', TaskState::Held), now);
        state.apply_task(task(2, 'A', TaskState::NeedsCheck), now);

        let labels: Vec<char> = state.held_tasks().iter().map(|t| t.label.0).collect();

        assert_eq!(labels, vec!['A', 'C']);
    }
}
