//! Codex 연결: `codex app-server` 프로세스 하나로 thread(= provider session) 여러 개를 다룬다.
//! 설계: docs/design/providers-and-sessions.md
//! TODO(#61): 자식 thread의 승인 요청 처리 미정. 정해지기 전에는 부모와 같이 `PermissionRequested`로 올린다

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{
    Activity, LineChange, ProviderEvent, ToolCategory, ToolDetail, TurnOrigin, UsageReport,
    UsageScope,
};
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};

use super::tool_detail::classify_command;
use super::{AppliedSettings, LaunchSpec, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, ProcessSpec, Supervisor};
use crate::secrets::Masker;

/// `/usage` 행 이름 앞부분.
pub(crate) const DISPLAY_NAME: &str = "codex";

/// 끼워 넣기 실측(#5, #27) 통과 전이라 거짓이고, 거짓이면 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

/// 화면 전용 명령과 Saturn session 명령이 대신하는 명령은 넣지 않는다. 초안 목록.
pub(crate) const COMMAND_METHODS: &[(&str, &str)] = &[
    ("compact", "thread/compact/start"),
    ("review", "review/start"),
];

/// 대응표에 없는 이름이 스킬 목록에 섞여 올 때를 대비한다. 초안 목록.
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["new", "resume", "fork", "quit", "exit"];

/// 사용자 설정에 권한 값이 없을 때만 넣는다. 초안 값(설계는 수정 허용만 정함).
const PERMISSION_ARGS: &[&str] = &["-c", "sandbox_mode=\"workspace-write\""];

const AUTO_COMPACT_KEY: &str = "model_auto_compact_token_limit";

const PERMISSION_KEYS: &[&str] = &["approval_policy", "sandbox_mode"];

const EVENT_BUFFER: usize = 1024;

/// 대응표에 설명이 없어 메서드 이름을 쓴다.
const COMMAND_DESCRIPTION_PREFIX: &str = "app-server ";

const APPROVAL_METHODS: &[(&str, &str)] = &[
    ("item/commandExecution/requestApproval", "run command"),
    ("item/fileChange/requestApproval", "change files"),
    ("item/permissions/requestApproval", "grant permissions"),
    ("execCommandApproval", "run command"),
    ("applyPatchApproval", "change files"),
];

/// `Ok`는 `result`, `Err`는 JSON-RPC `error` 객체.
type RpcReply = oneshot::Sender<Result<serde_json::Value, serde_json::Value>>;

type Pending = Arc<Mutex<HashMap<u64, RpcReply>>>;

type Threads = Arc<Mutex<HashMap<ProviderSessionId, ThreadState>>>;

#[derive(Debug)]
struct ThreadState {
    /// 자식 thread는 부모의 에이전트.
    agent: AgentId,
    /// 메인 thread면 `None`.
    parent: Option<ProviderSessionId>,
    /// 없으면 활성 턴 없음.
    active_turn: Option<String>,
    origin: TurnOriginTracker,
    applied: AppliedSettings,
    /// `turn/started`에서 정하고 `turn/completed`에서 쓴다.
    turn_origin: Option<TurnOrigin>,
    context_tokens: Option<u64>,
}

impl ThreadState {
    fn new(agent: AgentId, parent: Option<ProviderSessionId>, applied: AppliedSettings) -> Self {
        Self {
            agent,
            parent,
            active_turn: None,
            origin: TurnOriginTracker::default(),
            applied,
            turn_origin: None,
            context_tokens: None,
        }
    }
}

#[derive(Debug)]
pub struct CodexClient {
    supervisor: Supervisor,
    group: ProcessGroupId,
    stdin: ChildStdin,
    next_request_id: u64,
    /// 읽기 작업이 `id`로 찾아 결과를 넘긴다.
    pending: Pending,
    /// 자식 thread는 읽기 작업이 첫 신호 때 등록한다.
    threads: Threads,
    events: mpsc::Receiver<ProviderEvent>,
    /// 거르기 전 목록.
    commands: Vec<ProviderCommand>,
    /// 스킬 이름별 `SKILL.md` 경로.
    skill_paths: HashMap<String, String>,
}

impl CodexClient {
    /// `launch.hook_settings`는 쓰지 않는다.
    ///
    /// # Errors
    /// 실행 실패나 `initialize` 응답 없이 stdout이 닫히면 `ConnectionLost`.
    pub async fn start(launch: LaunchSpec, supervisor: Supervisor) -> Result<Self, ProviderError> {
        let found = read_user_config(&launch);
        let user = UserProviderConfig {
            has_permission: launch.user_config.has_permission || found.has_permission,
            has_auto_compact: launch.user_config.has_auto_compact || found.has_auto_compact,
        };
        let mut args = vec!["app-server".to_owned()];
        args.extend(default_args(user, &launch));
        let spawned = supervisor
            .spawn(ProcessSpec {
                program: launch.program.clone(),
                args,
                workdir: launch.workdir.clone(),
                env: launch.env.clone(),
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "failed to start codex app-server");
                ProviderError::ConnectionLost
            })?;
        let (tx, events) = mpsc::channel(EVENT_BUFFER);
        let pending = Pending::default();
        let threads = Threads::default();
        tokio::spawn(read_loop(
            spawned.io.stdout,
            Arc::clone(&pending),
            Arc::clone(&threads),
            tx,
            launch.masker.clone(),
        ));
        tokio::spawn(log_stderr(spawned.io.stderr, launch.masker.clone()));
        let mut client = Self {
            supervisor,
            group: spawned.group,
            stdin: spawned.io.stdin,
            next_request_id: 1,
            pending,
            threads,
            events,
            commands: Vec::new(),
            skill_paths: HashMap::new(),
        };
        client.initialize().await?;
        client.load_commands(&launch.workdir).await;
        Ok(client)
    }

    /// 묶음 중지는 이 연결의 다른 thread도 멈춘다.
    pub fn process_group(&self) -> ProcessGroupId {
        self.group
    }

    /// 모르는 thread면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        lock(&self.threads)
            .get(session)
            .map(|thread| thread.applied.clone())
    }

    async fn initialize(&mut self) -> Result<(), ProviderError> {
        let params = json!({
            "clientInfo": {
                "name": "saturn",
                "title": "Saturn",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        match self.request("initialize", params).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "codex app-server rejected initialize");
                return Err(ProviderError::ConnectionLost);
            }
            Err(_) => return Err(ProviderError::ConnectionLost),
        }
        self.write_line(&json!({ "method": "initialized" }))
            .await
            .map_err(|_| ProviderError::ConnectionLost)
    }

    /// 스킬 목록을 못 받으면 대응표만 둔다.
    async fn load_commands(&mut self, workdir: &Path) {
        self.commands = COMMAND_METHODS
            .iter()
            .map(|(name, method)| ProviderCommand {
                name: (*name).to_owned(),
                description: format!("{COMMAND_DESCRIPTION_PREFIX}{method}"),
                is_skill: false,
            })
            .collect();
        let params = json!({ "cwds": [workdir.to_string_lossy()] });
        let Ok(Ok(result)) = self.request("skills/list", params).await else {
            return;
        };
        let skills = result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["skills"].as_array().into_iter().flatten());
        for skill in skills {
            let (Some(name), Some(path)) = (skill["name"].as_str(), skill["path"].as_str()) else {
                continue;
            };
            if skill["enabled"].as_bool() == Some(false) {
                continue;
            }
            let description = skill["shortDescription"]
                .as_str()
                .or_else(|| skill["description"].as_str())
                .unwrap_or_default();
            self.skill_paths.insert(name.to_owned(), path.to_owned());
            self.commands.push(ProviderCommand {
                name: name.to_owned(),
                description: description.to_owned(),
                is_skill: true,
            });
        }
    }

    /// 쓰기 전에 실패하면 `NotSent`, 쓴 뒤 응답 없이 연결이 끊기면 `Unknown`, JSON-RPC 오류 응답은 `Err(오류 객체)`.
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Result<serde_json::Value, serde_json::Value>, ProviderError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let (reply, receive) = oneshot::channel();
        lock(&self.pending).insert(id, reply);
        let message = json!({ "id": id, "method": method, "params": params });
        if let Err(error) = self.write_line(&message).await {
            lock(&self.pending).remove(&id);
            return Err(ProviderError::NotSent {
                reason: format!("failed to write request: {}", error.kind()),
            });
        }
        receive.await.map_err(|_| ProviderError::Unknown)
    }

    async fn write_line(&mut self, message: &Value) -> std::io::Result<()> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await
    }

    /// `Subagent(id)`는 `id.0`인 자식 thread.
    fn interrupt_thread(
        &self,
        session: &ProviderSessionId,
        target: &InterruptTarget,
    ) -> Option<ProviderSessionId> {
        let threads = lock(&self.threads);
        let thread = match target {
            InterruptTarget::Main => session.clone(),
            InterruptTarget::Subagent(subagent) => ProviderSessionId(subagent.0.clone()),
        };
        threads.contains_key(&thread).then_some(thread)
    }

    fn ensure_thread(&self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        if lock(&self.threads).contains_key(session) {
            return Ok(());
        }
        Err(ProviderError::NotSent {
            reason: format!("unknown session {}", session.0),
        })
    }

    async fn call_command(
        &mut self,
        session: &ProviderSessionId,
        method: &str,
    ) -> Result<(), ProviderError> {
        let params = match method {
            "review/start" => json!({
                "threadId": session.0,
                "target": { "type": "uncommittedChanges" },
            }),
            _ => json!({ "threadId": session.0 }),
        };
        rejected_as_not_sent(self.request(method, params).await)
    }

    /// `/스킬이름 나머지`면 스킬 항목과 나머지 글, 아니면 글 하나.
    fn turn_input(&self, text: &str) -> Value {
        if let Some((name, rest)) = slash_command(text)
            && let Some(path) = self.skill_paths.get(name)
        {
            let mut items = vec![json!({ "type": "skill", "name": name, "path": path })];
            if !rest.is_empty() {
                items.push(text_input(rest));
            }
            return Value::Array(items);
        }
        Value::Array(vec![text_input(text)])
    }
}

impl ProviderClient for CodexClient {
    /// `packet`이 있으면 이어서 첫 턴으로 `turn/start`한다.
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        let cwd = spec.workdir.to_string_lossy().into_owned();
        let (method, params) = match &spec.resume {
            Some(thread) => (
                "thread/resume",
                json!({ "threadId": thread.0, "cwd": cwd, "model": spec.model }),
            ),
            None => ("thread/start", json!({ "cwd": cwd, "model": spec.model })),
        };
        let result = match self.request(method, params).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                return Err(ProviderError::NotSent {
                    reason: error_message(&error),
                });
            }
            Err(_) => return Err(ProviderError::ConnectionLost),
        };
        let Some(id) = result["thread"]["id"].as_str() else {
            return Err(ProviderError::NotSent {
                reason: "thread id missing in response".to_owned(),
            });
        };
        let thread = ProviderSessionId(id.to_owned());
        let applied = AppliedSettings {
            model: result["model"].as_str().map(str::to_owned),
            permission: value_text(&result["approvalPolicy"]),
        };
        lock(&self.threads).insert(thread.clone(), ThreadState::new(spec.agent, None, applied));
        if let Some(packet) = &spec.packet {
            self.send_turn(&thread, packet).await?;
        }
        Ok(SessionHandle {
            provider_session: thread,
            steer_verified: STEER_VERIFIED,
        })
    }

    /// `/이름`이 `COMMAND_METHODS`에 있으면 `turn/start` 대신 그 메서드를 부른다.
    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        self.ensure_thread(session)?;
        if let Some((name, _)) = slash_command(text)
            && let Some((_, method)) = COMMAND_METHODS.iter().find(|(command, _)| *command == name)
        {
            return self.call_command(session, method).await;
        }
        let input = self.turn_input(text);
        if let Some(thread) = lock(&self.threads).get_mut(session) {
            thread.origin.on_user_send();
        }
        let reply = self
            .request(
                "turn/start",
                json!({ "threadId": session.0, "input": input }),
            )
            .await;
        let cancel = |threads: &Threads| {
            if let Some(thread) = lock(threads).get_mut(session) {
                thread.origin.cancel_user_send();
            }
        };
        match reply {
            Ok(Ok(result)) => {
                if let (Some(turn), Some(thread)) = (
                    result["turn"]["id"].as_str(),
                    lock(&self.threads).get_mut(session),
                ) {
                    thread.active_turn = Some(turn.to_owned());
                }
                Ok(())
            }
            Ok(Err(error)) => {
                cancel(&self.threads);
                Err(ProviderError::NotSent {
                    reason: error_message(&error),
                })
            }
            Err(error @ ProviderError::NotSent { .. }) => {
                cancel(&self.threads);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    /// 활성 턴 없음은 확정 미전달이라 `NoActiveTurn`으로 돌려주고, 그 밖의 거절은 `NotSent`.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        self.ensure_thread(session)?;
        let active = lock(&self.threads)
            .get(session)
            .and_then(|thread| thread.active_turn.clone());
        let Some(turn) = active else {
            return Err(ProviderError::NoActiveTurn);
        };
        let params = json!({
            "threadId": session.0,
            "expectedTurnId": turn,
            "input": [text_input(text)],
        });
        match self.request("turn/steer", params).await? {
            Ok(_) => Ok(()),
            Err(error) if is_no_active_turn(&error) => {
                if let Some(thread) = lock(&self.threads).get_mut(session)
                    && thread.active_turn.as_deref() == Some(turn.as_str())
                {
                    thread.active_turn = None;
                }
                Err(ProviderError::NoActiveTurn)
            }
            Err(error) => Err(ProviderError::NotSent {
                reason: error_message(&error),
            }),
        }
    }

    /// 활성 턴이 없거나 이미 끝난 턴의 오류 응답은 `Ok`로 보고, 쓰기 실패나 응답 없는 끊김만 `ConnectionLost`.
    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        let Some(thread) = self.interrupt_thread(session, &target) else {
            return Ok(());
        };
        let active = lock(&self.threads)
            .get(&thread)
            .and_then(|state| state.active_turn.clone());
        let Some(turn) = active else {
            return Ok(());
        };
        let params = json!({ "threadId": thread.0, "turnId": turn });
        match self.request("turn/interrupt", params).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => {
                tracing::debug!(error = %error, "codex interrupt rejected");
                Ok(())
            }
            Err(_) => Err(ProviderError::ConnectionLost),
        }
    }

    /// 거절 응답은 `NotSent`.
    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        self.ensure_thread(session)?;
        self.call_command(session, "thread/compact/start").await
    }

    /// 자식 thread도 함께 빼고 app-server 프로세스는 남긴다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        let reply = self
            .request("thread/unsubscribe", json!({ "threadId": session.0 }))
            .await;
        remove_thread_tree(&mut lock(&self.threads), session);
        match reply {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => {
                tracing::debug!(error = %error, "codex thread unsubscribe rejected");
                Ok(())
            }
            Err(_) => Err(ProviderError::ConnectionLost),
        }
    }

    /// 읽기 작업이 끝나 채널이 닫히면 `None`.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        self.events.recv().await
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        super::filter_commands(self.commands.clone(), EXCLUDED_COMMANDS)
    }
}

pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !user.has_permission {
        args.extend(PERMISSION_ARGS.iter().map(|arg| (*arg).to_owned()));
    }
    if !user.has_auto_compact {
        args.push("-c".to_owned());
        args.push(format!(
            "{AUTO_COMPACT_KEY}={}",
            launch.defaults.auto_compact_tokens
        ));
    }
    args
}

/// `CODEX_HOME`, `HOME`은 부모 환경이 아니라 `launch.env`에서 읽고, 파일을 못 읽으면 값 없음으로 본다.
/// 작업 공간에 TOML 파서가 없어 키 존재만 줄 단위로 본다. 초안 키 목록.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    let env = |name: &str| {
        launch
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| std::path::PathBuf::from(value))
    };
    let Some(home) = env("CODEX_HOME").or_else(|| env("HOME").map(|home| home.join(".codex")))
    else {
        return UserProviderConfig::default();
    };
    let Ok(content) = std::fs::read_to_string(home.join("config.toml")) else {
        return UserProviderConfig::default();
    };
    scan_user_config(&content)
}

fn scan_user_config(content: &str) -> UserProviderConfig {
    let mut profile = None;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        if let Some(("profile", value)) = key_value(line) {
            profile = Some(value.trim_matches(|c| c == '"' || c == '\'').to_owned());
        }
    }
    let selected = profile.map(|name| format!("profiles.{name}"));
    let mut section = String::new();
    let mut found = UserProviderConfig::default();
    for line in content.lines() {
        let line = line.trim();
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            section = header.trim().trim_matches('"').to_owned();
            continue;
        }
        let in_scope = section.is_empty() || selected.as_deref() == Some(section.as_str());
        let Some((key, _)) = key_value(line) else {
            continue;
        };
        if !in_scope {
            continue;
        }
        if PERMISSION_KEYS.contains(&key) {
            found.has_permission = true;
        }
        if key == AUTO_COMPACT_KEY {
            found.has_auto_compact = true;
        }
    }
    found
}

/// 주석과 빈 줄은 `None`.
fn key_value(line: &str) -> Option<(&str, &str)> {
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim().trim_matches('"'), value.trim()))
}

/// 자식 thread 등록과 `active_turn` 갱신도 여기서 한다. 버릴 알림이면 빈 목록.
fn convert_notification(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    params: &serde_json::Value,
) -> Vec<ProviderEvent> {
    if method == "thread/started" {
        return register_child(threads, &params["thread"]);
    }
    let Some(thread_id) = params["threadId"].as_str() else {
        return Vec::new();
    };
    let thread = ProviderSessionId(thread_id.to_owned());
    if method == "thread/closed" {
        return close_child(threads, &thread);
    }
    let Some(state) = threads.get_mut(&thread) else {
        return Vec::new();
    };
    let agent = state.agent;
    let subagent = state.parent.as_ref().map(|_| subagent_id(&thread));
    match method {
        "turn/started" => {
            state.active_turn = params["turn"]["id"].as_str().map(str::to_owned);
            if state.parent.is_none() {
                state.turn_origin = Some(state.origin.on_turn_started());
            }
            Vec::new()
        }
        "turn/completed" => {
            state.active_turn = None;
            if let Some(subagent) = subagent {
                return vec![ProviderEvent::SubagentEnded { agent, subagent }];
            }
            let origin = state
                .turn_origin
                .take()
                .unwrap_or_else(|| state.origin.on_turn_started());
            vec![
                ProviderEvent::ContextSize {
                    agent,
                    tokens: state.context_tokens,
                },
                ProviderEvent::TurnCompleted { agent, origin },
            ]
        }
        "item/agentMessage/delta" => params["delta"]
            .as_str()
            .map(|text| ProviderEvent::Text {
                agent,
                subagent,
                text: text.to_owned(),
            })
            .into_iter()
            .collect(),
        "item/started" => activity_of(&params["item"])
            .zip(params["item"]["id"].as_str())
            .map(|(activity, id)| ProviderEvent::ToolCall {
                agent,
                subagent,
                call_id: id.to_owned(),
                activity,
                detail: detail_of(&params["item"]),
            })
            .into_iter()
            .collect(),
        "item/completed" => tool_output(&params["item"])
            .zip(params["item"]["id"].as_str())
            .map(|(output, id)| ProviderEvent::ToolResult {
                agent,
                subagent,
                call_id: id.to_owned(),
                output,
                exit_code: exit_code_of(&params["item"]),
            })
            .into_iter()
            .collect(),
        "thread/tokenUsage/updated" => {
            let usage = &params["tokenUsage"];
            state.context_tokens = usage["last"]["totalTokens"].as_u64();
            vec![ProviderEvent::Usage(cumulative_usage(
                agent,
                subagent,
                state.applied.model.clone(),
                &usage["total"],
            ))]
        }
        "thread/settings/updated" => {
            let settings = &params["threadSettings"];
            state.applied.model = settings["model"].as_str().map(str::to_owned);
            state.applied.permission = value_text(&settings["approvalPolicy"]);
            let values = [
                ("model", state.applied.model.clone()),
                ("approval_policy", state.applied.permission.clone()),
            ]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
            .collect();
            vec![ProviderEvent::SettingsApplied { agent, values }]
        }
        _ => Vec::new(),
    }
}

/// 승인 요청만 `PermissionRequested`로 올리고 나머지는 버린다.
fn convert_server_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    id: &Value,
    params: &Value,
) -> Vec<ProviderEvent> {
    let Some((_, summary)) = APPROVAL_METHODS.iter().find(|(name, _)| *name == method) else {
        return Vec::new();
    };
    let thread = params["threadId"]
        .as_str()
        .or_else(|| params["conversationId"].as_str())
        .map(|id| ProviderSessionId(id.to_owned()));
    let Some(state) = thread.as_ref().and_then(|thread| threads.get(thread)) else {
        return Vec::new();
    };
    let summary = params["command"].as_str().map_or_else(
        || (*summary).to_owned(),
        |command| format!("{summary}: {command}"),
    );
    vec![ProviderEvent::PermissionRequested {
        agent: state.agent,
        request_id: value_text(id).unwrap_or_default(),
        summary,
        reason: params["reason"].as_str().unwrap_or_default().to_owned(),
    }]
}

/// 부모를 모르거나 이미 있으면 빈 목록.
fn register_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    thread: &Value,
) -> Vec<ProviderEvent> {
    let (Some(id), Some(parent)) = (thread["id"].as_str(), parent_thread_id(thread)) else {
        return Vec::new();
    };
    let child = ProviderSessionId(id.to_owned());
    let parent = ProviderSessionId(parent.to_owned());
    if threads.contains_key(&child) {
        return Vec::new();
    }
    let Some(parent_state) = threads.get(&parent) else {
        return Vec::new();
    };
    let agent = parent_state.agent;
    let parent_subagent = parent_state.parent.as_ref().map(|_| subagent_id(&parent));
    let applied = AppliedSettings {
        model: thread["model"].as_str().map(str::to_owned),
        permission: None,
    };
    threads.insert(
        child.clone(),
        ThreadState::new(agent, Some(parent), applied),
    );
    vec![ProviderEvent::SubagentStarted {
        agent,
        subagent: subagent_id(&child),
        parent: parent_subagent,
    }]
}

/// 턴이 진행 중이었으면 끝으로 보고, 메인 thread는 그대로 둔다.
fn close_child(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    thread: &ProviderSessionId,
) -> Vec<ProviderEvent> {
    let Some(state) = threads.get(thread) else {
        return Vec::new();
    };
    if state.parent.is_none() {
        return Vec::new();
    }
    let ended = state
        .active_turn
        .is_some()
        .then(|| ProviderEvent::SubagentEnded {
            agent: state.agent,
            subagent: subagent_id(thread),
        });
    threads.remove(thread);
    ended.into_iter().collect()
}

/// `parentThreadId`, 없으면 `source.subAgent.thread_spawn.parent_thread_id`.
fn parent_thread_id(thread: &Value) -> Option<&str> {
    thread["parentThreadId"]
        .as_str()
        .or_else(|| thread["source"]["subAgent"]["thread_spawn"]["parent_thread_id"].as_str())
}

fn remove_thread_tree(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    root: &ProviderSessionId,
) {
    let mut doomed = vec![root.clone()];
    let mut index = 0;
    while let Some(current) = doomed.get(index).cloned() {
        doomed.extend(
            threads
                .iter()
                .filter(|(_, state)| state.parent.as_ref() == Some(&current))
                .map(|(id, _)| id.clone()),
        );
        index += 1;
    }
    for id in doomed {
        threads.remove(&id);
    }
}

/// 도구 항목이 아니면 `None`.
fn activity_of(item: &Value) -> Option<Activity> {
    match item["type"].as_str()? {
        "commandExecution" => {
            let actions = item["commandActions"].as_array();
            let only_reads = actions.is_some_and(|actions| {
                !actions.is_empty()
                    && actions.iter().all(|action| {
                        matches!(
                            action["type"].as_str(),
                            Some("read" | "listFiles" | "search")
                        )
                    })
            });
            if only_reads {
                return Some(Activity::ReadingFile);
            }
            Some(Activity::RunningCommand {
                command: item["command"].as_str().unwrap_or_default().to_owned(),
            })
        }
        "fileChange" => Some(Activity::EditingFile),
        "reasoning" => Some(Activity::Thinking),
        "contextCompaction" => Some(Activity::Compacting),
        "mcpToolCall" => Some(Activity::RunningCommand {
            command: format!(
                "{}/{}",
                item["server"].as_str().unwrap_or_default(),
                item["tool"].as_str().unwrap_or_default()
            ),
        }),
        "dynamicToolCall" | "webSearch" => Some(Activity::RunningCommand {
            command: item["tool"]
                .as_str()
                .or_else(|| item["query"].as_str())
                .unwrap_or_default()
                .to_owned(),
        }),
        _ => None,
    }
}

/// 도구 항목이 아니면 `None`.
fn tool_output(item: &Value) -> Option<String> {
    activity_of(item)?;
    let output = match item["type"].as_str()? {
        "commandExecution" => item["aggregatedOutput"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "mcpToolCall" => match &item["result"] {
            Value::Null => item["error"].to_string(),
            result => result.to_string(),
        },
        "fileChange" => patch_text(item),
        _ => String::new(),
    };
    Some(output)
}

// cost: time O(a + p), heap O(a + p), stack O(1), alloc 1
// vars: a = commandActions 수, p = 패치 글자 수
// basis: estimate
/// `commandActions`의 경로와 `fileChange`의 경로, 바뀐 줄 수를 옮긴다. 다른 항목은 기본값이다.
fn detail_of(item: &Value) -> ToolDetail {
    match item["type"].as_str() {
        Some("commandExecution") => {
            let actions = item["commandActions"].as_array();
            let paths = actions
                .into_iter()
                .flatten()
                .filter_map(|action| action["path"].as_str().map(str::to_owned))
                .collect();
            let command = item["command"].as_str().unwrap_or_default();
            let category = if matches!(activity_of(item), Some(Activity::ReadingFile)) {
                ToolCategory::FileRead
            } else {
                classify_command(command)
            };
            ToolDetail {
                category,
                paths,
                ..ToolDetail::default()
            }
        }
        Some("fileChange") => {
            let changes = item["changes"].as_array().map_or(&[][..], Vec::as_slice);
            let total = changes.iter().map(change_lines).fold(
                LineChange {
                    added: 0,
                    removed: 0,
                },
                |total, change| LineChange {
                    added: total.added.saturating_add(change.added),
                    removed: total.removed.saturating_add(change.removed),
                },
            );
            ToolDetail {
                category: ToolCategory::FileEdit,
                paths: changes
                    .iter()
                    .filter_map(|change| change["path"].as_str().map(str::to_owned))
                    .collect(),
                read_lines: None,
                changed: Some(total),
            }
        }
        _ => ToolDetail::default(),
    }
}

// cost: time O(d), heap O(1), stack O(1)
// vars: d = diff 글자 수
// basis: estimate
/// 새 파일은 글 전체를 더한 줄로, 지운 파일은 지운 줄로 세고, 고친 파일은 diff의 `+`와 `-` 줄을 센다.
fn change_lines(change: &Value) -> LineChange {
    let diff = change["diff"].as_str().unwrap_or_default();
    let lines = || u32::try_from(diff.lines().count()).unwrap_or(u32::MAX);
    match change["kind"]["type"].as_str() {
        Some("add") => LineChange {
            added: lines(),
            removed: 0,
        },
        Some("delete") => LineChange {
            added: 0,
            removed: lines(),
        },
        _ => {
            let count = |sign: &str, header: &str| {
                let count = diff
                    .lines()
                    .filter(|line| line.starts_with(sign) && !line.starts_with(header))
                    .count();
                u32::try_from(count).unwrap_or(u32::MAX)
            };
            LineChange {
                added: count("+", "+++"),
                removed: count("-", "---"),
            }
        }
    }
}

/// 파일마다 경로 줄과 diff를 이어 붙인다.
fn patch_text(item: &Value) -> String {
    item["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|change| {
            format!(
                "{}\n{}",
                change["path"].as_str().unwrap_or_default(),
                change["diff"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 명령이 코드로 끝났을 때만 값이 있다.
fn exit_code_of(item: &Value) -> Option<i32> {
    if item["type"] != "commandExecution" {
        return None;
    }
    i32::try_from(item["exitCode"].as_i64()?).ok()
}

/// 새 입력은 캐시 포함 입력에서 캐시 읽기를 뺀 값이다.
fn cumulative_usage(
    agent: AgentId,
    subagent: Option<SubagentId>,
    model: Option<String>,
    total: &Value,
) -> UsageReport {
    let input = total["inputTokens"].as_u64();
    let cache_read = total["cachedInputTokens"].as_u64();
    UsageReport {
        agent,
        subagent,
        model,
        scope: UsageScope::ThreadCumulative,
        input: input.map(|input| input.saturating_sub(cache_read.unwrap_or(0))),
        cache_read,
        cache_write: total["cacheWriteInputTokens"].as_u64(),
        output: total["outputTokens"].as_u64(),
        reasoning: total["reasoningOutputTokens"].as_u64(),
    }
}

/// 자식 `thread_id`를 그대로 쓴다.
fn subagent_id(thread: &ProviderSessionId) -> SubagentId {
    SubagentId(thread.0.clone())
}

/// 끝나면 진행 중이던 에이전트마다 `StreamLost`.
async fn read_loop(
    stdout: ChildStdout,
    pending: Pending,
    threads: Threads,
    events: mpsc::Sender<ProviderEvent>,
    masker: Masker,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!("skipping non-json line from codex app-server");
            continue;
        };
        mask_values(&mut message, &masker);
        for event in route_message(&message, &pending, &threads) {
            let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
        }
    }
    lock(&pending).clear();
    let lost: Vec<AgentId> = {
        let threads = lock(&threads);
        let mut agents: Vec<AgentId> = threads
            .values()
            .filter(|state| state.active_turn.is_some())
            .map(|state| state.agent)
            .collect();
        agents.sort_unstable();
        agents.dedup();
        agents
    };
    for agent in lost {
        let _ = events.send(ProviderEvent::StreamLost { agent }).await; // 받는 쪽이 연결을 버렸다
    }
}

pub(super) fn mask_values(value: &mut Value, masker: &Masker) {
    match value {
        Value::String(text) => *text = masker.mask(text).as_str().to_owned(),
        Value::Array(items) => {
            for item in items {
                mask_values(item, masker);
            }
        }
        Value::Object(fields) => {
            for item in fields.values_mut() {
                mask_values(item, masker);
            }
        }
        _ => {}
    }
}

fn route_message(message: &Value, pending: &Pending, threads: &Threads) -> Vec<ProviderEvent> {
    match (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) {
        (Some(method), Some(id)) => {
            convert_server_request(&lock(threads), method, id, &message["params"])
        }
        (Some(method), None) => {
            convert_notification(&mut lock(threads), method, &message["params"])
        }
        (None, Some(id)) => {
            let reply = id.as_u64().and_then(|id| lock(pending).remove(&id));
            if let Some(reply) = reply {
                let result = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = reply.send(result); // 기다리던 요청이 이미 포기했다
            }
            Vec::new()
        }
        (None, None) => Vec::new(),
    }
}

async fn log_stderr(stderr: ChildStderr, masker: Masker) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(line = %masker.mask(&line).as_str(), "codex app-server stderr");
    }
}

/// 쓰기 실패와 응답 없는 끊김은 그대로 둔다.
fn rejected_as_not_sent(
    reply: Result<Result<Value, Value>, ProviderError>,
) -> Result<(), ProviderError> {
    match reply? {
        Ok(_) => Ok(()),
        Err(error) => Err(ProviderError::NotSent {
            reason: error_message(&error),
        }),
    }
}

/// 오류 코드가 따로 없어 문구로 판정한다. 초안.
fn is_no_active_turn(error: &Value) -> bool {
    let message = error_message(error).to_ascii_lowercase();
    [
        "no active turn",
        "not active",
        "expected turn",
        "turn mismatch",
        "no turn",
    ]
    .iter()
    .any(|pattern| message.contains(pattern))
}

fn error_message(error: &Value) -> String {
    error["message"]
        .as_str()
        .map_or_else(|| error.to_string(), str::to_owned)
}

/// null이면 `None`.
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

fn text_input(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn slash_command(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (!name.is_empty()).then_some((name, args.trim()))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // 잠금을 쥔 채 panic하는 코드가 없으니 독이 든 잠금도 내용은 온전하다
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::Duration;

    use saturn_protocol::ids::{Provider, SettingsRevision};

    use super::*;
    use crate::providers::SaturnDefaults;

    /// 받은 요청에 schema 모양 그대로 응답한다.
    const FAKE_APP_SERVER: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP;
$| = 1;
my $json = JSON::PP->new->canonical;
sub out { print $json->encode($_[0]), "\n"; }
sub note { out({ method => $_[0], params => $_[1] }); }
my $active = "";
while (my $line = <STDIN>) {
  my $m = eval { $json->decode($line) } or next;
  my $method = $m->{method} // "";
  my $id = $m->{id};
  next unless defined $id;
  my $p = $m->{params} // {};
  my $tid = $p->{threadId} // "thr_main";
  if ($method eq "initialize") {
    out({ id => $id, result => { userAgent => "fake/0.158.0", platformFamily => "unix", platformOs => "macos", codexHome => "/fake" } });
  } elsif ($method eq "skills/list") {
    out({ id => $id, result => { data => [ { cwd => "/w", errors => [], skills => [
      { name => "lint", description => "Run the linter", shortDescription => "lint it", enabled => JSON::PP::true, path => "/skills/lint/SKILL.md", scope => "user" },
      { name => "off", description => "disabled", enabled => JSON::PP::false, path => "/skills/off/SKILL.md", scope => "user" } ] } ] } });
  } elsif ($method eq "thread/start" || $method eq "thread/resume") {
    out({ id => $id, result => { thread => { id => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" }, model => "gpt-test", modelProvider => "openai", approvalPolicy => "on-request", cwd => "/w" } });
  } elsif ($method eq "turn/start") {
    my $first = $p->{input}[0];
    if (($first->{text} // "") eq "crash") {
      out({ id => $id, result => { turn => { id => "turn_x", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_x", status => "inProgress", items => [] } });
      exit 1;
    }
    $active = "turn_1";
    out({ id => $id, result => { turn => { id => "turn_1", status => "inProgress", items => [] } } });
    if ($first->{type} eq "skill") {
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_1", itemId => "m0", delta => "skill:$first->{name}:$first->{path}" });
      next;
    }
    note("turn/started", { threadId => $tid, turn => { id => "turn_1", status => "inProgress", items => [] } });
    note("thread/started", { thread => { id => "thr_child", parentThreadId => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" } });
    note("turn/started", { threadId => "thr_child", turn => { id => "turn_c", status => "inProgress", items => [] } });
    note("item/agentMessage/delta", { threadId => "thr_child", turnId => "turn_c", itemId => "m1", delta => "looking" });
    note("item/started", { threadId => $tid, turnId => "turn_1", startedAtMs => 1, item => { type => "commandExecution", id => "item_1", command => "cargo test", cwd => "/w", status => "inProgress", commandActions => [ { type => "unknown", command => "cargo test" } ] } });
    note("item/completed", { threadId => $tid, turnId => "turn_1", completedAtMs => 2, item => { type => "commandExecution", id => "item_1", command => "cargo test", cwd => "/w", status => "completed", aggregatedOutput => "ok", exitCode => 0, commandActions => [] } });
    out({ id => "srv-1", method => "item/commandExecution/requestApproval", params => { threadId => $tid, turnId => "turn_1", itemId => "item_2", command => "rm -rf build", reason => "needs write", startedAtMs => 3 } });
    note("thread/tokenUsage/updated", { threadId => $tid, turnId => "turn_1", tokenUsage => { modelContextWindow => 272000,
      total => { inputTokens => 1200, cachedInputTokens => 200, outputTokens => 50, reasoningOutputTokens => 10, totalTokens => 1250 },
      last => { inputTokens => 900, cachedInputTokens => 100, outputTokens => 40, reasoningOutputTokens => 5, totalTokens => 940 } } });
    note("turn/completed", { threadId => "thr_child", turn => { id => "turn_c", status => "completed", items => [] } });
  } elsif ($method eq "turn/steer") {
    if ($p->{input}[0]{text} eq "late" || $p->{expectedTurnId} ne $active) {
      out({ id => $id, error => { code => -32600, message => "no active turn to steer" } });
    } else {
      out({ id => $id, result => { turnId => $active } });
    }
  } elsif ($method eq "turn/interrupt") {
    out({ id => $id, result => {} });
    note("turn/completed", { threadId => $tid, turn => { id => $p->{turnId}, status => "interrupted", items => [] } });
    $active = "";
  } elsif ($method eq "thread/compact/start" || $method eq "thread/unsubscribe" || $method eq "review/start") {
    out({ id => $id, result => {} });
  } else {
    out({ id => $id, error => { code => -32601, message => "method not found: $method" } });
  }
}
"#;

    fn launch(dir: &Path, env: Vec<(std::ffi::OsString, std::ffi::OsString)>) -> LaunchSpec {
        let program = dir.join("fake-codex");
        std::fs::write(&program, FAKE_APP_SERVER).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        LaunchSpec {
            provider: Provider::Codex,
            program,
            workdir: dir.to_path_buf(),
            settings: SettingsRevision(1),
            user_config: UserProviderConfig::default(),
            defaults: SaturnDefaults {
                allow_edits: true,
                auto_compact_tokens: 180_000,
            },
            env,
            hook_settings: None,
            masker: Masker::new(Vec::new()),
        }
    }

    fn spec(dir: &Path) -> SessionSpec {
        SessionSpec {
            agent: AgentId(7),
            workdir: dir.to_path_buf(),
            model: None,
            settings: SettingsRevision(1),
            resume: None,
            packet: None,
        }
    }

    async fn take(client: &mut CodexClient, count: usize) -> Vec<ProviderEvent> {
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

    async fn start(dir: &Path) -> (CodexClient, SessionHandle) {
        let mut client = CodexClient::start(launch(dir, Vec::new()), Supervisor::new())
            .await
            .unwrap();
        let handle = client.open_session(spec(dir)).await.unwrap();
        (client, handle)
    }

    #[tokio::test]
    async fn turn_events_are_converted_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session.clone();
        assert_eq!(main.0, "thr_main");
        assert!(!handle.steer_verified);
        assert_eq!(
            client.applied_settings(&main),
            Some(AppliedSettings {
                model: Some("gpt-test".to_owned()),
                permission: Some("on-request".to_owned()),
            })
        );

        client.send_turn(&main, "fix the build").await.unwrap();
        let events = take(&mut client, 7).await;

        let agent = AgentId(7);
        let child = Some(SubagentId("thr_child".to_owned()));
        assert_eq!(
            events,
            vec![
                ProviderEvent::SubagentStarted {
                    agent,
                    subagent: SubagentId("thr_child".to_owned()),
                    parent: None,
                },
                ProviderEvent::Text {
                    agent,
                    subagent: child.clone(),
                    text: "looking".to_owned(),
                },
                ProviderEvent::ToolCall {
                    agent,
                    subagent: None,
                    call_id: "item_1".to_owned(),
                    activity: Activity::RunningCommand {
                        command: "cargo test".to_owned(),
                    },
                    detail: ToolDetail {
                        category: ToolCategory::TestRun,
                        ..ToolDetail::default()
                    },
                },
                ProviderEvent::ToolResult {
                    agent,
                    subagent: None,
                    call_id: "item_1".to_owned(),
                    output: "ok".to_owned(),
                    exit_code: Some(0),
                },
                ProviderEvent::PermissionRequested {
                    agent,
                    request_id: "srv-1".to_owned(),
                    summary: "run command: rm -rf build".to_owned(),
                    reason: "needs write".to_owned(),
                },
                ProviderEvent::Usage(UsageReport {
                    agent,
                    subagent: None,
                    model: Some("gpt-test".to_owned()),
                    scope: UsageScope::ThreadCumulative,
                    input: Some(1000),
                    cache_read: Some(200),
                    cache_write: None,
                    output: Some(50),
                    reasoning: Some(10),
                }),
                ProviderEvent::SubagentEnded {
                    agent,
                    subagent: SubagentId("thr_child".to_owned()),
                },
            ]
        );

        client.steer(&main, "also run clippy").await.unwrap();
        client
            .interrupt(
                &main,
                InterruptTarget::Subagent(SubagentId("gone".to_owned())),
            )
            .await
            .unwrap();
        client
            .interrupt(&main, InterruptTarget::Main)
            .await
            .unwrap();
        let end = take(&mut client, 2).await;
        assert_eq!(
            end,
            vec![
                ProviderEvent::ContextSize {
                    agent,
                    tokens: Some(940),
                },
                ProviderEvent::TurnCompleted {
                    agent,
                    origin: TurnOrigin::User,
                },
            ]
        );
        assert!(matches!(
            client.steer(&main, "after end").await,
            Err(ProviderError::NoActiveTurn)
        ));
    }

    #[tokio::test]
    async fn rejected_steer_is_no_active_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, "start").await.unwrap();

        let error = client.steer(&main, "late").await.unwrap_err();

        assert!(matches!(error, ProviderError::NoActiveTurn));
        assert!(matches!(
            client.steer(&main, "again").await,
            Err(ProviderError::NoActiveTurn)
        ));
    }

    #[tokio::test]
    async fn commands_compact_skills_and_close() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;

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
        client.compact(&main).await.unwrap();
        client.send_turn(&main, "/compact").await.unwrap();
        client.send_turn(&main, "/lint src").await.unwrap();
        let skill = take(&mut client, 1).await;
        assert_eq!(
            skill,
            vec![ProviderEvent::Text {
                agent: AgentId(7),
                subagent: None,
                text: "skill:lint:/skills/lint/SKILL.md".to_owned(),
            }]
        );

        client.close_session(&main).await.unwrap();

        assert!(matches!(
            client.send_turn(&main, "hi").await,
            Err(ProviderError::NotSent { .. })
        ));
        assert!(client.applied_settings(&main).is_none());
    }

    #[tokio::test]
    async fn stream_end_mid_turn_reports_lost() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;

        client
            .send_turn(&handle.provider_session, "crash")
            .await
            .unwrap();
        let events = take(&mut client, 1).await;

        assert_eq!(
            events,
            vec![ProviderEvent::StreamLost { agent: AgentId(7) }]
        );
        let closed = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .unwrap();
        assert_eq!(closed, None);
        assert!(matches!(
            client.compact(&handle.provider_session).await,
            Err(ProviderError::NotSent { .. })
        ));
    }

    #[tokio::test]
    async fn missing_program_is_connection_lost() {
        let dir = tempfile::tempdir().unwrap();
        let mut launch = launch(dir.path(), Vec::new());
        launch.program = PathBuf::from("/nonexistent/codex");

        let error = CodexClient::start(launch, Supervisor::new())
            .await
            .unwrap_err();

        assert!(matches!(error, ProviderError::ConnectionLost));
    }

    #[test]
    fn defaults_respect_user_config() {
        let dir = tempfile::tempdir().unwrap();
        let launch = launch(dir.path(), Vec::new());

        let none = default_args(UserProviderConfig::default(), &launch);
        let both = default_args(
            UserProviderConfig {
                has_permission: true,
                has_auto_compact: true,
            },
            &launch,
        );

        assert_eq!(
            none,
            vec![
                "-c",
                "sandbox_mode=\"workspace-write\"",
                "-c",
                "model_auto_compact_token_limit=180000"
            ]
        );
        assert!(both.is_empty());
    }

    #[test]
    fn user_config_reads_root_and_selected_profile() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("config.toml"),
            "profile = \"fast\"\n# approval_policy = \"never\"\n[profiles.slow]\nsandbox_mode = \"read-only\"\n\
             [profiles.fast]\nmodel_auto_compact_token_limit = 9000\n",
        )
        .unwrap();
        let env = vec![("CODEX_HOME".into(), home.clone().into_os_string())];

        let found = read_user_config(&launch(dir.path(), env));

        assert_eq!(
            found,
            UserProviderConfig {
                has_permission: false,
                has_auto_compact: true,
            }
        );
        assert_eq!(
            scan_user_config("approval_policy = \"never\"\n"),
            UserProviderConfig {
                has_permission: true,
                has_auto_compact: false,
            }
        );
        assert_eq!(
            read_user_config(&launch(dir.path(), Vec::new())),
            UserProviderConfig::default()
        );
    }

    #[test]
    fn nested_child_threads_point_to_parent_subagent() {
        let mut threads = HashMap::new();
        let main = ProviderSessionId("main".to_owned());
        threads.insert(
            main,
            ThreadState::new(AgentId(1), None, AppliedSettings::default()),
        );
        let started =
            |id: &str, parent: &str| json!({ "thread": { "id": id, "parentThreadId": parent } });

        let first = convert_notification(&mut threads, "thread/started", &started("a", "main"));
        let second = convert_notification(&mut threads, "thread/started", &started("b", "a"));
        let unknown = convert_notification(&mut threads, "thread/started", &started("c", "zzz"));

        assert_eq!(
            second,
            vec![ProviderEvent::SubagentStarted {
                agent: AgentId(1),
                subagent: SubagentId("b".to_owned()),
                parent: Some(SubagentId("a".to_owned())),
            }]
        );
        assert_eq!(first.len(), 1);
        assert!(unknown.is_empty());
        remove_thread_tree(&mut threads, &ProviderSessionId("a".to_owned()));
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn read_only_commands_are_reading() {
        let item = json!({
            "type": "commandExecution",
            "command": "rg foo",
            "commandActions": [{ "type": "search", "command": "rg foo" }],
        });

        assert_eq!(activity_of(&item), Some(Activity::ReadingFile));
        assert_eq!(
            activity_of(&json!({ "type": "fileChange" })),
            Some(Activity::EditingFile)
        );
        assert_eq!(activity_of(&json!({ "type": "agentMessage" })), None);
        assert!(is_no_active_turn(&json!({ "message": "No active turn" })));
        assert!(!is_no_active_turn(&json!({ "message": "invalid input" })));
    }

    #[test]
    fn detail_of_read_command_keeps_action_paths() {
        let item = json!({
            "type": "commandExecution",
            "command": "/bin/zsh -lc 'cat notes/a.txt'",
            "commandActions": [
                { "type": "read", "command": "cat notes/a.txt", "name": "a.txt", "path": "/work/notes/a.txt" }
            ],
        });

        let detail = detail_of(&item);

        assert_eq!(detail.category, ToolCategory::FileRead);
        assert_eq!(detail.paths, vec!["/work/notes/a.txt".to_owned()]);
    }

    #[test]
    fn detail_of_test_command_is_test_run_without_paths() {
        let item = json!({
            "type": "commandExecution",
            "command": "/bin/zsh -lc 'python3 -m unittest tests.test_rules'",
            "commandActions": [{ "type": "unknown", "command": "python3 -m unittest tests.test_rules" }],
        });

        let detail = detail_of(&item);

        assert_eq!(detail.category, ToolCategory::TestRun);
        assert!(detail.paths.is_empty());
    }

    #[test]
    fn detail_of_file_change_counts_diff_lines() {
        let item = json!({
            "type": "fileChange",
            "changes": [{
                "path": "/work/config/app.cfg",
                "kind": { "type": "update", "move_path": null },
                "diff": "@@ -1,4 +1,4 @@\n name=demo\n-mode=draft\n-retries=1\n+mode=harbor\n+retries=tundra\n level=2\n",
            }],
        });

        let detail = detail_of(&item);

        assert_eq!(detail.category, ToolCategory::FileEdit);
        assert_eq!(detail.paths, vec!["/work/config/app.cfg".to_owned()]);
        assert_eq!(
            detail.changed,
            Some(LineChange {
                added: 2,
                removed: 2
            })
        );
    }

    #[test]
    fn detail_of_file_change_added_file_counts_whole_text() {
        let item = json!({
            "type": "fileChange",
            "changes": [{ "path": "/work/new.txt", "kind": { "type": "add" }, "diff": "a\nb\nc\n" }],
        });

        let changed = detail_of(&item).changed;

        assert_eq!(
            changed,
            Some(LineChange {
                added: 3,
                removed: 0
            })
        );
    }

    #[test]
    fn tool_output_file_change_is_path_and_diff() {
        let item = json!({
            "type": "fileChange",
            "changes": [{ "path": "/work/a.cfg", "kind": { "type": "update" }, "diff": "-a\n+b\n" }],
        });

        assert_eq!(tool_output(&item).as_deref(), Some("/work/a.cfg\n-a\n+b\n"));
    }

    #[test]
    fn exit_code_of_command_reads_code_and_ignores_other_items() {
        let failed = json!({ "type": "commandExecution", "exitCode": 3 });
        let running = json!({ "type": "commandExecution", "exitCode": null });
        let patch = json!({ "type": "fileChange", "exitCode": 1 });

        assert_eq!(exit_code_of(&failed), Some(3));
        assert_eq!(exit_code_of(&running), None);
        assert_eq!(exit_code_of(&patch), None);
    }

    #[test]
    fn app_server_values_hide_judge_key_before_routing() {
        let key = "sk-secret-1234";
        let mut message = json!({
            "id": 1,
            "error": { "message": format!("bad key {key}") },
            "params": { "item": { "aggregatedOutput": [key] } },
        });

        mask_values(&mut message, &Masker::new(vec![key.to_owned()]));

        assert!(!message.to_string().contains(key));
        assert_eq!(message["error"]["message"], "bad key [redacted]");
    }
}
