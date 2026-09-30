//! 채팅, 입력 접수와 상태, 중지 요청, 실행과 `effect_scope`, session, provider 이벤트, 사용량 원값.
//!
//! 설계: docs/design/records.md(기록 저장소), docs/design/input-handling.md(입력 접수), docs/design/engine-lifecycle.md(효과 범위, 크래시 뒤 복구).
//! 메서드마다 거래 하나. 입력은 여기서 접수(ACK)된 뒤에만 `core::queue`로 넘기고 provider로 보낸다.
//! TODO(#31): 표 이름(chats/inputs/runs/sessions/events/usage 초안)을 용어 표에 맞출지

use std::path::PathBuf;
use std::time::SystemTime;

use saturn_core::queue::Permission;
use saturn_core::sessions::SessionRecord;
use saturn_protocol::event::{ProviderEvent, UsageReport};
use saturn_protocol::ids::{
    AgentId, ChatId, InputId, LedgerSeq, Provider, RunId, SessionId, SettingsRevision, TaskId,
};
use saturn_protocol::rpc::UsageRange;
use saturn_protocol::state::{EffectScope, InputState, QueueReason};

use super::{Store, StoreError};

/// 접수할 입력. 접수 때 설정 번호와 권한을 고정하고 끝까지 바꾸지 않는다.
#[derive(Debug, Clone)]
pub struct NewInput {
    /// 채팅.
    pub chat: ChatId,
    /// 원문. 마스킹하지 않고 그대로 둔다(사용자 입력이고 judge 키가 아니다).
    pub text: String,
    /// 접수 때 고정한 설정 번호.
    pub settings: SettingsRevision,
    /// 접수 때 고정한 권한.
    pub permission: Permission,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// 사용자가 고정한 모델.
    pub pinned_model: Option<String>,
    /// `Tab`으로 관계 판단 없이 대기.
    pub skip_relation: bool,
}

/// 시작할 실행 하나(provider 턴 하나).
#[derive(Debug, Clone)]
pub struct NewRun {
    /// 실행을 만든 입력. `provider-wake` 턴이면 `None`.
    pub input: Option<InputId>,
    /// 작업.
    pub task: TaskId,
    /// 에이전트.
    pub agent: AgentId,
    /// session.
    pub session: SessionId,
    /// provider.
    pub provider: Provider,
    /// 시작 때 효과 범위. 적용된 provider 설정으로 증명되면 `ProvenByConfig`, 아니면 `NetworkPossible`에서 시작한다.
    pub effect_scope: EffectScope,
}

/// 실행 끝 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// 완료 신호를 받았다.
    Completed,
    /// provider가 실패를 알렸다.
    Failed,
    /// 사용자 멈춤으로 중단했다.
    Stopped,
}

/// 저장된 실행 한 건. 크래시 뒤 복구에서 읽는다.
#[derive(Debug, Clone)]
pub struct RunRecord {
    /// 실행 id.
    pub id: RunId,
    /// 만든 입력.
    pub input: Option<InputId>,
    /// 작업.
    pub task: TaskId,
    /// session.
    pub session: SessionId,
    /// 마지막으로 기록한 효과 범위. `allows_auto_resume()`이 참일 때만 자동 재개한다.
    pub effect_scope: EffectScope,
    /// 시작 시각.
    pub started_at: SystemTime,
    /// 끝 결과. 끝나지 않았으면 `None`(열린 실행).
    pub end: Option<RunEnd>,
}

/// 사용량 원값 한 행. 보고하지 않은 칸은 `None`이고 합계에서 0으로 세지 않는다.
#[derive(Debug, Clone)]
pub struct UsageRow {
    /// 실행.
    pub run: RunId,
    /// session. `ThreadCumulative`는 같은 session의 직전 누적을 빼서 턴 값을 구한다.
    pub session: SessionId,
    /// 원값 그대로.
    pub report: UsageReport,
    /// 받은 시각.
    pub at: SystemTime,
    /// 중간 보고가 빠져 이 값의 차이가 여러 턴에 걸친다. 화면은 `여러 턴 합계`로 표시한다.
    pub spans_turns: bool,
}

impl Store {
    /// 새 채팅을 만든다. 판단 기록 저장은 켜진 상태로 시작한다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn create_chat(&self, workdir: PathBuf) -> Result<ChatId, StoreError> {
        todo!("#82")
    }

    /// `/record off`·`/record on`. 끄면 그 채팅의 판단 기록을 새로 저장하지 않는다(이미 저장한 것은 두고).
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_recording(&self, chat: ChatId, on: bool) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 채팅 층 설정(TOML 원문). 없으면 `None`. 보조 에이전트도 같은 채팅이므로 부모 채팅의 값을 그대로 읽는다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn chat_layer(&self, chat: ChatId) -> Result<Option<String>, StoreError> {
        todo!("#82")
    }

    /// 채팅 층 설정을 바꾼다. 원본 파일이 없는 층이라 여기가 정본이다.
    ///
    /// # Errors
    /// 없는 채팅이면 `NotFound`.
    pub async fn set_chat_layer(&self, chat: ChatId, toml: &str) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 입력 접수(ACK). 한 거래로 입력 행을 쓰고 상태 `Judging`으로 둔 뒤 새 `InputId`를 돌려준다.
    /// 이 함수가 성공한 뒤에만 `Queue::accept`를 부르고 TUI에 에코를 보낸다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`. 이때 입력은 접수되지 않았고 TUI에 실패를 알린다.
    pub async fn accept_input(&self, input: &NewInput) -> Result<InputId, StoreError> {
        todo!("#82")
    }

    /// 입력 상태와 대기 이유를 바꾼다. 전이 규칙 검사는 `core::queue`가 먼저 한다.
    ///
    /// # Errors
    /// 없는 입력이면 `NotFound`.
    pub async fn set_input_state(
        &self,
        input: InputId,
        state: InputState,
        reason: Option<QueueReason>,
    ) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 끝 상태(`Applied`, `Rejected`, `Cancelled`)가 아닌 입력. 시작 때 대기열을 되살리는 데 쓴다. 접수 순서.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn open_inputs(&self) -> Result<Vec<(InputId, NewInput, InputState)>, StoreError> {
        todo!("#82")
    }

    /// 멈춤(`Ctrl+C`) 요청을 처리 중으로 기록한다. 처리 중인 동안 그 채팅은 정리 대상에서 빠진다. 채팅마다 동시에 하나.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn begin_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 멈춤 처리 끝. 보류 결과는 입력과 session 상태에 이미 반영되어 있다.
    ///
    /// # Errors
    /// 처리 중인 요청이 없으면 `NotFound`.
    pub async fn end_stop(&self, chat: ChatId) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 실행 시작을 기록하고 `RunId`를 돌려준다. provider에 보내기 전에 부른다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`. 이때 provider로 보내지 않는다.
    pub async fn start_run(&self, run: &NewRun) -> Result<RunId, StoreError> {
        todo!("#82")
    }

    /// 효과 범위를 바꾼다. 관찰이 끊기면(`StreamLost`, 읽는 중 종료, 끝이 없는 subagent) `Unobserved`로 바꾸고 되돌리지 않는다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub async fn set_effect_scope(&self, run: RunId, scope: EffectScope) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 실행 끝을 기록하고 같은 거래 뒤에 원시 기록을 gzip으로 압축한다(`raw::compress_run`).
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 압축 실패면 `Compression`(끝 기록은 남고 원시 기록은 압축 전 그대로).
    pub async fn finish_run(&self, run: RunId, end: RunEnd) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 끝나지 않은 실행. 크래시 뒤 복구에서 `effect_scope`로 자동 재개와 보류를 나눈다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn unfinished_runs(&self) -> Result<Vec<RunRecord>, StoreError> {
        todo!("#82")
    }

    /// session 행을 넣거나 고친다. `provider_session`, 상태, `delivered`를 쓴다. `idle_since`(`Instant`)는 저장하지 않는다.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn upsert_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 채팅의 session 목록. 닫은 session의 provider session id로 재개할 때 쓴다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn sessions(&self, chat: ChatId) -> Result<Vec<SessionRecord>, StoreError> {
        todo!("#82")
    }

    /// provider 이벤트를 채팅 기록에 붙이고 새 `LedgerSeq`를 돌려준다. 번호는 채팅 트리 전체에서 1씩 늘어난다.
    /// 이벤트는 JSON으로 저장한다. `Usage` 이벤트는 여기 말고 `record_usage`로 쓴다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 직렬화 실패면 `Json`.
    pub async fn append_event(
        &self,
        run: RunId,
        chat: ChatId,
        event: &ProviderEvent,
    ) -> Result<LedgerSeq, StoreError> {
        todo!("#82")
    }

    /// `after` 뒤의 이벤트를 번호 순서로. session에 변경분만 첨부하거나 TUI가 다시 붙을 때 쓴다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`, 저장된 JSON이 깨졌으면 `Json`.
    pub async fn events_since(
        &self,
        chat: ChatId,
        after: LedgerSeq,
    ) -> Result<Vec<(LedgerSeq, ProviderEvent)>, StoreError> {
        todo!("#82")
    }

    /// 사용량 원값을 그대로 쓴다. `None`인 칸은 NULL. 범위(`UsageScope`), 에이전트, subagent, 모델도 함께 쓴다.
    /// `ThreadCumulative`에서 직전 누적보다 작거나 중간 보고가 빠졌으면 `spans_turns`를 참으로 기록한다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub async fn record_usage(
        &self,
        run: RunId,
        session: SessionId,
        report: &UsageReport,
    ) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// `/usage` 범위의 원값 행. `Chat`이면 `chat`이 있어야 한다. 합계는 호출자가 NULL을 빼고 계산한다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn usage_rows(
        &self,
        range: UsageRange,
        chat: Option<ChatId>,
    ) -> Result<Vec<UsageRow>, StoreError> {
        todo!("#82")
    }
}
