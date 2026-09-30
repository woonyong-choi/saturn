//! Claude Code 연결. session마다 `claude` 프로세스 하나를 stream-json 입출력으로 켜 둔다.
//!
//! 설계: docs/design/providers-and-sessions.md(provider 연결, 이벤트 수신과 변환, subagent 트리 추적, 트리 전체 중지),
//! docs/design/judge-key-security.md(Saturn 소유 PreToolUse 훅).
//!
//! 실행 인자: `-p --input-format stream-json --output-format stream-json --verbose`
//! + `--resume <session_id>`(재개일 때) + `--model <모델>`(지정일 때) + `default_args` + `--settings <hook_settings JSON 문자열>`(있을 때).
//! 프로세스 수명은 턴 진행 중과 턴 끝 뒤 5분 유예까지이고, 유예 뒤 닫기는 `sessions`가 `close_session`으로 부른다.
//!
//! | Saturn 동작 | Claude stream-json |
//! |---|---|
//! | 새 턴 | stdin에 `{"type":"user","message":{"role":"user","content":[{"type":"text","text":...}]}}` 한 줄 |
//! | 끼워 넣기 | 턴 진행 중에 같은 모양 한 줄 추가. 턴 끝과 겹치면 provider가 다음 턴에 처리한다 |
//! | 멈춤 신호 | `{"type":"control_request","request_id":...,"request":{"subtype":"interrupt"}}` |
//! | compaction | 새 턴으로 `/compact` 전송 |
//! | 명령과 스킬 | 프롬프트에 `/이름 인자`를 그대로 넣어 새 턴으로 전송 |
//! | session 닫기 | stdin을 닫고 프로그램 종료를 기다린다 |
//! | session 재개 | 새 프로세스를 `--resume <session_id>`로 띄운다 |
//!
//! 이벤트 변환(stdout 한 줄에 JSON 하나):
//! - `system/init` → 이벤트 없음. `session_id`, `slash_commands`, `model`, `permissionMode`를 읽어 session 상태에 둔다.
//!   턴마다 다시 오면 적용값과 명령 목록을 새 값으로 바꾼다(설정을 바꾸는 명령의 결과도 여기서 읽는다).
//! - `assistant`의 글 → `Text`, `tool_use` → `ToolCall`, `user`의 `tool_result` → `ToolResult`.
//!   `parent_tool_use_id`가 있으면 그 id로 찾은 subagent를 `subagent`에 채운다.
//! - 이름이 `Task` 또는 `Agent`인 `tool_use` → `SubagentStarted { subagent: SubagentId(tool_use id), parent }`.
//!   parent는 그 호출의 `parent_tool_use_id`가 가리키는 subagent, 없으면 `None`. 같은 id의 `tool_result` → `SubagentEnded`.
//! - 허가 요청 → `PermissionRequested`. 스트림에서 오는 모양은 #26 실측으로 확인한다.
//! - `result` → `Usage { scope: UsageScope::MainTurn }`, `ContextSize`, `TurnCompleted { origin }` 순서.
//!   origin은 `TurnOriginTracker`로 정한다(Saturn 입력 없이 끝난 턴이면 `ProviderWake`).
//! - `result` 없이 stdout이 닫힘, 읽기 오류, 프로세스 종료, `tool_result` 없이 끝난 subagent → `StreamLost`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use saturn_core::providers::{
    InterruptTarget, ProviderClient, ProviderCommand, ProviderError, SessionHandle, SessionSpec,
};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId, SubagentId};
use tokio::process::ChildStdin;
use tokio::sync::mpsc;

use super::{AppliedSettings, LaunchSpec, TurnOriginTracker, UserProviderConfig};
use crate::processes::{ProcessGroupId, Supervisor};

/// 끼워 넣기(스트림 입력 추가) 실측(#5, #27) 통과 여부. 거짓이면 `SessionHandle::steer_verified`가 거짓이 되어 끼워 넣기를 대기로 바꾼다.
pub(crate) const STEER_VERIFIED: bool = false;

/// subagent를 띄우는 도구 이름.
const SUBAGENT_TOOLS: &[&str] = &["Task", "Agent"];

/// `slash_commands`에서 뺄 이름. 화면 전용 명령과 Saturn session 명령이 대신하는 명령(대화 비우기, 재개, 종료).
/// 설정을 바꾸는 명령(`model`, `permissions` 등)은 빼지 않는다. 목록은 후보다(설계에 목록 없음).
/// TODO(#87): 값 미정, 초안 `clear`, `resume`, `exit`, `quit`
pub(crate) const EXCLUDED_COMMANDS: &[&str] = &["clear", "resume", "exit", "quit"];

/// 권한 기본값 인자(수정 허용). 사용자 설정에 권한 값이 없을 때만 넣는다. 인자 값은 초안이다(설계는 수정 허용만 정함).
/// TODO(#87): 값 미정, 초안 `--permission-mode acceptEdits`
const PERMISSION_ARGS: &[&str] = &["--permission-mode", "acceptEdits"];

/// 자동 압축 안전망 인자. `--autocompact <T_hard>`로 넘긴다. `T_hard`가 `AUTO_COMPACT_MIN`보다 작으면 그 값으로 올린다.
const AUTO_COMPACT_FLAG: &str = "--autocompact";

/// `--autocompact` 최솟값(토큰). 초안 값이다(설계에 없고 Claude 규약 확인 전).
/// TODO(#87): 값 미정, 초안 100,000
const AUTO_COMPACT_MIN: u64 = 100_000;

/// session(프로세스) 하나의 변환 상태. 읽기 작업과 연결이 `Arc<Mutex<_>>`로 같이 쓴다.
#[derive(Debug)]
struct SessionState {
    /// 이 session을 쓰는 Saturn 에이전트.
    agent: AgentId,
    /// 턴 진행 중인지. 새 턴 입력을 쓴 때 참, `result`를 받은 때 거짓.
    turn_active: bool,
    /// 턴 시작 주체 판정.
    origin: TurnOriginTracker,
    /// 끝나지 않은 subagent. 키는 Task/Agent `tool_use` id, 값은 부모 subagent.
    running: HashMap<SubagentId, Option<SubagentId>>,
    /// `system/init`에서 읽은 적용값.
    applied: AppliedSettings,
    /// `system/init`의 `slash_commands`(거르기 전).
    commands: Vec<ProviderCommand>,
}

/// session 하나의 프로세스와 입력 창구.
#[derive(Debug)]
struct SessionLink {
    /// 이 프로세스의 묶음.
    group: ProcessGroupId,
    /// 입력을 쓰는 쪽. 닫으면 프로그램이 끝난다.
    stdin: ChildStdin,
    /// 읽기 작업과 같이 쓰는 변환 상태.
    state: Arc<Mutex<SessionState>>,
}

/// Claude Code 연결. session마다 프로세스 하나.
#[derive(Debug)]
pub struct ClaudeClient {
    /// 프로세스 감시자.
    supervisor: Supervisor,
    /// 실행 준비값. session을 열 때마다 이 값으로 프로세스를 띄운다.
    launch: LaunchSpec,
    /// session별 연결. 키는 `system/init`의 `session_id`.
    sessions: HashMap<ProviderSessionId, SessionLink>,
    /// 모든 session 읽기 작업이 보내는 이벤트 채널의 받는 쪽.
    events: mpsc::Receiver<ProviderEvent>,
    /// 새 session 읽기 작업에 복제해 주는 보내는 쪽.
    events_tx: mpsc::Sender<ProviderEvent>,
    /// 다음 제어 요청 id.
    next_request_id: u64,
}

impl ClaudeClient {
    /// 연결을 만든다. 프로세스는 `open_session`에서 띄운다.
    pub fn new(launch: LaunchSpec, supervisor: Supervisor) -> Self {
        todo!("#87")
    }

    /// session 프로세스의 묶음. 모르는 session이면 `None`.
    pub fn process_group(&self, session: &ProviderSessionId) -> Option<ProcessGroupId> {
        todo!("#87")
    }

    /// session에 적용된 설정. `system/init`을 받기 전이면 `None`.
    pub fn applied_settings(&self, session: &ProviderSessionId) -> Option<AppliedSettings> {
        todo!("#87")
    }

    /// 사용자 메시지 한 줄을 stdin에 쓴다. 쓰기 전 실패는 `NotSent`, 쓰는 중 실패(끊긴 파이프)는 `Unknown`.
    async fn write_user_message(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        todo!("#87")
    }
}

impl ProviderClient for ClaudeClient {
    /// 프로세스를 띄우고(`--resume`은 `spec.resume`이 있을 때) `system/init`을 기다려 `session_id`를 돌려준다.
    /// `packet`이 있으면 첫 턴으로 보낸다. `steer_verified`는 `STEER_VERIFIED`.
    /// stream-json은 첫 입력 전에 `system/init`을 내지 않을 수 있어, 그때는 `packet` 또는 재개 id로 session을 식별한다.
    ///
    /// 재개 실패(`--resume` 뒤 오류로 종료)는 `NotSent`.
    async fn open_session(&mut self, spec: SessionSpec) -> Result<SessionHandle, ProviderError> {
        todo!("#87")
    }

    /// 사용자 메시지 한 줄. 보내기 전 `TurnOriginTracker::on_user_send`, `turn_active = true`.
    async fn send_turn(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        todo!("#87")
    }

    /// `turn_active`가 거짓이면 쓰지 않고 `NoActiveTurn`. 참이면 사용자 메시지 한 줄을 더 쓴다.
    /// 쓰는 사이 턴이 끝나도 provider가 다음 턴에 처리하므로 `Ok`.
    async fn steer(
        &mut self,
        session: &ProviderSessionId,
        text: &str,
    ) -> Result<(), ProviderError> {
        todo!("#87")
    }

    /// `Main`이면 interrupt 제어 요청을 쓰고 `control_response`를 기다린다. `Subagent`는 따로 멈출 경로가 없어 보내지 않고 `Ok`.
    /// 백그라운드 subagent까지 멈추는지는 #18 실측으로 확인한다. 남은 프로세스는 호출자가 `Supervisor::stop_tree`로 정리한다.
    async fn interrupt(
        &mut self,
        session: &ProviderSessionId,
        target: InterruptTarget,
    ) -> Result<(), ProviderError> {
        todo!("#87")
    }

    /// `/compact`를 새 턴으로 보낸다. 턴 진행 중이면 `NotSent`(턴 끝 경계에서만 보낸다).
    async fn compact(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        todo!("#87")
    }

    /// stdin을 닫고 `Supervisor::wait`로 종료를 기다린다. 끝나지 않으면 `Supervisor::stop_tree(StopScope::Whole)`.
    /// session 상태를 뺀다. `session_id`는 호출자가 보관해 `--resume`에 쓴다.
    async fn close_session(&mut self, session: &ProviderSessionId) -> Result<(), ProviderError> {
        todo!("#87")
    }

    /// 이벤트 채널에서 받는다. 모든 session이 닫혀도 채널은 남으므로 `None`은 연결을 버릴 때만.
    async fn next_event(&mut self) -> Option<ProviderEvent> {
        todo!("#87")
    }

    /// 가장 최근 `system/init`의 `slash_commands`를 `EXCLUDED_COMMANDS`로 거른 목록.
    /// `system/init`에 스킬 이름 목록이 따로 오면 그 이름은 `is_skill = true`, 아니면 모두 거짓. 설명이 없으면 빈 문자열.
    fn commands(&self) -> Vec<ProviderCommand> {
        todo!("#87")
    }
}

/// Saturn 기본값 인자. `user.has_permission`이 거짓이면 `PERMISSION_ARGS`,
/// `user.has_auto_compact`가 거짓이면 `--autocompact max(auto_compact_tokens, AUTO_COMPACT_MIN)`.
pub(crate) fn default_args(user: UserProviderConfig, launch: &LaunchSpec) -> Vec<String> {
    todo!("#87")
}

/// 사용자 Claude 설정에 권한과 자동 압축 값이 있는지 본다.
/// `~/.claude/settings.json`, `<작업 폴더>/.claude/settings.json`, `<작업 폴더>/.claude/settings.local.json`의
/// `permissions.defaultMode` → 권한, `autoCompactEnabled` → 자동 압축.
/// 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있어도 자동 압축 값이 있는 것으로 본다. 파일을 못 읽으면 값 없음.
/// TODO(#87): 값 미정, 초안 위 파일·키·환경 변수 목록(Claude 규약에서 확인)
pub(crate) fn read_user_config(launch: &LaunchSpec) -> UserProviderConfig {
    todo!("#87")
}

/// stdout 한 줄을 이벤트로 바꾼다. subagent 등록과 `turn_active`, 적용값, 명령 목록 갱신도 여기서 한다. 버릴 줄이면 빈 목록.
fn convert_line(state: &mut SessionState, line: &serde_json::Value) -> Vec<ProviderEvent> {
    todo!("#87")
}
