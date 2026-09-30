//! 보존과 정리: 기본 무제한 보존, 설정으로 켜면 시작 때 한 번 자동 정리, `--yes` 없으면 미리보기, 열린 항목 제외, 삭제 흔적.
//!
//! 설계: docs/design/records.md(보존과 정리, 삭제 대상 제외, 삭제 절차).
//! 삭제 절차(`--yes`일 때 한 번에): `PRAGMA secure_delete=ON` → 대상 채팅 행 삭제와 삭제 흔적 기록(한 거래)
//! → `PRAGMA wal_checkpoint(TRUNCATE)` → `VACUUM`. 판단 기록은 지우지 않는다(전용 정리는 `prune_judgments`).
//! `~/.claude`, `~/.codex`의 provider 기록은 지우지 않는다.

use std::time::{Duration, SystemTime};

use saturn_protocol::ids::ChatId;

use super::{Store, StoreError};

/// 자동 정리 정책. 설정 층에서 읽는다. 기본값은 무제한 보존(자동 정리 꺼짐). TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// 마지막 활동 뒤 이 기간이 지난 채팅을 시작 때 정리한다. `None`이면 자동 정리하지 않는다.
    pub max_age: Option<Duration>,
}

/// 정리 대상 범위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneScope {
    /// 지정한 채팅들.
    Chats(Vec<ChatId>),
    /// 마지막 활동이 이 시각보다 이른 채팅 전부.
    InactiveBefore(SystemTime),
}

/// 정리 요청. `yes`가 거짓이면 아무것도 지우지 않고 계획만 돌려준다(`Request::Prune { yes }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneRequest {
    /// 대상 범위.
    pub scope: PruneScope,
    /// `--yes`. 참일 때만 지운다.
    pub yes: bool,
}

/// 대상에서 뺀 이유. 어떤 명령으로 지워도 이 항목은 남긴다. 사용자는 먼저 그 작업을 끝내거나 멈춰야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 끝 상태가 아닌 입력이 있다.
    OpenInput,
    /// 끝나지 않은 실행이 있다.
    OpenRun,
    /// 처리 중인 중지 요청이 있다(`begin_stop` 뒤 `end_stop` 전).
    PendingStop,
    /// 활성 session(`Open`)이 있다.
    ActiveSession,
    /// 대기 session(`ClosedResumable`, `Held`)이 있다.
    WaitingSession,
}

/// 정리 계획. 미리보기와 실제 삭제 모두 같은 계획으로 만든다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// 지울(지운) 채팅.
    pub chats: Vec<ChatId>,
    /// 범위 안이지만 뺀 채팅과 이유. 이유가 여럿이면 모두.
    pub skipped: Vec<(ChatId, Vec<SkipReason>)>,
    /// 지울 행 수(입력, 실행, 이벤트, 사용량, session 합). 판단 기록과 설정 스냅샷은 세지 않는다.
    pub rows: u64,
}

/// 지운 채팅의 삭제 흔적. 같은 id가 다시 들어오면 알아본다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    /// 지운 채팅.
    pub chat: ChatId,
    /// 지우기 직전 채팅 내용(입력 원문과 이벤트를 순서대로)의 해시(hex).
    pub hash: String,
    /// 삭제 시각.
    pub deleted_at: SystemTime,
}

/// 정리 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneOutcome {
    /// `yes`가 거짓. 아무것도 지우지 않았다.
    Preview(PrunePlan),
    /// 지웠다. 계획과 남긴 삭제 흔적.
    Deleted {
        /// 실행한 계획.
        plan: PrunePlan,
        /// 지운 채팅마다 하나.
        tombstones: Vec<Tombstone>,
    },
}

impl Store {
    /// 범위 안 채팅에서 `SkipReason`에 걸리는 채팅을 빼고 계획을 만든다. 쓰지 않는다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn plan_prune(&self, scope: &PruneScope) -> Result<PrunePlan, StoreError> {
        todo!("#82")
    }

    /// 정리. `yes`가 거짓이면 `Preview`. 참이면 계획을 다시 만든 뒤(미리보기 이후 열린 항목이 생겼을 수 있다) 모듈 문서의 삭제 절차대로 지운다.
    ///
    /// # Errors
    /// 삭제, `wal_checkpoint`, `VACUUM` 실패면 `Database`. 삭제 거래가 끝난 뒤 정리 단계만 실패하면 삭제와 흔적은 남는다.
    pub async fn prune(&self, request: &PruneRequest) -> Result<PruneOutcome, StoreError> {
        todo!("#82")
    }

    /// 시작 때 한 번 부른다. `policy.max_age`가 `None`이면 아무것도 하지 않고 `None`. 있으면 `InactiveBefore(now - max_age)`로 `yes: true` 정리.
    ///
    /// # Errors
    /// `prune`과 같다.
    pub async fn prune_on_start(
        &self,
        policy: RetentionPolicy,
        now: SystemTime,
    ) -> Result<Option<PruneOutcome>, StoreError> {
        todo!("#82")
    }

    /// 채팅의 삭제 흔적. 새로 들어온 채팅 id가 지운 것과 같은지 확인할 때 쓴다.
    ///
    /// # Errors
    /// 조회 실패면 `Database`.
    pub async fn tombstone(&self, chat: ChatId) -> Result<Option<Tombstone>, StoreError> {
        todo!("#82")
    }
}
