//! compaction 판정과 패킷 구성.
//!
//! 설계: docs/design/context-management.md. 기호:
//! - `A`: 턴 끝마다 잰 활성 맥락 크기(토큰)
//! - `T = min(T_abs, N% × 창 크기)`: 발동 기준. `T_abs`는 provider별 절대 토큰, `N`은 안전 비율
//! - `P`: 새 session에 넘길 패킷 크기, `P_max = T / 10`
//! - `r`, `w`: 캐시 읽기 배수, 캐시 쓰기 배수
//! - `k* = (P×w − A×r) / ((A − P) × r)`: 새 session으로 옮기는 비용과 그대로 잇는 비용이 같아지는 턴 수
//! - `H`: 턴당 증가량의 p95(관측 전에는 창의 10%), `T_hard = T + H`: provider 자동 압축 안전망

use std::time::Duration;

use saturn_protocol::ids::LedgerSeq;

/// provider별 기준값. 설정에서 읽는다. TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy)]
pub struct ContextBudget {
    /// provider별 절대 토큰 `T_abs`.
    pub t_abs: u64,
    /// 안전 비율 `N`(0~100).
    pub safety_percent: u8,
    /// 모델 맥락 창 크기.
    pub window: u64,
    /// 캐시 읽기 배수 `r`, 쓰기 배수 `w`.
    pub cache_read: f64,
    /// 캐시 쓰기 배수.
    pub cache_write: f64,
    /// provider 캐시 유지 시간. 마지막 턴 뒤 이만큼 지나면 유휴 복귀 조건을 본다.
    pub cache_ttl: Duration,
}

impl ContextBudget {
    /// 발동 기준 `T`.
    pub fn threshold(&self) -> u64 {
        todo!("#78")
    }

    /// provider 자동 압축에 넘길 안전망 `T_hard = T + H`. 사용자가 provider 설정에 값을 정해 두면 넣지 않는다.
    pub fn hard_limit(&self, p95_growth: Option<u64>) -> u64 {
        todo!("#78")
    }
}

/// 턴 끝에 판정에 쓰는 값.
#[derive(Debug, Clone, Copy)]
pub struct ContextMeasure {
    /// `A`. 잴 수 없으면 `None`이고 Saturn 재시작 없이 provider 자동 압축에 맡긴다. TODO(#62): 루트 메시지로만 계산할지
    pub active: Option<u64>,
    /// 예상 패킷 크기 `P`.
    pub packet: u64,
    /// 트리 유휴인지.
    pub tree_idle: bool,
    /// A에 합칠 대기 입력이 있는지.
    pub has_mergeable_queue: bool,
    /// 마지막 턴 뒤 지난 시간.
    pub since_last_turn: Duration,
    /// 기대 잔여 턴. 모르면 `None`(3으로 본다).
    pub expected_turns: Option<u32>,
}

/// compaction 판정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionDecision {
    /// 그대로 계속한다.
    Continue,
    /// 다음 턴 경계까지 미룬다(트리 유휴가 아니거나 합칠 대기 입력이 있음).
    Defer,
    /// 새 session을 열고 패킷을 넘긴다.
    Restart,
}

/// 판정 순서: 유휴 아님이나 합칠 입력 → `Defer`, 캐시 만료이고 `P < A` → `Restart`, `A < T` → `Continue`,
/// `A ≥ T`이고 `k*` ≤ 기대 잔여 턴 → `Restart`, 그 밖 → `Continue`.
pub fn decide(budget: &ContextBudget, measure: &ContextMeasure) -> CompactionDecision {
    todo!("#78")
}

/// 본전 턴 수 `k*`.
pub fn break_even_turns(budget: &ContextBudget, active: u64, packet: u64) -> f64 {
    todo!("#78")
}

/// 패킷 재료. 모두 Saturn 기록 원문에서 고른다(provider 요약은 정본이 아니다).
#[derive(Debug, Clone, Default)]
pub struct PacketSource {
    /// 사용자가 명시한 제약과 결정 원문. 항상 넣는다.
    pub pinned: Vec<String>,
    /// 현재 목표와 마지막 사용자 입력 원문.
    pub goal_and_last_input: Vec<String>,
    /// 끝나지 않은 항목과 효과를 모르는 항목.
    pub open_items: Vec<String>,
    /// 최근 3턴 원문.
    pub recent_turns: Vec<String>,
    /// 그 이전 도구 호출. `compact` 질문이 남기라고 한 것만 들어온다. 결과는 앞 300자와 한 줄 메모로 줄인다.
    pub kept_tool_calls: Vec<(String, String)>,
    /// 파일은 내용 대신 경로만.
    pub file_paths: Vec<String>,
    /// 이 패킷이 포함하는 마지막 기록 번호.
    pub up_to: LedgerSeq,
}

/// 새 session에 넘길 패킷.
#[derive(Debug, Clone)]
pub struct Packet {
    /// 본문.
    pub text: String,
    /// 토큰 수 추정.
    pub tokens: u64,
    /// 포함한 마지막 기록 번호. 새 session의 `delivered`가 된다.
    pub up_to: LedgerSeq,
}

/// 패킷을 위 순서대로 만들고 `P_max`를 넘으면 뒤쪽 항목부터 줄인다.
/// provider가 스스로 읽는 문서(`AGENTS.md`, `CLAUDE.md`)는 넣지 않는다.
pub fn build_packet(source: &PacketSource, max_tokens: u64) -> Packet {
    todo!("#78")
}
