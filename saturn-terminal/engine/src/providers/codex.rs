//! Codex 연결: `codex app-server` 프로세스 하나로 thread(= provider session) 여러 개를 다룬다.
//! 설계: docs/design/providers-and-sessions.md
//! TODO(#61): 자식 thread의 승인 요청 처리 미정. 정해지기 전에는 부모와 같이 `PermissionRequested`로 올린다

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::{ProviderEvent, TurnOrigin};
use saturn_protocol::ids::{AgentId, ProviderSessionId};
use saturn_protocol::input::{InputAnswer, InputRequest};
use saturn_protocol::rpc::{ModelInfo, PermissionAnswer};
use serde_json::{Value, json};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use super::codex_input::{self, InputKind};
use super::codex_permission::{APPROVAL_POLICY, SANDBOX, check_applied};
use super::{AppliedSettings, TurnOriginTracker};
use crate::processes::{ProcessGroupId, Supervisor};
use crate::secrets::Masker;

mod config;
mod connection;
mod convert;
mod stream;
mod threads;

use convert::{approval_result, model_info};
use threads::remove_thread_tree;

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
mod tests;
