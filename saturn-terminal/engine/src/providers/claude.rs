//! Claude Code 연결: session마다 `claude` 프로세스 하나를 stream-json 입출력으로 켜 둔다.
//! 설계: docs/design/providers-and-sessions.md

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{
    Activity, LineChange, LineRange, ProviderEvent, ToolCategory, ToolDetail, UsageReport,
    UsageScope,
};
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};

use super::codex::mask_values;
use super::tool_detail::{classify_command, line_change};
use super::{AppliedSettings, LaunchSpec, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, ProcessSpec, StopScope, Supervisor};
use crate::secrets::Masker;

/// `/usage` 행 이름 앞부분.
pub(crate) const DISPLAY_NAME: &str = "claude";

/// 끼워 넣기 실측(#5, #27) 통과 전이라 거짓이고, 거짓이면 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

const SHELL_TOOL: &str = "Bash";

/// 설정을 바꾸는 명령(`model`, `permissions` 등)은 빼지 않는다. 초안 목록.
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["clear", "resume", "exit", "quit"];

/// 사용자 설정에 권한 값이 없을 때만 넣는다. 초안 값(설계는 수정 허용만 정함).
const PERMISSION_ARGS: &[&str] = &["--permission-mode", "acceptEdits"];

const AUTO_COMPACT_FLAG: &str = "--autocompact";

/// Claude Code 2.1.285 `--help`의 허용 범위 100k~1M에서 가져왔다.
const AUTO_COMPACT_MIN: u64 = 100_000;

/// 같은 허용 범위의 위 끝.
const AUTO_COMPACT_MAX: u64 = 1_000_000;

/// `--resume` 뒤 이 안에 끝나면 재개 실패로 본다. 초안 값.
const RESUME_SETTLE: Duration = Duration::from_millis(500);

/// 넘으면 기다리지 않고 돌아가 호출자가 프로세스 묶음 중지로 넘어간다. 초안 값.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);

/// 넘으면 묶음을 멈춘다. 초안 값.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

const EVENT_BUFFER: usize = 1024;

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
    origin: TurnOriginTracker,
    /// 키는 Task/Agent `tool_use` id, 값은 부모 subagent.
    running: HashMap<SubagentId, Option<SubagentId>>,
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
}

impl SessionState {
    fn new(agent: AgentId) -> Self {
        Self {
            agent,
            turn_active: false,
            origin: TurnOriginTracker::default(),
            running: HashMap::new(),
            shell_calls: HashSet::new(),
            applied: AppliedSettings::default(),
            commands: Vec::new(),
            initialized: false,
            context_tokens: None,
            control_waiters: HashMap::new(),
        }
    }
}

#[derive(Debug)]
struct SessionLink {
    group: ProcessGroupId,
    /// 닫으면 프로그램이 끝난다.
    stdin: ChildStdin,
    state: Arc<Mutex<SessionState>>,
}

#[derive(Debug)]
pub struct ClaudeClient {
    supervisor: Supervisor,
    /// session을 열 때마다 이 값으로 프로세스를 띄운다.
    launch: LaunchSpec,
    /// 키는 `--session-id`로 넘긴 id(새 session) 또는 `--resume` id.
    sessions: HashMap<ProviderSessionId, SessionLink>,
    events: mpsc::Receiver<ProviderEvent>,
    /// 새 session 읽기 작업에 복제해 준다.
    events_tx: mpsc::Sender<ProviderEvent>,
    next_request_id: u64,
    /// 거르기 전 목록. 읽기 작업이 갱신한다.
    latest_commands: Arc<Mutex<Vec<ProviderCommand>>>,
    resume_settle: Duration,
}

impl ClaudeClient {
    /// 프로세스는 `open_session`에서 띄운다.
    pub fn new(launch: LaunchSpec, supervisor: Supervisor) -> Self {
        let (events_tx, events) = mpsc::channel(EVENT_BUFFER);
        Self {
            supervisor,
            launch,
            sessions: HashMap::new(),
            events,
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
    pub fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        self.sessions.get(session).map(|link| link.group)
    }

    /// `system/init`을 받기 전이면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        let link = self.sessions.get(session)?;
        let state = lock(&link.state);
        state.initialized.then(|| state.applied.clone())
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
        if let Some(model) = &spec.model {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        let found = read_user_config(&self.launch);
        let user = UserProviderConfig {
            has_permission: self.launch.user_config.has_permission || found.has_permission,
            has_auto_compact: self.launch.user_config.has_auto_compact || found.has_auto_compact,
        };
        args.extend(default_args(user, &self.launch));
        if let Some(settings) = &self.launch.hook_settings {
            args.extend(["--settings".to_owned(), settings.to_string()]);
        }
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
        let spawned = self
            .supervisor
            .spawn(ProcessSpec {
                program: self.launch.program.clone(),
                args: self.launch_args(&spec, &session_arg),
                workdir: spec.workdir.clone(),
                env: self.launch.env.clone(),
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
            self.launch.masker.clone(),
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

    /// 보내기 전 실패면 `on_user_send`와 `turn_active`를 되돌린다.
    async fn send_turn(
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
        let request_id = format!("saturn-{}", self.next_request_id);
        self.next_request_id += 1;
        let (reply, receive) = oneshot::channel();
        lock(&state)
            .control_waiters
            .insert(request_id.clone(), reply);
        let message = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": { "subtype": "interrupt" },
        });
        if self.write_line(session, &message).await.is_err() {
            lock(&state).control_waiters.remove(&request_id);
            return Err(ProviderError::ConnectionLost);
        }
        match tokio::time::timeout(CONTROL_TIMEOUT, receive).await {
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

    /// `session_id`는 호출자가 보관해 `--resume`에 쓴다. 모르는 session이면 아무것도 하지 않는다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        let Some(link) = self.sessions.remove(session) else {
            return Ok(());
        };
        let group = link.group;
        drop(link.stdin);
        let exited = tokio::time::timeout(CLOSE_GRACE, self.supervisor.wait(group)).await;
        if !matches!(exited, Ok(Ok(_))) {
            match self.supervisor.stop_tree(group, StopScope::Whole).await {
                Ok(outcome) => tracing::debug!(?outcome, "stopped claude process group"),
                Err(error) => {
                    tracing::warn!(error = %error, "failed to stop claude process group");
                    return Err(ProviderError::ConnectionLost);
                }
            }
        }
        self.supervisor.release(group);
        Ok(())
    }

    /// 모든 session이 닫혀도 채널은 남으므로 `None`은 연결을 버릴 때만.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        self.events.recv().await
    }

    /// `system/init`에 스킬 이름 목록(`skills`)이 따로 오면 그 이름만 `is_skill = true`.
    fn commands(&self) -> Vec<ProviderCommand> {
        super::filter_commands(lock(&self.latest_commands).clone(), EXCLUDED_COMMANDS)
    }
}

pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !user.has_permission {
        args.extend(PERMISSION_ARGS.iter().map(|arg| (*arg).to_owned()));
    }
    if !user.has_auto_compact {
        let tokens = launch
            .defaults
            .auto_compact_tokens
            .clamp(AUTO_COMPACT_MIN, AUTO_COMPACT_MAX);
        args.push(AUTO_COMPACT_FLAG.to_owned());
        args.push(tokens.to_string());
    }
    args
}

/// 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있어도 자동 압축 값이 있는 것으로 본다.
/// `HOME`과 환경 변수는 부모 환경이 아니라 `launch.env`에서 읽고, 파일을 못 읽으면 값 없음. 초안 목록.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    let env = |name: &str| {
        launch
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let mut files = Vec::new();
    if let Some(home) = env("HOME") {
        files.push(Path::new(home).join(".claude").join("settings.json"));
    }
    let project = launch.workdir.join(".claude");
    files.push(project.join("settings.json"));
    files.push(project.join("settings.local.json"));
    let mut found = UserProviderConfig {
        has_permission: false,
        has_auto_compact: AUTO_COMPACT_ENV.iter().any(|name| env(name).is_some()),
    };
    for file in files {
        let Some(settings) = read_json(&file) else {
            continue;
        };
        found.has_permission |= !settings["permissions"]["defaultMode"].is_null();
        found.has_auto_compact |= !settings["autoCompactEnabled"].is_null();
    }
    found
}

/// 없거나 깨졌으면 `None`.
fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// subagent 등록과 `turn_active`, 적용값, 명령 목록 갱신도 여기서 한다. 버릴 줄이면 빈 목록.
fn convert_line(state: &mut SessionState, line: &serde_json::Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    match line["type"].as_str() {
        Some("system") if line["subtype"] == "init" => apply_init(state, line),
        Some("assistant") => convert_assistant(state, line),
        Some("user") => convert_tool_results(state, line),
        Some("result") => {
            state.turn_active = false;
            let origin = state.origin.on_turn_started();
            let usage = &line["usage"];
            vec![
                ProviderEvent::Usage(UsageReport {
                    agent,
                    subagent: None,
                    model: state.applied.model.clone(),
                    scope: UsageScope::MainTurn,
                    input: usage["input_tokens"].as_u64(),
                    cache_read: usage["cache_read_input_tokens"].as_u64(),
                    cache_write: usage["cache_creation_input_tokens"].as_u64(),
                    output: usage["output_tokens"].as_u64(),
                    reasoning: None,
                }),
                ProviderEvent::ContextSize {
                    agent,
                    tokens: state.context_tokens,
                },
                ProviderEvent::TurnCompleted { agent, origin },
            ]
        }
        Some("control_request") if line["request"]["subtype"] == "can_use_tool" => {
            let request = &line["request"];
            let tool = request["tool_name"].as_str().unwrap_or_default();
            let target = request["input"]["command"]
                .as_str()
                .or_else(|| request["input"]["file_path"].as_str());
            let summary =
                target.map_or_else(|| tool.to_owned(), |target| format!("{tool}: {target}"));
            vec![ProviderEvent::PermissionRequested {
                agent,
                request_id: line["request_id"].as_str().unwrap_or_default().to_owned(),
                summary,
                reason: request["decision_reason"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            }]
        }
        _ => Vec::new(),
    }
}

/// 두 번째부터 모델이나 권한 방식이 바뀌었으면 `SettingsApplied`.
fn apply_init(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let applied = AppliedSettings {
        model: line["model"].as_str().map(str::to_owned),
        permission: line["permissionMode"].as_str().map(str::to_owned),
    };
    let skills: Vec<&str> = line["skills"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|skill| skill.as_str().or_else(|| skill["name"].as_str()))
        .collect();
    state.commands = line["slash_commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|name| ProviderCommand {
            name: name.trim_start_matches('/').to_owned(),
            description: String::new(),
            is_skill: skills.contains(&name),
        })
        .collect();
    let changed = state.initialized && applied != state.applied;
    state.initialized = true;
    state.applied = applied;
    if !changed {
        return Vec::new();
    }
    let values = [
        ("model", state.applied.model.clone()),
        ("permission_mode", state.applied.permission.clone()),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
    .collect();
    vec![ProviderEvent::SettingsApplied {
        agent: state.agent,
        values,
    }]
}

/// Task/Agent 호출은 subagent 시작으로 등록한다.
fn convert_assistant(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    let parent = parent_subagent(line);
    if parent.is_none() {
        let usage = &line["message"]["usage"];
        let parts = [
            "input_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .map(|key| usage[key].as_u64());
        if parts.iter().any(Option::is_some) {
            state.context_tokens = Some(parts.iter().flatten().sum());
        }
    }
    let mut events = Vec::new();
    for item in line["message"]["content"].as_array().into_iter().flatten() {
        match item["type"].as_str() {
            Some("text") => {
                if let Some(text) = item["text"].as_str() {
                    events.push(ProviderEvent::Text {
                        agent,
                        subagent: parent.clone(),
                        text: text.to_owned(),
                    });
                }
            }
            Some("tool_use") => {
                let (Some(id), Some(name)) = (item["id"].as_str(), item["name"].as_str()) else {
                    continue;
                };
                if SUBAGENT_TOOLS.contains(&name) {
                    let subagent = SubagentId(id.to_owned());
                    state.running.insert(subagent.clone(), parent.clone());
                    events.push(ProviderEvent::SubagentStarted {
                        agent,
                        subagent,
                        parent: parent.clone(),
                    });
                } else {
                    if name == SHELL_TOOL {
                        state.shell_calls.insert(id.to_owned());
                    }
                    events.push(ProviderEvent::ToolCall {
                        agent,
                        subagent: parent.clone(),
                        call_id: id.to_owned(),
                        activity: activity_of(name, &item["input"]),
                        detail: detail_of(name, &item["input"]),
                    });
                }
            }
            _ => {}
        }
    }
    events
}

/// subagent 호출의 결과는 subagent 끝이다.
fn convert_tool_results(state: &mut SessionState, line: &Value) -> Vec<ProviderEvent> {
    let agent = state.agent;
    let parent = parent_subagent(line);
    let mut events = Vec::new();
    for item in line["message"]["content"].as_array().into_iter().flatten() {
        if item["type"] != "tool_result" {
            continue;
        }
        let Some(id) = item["tool_use_id"].as_str() else {
            continue;
        };
        let subagent = SubagentId(id.to_owned());
        if state.running.remove(&subagent).is_some() {
            events.push(ProviderEvent::SubagentEnded { agent, subagent });
            continue;
        }
        let output = tool_result_text(&item["content"]);
        let exit_code = if state.shell_calls.remove(id) {
            shell_exit_code(item["is_error"].as_bool().unwrap_or(false), &output)
        } else {
            None
        };
        events.push(ProviderEvent::ToolResult {
            agent,
            subagent: parent.clone(),
            call_id: id.to_owned(),
            output,
            exit_code,
        });
    }
    events
}

fn parent_subagent(line: &Value) -> Option<SubagentId> {
    line["parent_tool_use_id"]
        .as_str()
        .map(|id| SubagentId(id.to_owned()))
}

fn activity_of(name: &str, input: &Value) -> Activity {
    if READ_TOOLS.contains(&name) {
        return Activity::ReadingFile;
    }
    if EDIT_TOOLS.contains(&name) {
        return Activity::EditingFile;
    }
    let command = match name {
        SHELL_TOOL => input["command"].as_str().unwrap_or_default().to_owned(),
        other => other.to_owned(),
    };
    Activity::RunningCommand { command }
}

// cost: time O(i), heap O(i), stack O(1), alloc 1
// vars: i = 입력 글자 수
// basis: estimate
fn detail_of(name: &str, input: &Value) -> ToolDetail {
    let path_of = |key: &str| input[key].as_str().map(str::to_owned).into_iter().collect();
    if name == SHELL_TOOL {
        let command = input["command"].as_str().unwrap_or_default();
        return ToolDetail {
            category: classify_command(command),
            ..ToolDetail::default()
        };
    }
    if name == "Read" {
        return ToolDetail {
            category: ToolCategory::FileRead,
            paths: path_of("file_path"),
            read_lines: read_range(input),
            changed: None,
        };
    }
    if READ_TOOLS.contains(&name) {
        return ToolDetail {
            category: ToolCategory::FileRead,
            paths: path_of("path"),
            ..ToolDetail::default()
        };
    }
    if EDIT_TOOLS.contains(&name) {
        let key = if name == "NotebookEdit" {
            "notebook_path"
        } else {
            "file_path"
        };
        return ToolDetail {
            category: ToolCategory::FileEdit,
            paths: path_of(key),
            read_lines: None,
            changed: edit_change(name, input),
        };
    }
    ToolDetail::default()
}

/// 시작 줄과 줄 수가 모두 있을 때만 범위를 낸다. 한쪽만 있으면 전체 범위를 알 수 없다.
fn read_range(input: &Value) -> Option<LineRange> {
    let first = u32::try_from(input["offset"].as_u64()?).ok()?;
    let limit = u32::try_from(input["limit"].as_u64()?).ok()?;
    let last = first.checked_add(limit)?.checked_sub(1)?;
    (first >= 1 && first <= last).then_some(LineRange { first, last })
}

// cost: time O(i), heap O(i), stack O(1), alloc 2
// vars: i = 입력 글자 수
// basis: estimate
/// `Edit`와 `MultiEdit`만 센다. `Write`는 덮어쓴 줄을 입력으로 알 수 없어 `None`이다.
fn edit_change(name: &str, input: &Value) -> Option<LineChange> {
    let one = |edit: &Value| -> Option<LineChange> {
        Some(line_change(
            edit["old_string"].as_str()?,
            edit["new_string"].as_str()?,
        ))
    };
    let edits: Vec<Option<LineChange>> = match name {
        "Edit" => vec![one(input)],
        "MultiEdit" => input["edits"].as_array()?.iter().map(one).collect(),
        _ => return None,
    };
    edits.into_iter().try_fold(
        LineChange {
            added: 0,
            removed: 0,
        },
        |total, change| {
            let change = change?;
            Some(LineChange {
                added: total.added.saturating_add(change.added),
                removed: total.removed.saturating_add(change.removed),
            })
        },
    )
}

/// 실패한 `Bash` 결과는 `Exit code N`으로 시작하고, 성공은 0이다. 중단처럼 코드가 없으면 `None`.
fn shell_exit_code(is_error: bool, output: &str) -> Option<i32> {
    if !is_error {
        return Some(0);
    }
    output
        .strip_prefix("Exit code ")?
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .next()?
        .parse()
        .ok()
}

/// 문자열이거나 `text` 항목 배열이다.
fn tool_result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 끝나면 턴이나 subagent가 남았을 때 `StreamLost`.
async fn read_loop(
    stdout: ChildStdout,
    state: Arc<Mutex<SessionState>>,
    events: mpsc::Sender<ProviderEvent>,
    latest_commands: Arc<Mutex<Vec<ProviderCommand>>>,
    masker: Masker,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!("skipping non-json line from claude");
            continue;
        };
        mask_values(&mut message, &masker);
        let converted = {
            let mut state = lock(&state);
            if message["type"] == "control_response" {
                let id = message["response"]["request_id"]
                    .as_str()
                    .unwrap_or_default();
                if let Some(waiter) = state.control_waiters.remove(id) {
                    let _ = waiter.send(message["response"].clone()); // 기다리던 쪽이 시간을 넘겨 포기했다
                }
                continue;
            }
            let converted = convert_line(&mut state, &message);
            if message["type"] == "system" && message["subtype"] == "init" {
                *lock(&latest_commands) = state.commands.clone();
            }
            converted
        };
        for event in converted {
            let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
        }
    }
    let lost = {
        let mut state = lock(&state);
        state.control_waiters.clear();
        let lost = state.turn_active || !state.running.is_empty();
        state.turn_active = false;
        state.running.clear();
        lost.then_some(state.agent)
    };
    if let Some(agent) = lost {
        let _ = events.send(ProviderEvent::StreamLost { agent }).await; // 받는 쪽이 연결을 버렸다
    }
}

async fn log_stderr(stderr: ChildStderr, masker: Masker) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(line = %masker.mask(&line).as_str(), "claude stderr");
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
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;

    use saturn_protocol::event::TurnOrigin;
    use saturn_protocol::ids::{Provider, SettingsRevision};

    use super::*;
    use crate::providers::SaturnDefaults;

    /// 받은 사용자 메시지 글에 따라 정해진 줄을 낸다.
    const FAKE_CLAUDE: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP;
$| = 1;
my $json = JSON::PP->new->canonical;
my %arg; for (my $i = 0; $i < @ARGV; $i++) { $arg{$ARGV[$i]} = $ARGV[$i + 1] if $ARGV[$i] =~ /^--/; }
exit 3 if ($arg{"--resume"} // "") eq "missing";
my $sid = $arg{"--resume"} // $arg{"--session-id"};
my $model = "claude-test";
my $init = 0;
sub out { print $json->encode($_[0]), "\n"; }
sub init { out({ type => "system", subtype => "init", session_id => $sid, model => $model, permissionMode => "acceptEdits", cwd => "/w", tools => ["Bash"], slash_commands => ["compact", "review", "clear", "lint"], skills => ["lint"] }); }
sub assistant { my ($content, $parent, $usage) = @_; out({ type => "assistant", session_id => $sid, parent_tool_use_id => $parent, message => { role => "assistant", model => $model, content => $content, usage => $usage // { input_tokens => 10, cache_read_input_tokens => 1000, cache_creation_input_tokens => 200, output_tokens => 5 } } }); }
sub tool_result { my ($id, $text, $parent) = @_; out({ type => "user", session_id => $sid, parent_tool_use_id => $parent, message => { role => "user", content => [ { type => "tool_result", tool_use_id => $id, content => $text } ] } }); }
sub result { out({ type => "result", subtype => $_[0] // "success", is_error => JSON::PP::false, session_id => $sid, usage => { input_tokens => 30, cache_read_input_tokens => 2000, cache_creation_input_tokens => 200, output_tokens => 40 } }); }
while (my $line = <STDIN>) {
  my $m = eval { $json->decode($line) } or next;
  if ($m->{type} eq "control_request") {
    out({ type => "control_response", response => { subtype => "success", request_id => $m->{request_id}, response => {} } });
    result("error_during_execution");
    next;
  }
  my $text = $m->{message}{content}[0]{text};
  if (!$init) { init(); $init = 1; }
  if ($text eq "hello") {
    assistant([ { type => "text", text => "hi" }, { type => "tool_use", id => "toolu_bash", name => "Bash", input => { command => "ls -la" } } ], undef);
    tool_result("toolu_bash", "total 0", undef);
    assistant([ { type => "tool_use", id => "toolu_task", name => "Task", input => { prompt => "look" } } ], undef);
    assistant([ { type => "tool_use", id => "toolu_read", name => "Read", input => { file_path => "/w/a.rs" } } ], "toolu_task", { input_tokens => 1 });
    tool_result("toolu_read", [ { type => "text", text => "fn main" } ], "toolu_task");
    tool_result("toolu_task", "found it", undef);
    assistant([ { type => "text", text => "done" } ], undef, { input_tokens => 20, cache_read_input_tokens => 3000, cache_creation_input_tokens => 100, output_tokens => 7 });
    result();
  } elsif ($text eq "wait") {
    assistant([ { type => "text", text => "working" } ], undef);
  } elsif ($text eq "more") {
    assistant([ { type => "text", text => "got more" } ], undef);
    result();
  } elsif ($text eq "ask") {
    out({ type => "control_request", request_id => "perm-1", request => { subtype => "can_use_tool", tool_name => "Bash", input => { command => "rm -rf build" }, decision_reason => "outside workdir" } });
  } elsif ($text eq "secret") {
    assistant([ { type => "text", text => "sk-secret-1234" } ], undef);
    result();
  } elsif ($text eq "/compact") {
    $model = "claude-other";
    init();
    result();
  } elsif ($text eq "orphan") {
    assistant([ { type => "tool_use", id => "toolu_bg", name => "Agent", input => {} } ], undef);
    result();
    exit 0;
  } elsif ($text eq "crash") {
    exit 1;
  }
}
"#;

    fn launch(dir: &Path, env: Vec<(OsString, OsString)>) -> LaunchSpec {
        let program = dir.join("fake-claude");
        std::fs::write(&program, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        // 새로 만든 실행 파일의 첫 실행은 macOS 검사로 수백 ms 늦다. 재개 실패 판정 시간에 걸리지 않게 한 번 미리 실행한다
        let warmed = std::process::Command::new(&program)
            .args(["--resume", "missing"])
            .status()
            .unwrap();
        assert_eq!(warmed.code(), Some(3));
        LaunchSpec {
            provider: Provider::Claude,
            program,
            workdir: dir.to_path_buf(),
            settings: SettingsRevision(1),
            user_config: UserProviderConfig::default(),
            defaults: SaturnDefaults {
                allow_edits: true,
                auto_compact_tokens: 60_000,
            },
            env,
            hook_settings: Some(json!({ "hooks": { "PreToolUse": [] } })),
            masker: Masker::new(Vec::new()),
        }
    }

    fn spec(dir: &Path, resume: Option<&str>) -> SessionSpec {
        SessionSpec {
            agent: AgentId(3),
            workdir: dir.to_path_buf(),
            model: Some("sonnet".to_owned()),
            settings: SettingsRevision(1),
            resume: resume.map(|id| ProviderSessionId(id.to_owned())),
            packet: None,
        }
    }

    async fn take(client: &mut ClaudeClient, count: usize) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        for _ in 0..count {
            let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
                .await
                .expect("event should arrive")
                .expect("stream should be open");
            events.push(event);
        }
        events
    }

    #[test]
    fn detail_of_edit_counts_changed_lines_and_keeps_path() {
        let input = json!({
            "file_path": "/w/app.cfg",
            "old_string": "mode=draft\nretries=1",
            "new_string": "mode=harbor\nretries=tundra"
        });

        let detail = detail_of("Edit", &input);

        assert_eq!(detail.category, ToolCategory::FileEdit);
        assert_eq!(detail.paths, vec!["/w/app.cfg".to_owned()]);
        assert_eq!(
            detail.changed,
            Some(LineChange {
                added: 2,
                removed: 2
            })
        );
    }

    #[test]
    fn detail_of_multi_edit_sums_every_edit() {
        let input = json!({
            "file_path": "/w/a.rs",
            "edits": [
                { "old_string": "a", "new_string": "b\nc" },
                { "old_string": "x\ny", "new_string": "z" }
            ]
        });

        let changed = detail_of("MultiEdit", &input).changed;

        assert_eq!(
            changed,
            Some(LineChange {
                added: 3,
                removed: 3
            })
        );
    }

    #[test]
    fn detail_of_write_has_no_line_change() {
        let input = json!({ "file_path": "/w/a.rs", "content": "fn main() {}" });

        let detail = detail_of("Write", &input);

        assert_eq!(detail.category, ToolCategory::FileEdit);
        assert_eq!(detail.changed, None);
    }

    #[test]
    fn detail_of_read_range_needs_offset_and_limit() {
        let ranged = json!({ "file_path": "/w/a.rs", "offset": 10, "limit": 5 });
        let partial = json!({ "file_path": "/w/a.rs", "limit": 5 });

        assert_eq!(
            detail_of("Read", &ranged).read_lines,
            Some(LineRange {
                first: 10,
                last: 14
            })
        );
        assert_eq!(detail_of("Read", &partial).read_lines, None);
    }

    #[test]
    fn detail_of_test_command_is_test_run() {
        let input = json!({ "command": "python3 -m unittest tests.test_rules" });

        assert_eq!(detail_of("Bash", &input).category, ToolCategory::TestRun);
    }

    #[test]
    fn shell_exit_code_reads_prefix_only_for_errors() {
        assert_eq!(shell_exit_code(false, "ok"), Some(0));
        assert_eq!(shell_exit_code(true, "Exit code 3\nboom"), Some(3));
        assert_eq!(shell_exit_code(true, "interrupted"), None);
    }

    #[tokio::test]
    async fn stdout_hides_judge_key_before_emitting_events() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = launch(dir.path(), Vec::new());
        config.masker = Masker::new(vec!["sk-secret-1234".to_owned()]);
        let mut client = ClaudeClient::new(config, Supervisor::new());
        let session = client
            .open_session(spec(dir.path(), None))
            .await
            .unwrap()
            .provider_session;

        client.send_turn(&session, "secret").await.unwrap();
        let events = take(&mut client, 4).await;

        assert!(matches!(&events[0], ProviderEvent::Text { text, .. } if text == "[redacted]"));
        assert!(!format!("{events:?}").contains("sk-secret-1234"));
        client.close_session(&session).await.unwrap();
    }

    #[tokio::test]
    async fn stream_input_converts_tree_and_usage() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
        let handle = client.open_session(spec(dir.path(), None)).await.unwrap();
        let session = handle.provider_session.clone();
        assert_eq!(session.0.len(), 36);
        assert!(!handle.steer_verified);

        client.send_turn(&session, "hello").await.unwrap();
        let events = take(&mut client, 11).await;

        let agent = AgentId(3);
        let task = SubagentId("toolu_task".to_owned());
        assert_eq!(
            events,
            vec![
                ProviderEvent::Text {
                    agent,
                    subagent: None,
                    text: "hi".to_owned()
                },
                ProviderEvent::ToolCall {
                    agent,
                    subagent: None,
                    call_id: "toolu_bash".to_owned(),
                    activity: Activity::RunningCommand {
                        command: "ls -la".to_owned()
                    },
                    detail: ToolDetail {
                        category: ToolCategory::Shell,
                        ..ToolDetail::default()
                    },
                },
                ProviderEvent::ToolResult {
                    agent,
                    subagent: None,
                    call_id: "toolu_bash".to_owned(),
                    output: "total 0".to_owned(),
                    exit_code: Some(0),
                },
                ProviderEvent::SubagentStarted {
                    agent,
                    subagent: task.clone(),
                    parent: None
                },
                ProviderEvent::ToolCall {
                    agent,
                    subagent: Some(task.clone()),
                    call_id: "toolu_read".to_owned(),
                    activity: Activity::ReadingFile,
                    detail: ToolDetail {
                        category: ToolCategory::FileRead,
                        paths: vec!["/w/a.rs".to_owned()],
                        ..ToolDetail::default()
                    },
                },
                ProviderEvent::ToolResult {
                    agent,
                    subagent: Some(task.clone()),
                    call_id: "toolu_read".to_owned(),
                    output: "fn main".to_owned(),
                    exit_code: None,
                },
                ProviderEvent::SubagentEnded {
                    agent,
                    subagent: task
                },
                ProviderEvent::Text {
                    agent,
                    subagent: None,
                    text: "done".to_owned()
                },
                ProviderEvent::Usage(UsageReport {
                    agent,
                    subagent: None,
                    model: Some("claude-test".to_owned()),
                    scope: UsageScope::MainTurn,
                    input: Some(30),
                    cache_read: Some(2000),
                    cache_write: Some(200),
                    output: Some(40),
                    reasoning: None,
                }),
                ProviderEvent::ContextSize {
                    agent,
                    tokens: Some(3120)
                },
                ProviderEvent::TurnCompleted {
                    agent,
                    origin: TurnOrigin::User
                },
            ]
        );
        assert_eq!(
            client.applied_settings(&session),
            Some(AppliedSettings {
                model: Some("claude-test".to_owned()),
                permission: Some("acceptEdits".to_owned()),
            })
        );
        let commands: Vec<(String, bool)> = client
            .commands()
            .into_iter()
            .map(|command| (command.name, command.is_skill))
            .collect();
        assert_eq!(
            commands,
            vec![
                ("compact".to_owned(), false),
                ("review".to_owned(), false),
                ("lint".to_owned(), true),
            ]
        );
        client.close_session(&session).await.unwrap();
        assert!(client.process_group(&session).is_none());
    }

    #[tokio::test]
    async fn steer_needs_active_turn_and_interrupt_waits_for_response() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
        let session = client
            .open_session(spec(dir.path(), None))
            .await
            .unwrap()
            .provider_session;
        let agent = AgentId(3);

        assert!(matches!(
            client.steer(&session, "early").await,
            Err(ProviderError::NoActiveTurn)
        ));
        client.send_turn(&session, "wait").await.unwrap();
        assert_eq!(
            take(&mut client, 1).await,
            vec![ProviderEvent::Text {
                agent,
                subagent: None,
                text: "working".to_owned()
            }]
        );
        assert!(matches!(
            client.compact(&session).await,
            Err(ProviderError::NotSent { .. })
        ));
        client.steer(&session, "more").await.unwrap();
        let steered = take(&mut client, 4).await;
        assert_eq!(
            steered[3],
            ProviderEvent::TurnCompleted {
                agent,
                origin: TurnOrigin::User
            }
        );

        client.send_turn(&session, "wait").await.unwrap();
        take(&mut client, 1).await;
        client
            .interrupt(
                &session,
                InterruptTarget::Subagent(SubagentId("x".to_owned())),
            )
            .await
            .unwrap();
        client
            .interrupt(&session, InterruptTarget::Main)
            .await
            .unwrap();
        let stopped = take(&mut client, 3).await;
        assert_eq!(
            stopped[2],
            ProviderEvent::TurnCompleted {
                agent,
                origin: TurnOrigin::User
            }
        );

        client.compact(&session).await.unwrap();
        let compacted = take(&mut client, 4).await;
        assert_eq!(
            compacted[0],
            ProviderEvent::SettingsApplied {
                agent,
                values: vec![
                    ("model".to_owned(), "claude-other".to_owned()),
                    ("permission_mode".to_owned(), "acceptEdits".to_owned()),
                ],
            }
        );
        client.close_session(&session).await.unwrap();
    }

    #[tokio::test]
    async fn permission_request_and_stream_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
        let session = client
            .open_session(spec(dir.path(), None))
            .await
            .unwrap()
            .provider_session;
        let agent = AgentId(3);

        client.send_turn(&session, "ask").await.unwrap();
        assert_eq!(
            take(&mut client, 1).await,
            vec![ProviderEvent::PermissionRequested {
                agent,
                request_id: "perm-1".to_owned(),
                summary: "Bash: rm -rf build".to_owned(),
                reason: "outside workdir".to_owned(),
            }]
        );
        client.send_turn(&session, "crash").await.unwrap();
        assert_eq!(
            take(&mut client, 1).await,
            vec![ProviderEvent::StreamLost { agent }]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(matches!(
            client.send_turn(&session, "again").await,
            Err(ProviderError::NotSent { .. })
        ));
        client.close_session(&session).await.unwrap();
    }

    #[tokio::test]
    async fn subagent_without_result_is_stream_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
        let session = client
            .open_session(spec(dir.path(), None))
            .await
            .unwrap()
            .provider_session;

        client.send_turn(&session, "orphan").await.unwrap();
        let events = take(&mut client, 5).await;

        assert_eq!(events[4], ProviderEvent::StreamLost { agent: AgentId(3) });
        client.close_session(&session).await.unwrap();
    }

    #[tokio::test]
    async fn resume_failure_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        // 실패 쪽은 종료를 기다리는 시간만 넉넉히, 성공 쪽은 살아 있는지만 보므로 짧게 둔다
        let mut failing = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new())
            .with_resume_settle(Duration::from_secs(30));
        let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new())
            .with_resume_settle(Duration::from_millis(100));

        let error = failing
            .open_session(spec(dir.path(), Some("missing")))
            .await
            .unwrap_err();
        let resumed = client
            .open_session(spec(dir.path(), Some("kept-session")))
            .await
            .unwrap();

        assert!(matches!(error, ProviderError::NotSent { .. }));
        assert_eq!(resumed.provider_session.0, "kept-session");
        client
            .close_session(&resumed.provider_session)
            .await
            .unwrap();
    }

    #[test]
    fn launch_args_add_defaults_and_hook_settings() {
        let dir = tempfile::tempdir().unwrap();
        let client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());

        let args = client.launch_args(&spec(dir.path(), None), &SessionArg::New("id-1".to_owned()));

        assert_eq!(
            args,
            vec![
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--session-id",
                "id-1",
                "--model",
                "sonnet",
                "--permission-mode",
                "acceptEdits",
                "--autocompact",
                "100000",
                "--settings",
                "{\"hooks\":{\"PreToolUse\":[]}}",
            ]
        );
    }

    #[test]
    fn user_settings_suppress_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude").join("settings.json"),
            r#"{"permissions":{"defaultMode":"plan"}}"#,
        )
        .unwrap();
        let env = vec![
            ("HOME".into(), home.into_os_string()),
            ("DISABLE_COMPACT".into(), "1".into()),
        ];
        let launch = launch(dir.path(), env);

        let found = read_user_config(&launch);

        assert_eq!(
            found,
            UserProviderConfig {
                has_permission: true,
                has_auto_compact: true,
            }
        );
        assert!(default_args(found, &launch).is_empty());
        let mut big = launch.clone();
        big.defaults.auto_compact_tokens = 5_000_000;
        assert_eq!(
            default_args(
                UserProviderConfig {
                    has_permission: true,
                    has_auto_compact: false
                },
                &big
            ),
            vec!["--autocompact", "1000000"]
        );
    }

    #[test]
    fn session_uuid_is_version_four() {
        let id = new_session_uuid().unwrap();

        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }
}
