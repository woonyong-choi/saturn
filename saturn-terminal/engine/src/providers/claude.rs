//! Claude Code 연결: session마다 `claude` 프로세스 하나를 stream-json 입출력으로 켜 둔다.
//! 설계: docs/design/providers-and-sessions.md

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use saturn_protocol::input::InputAnswer;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use super::{AppliedSettings, LaunchSpec, REPLY_TIMEOUT, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, ProcessSpec, StopScope, Supervisor};

mod adapter;
mod background;
mod config;
mod convert;
mod direct;
mod extensions;
mod hook;
mod input;
mod stream;

use crate::secrets::with_read_scope;
use config::{
    default_args, read_user_config, sandbox_exclusions, with_ask_tools, with_key_sandbox,
};
use convert::permission_response;
pub use hook::{HookInputError, ReadScope, run_pre_tool_use};
use stream::{log_stderr, read_loop};

pub(crate) use adapter::adapter;

/// 끼워 넣기 실측(#5, #27) 통과 전이라 거짓이고, 거짓이면 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

/// 구독 로그인으로 쓸 때 프롬프트 캐시가 남는 시간. API 키 같은 그 밖의 인증은 5분이다.
const SUBSCRIPTION_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// API 키 없이 로그인으로 쓰면 `system/init`의 `apiKeySource`가 `none`이다. 키를 쓰면 다른 값이 온다.
const SUBSCRIPTION_KEY_SOURCE: &str = "none";

/// 설치본 Claude Code 2.1.288이 작업자 재시작으로 끊긴 턴을 `--resume`할 때 자동으로 다시 실행하게 하는 환경 변수.
/// 크래시 뒤 끊긴 턴을 사용자 몰래 이어 가지 않도록 실행 환경에서 뺀다.
const RESUME_INTERRUPTED_TURN_ENV: &str = "CLAUDE_CODE_RESUME_INTERRUPTED_TURN";

/// 넘겨받은 환경에서 `RESUME_INTERRUPTED_TURN_ENV`만 뺀다.
fn without_resume_interrupted_turn(
    env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    env.into_iter()
        .filter(|(name, _)| name != RESUME_INTERRUPTED_TURN_ENV)
        .collect()
}

const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

const SHELL_TOOL: &str = "Bash";

/// 설정을 바꾸는 명령(`model`, `permissions` 등)은 빼지 않는다. 초안 목록.
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["clear", "resume", "exit", "quit", "model"];

/// 모델을 고르지 않은 것과 같다. `--model`을 넘기지 않는다.
const DEFAULT_MODEL: &str = "default";

/// Claude Code `/model`이 보이는 별칭. 기본, opus, sonnet, haiku 순.
const MODEL_ALIASES: [&str; 4] = [DEFAULT_MODEL, "opus", "sonnet", "haiku"];

/// 모든 도구 승인을 호스트가 받게 하는 인자. 사용자 설정이 `bypassPermissions`여도 요청이 온다(`Bash`로 확인).
const PERMISSION_PROMPT_ARGS: &[&str] = &["--permission-prompt-tool", "stdio"];

/// 규칙 대상 도구. 이름을 나열해야 요청이 호스트로 온다. `Edit`, `Write`, MCP, subagent 도구 모두 `can_use_tool`로 온다.
/// 단 `Read` 없이 낸 `Edit`은 요청 전에 Claude Code가 도구 오류로 끝내 요청이 오지 않는다.
/// 실측은 `docs/experiments/provider-permission-real-claude/report.md`.
/// MCP는 서버를 알 수 없어 `mcp__*` 하나로 둔다. 초안.
pub(crate) const ASK_TOOLS: &[&str] = &[
    "Bash",
    "Edit",
    "MultiEdit",
    "Write",
    "NotebookEdit",
    "Task",
    "Agent",
    "mcp__*",
];

/// 에이전트 질문 도구. 권한 모드 `full`이면 `--disallowedTools`로 뺀다.
const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";

const DISALLOWED_TOOLS_FLAG: &str = "--disallowedTools";

const AUTO_COMPACT_FLAG: &str = "--autocompact";

/// Claude Code 2.1.285 `--help`의 허용 범위 100k~1M에서 가져왔다.
const AUTO_COMPACT_MIN: u64 = 100_000;

/// 같은 허용 범위의 위 끝.
const AUTO_COMPACT_MAX: u64 = 1_000_000;

/// `--resume` 뒤 이 안에 끝나면 재개 실패로 본다. 초안 값.
const RESUME_SETTLE: Duration = Duration::from_millis(500);

/// 넘으면 묶음을 멈춘다. 초안 값.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

const EVENT_BUFFER: usize = 1024;

/// 거부 응답에 붙여 모델에 전달되는 문구. 초안.
const DENY_MESSAGE: &str = "The user denied this tool call in Saturn.";

/// `ReadingFile`로 보인다.
const READ_TOOLS: &[&str] = &["Read", "Glob", "Grep", "LS"];

/// `EditingFile`로 보인다.
const EDIT_TOOLS: &[&str] = &["Edit", "MultiEdit", "Write", "NotebookEdit"];

/// 있으면 자동 압축 값이 있는 것으로 본다.
const AUTO_COMPACT_ENV: &[&str] = &["CLAUDE_CODE_AUTO_COMPACT_WINDOW", "DISABLE_COMPACT"];

/// 읽기 작업과 연결이 `Arc<Mutex<_>>`로 같이 쓴다.
#[derive(Debug)]
struct SessionState {
    agent: AgentId,
    /// 새 턴 입력을 쓴 때 참, `result`를 받은 때 거짓.
    turn_active: bool,
    /// 진행 중인 턴에 멈춤 요청을 쓴 때 참, 다음 `result`를 받은 때 거짓. 그 결과는 요청한 완료다.
    stop_requested: bool,
    origin: TurnOriginTracker,
    /// 키는 Task/Agent `tool_use` id, 값은 부모 subagent.
    running: HashMap<SubagentId, Option<SubagentId>>,
    /// 백그라운드로 시작한 subagent의 수명. 키는 `running`의 부분집합이다.
    background: HashMap<SubagentId, background::Background>,
    /// subagent가 소유한 백그라운드 작업(셸)의 `task_id`. 남아 있으면 그 subagent들은 끝나지 않았다.
    owned_tasks: HashSet<String>,
    /// 결과를 기다리는 `Bash` 호출 id. 종료 코드를 결과 글에서 읽는 대상이다.
    shell_calls: HashSet<String>,
    applied: AppliedSettings,
    /// 거르기 전 목록.
    commands: Vec<ProviderCommand>,
    /// 두 번째 `system/init`부터 바뀐 적용값을 `SettingsApplied`로 알린다.
    initialized: bool,
    /// 마지막 메인 `assistant` 메시지의 맥락 크기.
    context_tokens: Option<u64>,
    /// 키는 `request_id`.
    control_waiters: HashMap<String, oneshot::Sender<Value>>,
    /// 답을 기다리는 `can_use_tool` 요청. 키는 `request_id`, 값은 허용 답에 되돌려 줄 요청 `input`.
    permissions: HashMap<String, Value>,
    /// 답을 기다리는 `AskUserQuestion` 요청. 키는 `request_id`, 값은 답에 되돌려 줄 요청 `input`.
    inputs: HashMap<String, Value>,
}

impl SessionState {
    fn new(agent: AgentId) -> Self {
        Self {
            agent,
            turn_active: false,
            stop_requested: false,
            origin: TurnOriginTracker::default(),
            running: HashMap::new(),
            background: HashMap::new(),
            owned_tasks: HashSet::new(),
            shell_calls: HashSet::new(),
            applied: AppliedSettings::default(),
            commands: Vec::new(),
            initialized: false,
            context_tokens: None,
            control_waiters: HashMap::new(),
            permissions: HashMap::new(),
            inputs: HashMap::new(),
        }
    }
}

#[derive(Debug)]
struct SessionLink {
    group: ProcessGroupId,
    /// 닫으면 프로그램이 끝난다.
    stdin: ChildStdin,
    state: Arc<Mutex<SessionState>>,
    /// 턴 진행 중에 받은 새 턴 입력. Claude Code는 진행 중인 턴에 쓴 사용자 메시지를 그 턴에 합쳐 `result`를 하나만 내므로,
    /// 앞 턴의 `result`를 전달한 뒤에 하나씩 쓴다.
    queued_turns: VecDeque<String>,
}

#[derive(Debug)]
pub(crate) struct ClaudeClient {
    supervisor: Supervisor,
    /// session을 열 때마다 이 값으로 프로세스를 띄운다.
    launch: LaunchSpec,
    /// 키는 `--session-id`로 넘긴 id(새 session) 또는 `--resume` id.
    sessions: HashMap<ProviderSessionId, SessionLink>,
    events: mpsc::Receiver<ProviderEvent>,
    /// 읽기 작업을 거치지 않고 이 연결이 직접 알릴 이벤트. `next_event`가 먼저 돌려준다.
    own_events: VecDeque<ProviderEvent>,
    /// 새 session 읽기 작업에 복제해 준다.
    events_tx: mpsc::Sender<ProviderEvent>,
    next_request_id: u64,
    /// 거르기 전 목록. 읽기 작업이 갱신한다.
    latest_commands: Arc<Mutex<Vec<ProviderCommand>>>,
    resume_settle: Duration,
}

impl ClaudeClient {
    /// 프로세스는 `open_session`에서 띄운다.
    pub(crate) fn new(launch: LaunchSpec, supervisor: Supervisor) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_BUFFER);
        Self {
            supervisor,
            launch,
            sessions: HashMap::new(),
            events,
            own_events: VecDeque::new(),
            events_tx,
            next_request_id: 1,
            latest_commands: Arc::default(),
            resume_settle: RESUME_SETTLE,
        }
    }

    /// 부하에서 프로세스 시작이 늦어지는 테스트가 판정 시간을 늘리거나 줄이는 데 쓴다.
    #[cfg(test)]
    fn with_resume_settle(mut self, resume_settle: Duration) -> Self {
        self.resume_settle = resume_settle;
        self
    }

    /// 모르는 session이면 `None`.
    pub(crate) fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        self.sessions.get(session).map(|link| link.group)
    }

    /// `system/init`을 받기 전이면 `None`.
    #[cfg(test)]
    pub(crate) fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        let link = self.sessions.get(session)?;
        let state = lock(&link.state);
        state.initialized.then(|| state.applied.clone())
    }

    /// 새 턴 입력을 쓰고 `turn_active`를 켠다. 실패하면 `on_user_send`와 `turn_active`를 되돌린다.
    async fn write_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let state = self
            .sessions
            .get(session)
            .map(|link| Arc::clone(&link.state))
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        let was_active = {
            let mut state = lock(&state);
            state.origin.on_user_send();
            std::mem::replace(&mut state.turn_active, true)
        };
        let written = self.write_user_message(session, text).await;
        if let Err(ProviderError::NotSent { .. }) = &written {
            let mut state = lock(&state);
            state.origin.cancel_user_send();
            state.turn_active = was_active;
        }
        written
    }

    /// 턴이 끝난 session에 줄 세워 둔 첫 입력을 쓴다. 쓰는 도중 막힐 수 있어 취소하면 안 된다.
    /// 완료 이벤트를 처리한 호출자가 끝까지 기다린다.
    pub(crate) async fn start_queued_turn(&mut self, agent: AgentId) {
        let Some((session, text)) = self.sessions.iter_mut().find_map(|(id, link)| {
            if lock(&link.state).agent != agent {
                return None;
            }
            link.queued_turns.pop_front().map(|text| (id.clone(), text))
        }) else {
            return;
        };
        if let Err(error) = self.write_turn(&session, &text).await {
            tracing::warn!(%error, "failed to send the queued turn");
            self.own_events
                .push_back(ProviderEvent::StreamLost { agent });
        }
    }

    /// 모르는 session이거나 프로세스가 이미 끝났으면 `NotSent`, 쓰는 중 실패(끊긴 파이프)는 `Unknown`.
    async fn write_user_message(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let message = json!({
            "type": "user",
            "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        });
        self.write_line(session, &message).await
    }

    /// 실패 구분은 `write_user_message`와 같다.
    async fn write_line(
        &mut self,
        session: &ProviderSessionId,
        message: &Value,
    ) -> Result<(), ProviderError> {
        let link = self
            .sessions
            .get_mut(session)
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        if !self.supervisor.is_running(link.group) {
            return Err(ProviderError::NotSent {
                reason: "claude process has exited".to_owned(),
            });
        }
        let mut line = message.to_string();
        line.push('\n');
        let written = async {
            link.stdin.write_all(line.as_bytes()).await?;
            link.stdin.flush().await
        };
        written.await.map_err(|error| {
            tracing::warn!(error = %error.kind(), "failed to write to claude");
            ProviderError::Unknown
        })
    }

    async fn stop_group(&self, group: ProcessGroupId) -> Result<(), ProviderError> {
        let outcome = self
            .supervisor
            .stop_tree(group, StopScope::Whole)
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "failed to stop claude process group");
                ProviderError::ConnectionLost
            })?;
        tracing::debug!(?outcome, "stopped claude process group");
        Ok(())
    }

    fn launch_args(&self, spec: &SessionSpec, session: &SessionArg) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ]
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect();
        match session {
            SessionArg::Resume(id) => args.extend(["--resume".to_owned(), id.clone()]),
            SessionArg::New(id) => args.extend(["--session-id".to_owned(), id.clone()]),
        }
        if let Some(model) = spec.model.as_ref().filter(|model| *model != DEFAULT_MODEL) {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        if !spec.add_dirs.is_empty() {
            args.push("--add-dir".to_owned());
            args.extend(
                spec.add_dirs
                    .iter()
                    .map(|dir| dir.to_string_lossy().into_owned()),
            );
        }
        let found = read_user_config(&self.launch);
        let user = UserProviderConfig {
            has_auto_compact: self.launch.user_config.has_auto_compact || found.has_auto_compact,
        };
        args.extend(default_args(user, &self.launch));
        let hooks = with_read_scope(
            self.launch.hook_settings.clone().unwrap_or(Value::Null),
            &spec.workdir,
            &spec.add_dirs,
        );
        let settings = with_key_sandbox(with_ask_tools(hooks), &self.launch.key_deny_read);
        args.extend(["--settings".to_owned(), settings.to_string()]);
        args
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionArg {
    Resume(String),
    /// Saturn이 만든 새 session id.
    New(String),
}

impl ProviderClient for ClaudeClient {
    /// 실행 실패는 `ConnectionLost`, 재개 실패(`--resume` 뒤 `RESUME_SETTLE` 안에 종료)는 `NotSent`.
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        if let Some(file) = sandbox_exclusions(&self.launch).first() {
            // 제외된 명령은 샌드박스 밖에서 돌아 router 키 저장소 읽기 금지가 닿지 않고, 실행별 설정으로 비울 수 없다
            return Err(ProviderError::NotSent {
                reason: format!(
                    "claude settings {} set sandbox.excludedCommands, which runs commands outside the sandbox that protects the router key; remove it to use claude with saturn",
                    file.display()
                ),
            });
        }
        let session_arg = match &spec.resume {
            Some(id) => SessionArg::Resume(id.0.clone()),
            None => SessionArg::New(new_session_uuid().map_err(|error| {
                tracing::warn!(error = %error, "failed to make claude session id");
                ProviderError::ConnectionLost
            })?),
        };
        let session = match &session_arg {
            SessionArg::Resume(id) | SessionArg::New(id) => ProviderSessionId(id.clone()),
        };
        // 오류 결과로 흐름을 잃은 session의 프로세스는 아직 살아 있을 수 있다. 같은 session을 두 프로세스가 쓰지 않게 먼저 닫는다
        self.close_session(&session).await?;
        let spawned = self
            .supervisor
            .spawn(ProcessSpec {
                program: self.launch.program.clone(),
                args: self.launch_args(&spec, &session_arg),
                workdir: spec.workdir.clone(),
                env: without_resume_interrupted_turn(self.launch.env.clone()),
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "failed to start claude");
                ProviderError::ConnectionLost
            })?;
        let group = spawned.group;
        let state = Arc::new(Mutex::new(SessionState::new(spec.agent)));
        tokio::spawn(read_loop(
            spawned.io.stdout,
            Arc::clone(&state),
            self.events_tx.clone(),
            Arc::clone(&self.latest_commands),
            (self.launch.masker.clone(), self.launch.events.clone()),
        ));
        tokio::spawn(log_stderr(spawned.io.stderr, self.launch.masker.clone()));
        if matches!(session_arg, SessionArg::Resume(_))
            && let Ok(Ok(exit)) =
                tokio::time::timeout(self.resume_settle, self.supervisor.wait(group)).await
        {
            self.supervisor.release(group);
            return Err(ProviderError::NotSent {
                reason: format!("claude resume failed with {exit:?}"),
            });
        }
        self.sessions.insert(
            session.clone(),
            SessionLink {
                group,
                stdin: spawned.io.stdin,
                state,
                queued_turns: VecDeque::new(),
            },
        );
        if let Some(packet) = &spec.packet {
            self.send_turn(&session, packet).await?;
        }
        Ok(SessionHandle {
            provider_session: session,
            steer_verified: STEER_VERIFIED,
        })
    }

    /// 앞 턴이 진행 중이면 쓰지 않고 줄 세워 두었다가 그 턴의 `TurnCompleted`를 전달할 때 쓴다.
    /// 쓰기 전 실패면 `on_user_send`와 `turn_active`를 되돌린다.
    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let link = self
            .sessions
            .get_mut(session)
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        let state = Arc::clone(&link.state);
        let busy = lock(&state).turn_active || !link.queued_turns.is_empty();
        if busy {
            link.queued_turns.push_back(text.to_owned());
            return Ok(());
        }
        self.write_turn(session, text).await
    }

    /// 쓰는 사이 턴이 끝나도 provider가 다음 턴에 처리하므로 `Ok`.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        let active = self
            .sessions
            .get(session)
            .map(|link| lock(&link.state).turn_active)
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        if !active {
            return Err(ProviderError::NoActiveTurn);
        }
        self.write_user_message(session, text).await
    }

    /// `Subagent`는 따로 멈출 경로가 없어 보내지 않고 `Ok`.
    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        if matches!(target, InterruptTarget::Subagent(_)) {
            return Ok(());
        }
        let Some(state) = self
            .sessions
            .get(session)
            .map(|link| Arc::clone(&link.state))
        else {
            return Ok(());
        };
        if let Some(link) = self.sessions.get_mut(session) {
            link.queued_turns.clear();
        }
        let request_id = format!("saturn-{}", self.next_request_id);
        self.next_request_id += 1;
        let (reply, receive) = oneshot::channel();
        {
            let mut state = lock(&state);
            state.stop_requested = state.turn_active;
            state.control_waiters.insert(request_id.clone(), reply);
        }
        let message = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": { "subtype": "interrupt" },
        });
        if self.write_line(session, &message).await.is_err() {
            lock(&state).control_waiters.remove(&request_id);
            return Err(ProviderError::ConnectionLost);
        }
        match tokio::time::timeout(REPLY_TIMEOUT, receive).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(_)) => Err(ProviderError::ConnectionLost),
            Err(_) => {
                lock(&state).control_waiters.remove(&request_id);
                tracing::warn!("claude interrupt response timed out");
                Ok(())
            }
        }
    }

    /// 턴 끝 경계에서만 보내므로 턴 진행 중이면 `NotSent`.
    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        let active = self
            .sessions
            .get(session)
            .map(|link| lock(&link.state).turn_active);
        if active == Some(true) {
            return Err(ProviderError::NotSent {
                reason: "turn in progress".to_owned(),
            });
        }
        self.send_turn(session, "/compact").await
    }

    /// `control_response`로 답한다. 모르는 session이나 요청이면 `NotSent`. 쓰기 전에 실패하면 요청을 되돌려
    /// 다시 답할 수 있게 한다.
    async fn answer_permission(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        let state = self
            .sessions
            .get(session)
            .map(|link| Arc::clone(&link.state))
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        let input =
            lock(&state)
                .permissions
                .remove(request_id)
                .ok_or_else(|| ProviderError::NotSent {
                    reason: format!("unknown permission request {request_id}"),
                })?;
        let message = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": permission_response(&answer, &input),
            },
        });
        let written = self.write_line(session, &message).await;
        if let Err(ProviderError::NotSent { .. }) = &written {
            lock(&state)
                .permissions
                .insert(request_id.to_owned(), input);
        }
        written
    }

    /// `AskUserQuestion`에 `control_response`로 답한다. 모르는 session이나 요청이면 `NotSent`. 쓰기 전에 실패하면
    /// 요청을 되돌려 다시 답할 수 있게 한다.
    async fn answer_input(
        &mut self,
        session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        let state = self
            .sessions
            .get(session)
            .map(|link| Arc::clone(&link.state))
            .ok_or_else(|| ProviderError::NotSent {
                reason: format!("unknown session {}", session.0),
            })?;
        let input =
            lock(&state)
                .inputs
                .remove(request_id)
                .ok_or_else(|| ProviderError::NotSent {
                    reason: format!("unknown input request {request_id}"),
                })?;
        let message = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": input::response(&input, &answer),
            },
        });
        let written = self.write_line(session, &message).await;
        if let Err(ProviderError::NotSent { .. }) = &written {
            lock(&state).inputs.insert(request_id.to_owned(), input);
        }
        written
    }

    /// `session_id`는 호출자가 보관해 `--resume`에 쓴다. 모르는 session이면 아무것도 하지 않는다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        let Some(link) = self.sessions.remove(session) else {
            return Ok(());
        };
        let group = link.group;
        drop(link.stdin);
        let exited = tokio::time::timeout(CLOSE_GRACE, self.supervisor.wait(group)).await;
        if !matches!(exited, Ok(Ok(_))) {
            self.stop_group(group).await?;
        }
        self.supervisor.release(group);
        Ok(())
    }

    /// 모든 session이 닫혀도 채널은 남으므로 `None`은 연결을 버릴 때만. 기다리기만 하므로 취소해도 이벤트를 잃지 않는다.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        if let Some(event) = self.own_events.pop_front() {
            return Some(event);
        }
        self.events.recv().await
    }

    /// Claude Code `/model`이 보이는 별칭이다. 프로세스를 띄우지 않고 답한다.
    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(MODEL_ALIASES
            .iter()
            .map(|alias| ModelInfo {
                choice: ModelChoice {
                    provider: adapter::ID,
                    model: (*alias).to_owned(),
                },
                name: (*alias).to_owned(),
            })
            .collect())
    }

    /// `system/init`에 스킬 이름 목록(`skills`)이 따로 오면 그 이름만 `is_skill = true`.
    fn commands(&self) -> Vec<ProviderCommand> {
        super::filter_commands(lock(&self.latest_commands).clone(), EXCLUDED_COMMANDS)
    }
}

/// `/dev/urandom` 16바이트로 만든 UUID v4.
fn new_session_uuid() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 잠금을 쥔 채 panic하는 코드가 없으니 독이 든 잠금도 내용은 온전하다
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
