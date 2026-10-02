//! Codex 연결: `codex app-server` 프로세스 하나로 thread(= provider session) 여러 개를 다룬다.
//! 설계: docs/design/providers-and-sessions.md
//! TODO(#61): 자식 thread의 승인 요청 처리 미정. 정해지기 전에는 부모와 같이 `PermissionRequested`로 올린다

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{
    Activity, LineChange, ProviderEvent, ToolCategory, ToolDetail, TurnOrigin, UsageReport,
    UsageScope,
};
use saturn_protocol::ids::{AgentId, Provider, ProviderSessionId, SubagentId};
use saturn_protocol::input::{InputAnswer, InputRequest};
use saturn_protocol::rpc::{ModelChoice, ModelInfo, PermissionAnswer};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};

use super::codex_input::{self, InputKind};
use super::codex_permission::{
    APPROVAL_POLICY, McpState, SANDBOX, call_of, check_applied, file_change_paths, mcp_state,
};
use super::tool_detail::{classify_command, unwrap_shell};
use super::{AppliedSettings, LaunchSpec, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, ProcessSpec, Supervisor};
use crate::secrets::Masker;

/// `/usage` 행 이름 앞부분.
pub(crate) const DISPLAY_NAME: &str = "codex";

/// 설정에 실행 파일이 없을 때 `PATH`에서 찾는 이름.
pub(crate) const PROGRAM: &str = "codex";

/// 끼워 넣기 실측(#5, #27) 통과 전이라 거짓이고, 거짓이면 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

/// 화면 전용 명령과 Saturn session 명령이 대신하는 명령은 넣지 않는다. 초안 목록.
pub(crate) const COMMAND_METHODS: &[(&str, &str)] = &[
    ("compact", "thread/compact/start"),
    ("review", "review/start"),
];

/// 대응표에 없는 이름이 스킬 목록에 섞여 올 때를 대비한다. 초안 목록.
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["new", "resume", "fork", "quit", "exit", "model"];

const AUTO_COMPACT_KEY: &str = "model_auto_compact_token_limit";

const EVENT_BUFFER: usize = 1024;

/// 첫 턴 전에 MCP 서버가 준비되기를 기다리는 최대 시간. 서버 유예 12초에 시작 시간 제한을 더한 값이다. 초안.
const MCP_READY_TIMEOUT: Duration = Duration::from_secs(30);

const MCP_READY_POLL: Duration = Duration::from_millis(250);

/// 대응표에 설명이 없어 메서드 이름을 쓴다.
const COMMAND_DESCRIPTION_PREFIX: &str = "app-server ";

const APPROVAL_METHODS: &[(&str, &str)] = &[
    ("item/commandExecution/requestApproval", "run command"),
    ("item/fileChange/requestApproval", "change files"),
    ("item/permissions/requestApproval", "grant permissions"),
    (ELICITATION_METHOD, "use MCP tool"),
    ("execCommandApproval", "run command"),
    ("applyPatchApproval", "change files"),
];

/// MCP 도구 승인은 이 요청의 `_meta.codex_approval_kind`가 있는 것만 허가 요청으로 보고, 없는 것은 입력 요청이다.
const ELICITATION_METHOD: &str = "mcpServer/elicitation/request";

/// 에이전트 질문. `default_mode_request_user_input` 기능이 켜져 있을 때만 온다.
const USER_INPUT_METHOD: &str = "item/tool/requestUserInput";

const PERMISSIONS_METHOD: &str = "item/permissions/requestApproval";

/// `ProviderSessionId`가 아니라 `request_id`로 찾는다.
type Approvals = Arc<Mutex<HashMap<String, PendingApproval>>>;

/// `Ok`는 `result`, `Err`는 JSON-RPC `error` 객체.
type RpcReply = oneshot::Sender<Result<serde_json::Value, serde_json::Value>>;

type Pending = Arc<Mutex<HashMap<u64, RpcReply>>>;

type Threads = Arc<Mutex<HashMap<ProviderSessionId, ThreadState>>>;

/// 답을 기다리는 승인 요청. 같은 JSON-RPC 번호로 응답해야 해서 요청이 보낸 번호를 그대로 둔다.
#[derive(Debug, Clone)]
struct PendingApproval {
    /// 숫자와 문자열 둘 다 올 수 있어 원래 값을 쓴다.
    id: Value,
    method: String,
    /// 요청이 알려 준 결정 이름. 요청에 목록이 없으면 `None`.
    available: Option<Vec<String>>,
    /// `item/permissions/requestApproval`이 원한 권한. 허용 응답에 그대로 돌려준다.
    permissions: Value,
    /// 입력 요청이면 응답 모양과 올린 요청. 승인이면 `None`.
    input: Option<PendingInput>,
}

#[derive(Debug, Clone)]
struct PendingInput {
    kind: InputKind,
    request: InputRequest,
}

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
    /// 키는 `fileChange` 항목 id. 편집 승인 요청에는 경로가 없어 항목이 시작될 때 받은 경로를 둔다.
    file_changes: HashMap<String, Vec<String>>,
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
            file_changes: HashMap::new(),
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
    /// 읽기 작업이 승인 요청을 넣고 `answer_permission`이 꺼낸다.
    approvals: Approvals,
    events: mpsc::Receiver<ProviderEvent>,
    /// 거르기 전 목록.
    commands: Vec<ProviderCommand>,
    /// 스킬 이름별 `SKILL.md` 경로.
    skill_paths: HashMap<String, String>,
    /// 첫 턴 전에 준비를 확인할 MCP 서버. 비면 확인하지 않는다.
    mcp_servers: Vec<String>,
    /// 서버가 모두 준비된 것을 확인했다.
    is_mcp_ready: bool,
    mcp_ready_timeout: Duration,
}

impl CodexClient {
    /// `launch.hook_settings`는 쓰지 않는다. `launch.permission.codex_home`이 있으면 `CODEX_HOME`을 그 폴더로 바꿔
    /// 사용자 설정과 규칙이 끼어들지 못하게 한다.
    ///
    /// # Errors
    /// 실행 실패나 `initialize` 응답 없이 stdout이 닫히면 `ConnectionLost`, 지원하지 않는 버전이면 `NotSent`.
    pub async fn start(launch: LaunchSpec, supervisor: Supervisor) -> Result<Self, ProviderError> {
        let launch = with_codex_home(launch);
        let found = read_user_config(&launch);
        let user = UserProviderConfig {
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
        let approvals = Approvals::default();
        tokio::spawn(read_loop(
            spawned.io.stdout,
            Arc::clone(&pending),
            Arc::clone(&threads),
            Arc::clone(&approvals),
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
            approvals,
            events,
            commands: Vec::new(),
            skill_paths: HashMap::new(),
            mcp_servers: launch.permission.mcp_servers.clone(),
            is_mcp_ready: false,
            mcp_ready_timeout: MCP_READY_TIMEOUT,
        };
        if let Err(error) = client.initialize().await {
            client.abort_start().await;
            return Err(error);
        }
        client.load_commands(&launch.workdir).await;
        Ok(client)
    }

    /// 열지 않기로 한 app-server를 남기지 않는다.
    async fn abort_start(&mut self) {
        let stopped = self
            .supervisor
            .stop_tree(self.group, crate::processes::StopScope::Whole)
            .await;
        if let Err(error) = stopped {
            tracing::warn!(%error, "failed to stop rejected codex app-server");
        }
        self.supervisor.release(self.group);
    }

    /// 부하에서 준비가 늦어지는 시험이 기다리는 시간을 줄이거나 늘리는 데 쓴다.
    #[cfg(test)]
    fn with_mcp_ready_timeout(mut self, timeout: Duration) -> Self {
        self.mcp_ready_timeout = timeout;
        self
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
            Ok(Ok(result)) => {
                // 버전으로 열기를 막지 않는다. 적용된 정책은 thread를 열 때 확인한다
                tracing::info!(
                    user_agent = result["userAgent"].as_str().unwrap_or_default(),
                    "codex app-server initialized"
                );
            }
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

    // cost: time O(t/p·s), heap O(1), stack O(1), io t/p
    // vars: t = 제한 시간, p = 확인 간격, s = 서버 수
    // basis: estimate
    /// 첫 session을 열기 전에 대상 MCP 서버가 모두 준비될 때까지 `mcpServerStatus/list`로 확인한다.
    /// 제한 시간 안에 준비되지 않으면 `NotSent`이고 첫 턴을 보내지 않는다.
    async fn wait_for_mcp(&mut self) -> Result<(), ProviderError> {
        if self.is_mcp_ready || self.mcp_servers.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + self.mcp_ready_timeout;
        loop {
            let waiting = match self.request("mcpServerStatus/list", json!({})).await? {
                Ok(result) => match mcp_state(&result, &self.mcp_servers) {
                    McpState::Ready => {
                        self.is_mcp_ready = true;
                        return Ok(());
                    }
                    McpState::Waiting(reason) => reason,
                },
                Err(error) => error_message(&error),
            };
            if Instant::now() >= deadline {
                return Err(ProviderError::NotSent {
                    reason: format!("mcp servers are not ready: {waiting}"),
                });
            }
            tokio::time::sleep(MCP_READY_POLL).await;
        }
    }

    /// 적용값이 기대와 달라 쓰지 않는 thread를 구독에서 뺀다. 실패해도 이미 쓰지 않기로 했다.
    async fn release_unchecked_thread(&mut self, thread: &ProviderSessionId) {
        let released = self
            .request("thread/unsubscribe", json!({ "threadId": thread.0 }))
            .await;
        if let Err(error) = released {
            tracing::debug!(%error, "failed to release unchecked codex thread");
        }
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
        self.wait_for_mcp().await?;
        let cwd = spec.workdir.to_string_lossy().into_owned();
        let mut params = json!({
            "cwd": cwd,
            "model": spec.model,
            "approvalPolicy": APPROVAL_POLICY,
            "sandbox": SANDBOX,
        });
        let method = match &spec.resume {
            Some(thread) => {
                params["threadId"] = json!(thread.0);
                "thread/resume"
            }
            None => "thread/start",
        };
        if !spec.add_dirs.is_empty() {
            // app-server에는 `--add-dir` 인자가 없어 thread 설정의 쓰기 가능 폴더로 넘긴다. 읽기 전용 샌드박스에서의 효과는 실측 전이다
            let dirs: Vec<String> = spec
                .add_dirs
                .iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect();
            params["config"] = json!({ "sandbox_workspace_write": { "writable_roots": dirs } });
        }
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
        if let Err(reason) = check_applied(&result) {
            self.release_unchecked_thread(&thread).await;
            return Err(ProviderError::NotSent { reason });
        }
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

    /// 요청이 보낸 JSON-RPC 번호로 결정을 돌려준다. 모르는 요청이면 `NotSent`이고, 쓰기에 실패하면 요청을
    /// 되돌려 다시 답할 수 있게 한다. 자식 thread의 승인도 부모 요청과 같은 경로로 답한다.
    /// TODO(#61): 자식 thread 승인 요청의 처리 방식
    async fn answer_permission(
        &mut self,
        _session: &ProviderSessionId,
        request_id: &str,
        answer: PermissionAnswer,
    ) -> Result<(), ProviderError> {
        let not_sent = || ProviderError::NotSent {
            reason: format!("unknown permission request {request_id}"),
        };
        let pending = lock(&self.approvals)
            .remove(request_id)
            .ok_or_else(not_sent)?;
        if pending.input.is_some() {
            lock(&self.approvals).insert(request_id.to_owned(), pending);
            return Err(not_sent());
        }
        let message = json!({ "id": pending.id, "result": approval_result(&pending, &answer) });
        if let Err(error) = self.write_line(&message).await {
            lock(&self.approvals).insert(request_id.to_owned(), pending);
            return Err(ProviderError::NotSent {
                reason: format!("failed to write approval response: {}", error.kind()),
            });
        }
        Ok(())
    }

    /// 요청이 보낸 JSON-RPC 번호로 입력 요청에 답한다. 승인 요청이나 모르는 요청이면 `NotSent`이고, 쓰기에
    /// 실패하면 요청을 되돌려 다시 답할 수 있게 한다.
    async fn answer_input(
        &mut self,
        _session: &ProviderSessionId,
        request_id: &str,
        answer: InputAnswer,
    ) -> Result<(), ProviderError> {
        let not_sent = || ProviderError::NotSent {
            reason: format!("unknown input request {request_id}"),
        };
        let pending = lock(&self.approvals)
            .remove(request_id)
            .ok_or_else(not_sent)?;
        let Some(input) = pending.input.clone() else {
            lock(&self.approvals).insert(request_id.to_owned(), pending);
            return Err(not_sent());
        };
        let message = json!({
            "id": pending.id,
            "result": codex_input::result(input.kind, &input.request, &answer),
        });
        if let Err(error) = self.write_line(&message).await {
            lock(&self.approvals).insert(request_id.to_owned(), pending);
            return Err(ProviderError::NotSent {
                reason: format!("failed to write input response: {}", error.kind()),
            });
        }
        Ok(())
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

    /// app-server `model/list`. 숨긴 모델은 빼고 쪽마다 다음 쪽 표시(`nextCursor`)를 따라간다.
    async fn list_models(&mut self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({ "cursor": cursor }),
                None => json!({}),
            };
            let result = self.request("model/list", params).await?.map_err(|error| {
                ProviderError::NotSent {
                    reason: error_message(&error),
                }
            })?;
            models.extend(
                result["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(model_info),
            );
            cursor = result["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() {
                return Ok(models);
            }
        }
    }

    fn commands(&self) -> Vec<ProviderCommand> {
        super::filter_commands(self.commands.clone(), EXCLUDED_COMMANDS)
    }
}

/// 권한은 Saturn 규칙이 정하므로 권한 인자는 넣지 않는다. 승인 정책과 샌드박스는 `thread/start`가 정한다.
/// 숨긴 모델이거나 모델 이름이 없으면 `None`.
fn model_info(entry: &Value) -> Option<ModelInfo> {
    if entry["hidden"].as_bool() == Some(true) {
        return None;
    }
    let model = entry["model"].as_str().or_else(|| entry["id"].as_str())?;
    Some(ModelInfo {
        choice: ModelChoice {
            provider: Provider::Codex,
            model: model.to_owned(),
        },
        name: entry["displayName"].as_str().unwrap_or(model).to_owned(),
    })
}

pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    let mut args = Vec::new();
    if !user.has_auto_compact {
        args.push("-c".to_owned());
        args.push(format!(
            "{AUTO_COMPACT_KEY}={}",
            launch.defaults.auto_compact_tokens
        ));
    }
    args
}

/// 전용 `CODEX_HOME`이 정해져 있으면 환경의 `CODEX_HOME`을 그 폴더로 바꾼다.
fn with_codex_home(mut launch: LaunchSpec) -> LaunchSpec {
    let Some(home) = launch.permission.codex_home.clone() else {
        return launch;
    };
    launch.env.retain(|(name, _)| name != "CODEX_HOME");
    launch
        .env
        .push((OsString::from("CODEX_HOME"), home.into_os_string()));
    launch
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
        "item/started" => {
            let item = &params["item"];
            if let (Some("fileChange"), Some(id)) = (item["type"].as_str(), item["id"].as_str()) {
                state
                    .file_changes
                    .insert(id.to_owned(), file_change_paths(item));
            }
            activity_of(item)
                .zip(item["id"].as_str())
                .map(|(activity, id)| ProviderEvent::ToolCall {
                    agent,
                    subagent,
                    call_id: id.to_owned(),
                    activity,
                    detail: detail_of(item),
                })
                .into_iter()
                .collect()
        }
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

/// 승인 요청은 `PermissionRequested`로, 입력 요청은 `InputRequested`로 올리고 나머지는 버린다. 올린 요청은
/// 답할 수 있게 `approvals`에 둔다.
fn convert_server_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    approvals: &mut HashMap<String, PendingApproval>,
    method: &str,
    id: &Value,
    params: &Value,
) -> Vec<ProviderEvent> {
    if let Some(kind) = input_kind(method, params) {
        return convert_input_request(threads, approvals, kind, id, params)
            .into_iter()
            .collect();
    }
    let Some((_, summary)) = APPROVAL_METHODS.iter().find(|(name, _)| *name == method) else {
        return Vec::new();
    };
    let Some(state) = thread_of_request(threads, params) else {
        return Vec::new();
    };
    let summary = params["command"]
        .as_str()
        .map(|command| format!("{summary}: {command}"))
        .or_else(|| {
            (method == ELICITATION_METHOD)
                .then(|| params["message"].as_str().map(str::to_owned))
                .flatten()
        })
        .unwrap_or_else(|| (*summary).to_owned());
    let request_id = value_text(id).unwrap_or_default();
    approvals.insert(
        request_id.clone(),
        PendingApproval {
            id: id.clone(),
            method: method.to_owned(),
            available: params["availableDecisions"].as_array().map(|decisions| {
                decisions
                    .iter()
                    .filter_map(|decision| decision.as_str().map(str::to_owned))
                    .collect()
            }),
            permissions: params["permissions"].clone(),
            input: None,
        },
    );
    vec![ProviderEvent::PermissionRequested {
        agent: state.agent,
        request_id,
        summary,
        reason: params["reason"].as_str().unwrap_or_default().to_owned(),
        call: call_of(method, params, &state.file_changes),
    }]
}

/// 승인이 아닌 elicitation과 에이전트 질문.
fn input_kind(method: &str, params: &Value) -> Option<InputKind> {
    match method {
        USER_INPUT_METHOD => Some(InputKind::UserInput),
        ELICITATION_METHOD if !params["_meta"]["codex_approval_kind"].is_string() => {
            Some(InputKind::Elicitation)
        }
        _ => None,
    }
}

fn thread_of_request<'a>(
    threads: &'a HashMap<ProviderSessionId, ThreadState>,
    params: &Value,
) -> Option<&'a ThreadState> {
    let thread = params["threadId"]
        .as_str()
        .or_else(|| params["conversationId"].as_str())
        .map(|id| ProviderSessionId(id.to_owned()))?;
    threads.get(&thread)
}

/// 읽을 수 없는 요청(모르는 `mode`, 모르는 thread)은 올리지 않는다.
fn convert_input_request(
    threads: &HashMap<ProviderSessionId, ThreadState>,
    approvals: &mut HashMap<String, PendingApproval>,
    kind: InputKind,
    id: &Value,
    params: &Value,
) -> Option<ProviderEvent> {
    let state = thread_of_request(threads, params)?;
    let request = match kind {
        InputKind::Elicitation => codex_input::elicitation_request(params)?,
        InputKind::UserInput => codex_input::user_input_request(params),
    };
    let request_id = value_text(id).unwrap_or_default();
    approvals.insert(
        request_id.clone(),
        PendingApproval {
            id: id.clone(),
            method: request_method(kind).to_owned(),
            available: None,
            permissions: Value::Null,
            input: Some(PendingInput {
                kind,
                request: request.clone(),
            }),
        },
    );
    Some(ProviderEvent::InputRequested {
        agent: state.agent,
        request_id,
        request,
    })
}

fn request_method(kind: InputKind) -> &'static str {
    match kind {
        InputKind::Elicitation => ELICITATION_METHOD,
        InputKind::UserInput => USER_INPUT_METHOD,
    }
}

/// 답을 요청 종류별 결정 값으로 바꾼다. `AllowAlways`는 Codex 세션 동안 허용(`acceptForSession`,
/// `approved_for_session`, 권한 `scope: session`)으로 보내고, 요청이 그 결정을 허용 목록에서 뺐거나 보낼 값이
/// 없는(MCP elicitation) 요청이면 `AllowOnce`와 같게 보낸다.
/// `decline`은 `availableDecisions`에 없어도 받아들여지는 것을 실측했다.
/// engine은 규칙으로 읽은 호출의 `항상 허용`을 직접 저장하고 `AllowOnce`로 보내므로, 이 값은 읽을 수 없는
/// 요청(권한 요청 등)에만 쓰인다.
/// TODO(#56): 거부와 함께 남기는 말은 정해지기 전에는 보내지 않는다
fn approval_result(pending: &PendingApproval, answer: &PermissionAnswer) -> Value {
    let denied = matches!(answer, PermissionAnswer::Deny { .. });
    let always = matches!(answer, PermissionAnswer::AllowAlways);
    match pending.method.as_str() {
        ELICITATION_METHOD => {
            if always {
                tracing::debug!("codex MCP approval has no always value, sent as allow once");
            }
            if denied {
                json!({ "action": "decline" })
            } else {
                json!({ "action": "accept", "content": {} })
            }
        }
        PERMISSIONS_METHOD => {
            let scope = if always { "session" } else { "turn" };
            let permissions = if denied {
                json!({})
            } else {
                pending.permissions.clone()
            };
            json!({ "permissions": permissions, "scope": scope })
        }
        "execCommandApproval" | "applyPatchApproval" => {
            let decision = match (denied, always) {
                (true, _) => "denied",
                (false, true) => "approved_for_session",
                (false, false) => "approved",
            };
            json!({ "decision": decision })
        }
        _ => {
            let for_session_listed = pending.available.as_ref().is_none_or(|available| {
                available
                    .iter()
                    .any(|decision| decision == "acceptForSession")
            });
            let decision = match (denied, always && for_session_listed) {
                (true, _) => "decline",
                (false, true) => "acceptForSession",
                (false, false) => "accept",
            };
            json!({ "decision": decision })
        }
    }
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
                command: unwrap_shell(item["command"].as_str().unwrap_or_default()),
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
            let command = unwrap_shell(item["command"].as_str().unwrap_or_default());
            let category = if matches!(activity_of(item), Some(Activity::ReadingFile)) {
                ToolCategory::FileRead
            } else {
                classify_command(&command)
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
        Some("reasoning") => ToolDetail {
            category: ToolCategory::Reasoning,
            ..ToolDetail::default()
        },
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
    approvals: Approvals,
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
        for event in route_message(&message, &pending, &threads, &approvals) {
            let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
        }
    }
    lock(&pending).clear();
    lock(&approvals).clear();
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

fn route_message(
    message: &Value,
    pending: &Pending,
    threads: &Threads,
    approvals: &Approvals,
) -> Vec<ProviderEvent> {
    match (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) {
        (Some(method), Some(id)) => convert_server_request(
            &lock(threads),
            &mut lock(approvals),
            method,
            id,
            &message["params"],
        ),
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
pub(super) fn value_text(value: &Value) -> Option<String> {
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

    use saturn_core::permission::{Mode, Policy, Rule, Verdict};
    use saturn_protocol::event::{PermissionCall, PermissionTool};
    use saturn_protocol::ids::{Provider, SettingsRevision};

    use super::*;
    use crate::providers::{HomeInput, PermissionLaunch, SaturnDefaults, prepare_codex_home};
    use saturn_protocol::input::InputValue;

    /// 받은 요청에 schema 모양 그대로 응답한다.
    const FAKE_APP_SERVER: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP;
$| = 1;
my $json = JSON::PP->new->canonical;
sub out { print $json->encode($_[0]), "\n"; }
sub note { out({ method => $_[0], params => $_[1] }); }
my $active = "";
my $mcp_polls = 0;
sub rules_text {
  my $home = $ENV{CODEX_HOME} // "";
  open(my $file, "<", "$home/rules/default.rules") or return "";
  local $/;
  my $text = <$file>;
  return $text // "";
}
my %gates = (
  "gate-file-known" => ["srv-file2", "item/fileChange/requestApproval", { itemId => "item_f", reason => "write file" }],
  "run-sort" => [21, "item/commandExecution/requestApproval", { itemId => "item_s", command => "sort a.txt", reason => "needs approval" }],
  "gate-command" => [7, "item/commandExecution/requestApproval", { itemId => "item_g", command => "touch a.txt", reason => "needs write", availableDecisions => ["accept", "acceptForSession", "decline"] }],
  "gate-command-once-only" => [8, "item/commandExecution/requestApproval", { itemId => "item_g", command => "ls", availableDecisions => ["accept", "cancel"] }],
  "gate-file" => ["srv-file", "item/fileChange/requestApproval", { itemId => "item_g", reason => "write file" }],
  "gate-mcp" => [0, "mcpServer/elicitation/request", { serverName => "probe", mode => "form", message => "Allow the probe MCP server to run tool \"echo\"?", _meta => { codex_approval_kind => "mcp_tool_call" }, requestedSchema => { type => "object", properties => {} } }],
  "gate-permissions" => [9, "item/permissions/requestApproval", { itemId => "item_g", reason => "needs network", permissions => { network => { enabled => JSON::PP::true } } }],
  "gate-legacy" => ["legacy-1", "execCommandApproval", { conversationId => "thr_main", command => ["ls"], reason => "legacy" }],
  "gate-elicit-form" => [41, "mcpServer/elicitation/request", { serverName => "probe", mode => "form", message => "실험 입력을 작성하라", requestedSchema => { type => "object", properties => { choice => { type => "string", title => "단일 선택", enum => ["a", "b"] }, count => { type => "integer", title => "횟수" }, enabled => { type => "boolean", title => "사용" }, tags => { type => "array", title => "다중 선택", items => { type => "string", enum => ["x", "y"] } }, title => { type => "string", title => "제목" } }, required => ["title", "count", "enabled", "choice", "tags"] } }],
  "gate-elicit-url" => [42, "mcpServer/elicitation/request", { serverName => "probe", mode => "url", message => "URL을 열어라", url => "https://example.invalid/experiment-252", elicitationId => "experiment-252-url" }],
  "gate-user-input" => [43, "item/tool/requestUserInput", { itemId => "call_1", isBlocking => JSON::PP::false, autoResolutionMs => undef, questions => [ { id => "preference", header => "선호 확인", question => "무엇을 먼저 할까요?", isOther => JSON::PP::true, isSecret => JSON::PP::false, options => [ { label => "코드 변경", description => "구현을 고칩니다" }, { label => "설계 검토", description => "문서를 검토합니다" } ] } ] }],
);
while (my $line = <STDIN>) {
  my $m = eval { $json->decode($line) } or next;
  my $method = $m->{method} // "";
  my $id = $m->{id};
  next unless defined $id;
  if ($method eq "" && exists $m->{result}) {
    note("item/agentMessage/delta", { threadId => "thr_main", turnId => "turn_g", itemId => "mg", delta => "answer:" . $json->encode({ id => $m->{id}, result => $m->{result} }) });
    note("turn/completed", { threadId => "thr_main", turn => { id => "turn_g", status => "completed", items => [] } });
    next;
  }
  my $p = $m->{params} // {};
  my $tid = $p->{threadId} // "thr_main";
  if ($method eq "initialize") {
    out({ id => $id, result => { userAgent => ($ENV{FAKE_USER_AGENT} // "fake/0.158.0"), platformFamily => "unix", platformOs => "macos", codexHome => ($ENV{CODEX_HOME} // "/fake") } });
  } elsif ($method eq "skills/list") {
    out({ id => $id, result => { data => [ { cwd => "/w", errors => [], skills => [
      { name => "lint", description => "Run the linter", shortDescription => "lint it", enabled => JSON::PP::true, path => "/skills/lint/SKILL.md", scope => "user" },
      { name => "off", description => "disabled", enabled => JSON::PP::false, path => "/skills/off/SKILL.md", scope => "user" } ] } ] } });
  } elsif ($method eq "thread/start" || $method eq "thread/resume") {
    my $model = $p->{model} // "";
    if (($ENV{FAKE_REQUIRE_ADD_DIR} // "") ne "") {
      my $roots = $p->{config}{sandbox_workspace_write}{writable_roots} // [];
      if (($roots->[0] // "") ne $ENV{FAKE_REQUIRE_ADD_DIR}) {
        out({ id => $id, error => { code => -32000, message => "added folders are missing" } });
        next;
      }
    }
    if (($ENV{FAKE_REQUIRE_MCP} // "") ne "" && $mcp_polls < 3) {
      out({ id => $id, error => { code => -32000, message => "mcp tools are not ready" } });
      next;
    }
    my $policy = $model eq "wrong-policy" ? "on-request" : ($p->{approvalPolicy} // "on-request");
    my $asked = $p->{sandbox} // "";
    my $sandbox = $model eq "wrong-sandbox" ? { type => "dangerFullAccess" } : ($asked eq "read-only" ? { type => "readOnly" } : { type => "workspaceWrite" });
    out({ id => $id, result => { thread => { id => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" }, model => "gpt-test", modelProvider => "openai", approvalPolicy => $policy, approvalsReviewer => "user", sandbox => $sandbox, cwd => "/w" } });
  } elsif ($method eq "mcpServerStatus/list") {
    $mcp_polls++;
    my $ready = $mcp_polls >= 3;
    out({ id => $id, result => { data => [ { name => "docs", runtimeStatus => ($ready ? "ready" : "starting"), tools => ($ready ? { echo => {} } : undef), toolsError => undef } ], nextCursor => undef } });
  } elsif ($method eq "turn/start") {
    my $first = $p->{input}[0];
    if (($first->{text} // "") eq "crash") {
      out({ id => $id, result => { turn => { id => "turn_x", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_x", status => "inProgress", items => [] } });
      exit 1;
    }
    my $gate = $gates{$first->{text} // ""};
    if ($gate && ($first->{text} eq "run-sort") && rules_text() !~ /pattern = \["sort"\], decision = "prompt"/) {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_g", itemId => "mg", delta => "ran-without-asking" });
      note("turn/completed", { threadId => $tid, turn => { id => "turn_g", status => "completed", items => [] } });
      next;
    }
    if (($first->{text} // "") eq "gate-child") {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      note("thread/started", { thread => { id => "thr_child", parentThreadId => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" } });
      out({ id => 31, method => "item/commandExecution/requestApproval", params => { threadId => "thr_child", turnId => "turn_c", itemId => "item_c", command => "rm -rf build", reason => "child needs write" } });
      next;
    }
    if ($gate) {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      if ($first->{text} eq "gate-file-known") {
        note("item/started", { threadId => $tid, turnId => "turn_g", startedAtMs => 1, item => { type => "fileChange", id => "item_f", status => "inProgress", changes => [ { path => "src/a.rs", kind => { type => "update" }, diff => "" } ] } });
      }
      out({ id => $gate->[0], method => $gate->[1], params => { threadId => $tid, turnId => "turn_g", %{ $gate->[2] } } });
      next;
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
                auto_compact_tokens: 180_000,
            },
            env,
            hook_settings: None,
            permission: PermissionLaunch::default(),
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
            add_dirs: Vec::new(),
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
                permission: Some("untrusted".to_owned()),
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
                    call: Some(PermissionCall {
                        tool: PermissionTool::Shell,
                        target: "rm -rf build".to_owned(),
                        paths: Vec::new(),
                    }),
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

    /// `text` 턴이 승인 요청에서 멈추게 한 뒤, 답이 없는 동안 턴이 멈춰 있는지 확인하고 답한다.
    /// 올라온 요청과 가짜 app-server가 받은 응답(`id`, `result`)을 돌려준다.
    async fn answer_gate(text: &str, answer: PermissionAnswer) -> (ProviderEvent, Value) {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, text).await.unwrap();
        let requested = take(&mut client, 1).await.remove(0);
        let ProviderEvent::PermissionRequested { request_id, .. } = &requested else {
            panic!("expected a permission request, got {requested:?}");
        };
        let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
        assert!(stalled.is_err(), "turn should wait for the answer");

        client
            .answer_permission(&main, request_id, answer)
            .await
            .unwrap();
        let resumed = take(&mut client, 3).await;

        let ProviderEvent::Text { text, .. } = &resumed[0] else {
            panic!(
                "expected the turn to continue with text, got {:?}",
                resumed[0]
            );
        };
        assert!(matches!(resumed[2], ProviderEvent::TurnCompleted { .. }));
        let received = text.strip_prefix("answer:").expect("answer prefix");
        (requested, serde_json::from_str(received).unwrap())
    }

    fn deny() -> PermissionAnswer {
        PermissionAnswer::Deny { note: None }
    }

    #[tokio::test]
    async fn command_approval_is_answered_with_the_same_numeric_request_id() {
        let (requested, received) = answer_gate("gate-command", PermissionAnswer::AllowOnce).await;

        assert_eq!(
            requested,
            ProviderEvent::PermissionRequested {
                agent: AgentId(7),
                request_id: "7".to_owned(),
                summary: "run command: touch a.txt".to_owned(),
                reason: "needs write".to_owned(),
                call: Some(PermissionCall {
                    tool: PermissionTool::Shell,
                    target: "touch a.txt".to_owned(),
                    paths: Vec::new(),
                }),
            }
        );
        assert_eq!(
            received,
            json!({ "id": 7, "result": { "decision": "accept" } })
        );
    }

    #[tokio::test]
    async fn command_decisions_follow_the_answer_and_the_available_list() {
        let always = answer_gate("gate-command", PermissionAnswer::AllowAlways)
            .await
            .1;
        let listed_without_session =
            answer_gate("gate-command-once-only", PermissionAnswer::AllowAlways)
                .await
                .1;
        let denied = answer_gate("gate-command", deny()).await.1;
        let denied_without_decline_listed = answer_gate("gate-command-once-only", deny()).await.1;

        assert_eq!(always["result"], json!({ "decision": "acceptForSession" }));
        assert_eq!(listed_without_session["id"], 8);
        assert_eq!(
            listed_without_session["result"],
            json!({ "decision": "accept" })
        );
        assert_eq!(denied["result"], json!({ "decision": "decline" }));
        assert_eq!(
            denied_without_decline_listed["result"],
            json!({ "decision": "decline" })
        );
    }

    #[tokio::test]
    async fn file_change_approval_keeps_a_string_request_id() {
        let (requested, received) = answer_gate("gate-file", PermissionAnswer::AllowOnce).await;

        assert!(matches!(
            requested,
            ProviderEvent::PermissionRequested { ref request_id, ref summary, .. }
                if request_id == "srv-file" && summary == "change files"
        ));
        assert_eq!(
            received,
            json!({ "id": "srv-file", "result": { "decision": "accept" } })
        );
    }

    #[tokio::test]
    async fn mcp_tool_approval_is_answered_with_an_elicitation_action() {
        let (requested, allowed) = answer_gate("gate-mcp", PermissionAnswer::AllowOnce).await;
        let always = answer_gate("gate-mcp", PermissionAnswer::AllowAlways)
            .await
            .1;
        let denied = answer_gate("gate-mcp", deny()).await.1;

        assert!(matches!(
            requested,
            ProviderEvent::PermissionRequested { ref request_id, ref summary, .. }
                if request_id == "0" && summary.contains("run tool")
        ));
        assert_eq!(
            allowed,
            json!({ "id": 0, "result": { "action": "accept", "content": {} } })
        );
        assert_eq!(always, allowed);
        assert_eq!(denied["result"], json!({ "action": "decline" }));
    }

    #[tokio::test]
    async fn permission_grant_approval_returns_the_requested_permissions() {
        let once = answer_gate("gate-permissions", PermissionAnswer::AllowOnce)
            .await
            .1;
        let always = answer_gate("gate-permissions", PermissionAnswer::AllowAlways)
            .await
            .1;
        let denied = answer_gate("gate-permissions", deny()).await.1;

        let network = json!({ "network": { "enabled": true } });
        assert_eq!(
            once["result"],
            json!({ "permissions": network, "scope": "turn" })
        );
        assert_eq!(
            always["result"],
            json!({ "permissions": network, "scope": "session" })
        );
        assert_eq!(
            denied["result"],
            json!({ "permissions": {}, "scope": "turn" })
        );
    }

    #[tokio::test]
    async fn legacy_approvals_use_review_decisions() {
        let once = answer_gate("gate-legacy", PermissionAnswer::AllowOnce)
            .await
            .1;
        let always = answer_gate("gate-legacy", PermissionAnswer::AllowAlways)
            .await
            .1;
        let denied = answer_gate("gate-legacy", deny()).await.1;

        assert_eq!(once["id"], "legacy-1");
        assert_eq!(once["result"], json!({ "decision": "approved" }));
        assert_eq!(
            always["result"],
            json!({ "decision": "approved_for_session" })
        );
        assert_eq!(denied["result"], json!({ "decision": "denied" }));
    }

    /// `text` 턴이 입력 요청에서 멈추게 한 뒤, 답이 없는 동안 턴이 멈춰 있는지 확인하고 답한다.
    /// 올라온 요청과 가짜 app-server가 받은 응답(`id`, `result`)을 돌려준다.
    async fn answer_input_gate(text: &str, answer: InputAnswer) -> (ProviderEvent, Value) {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, text).await.unwrap();
        let requested = take(&mut client, 1).await.remove(0);
        let ProviderEvent::InputRequested { request_id, .. } = &requested else {
            panic!("expected an input request, got {requested:?}");
        };
        let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
        assert!(stalled.is_err(), "turn should wait for the answer");

        client
            .answer_input(&main, request_id, answer)
            .await
            .unwrap();
        let resumed = take(&mut client, 3).await;

        let ProviderEvent::Text { text, .. } = &resumed[0] else {
            panic!(
                "expected the turn to continue with text, got {:?}",
                resumed[0]
            );
        };
        let received = text.strip_prefix("answer:").expect("answer prefix");
        (requested, serde_json::from_str(received).unwrap())
    }

    #[tokio::test]
    async fn elicitation_form_round_trip_answers_accept_with_content() {
        let submit = InputAnswer::Submit {
            values: vec![
                ("title".to_owned(), InputValue::Text("Saturn".to_owned())),
                ("count".to_owned(), InputValue::Integer(3)),
                ("enabled".to_owned(), InputValue::Boolean(true)),
                (
                    "choice".to_owned(),
                    InputValue::Selected(vec!["a".to_owned()]),
                ),
                (
                    "tags".to_owned(),
                    InputValue::Selected(vec!["x".to_owned(), "y".to_owned()]),
                ),
            ],
        };

        let (requested, received) = answer_input_gate("gate-elicit-form", submit).await;

        let ProviderEvent::InputRequested { request, .. } = requested else {
            unreachable!("answer_input_gate returns an input request");
        };
        assert_eq!(request.message, "실험 입력을 작성하라");
        assert_eq!(request.fields.len(), 5);
        assert_eq!(request.fields[0].id, "title");
        assert_eq!(received["id"], json!(41));
        assert_eq!(
            received["result"],
            json!({ "action": "accept", "content": {
                "title": "Saturn", "count": 3, "enabled": true, "choice": "a", "tags": ["x", "y"]
            } })
        );
    }

    #[tokio::test]
    async fn elicitation_form_round_trip_answers_decline_and_cancel() {
        let declined = answer_input_gate("gate-elicit-form", InputAnswer::Decline)
            .await
            .1;
        let cancelled = answer_input_gate("gate-elicit-form", InputAnswer::Cancel)
            .await
            .1;

        assert_eq!(declined["result"], json!({ "action": "decline" }));
        assert_eq!(cancelled["result"], json!({ "action": "cancel" }));
        assert_eq!(declined["id"], json!(41));
    }

    #[tokio::test]
    async fn elicitation_url_round_trip_answers_accept_decline_and_cancel() {
        let accepted = answer_input_gate(
            "gate-elicit-url",
            InputAnswer::Submit { values: Vec::new() },
        )
        .await;
        let declined = answer_input_gate("gate-elicit-url", InputAnswer::Decline)
            .await
            .1;
        let cancelled = answer_input_gate("gate-elicit-url", InputAnswer::Cancel)
            .await
            .1;

        let ProviderEvent::InputRequested { request, .. } = &accepted.0 else {
            unreachable!("answer_input_gate returns an input request");
        };
        assert_eq!(
            request.url.as_deref(),
            Some("https://example.invalid/experiment-252")
        );
        assert!(request.fields.is_empty());
        assert_eq!(accepted.1["result"], json!({ "action": "accept" }));
        assert_eq!(declined["result"], json!({ "action": "decline" }));
        assert_eq!(cancelled["result"], json!({ "action": "cancel" }));
    }

    #[tokio::test]
    async fn elicitation_user_input_round_trip_answers_by_question_id() {
        let submit = InputAnswer::Submit {
            values: vec![(
                "preference".to_owned(),
                InputValue::Selected(vec!["설계 검토".to_owned()]),
            )],
        };

        let (requested, received) = answer_input_gate("gate-user-input", submit).await;
        let cancelled = answer_input_gate("gate-user-input", InputAnswer::Cancel)
            .await
            .1;

        let ProviderEvent::InputRequested { request, .. } = requested else {
            unreachable!("answer_input_gate returns an input request");
        };
        assert_eq!(request.fields[0].id, "preference");
        assert_eq!(received["id"], json!(43));
        assert_eq!(
            received["result"],
            json!({ "answers": { "preference": { "answers": ["설계 검토"] } } })
        );
        assert_eq!(cancelled["result"], json!({ "answers": {} }));
    }

    #[tokio::test]
    async fn elicitation_input_and_permission_answers_do_not_cross() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, "gate-elicit-url").await.unwrap();
        take(&mut client, 1).await;

        let as_permission = client
            .answer_permission(&main, "42", PermissionAnswer::AllowOnce)
            .await;
        let unknown = client.answer_input(&main, "99", InputAnswer::Cancel).await;
        let as_input = client.answer_input(&main, "42", InputAnswer::Cancel).await;
        let again = client.answer_input(&main, "42", InputAnswer::Cancel).await;

        assert!(matches!(as_permission, Err(ProviderError::NotSent { .. })));
        assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
        assert!(as_input.is_ok());
        assert!(matches!(again, Err(ProviderError::NotSent { .. })));
    }

    #[tokio::test]
    async fn elicitation_approval_is_still_a_permission_request() {
        let (requested, _) = answer_gate("gate-mcp", PermissionAnswer::AllowOnce).await;

        assert!(matches!(
            requested,
            ProviderEvent::PermissionRequested { .. }
        ));
    }

    #[tokio::test]
    async fn answering_an_unknown_or_answered_request_is_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, "gate-command").await.unwrap();
        take(&mut client, 1).await;

        let unknown = client
            .answer_permission(&main, "99", PermissionAnswer::AllowOnce)
            .await;
        client
            .answer_permission(&main, "7", PermissionAnswer::AllowOnce)
            .await
            .unwrap();
        let again = client
            .answer_permission(&main, "7", PermissionAnswer::AllowOnce)
            .await;

        assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
        assert!(matches!(again, Err(ProviderError::NotSent { .. })));
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
        let compact_set = default_args(
            UserProviderConfig {
                has_auto_compact: true,
            },
            &launch,
        );

        assert_eq!(none, vec!["-c", "model_auto_compact_token_limit=180000"]);
        assert!(compact_set.is_empty());
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
                has_auto_compact: true,
            }
        );
        assert_eq!(
            scan_user_config("approval_policy = \"never\"\n"),
            UserProviderConfig::default()
        );
        assert_eq!(
            read_user_config(&launch(dir.path(), Vec::new())),
            UserProviderConfig::default()
        );
    }

    #[test]
    fn model_list_entries_skip_hidden_models_and_fall_back_to_the_id() {
        let visible =
            json!({ "id": "a", "model": "gpt-a", "displayName": "GPT A", "hidden": false });
        let hidden = json!({ "id": "b", "model": "gpt-b", "hidden": true });
        let unnamed = json!({ "id": "c" });

        let infos: Vec<(String, String)> = [visible, hidden, unnamed]
            .iter()
            .filter_map(model_info)
            .map(|info| (info.choice.model, info.name))
            .collect();

        assert_eq!(
            infos,
            vec![
                ("gpt-a".to_owned(), "GPT A".to_owned()),
                ("c".to_owned(), "c".to_owned())
            ]
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
    fn activity_of_wrapped_command_is_inner_command() {
        let item = json!({
            "type": "commandExecution",
            "command": "/bin/zsh -lc 'python3 scripts/check.py'",
            "commandActions": [{ "type": "unknown", "command": "python3 scripts/check.py" }],
        });

        assert_eq!(
            activity_of(&item),
            Some(Activity::RunningCommand {
                command: "python3 scripts/check.py".to_owned()
            })
        );
    }

    #[test]
    fn detail_of_reasoning_is_not_a_candidate() {
        let detail = detail_of(&json!({ "type": "reasoning", "summary": [] }));

        assert_eq!(detail.category, ToolCategory::Reasoning);
        assert!(!detail.category.is_candidate());
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
    fn app_server_values_hide_router_key_before_routing() {
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

    fn rule(tool: PermissionTool, pattern: &str, verdict: Verdict) -> Rule {
        Rule {
            tool,
            pattern: pattern.to_owned(),
            verdict,
        }
    }

    fn policy(rules: Vec<Rule>) -> Policy {
        Policy {
            mode: Mode::Edit,
            workdir: PathBuf::from("/work"),
            extra_dirs: Vec::new(),
            rules,
            always: Vec::new(),
        }
    }

    /// `text` 턴이 올린 승인 요청을 Saturn 규칙으로 판정한다. 허용과 거부는 그 답을 보내 가짜 app-server가 받은
    /// 응답(`id`, `result`)을 함께 돌려주고, 묻기는 답하지 않고 `None`을 돌려준다.
    async fn answer_by_rules(text: &str, policy: &Policy) -> (Verdict, Option<Value>) {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;
        let main = handle.provider_session;
        client.send_turn(&main, text).await.unwrap();
        let requested = loop {
            let event = take(&mut client, 1).await.remove(0);
            if matches!(event, ProviderEvent::PermissionRequested { .. }) {
                break event;
            }
        };
        let ProviderEvent::PermissionRequested {
            request_id, call, ..
        } = &requested
        else {
            unreachable!("loop should stop at a permission request");
        };
        let call = call.as_ref().expect("request should be readable");
        let verdict = policy.decide(call);
        let answer = match verdict {
            Verdict::Allow => PermissionAnswer::AllowOnce,
            Verdict::Deny => deny(),
            Verdict::Ask => return (verdict, None),
        };
        client
            .answer_permission(&main, request_id, answer)
            .await
            .unwrap();
        let resumed = take(&mut client, 3).await;
        let ProviderEvent::Text { text, .. } = &resumed[0] else {
            panic!("expected text after the answer, got {:?}", resumed[0]);
        };
        let received = text.strip_prefix("answer:").expect("answer prefix");
        (verdict, Some(serde_json::from_str(received).unwrap()))
    }

    #[tokio::test]
    async fn permission_shell() {
        let allow = policy(vec![rule(PermissionTool::Shell, "touch *", Verdict::Allow)]);
        let denied = policy(vec![rule(PermissionTool::Shell, "touch *", Verdict::Deny)]);

        let allowed = answer_by_rules("gate-command", &allow).await;
        let refused = answer_by_rules("gate-command", &denied).await;
        let asked = answer_by_rules("gate-command", &policy(Vec::new())).await;

        assert_eq!(
            allowed,
            (
                Verdict::Allow,
                Some(json!({ "id": 7, "result": { "decision": "accept" } }))
            )
        );
        assert_eq!(
            refused,
            (
                Verdict::Deny,
                Some(json!({ "id": 7, "result": { "decision": "decline" } }))
            )
        );
        assert_eq!(asked, (Verdict::Ask, None));
    }

    #[tokio::test]
    async fn permission_edit() {
        let denied = policy(vec![rule(PermissionTool::Edit, "src/*", Verdict::Deny)]);

        let inside = answer_by_rules("gate-file-known", &policy(Vec::new())).await;
        let refused = answer_by_rules("gate-file-known", &denied).await;
        let unknown_path = answer_by_rules("gate-file", &policy(Vec::new())).await;

        assert_eq!(
            inside,
            (
                Verdict::Allow,
                Some(json!({ "id": "srv-file2", "result": { "decision": "accept" } }))
            )
        );
        assert_eq!(
            refused,
            (
                Verdict::Deny,
                Some(json!({ "id": "srv-file2", "result": { "decision": "decline" } }))
            )
        );
        assert_eq!(unknown_path, (Verdict::Ask, None));
    }

    #[tokio::test]
    async fn permission_subagent() {
        let allow = policy(vec![rule(PermissionTool::Shell, "rm *", Verdict::Allow)]);
        let denied = policy(vec![rule(PermissionTool::Shell, "rm *", Verdict::Deny)]);

        let allowed = answer_by_rules("gate-child", &allow).await;
        let refused = answer_by_rules("gate-child", &denied).await;

        assert_eq!(
            allowed,
            (
                Verdict::Allow,
                Some(json!({ "id": 31, "result": { "decision": "accept" } }))
            )
        );
        assert_eq!(
            refused,
            (
                Verdict::Deny,
                Some(json!({ "id": 31, "result": { "decision": "decline" } }))
            )
        );
    }

    #[tokio::test]
    async fn permission_subagent_request_belongs_to_the_parent_agent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut client, handle) = start(dir.path()).await;

        client
            .send_turn(&handle.provider_session, "gate-child")
            .await
            .unwrap();
        let events = take(&mut client, 2).await;

        assert!(matches!(
            events[0],
            ProviderEvent::SubagentStarted {
                agent: AgentId(7),
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            ProviderEvent::PermissionRequested { agent: AgentId(7), call: Some(call), .. }
                if call.target == "rm -rf build"
        ));
    }

    #[tokio::test]
    async fn permission_mcp() {
        let allow = policy(vec![rule(
            PermissionTool::Mcp,
            "mcp__probe__echo",
            Verdict::Allow,
        )]);
        let denied = policy(vec![rule(
            PermissionTool::Mcp,
            "mcp__probe__*",
            Verdict::Deny,
        )]);

        let allowed = answer_by_rules("gate-mcp", &allow).await;
        let refused = answer_by_rules("gate-mcp", &denied).await;
        let asked = answer_by_rules("gate-mcp", &policy(Vec::new())).await;

        assert_eq!(
            allowed,
            (
                Verdict::Allow,
                Some(json!({ "id": 0, "result": { "action": "accept", "content": {} } }))
            )
        );
        assert_eq!(
            refused,
            (
                Verdict::Deny,
                Some(json!({ "id": 0, "result": { "action": "decline" } }))
            )
        );
        assert_eq!(asked, (Verdict::Ask, None));
    }

    #[tokio::test]
    async fn startup_checks_applied_policy_whatever_the_version() {
        let dir = tempfile::tempdir().unwrap();
        let newer = vec![("FAKE_USER_AGENT".into(), "fake/0.200.0".into())];

        let mut client = CodexClient::start(launch(dir.path(), newer), Supervisor::new())
            .await
            .unwrap();

        assert!(client.open_session(spec(dir.path())).await.is_ok());
        for (model, expected) in [
            ("wrong-policy", "approval policy"),
            ("wrong-sandbox", "sandbox"),
        ] {
            let mut client = CodexClient::start(launch(dir.path(), Vec::new()), Supervisor::new())
                .await
                .unwrap();
            let mut wrong = spec(dir.path());
            wrong.model = Some(model.to_owned());

            let error = client.open_session(wrong).await.unwrap_err();

            assert!(matches!(
                error,
                ProviderError::NotSent { ref reason } if reason.contains(expected)
            ));
            let thread = ProviderSessionId("thr_main".to_owned());
            assert!(client.applied_settings(&thread).is_none());
        }
        let (client, handle) = start(dir.path()).await;
        let applied = client.applied_settings(&handle.provider_session).unwrap();
        assert_eq!(applied.permission.as_deref(), Some("untrusted"));
    }

    #[tokio::test]
    async fn add_dir_goes_to_the_thread_config_when_a_session_opens() {
        let dir = tempfile::tempdir().unwrap();
        let env = || vec![("FAKE_REQUIRE_ADD_DIR".into(), "/extra".into())];
        let mut with_dir = spec(dir.path());
        with_dir.add_dirs = vec![PathBuf::from("/extra")];
        let mut resumed = with_dir.clone();
        resumed.resume = Some(ProviderSessionId("thr_main".to_owned()));
        let mut client = CodexClient::start(launch(dir.path(), env()), Supervisor::new())
            .await
            .unwrap();

        let opened = client.open_session(with_dir).await;
        let reopened = client.open_session(resumed).await;
        let without = client.open_session(spec(dir.path())).await;

        assert!(opened.is_ok());
        assert!(reopened.is_ok());
        assert!(matches!(
            without,
            Err(ProviderError::NotSent { ref reason }) if reason.contains("added folders")
        ));
    }

    #[tokio::test]
    async fn first_turn_waits_for_mcp_ready() {
        let dir = tempfile::tempdir().unwrap();
        let mut launch = launch(dir.path(), vec![("FAKE_REQUIRE_MCP".into(), "1".into())]);
        launch.permission.mcp_servers = vec!["docs".to_owned()];
        let mut client = CodexClient::start(launch, Supervisor::new()).await.unwrap();

        let handle = client.open_session(spec(dir.path())).await.unwrap();

        client
            .send_turn(&handle.provider_session, "hello")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn first_turn_is_not_sent_when_mcp_never_gets_ready() {
        let dir = tempfile::tempdir().unwrap();
        let mut launch = launch(dir.path(), vec![("FAKE_REQUIRE_MCP".into(), "1".into())]);
        launch.permission.mcp_servers = vec!["docs".to_owned()];
        let mut client = CodexClient::start(launch, Supervisor::new())
            .await
            .unwrap()
            .with_mcp_ready_timeout(Duration::from_millis(10));

        let error = client.open_session(spec(dir.path())).await.unwrap_err();

        assert!(matches!(
            error,
            ProviderError::NotSent { ref reason } if reason.contains("not ready")
        ));
    }

    #[tokio::test]
    async fn codex_home_ignores_user_rules() {
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("user-codex");
        std::fs::create_dir_all(user.join("rules")).unwrap();
        std::fs::write(
            user.join("rules/default.rules"),
            "prefix_rule(pattern = [\"sort\"], decision = \"allow\")\n",
        )
        .unwrap();
        let prepared = prepare_codex_home(HomeInput {
            saturn_home: &dir.path().join("saturn"),
            user_codex_home: &user,
            rules: &[rule(PermissionTool::Shell, "sort *", Verdict::Ask)],
        })
        .unwrap();
        let env = vec![("CODEX_HOME".into(), user.clone().into_os_string())];
        let mut dedicated = launch(dir.path(), env.clone());
        dedicated.permission.codex_home = Some(prepared.path.clone());
        let mut with_user_home = CodexClient::start(launch(dir.path(), env), Supervisor::new())
            .await
            .unwrap();
        let mut with_saturn_home = CodexClient::start(dedicated, Supervisor::new())
            .await
            .unwrap();
        let mut sessions = Vec::new();
        for client in [&mut with_user_home, &mut with_saturn_home] {
            sessions.push(client.open_session(spec(dir.path())).await.unwrap());
        }

        with_user_home
            .send_turn(&sessions[0].provider_session, "run-sort")
            .await
            .unwrap();
        with_saturn_home
            .send_turn(&sessions[1].provider_session, "run-sort")
            .await
            .unwrap();
        let user_rules_ran = take(&mut with_user_home, 1).await.remove(0);
        let saturn_rules_asked = take(&mut with_saturn_home, 1).await.remove(0);

        assert!(matches!(
            user_rules_ran,
            ProviderEvent::Text { ref text, .. } if text == "ran-without-asking"
        ));
        assert!(matches!(
            saturn_rules_asked,
            ProviderEvent::PermissionRequested { .. }
        ));
    }

    #[test]
    fn file_change_paths_are_remembered_from_the_item_start() {
        let mut threads = HashMap::new();
        let main = ProviderSessionId("main".to_owned());
        threads.insert(
            main,
            ThreadState::new(AgentId(1), None, AppliedSettings::default()),
        );
        let started = json!({
            "threadId": "main",
            "item": { "type": "fileChange", "id": "item_f", "changes": [{ "path": "a.rs", "diff": "" }] },
        });

        convert_notification(&mut threads, "item/started", &started);

        let state = threads.values().next().unwrap();
        assert_eq!(state.file_changes["item_f"], vec!["a.rs"]);
    }
}
