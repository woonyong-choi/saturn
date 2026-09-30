//! Codex 연결. `codex app-server` 프로세스 하나를 창구로 thread(= provider session) 여러 개를 다룬다.
//!
//! 설계: docs/design/providers-and-sessions.md(provider 연결, 이벤트 수신과 변환, subagent 트리 추적, 트리 전체 중지).
//!
//! 규약: stdin/stdout 한 줄에 JSON-RPC 메시지 하나. 요청에는 `id`, 알림에는 `id`가 없다.
//! 읽기 작업 하나가 stdout을 읽어 응답은 `pending`의 대기자에게, 알림은 변환해 이벤트 채널로 보낸다.
//!
//! | Saturn 동작 | app-server 메서드 |
//! |---|---|
//! | 초기화 | `initialize` 요청 뒤 `initialized` 알림 |
//! | session 열기 / 재개 | `thread/start` / `thread/resume`(`threadId`) |
//! | 새 턴 | `turn/start` |
//! | 끼워 넣기 | `turn/steer` |
//! | 멈춤 신호 | `turn/interrupt`(`threadId`, `turnId`). 자식 thread마다 따로 |
//! | compaction | `thread/compact/start` |
//! | session 닫기 | app-server에 thread 정리 요청. 프로세스는 남긴다 |
//! | 스킬 목록 | `skills/list` |
//!
//! 이벤트 변환(`thread_id`별로 나눠 Saturn session과 subagent를 찾는다):
//! - 메인 thread의 `turn/started` → `TurnOriginTracker`로 시작 주체만 기록. 메인 thread의 `turn/completed` → `TurnCompleted`.
//! - 자식 thread(부모 thread가 있는 thread)의 첫 신호 → `SubagentStarted { subagent: SubagentId(자식 thread_id), parent }`.
//!   parent는 부모가 메인 thread면 `None`, 자식 thread면 그 `SubagentId`. 자식의 `turn/completed` → `SubagentEnded`(작업 끝 아님).
//! - 글 조각 → `Text`, 명령·파일 항목 시작 → `ToolCall`(`Activity` 대응), 항목 끝 → `ToolResult`. 자식 thread면 `subagent`를 채운다.
//! - 승인 요청 → `PermissionRequested`. TODO(#61): 자식 thread의 승인 요청 처리 미정. 정해지기 전에는 부모와 같이 `PermissionRequested`로 올린다
//! - `tokenUsage` 갱신 → `Usage { scope: UsageScope::ThreadCumulative }`. 보고하지 않은 값은 `None`.
//! - 턴 끝 맥락 크기 → `ContextSize`.
//! - 메인 턴 진행 중이거나 자식 thread가 끝나지 않았는데 stdout이 닫힘, 읽기 오류, 프로세스 종료 → `StreamLost`.
//!
//! TODO(#86): 자식 thread를 부모에 잇는 필드, thread 정리 메서드, 활성 턴 없음 오류 코드, 맥락 크기 필드 이름은 app-server 규약에서 확인해 이 표에 적는다

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use tokio::process::ChildStdin;
use tokio::sync::{mpsc, oneshot};

use super::{AppliedSettings, LaunchSpec, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, Supervisor};

/// 끼워 넣기(`turn/steer`) 실측(#5, #27) 통과 여부. 거짓이면 `SessionHandle::steer_verified`가 거짓이 되어 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

/// Saturn 명령 이름과 app-server 메서드 대응표. 여기 있는 명령과 `skills/list` 결과만 명령 목록에 넣는다.
/// 화면 전용 명령과 Saturn session 명령이 대신하는 명령(새 대화, 재개, 종료)은 표에 넣지 않는다.
pub(crate) const COMMAND_METHODS: &[(&str, &str)] = &[
    ("compact", "thread/compact/start"),
    ("review", "review/start"),
];

/// 명령 목록에서 뺄 이름. 대응표에 없는 이름이 스킬 목록에 섞여 올 때를 대비한다.
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["new", "resume", "fork", "quit", "exit"];

/// 권한 기본값 인자(수정 허용). 사용자 설정에 권한 값이 없을 때만 넣는다.
const PERMISSION_ARGS: &[&str] = &["-c", "sandbox_mode=\"workspace-write\""];

/// 자동 압축 안전망 설정 키. `-c model_auto_compact_token_limit=<T_hard>`로 넘긴다.
const AUTO_COMPACT_KEY: &str = "model_auto_compact_token_limit";

/// 요청 응답을 넘기는 쪽. `Ok`는 `result`, `Err`는 JSON-RPC `error` 객체.
type RpcReply = oneshot::Sender<Result<serde_json::Value, serde_json::Value>>;

/// thread 하나의 상태.
#[derive(Debug)]
struct ThreadState {
    /// 이 thread를 쓰는 Saturn 에이전트. 자식 thread는 부모의 에이전트.
    agent: AgentId,
    /// 부모 thread. 메인 thread면 `None`.
    parent: Option<ProviderSessionId>,
    /// 진행 중인 턴 id. `turn/steer`, `turn/interrupt`에 쓴다. 없으면 활성 턴 없음.
    active_turn: Option<String>,
    /// 턴 시작 주체 판정.
    origin: TurnOriginTracker,
    /// `thread/start`·`thread/resume` 응답과 설정 변경 알림에서 읽은 적용값.
    applied: AppliedSettings,
}

/// Codex app-server 연결. app-server 프로세스 하나, thread 여러 개.
#[derive(Debug)]
pub struct CodexClient {
    /// 프로세스 감시자.
    supervisor: Supervisor,
    /// app-server 묶음.
    group: ProcessGroupId,
    /// 요청을 쓰는 쪽.
    stdin: ChildStdin,
    /// 다음 요청 id.
    next_request_id: u64,
    /// 응답을 기다리는 요청. 읽기 작업이 `id`로 찾아 결과를 넘긴다.
    pending: Arc<Mutex<HashMap<u64, RpcReply>>>,
    /// thread별 상태. 키는 `thread_id`. 자식 thread는 읽기 작업이 첫 신호 때 등록한다.
    threads: Arc<Mutex<HashMap<ProviderSessionId, ThreadState>>>,
    /// 변환한 이벤트. 읽기 작업이 보낸다.
    events: mpsc::Receiver<ProviderEvent>,
    /// `initialize` 뒤 대응표와 `skills/list`로 모은 명령 목록(거르기 전).
    commands: Vec<ProviderCommand>,
}

impl CodexClient {
    /// app-server를 띄우고 초기화한다.
    ///
    /// 인자: `app-server` + `default_args`. `Supervisor::spawn`에 `launch.env`를 넘긴다. `launch.hook_settings`는 쓰지 않는다.
    /// 초기화 뒤 `skills/list`로 스킬을 모으고 읽기 작업을 띄운다.
    ///
    /// # Errors
    /// 실행 실패나 `initialize` 응답 없이 stdout이 닫히면 `ConnectionLost`.
    pub async fn start(launch: LaunchSpec, supervisor: Supervisor) -> Result<Self, ProviderError> {
        todo!("#86")
    }

    /// app-server 묶음. 모든 thread가 같이 쓴다. 묶음 중지는 이 연결의 다른 thread도 멈춘다.
    pub fn process_group(&self) -> ProcessGroupId {
        todo!("#86")
    }

    /// thread에 적용된 설정. 모르는 thread면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        todo!("#86")
    }

    /// 요청 하나를 보내고 응답을 기다린다.
    /// 쓰기 전에 실패하면 `NotSent`, 쓴 뒤 응답 없이 연결이 끊기면 `Unknown`, JSON-RPC 오류 응답은 `Err(오류 객체)`로 돌려준다.
    async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Result<serde_json::Value, serde_json::Value>, ProviderError> {
        todo!("#86")
    }

    /// `InterruptTarget`을 멈출 thread로 바꾼다. `Main`은 메인 thread, `Subagent(id)`는 `id.0`인 자식 thread.
    fn interrupt_thread(
        &self,
        session: &ProviderSessionId,
        target: &InterruptTarget,
    ) -> Option<ProviderSessionId> {
        todo!("#86")
    }
}

impl ProviderClient for CodexClient {
    /// `resume`이 있으면 `thread/resume`, 없으면 `thread/start`(`cwd`, `model`). 응답의 `thread_id`를 돌려준다.
    /// `packet`이 있으면 이어서 첫 턴으로 `turn/start`한다. `steer_verified`는 `STEER_VERIFIED`.
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        todo!("#86")
    }

    /// `turn/start`. 보내기 전 `TurnOriginTracker::on_user_send`.
    /// `text`가 `/이름`으로 시작하고 이름이 `COMMAND_METHODS`에 있으면 `turn/start` 대신 그 메서드를 부른다.
    /// 스킬 이름이면 `turn/start` 입력에 스킬 항목으로 넣는다. TODO(#86): 스킬 입력 항목 모양은 app-server 규약에서 확인
    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        todo!("#86")
    }

    /// `turn/steer`(`active_turn`). 활성 턴이 없다고 알고 있거나 app-server가 활성 턴 없음으로 거절하면 `NoActiveTurn`.
    /// 이 실패는 확정 미전달이므로 호출자가 다시 판단하지 않고 `send_turn`으로 보낸다.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        todo!("#86")
    }

    /// 대상 thread의 활성 턴에 `turn/interrupt`. 대상이 활성 턴이 없으면 보내지 않고 `Ok`.
    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        todo!("#86")
    }

    /// `thread/compact/start`. 거절 응답은 `NotSent`.
    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        todo!("#86")
    }

    /// app-server에 thread 정리를 요청하고 `threads`에서 뺀다. 자식 thread도 함께 뺀다. app-server는 남긴다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        todo!("#86")
    }

    /// 이벤트 채널에서 받는다. 읽기 작업이 끝나 채널이 닫히면 `None`.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        todo!("#86")
    }

    /// `commands`를 `EXCLUDED_COMMANDS`로 거른 목록. 스킬은 `is_skill = true`.
    fn commands(&self) -> Vec<ProviderCommand> {
        todo!("#86")
    }
}

/// Saturn 기본값 인자. `user.has_permission`이 거짓이면 `PERMISSION_ARGS`,
/// `user.has_auto_compact`가 거짓이면 `-c model_auto_compact_token_limit=<auto_compact_tokens>`.
pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    todo!("#86")
}

/// 사용자 Codex 설정에 권한과 자동 압축 값이 있는지 본다.
/// `$CODEX_HOME/config.toml`(기본 `~/.codex/config.toml`)과 선택된 프로필에서
/// `approval_policy`·`sandbox_mode` → 권한, `model_auto_compact_token_limit` → 자동 압축.
/// 파일을 못 읽으면 값 없음으로 본다. 작업 공간에 TOML 파서가 없어 키 존재만 줄 단위로 본다.
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    todo!("#86")
}

/// 알림 하나를 이벤트로 바꾼다. 자식 thread 등록과 `active_turn` 갱신도 여기서 한다. 버릴 알림이면 빈 목록.
fn convert_notification(
    threads: &mut HashMap<ProviderSessionId, ThreadState>,
    method: &str,
    params: &serde_json::Value,
) -> Vec<ProviderEvent> {
    todo!("#86")
}

/// 자식 thread의 `SubagentId`. 자식 `thread_id`를 그대로 쓴다.
fn subagent_id(thread: &ProviderSessionId) -> SubagentId {
    todo!("#86")
}
