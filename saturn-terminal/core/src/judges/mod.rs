//! judge 공통 규격과 판단 규칙: 질문 세트, 답 형식, 확신도, 질문별 기준값과 대체 규칙.
//!
//! 구현(외부 API, 로컬 Saturn 모델)은 `saturn-engine`의 `judges`. 여기는 요청을 만들고 답을 행동으로 바꾸는 순수 규칙만 둔다.
//! 설계: docs/design/judge.md. 기계로 정할 수 있으면 코드가, 뜻을 이해해야 하면 judge가 정한다.

pub mod calibration;

use std::future::Future;

use saturn_protocol::ids::{ChatRevision, SettingsRevision};
use saturn_protocol::state::Disposition;

/// judge 호출과 답 해석 오류.
#[derive(Debug, thiserror::Error)]
pub enum JudgeError {
    /// 응답이 없거나 시간 초과. 입력을 대기로 보낸다.
    #[error("judge did not respond")]
    NoResponse,
    /// 보낸 뒤 시간 초과. `cost-unknown`으로 기록하고 다시 보내지 않는다.
    #[error("judge timed out after send")]
    TimedOutAfterSend,
    /// 키가 없거나 거절됐다. 연결 실패와 구분해 키 입력 창을 띄운다.
    #[error("judge rejected the key")]
    Unauthorized,
    /// 속도 제한. 기다렸다가 다시 보낸다.
    #[error("judge rate limited")]
    RateLimited,
    /// 후보 밖 선택, NaN, 확률 누락. 판단을 `invalid`로 기록하고 대체 규칙을 적용한다.
    #[error("judge answer is invalid: {reason}")]
    Invalid { reason: String },
    /// 판단 중 채팅 revision이 바뀌었다. `superseded`로 기록한다.
    #[error("chat revision changed during judgment")]
    Superseded,
}

/// 질문 세트 이름과 버전. 기록에 `route@3.1`처럼 남는다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct QuestionSetId {
    /// `route`, `relation`, `send-opt`, `file-rank`, `context-select`, `compact`, `doc-filter`, `loop`, `feedback`.
    pub name: String,
    /// 주 버전. 뜻이나 선택지가 바뀌면 올리고 옛 선택지 → 새 선택지 대응표를 함께 둔다.
    pub major: u16,
    /// 소수 버전. 뜻이 같은 작은 변경. 옛 답과 라벨을 그대로 쓴다.
    pub minor: u16,
}

/// 질문 하나. 문장과 선택지는 영어로 쓰고 사용자 원문은 `state`에 그대로 넣는다.
#[derive(Debug, Clone)]
pub struct Question {
    /// 질문 id(`keep_current`, `file_3_relevant` 등).
    pub id: String,
    /// 영어 질문 문장.
    pub text: String,
    /// 답 형식.
    pub kind: AnswerKind,
}

/// 답 형식.
#[derive(Debug, Clone)]
pub enum AnswerKind {
    /// 선택지 하나. 255개 이하, 답이 하나인 질문에는 `other`를 넣는다. 넘으면 계층 선택으로 나눈다.
    Choice { options: Vec<String> },
    /// 예·아니요. 여러 개가 맞을 수 있는 속성은 `noul`로 하나씩 묻는다.
    Noul,
    /// 2~10단계 분포.
    Score { levels: u8 },
}

/// judge 답 하나.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Answer {
    /// 선택지별 확률. 순서는 `options`와 같다.
    Choice(Vec<f64>),
    /// P(yes).
    Noul(f64),
    /// 단계별 확률.
    Score(Vec<f64>),
}

impl Answer {
    /// 확신도 `(N·pmax − 1)/(N − 1)`. 균등이면 0, 한 선택지가 1이면 1. `noul`은 두 선택지로 본다.
    pub fn confidence(&self) -> f64 {
        todo!("#80")
    }
}

/// 판단 호출 하나의 결과 종류. 판단 기록에 남는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JudgmentOutcome {
    /// 정상 답.
    Ok,
    /// 형식 오류. 대체 규칙을 적용했다.
    Invalid,
    /// 판단 중 채팅 revision이 바뀌었다.
    Superseded,
    /// 보낸 뒤 시간 초과. 비용을 모른다.
    CostUnknown,
    /// 응답 없음.
    NoResponse,
}

/// judge 요청 한 건. 입력당 한 번 부르고 필요한 질문을 모두 묶는다.
#[derive(Debug, Clone)]
pub struct JudgeRequest {
    /// 고정 버전 모델 이름. 별칭을 쓰면 응답의 `model`을 기록한다.
    pub model: String,
    /// 채팅 상태 요약. 비밀값, 절대 경로, 다른 대화 원문은 넣지 않는다. 앞 입력의 판단 결과를 넣는다.
    pub state: String,
    /// 이번에 묻는 질문 세트들.
    pub sets: Vec<(QuestionSetId, Vec<Question>)>,
}

/// judge 응답.
#[derive(Debug, Clone)]
pub struct JudgeResponse {
    /// 응답이 보고한 실제 모델.
    pub model: String,
    /// 질문 id별 답.
    pub answers: Vec<(String, Answer)>,
    /// 입력 토큰과 출력 토큰. 비용 표시에 쓴다.
    pub tokens: (u64, u64),
}

/// judge 연결. 모든 judge는 이 trait의 구현으로만 붙는다.
pub trait JudgeClient: Send + Sync {
    /// 시작 확인. 외부 judge는 `GET /v1/models`와 실제 판단 1건, 로컬은 모델 로드나 서버 응답.
    ///
    /// # Errors
    /// 확인 실패. 호출자는 키를 다시 받거나 Saturn을 실행하지 않는다.
    fn check(&self) -> impl Future<Output = Result<(), JudgeError>> + Send;

    /// 판단 요청. 64K를 넘거나 `state`와 가장 긴 질문 합이 32K를 넘으면 구현이 나눠 보낸다.
    ///
    /// # Errors
    /// `JudgeError` 종류별로 호출자가 대기, 대체 규칙, 재전송을 고른다.
    fn judge(
        &self,
        request: JudgeRequest,
    ) -> impl Future<Output = Result<JudgeResponse, JudgeError>> + Send;
}

/// 판단 방식. 판단 기록은 방식과 관계없이 전부 남긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// 기준 judge(외부 API)가 행동을 정한다.
    Jev,
    /// Saturn 모델이 정한다. 확신도가 기준보다 낮으면 행동하지 않고 대체 규칙으로 간다.
    Saturn,
    /// TODO(#40): `collect`의 뜻 미정
    Collect,
}

/// 질문별 기준값. 설정 층에 두고 사용자 층과 폴더 층에서 바꿀 수 있다. 기본값은 docs/design/judge.md 표.
#[derive(Debug, Clone)]
pub struct Thresholds {
    /// `keep_current` 유지 기준(0.8). 0.3~0.8이면 판단 없음과 같이 현재 에이전트 유지.
    pub keep_current: f64,
    /// `is_actionable` 명확 기준(0.7).
    pub is_actionable: f64,
    /// `choice`와 `score` 확신도 기준(0.6).
    pub min_confidence: f64,
    /// `resume_held` 재개 기준(0.85). 아래면 무시 횟수 1 증가.
    pub resume_held: f64,
    /// `file_<n>_relevant` 존재 기준(0.7)과 없음 기준(0.35).
    pub file_relevant: (f64, f64),
    /// `context-select` 게이트 평균 없음 기준(0.3).
    pub context_gate: f64,
    /// `compact` 유지 기준(0.5).
    pub compact_keep: f64,
    /// `doc-filter` `injection` 제외 기준(0.7).
    pub injection: f64,
    /// `loop` `is_progressing` 루프 기준(0.2).
    pub progressing: f64,
    /// `feedback` 원인 사용 기준(0.7).
    pub feedback_cause: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        todo!("#80")
    }
}

/// 입력 하나의 판단 결과. `queue`가 적용 직전에 `revision`을 비교한다.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// 판단 시점의 채팅 revision.
    pub revision: ChatRevision,
    /// 판단 때 쓴 설정 번호.
    pub settings: SettingsRevision,
    /// 끼워 넣기, 새 작업, 대기.
    pub disposition: Disposition,
    /// 하던 에이전트로 보낼지(`keep_current`). 거짓이면 새 작업이나 보조 에이전트.
    pub keep_current: bool,
    /// 쓸 모델. `None`이면 사용자 고정 모델이나 현재 모델.
    pub model: Option<String>,
    /// 보류 작업을 재개할지(`resume_held`).
    pub resume_held: bool,
    /// 대체 규칙을 적용한 질문 id. 기록에 사유와 함께 남긴다.
    pub fallbacks: Vec<String>,
}

/// 입력 판단에 필요한 질문을 고른다. 제어 명령이 아닌 입력만. 사용자가 모델을 고정했으면 `target_model`을 뺀다.
/// 실행 중이면 `relation`과 `send-opt`를 더한다.
pub fn questions_for_input(
    running: bool,
    model_pinned: bool,
    has_held: bool,
) -> Vec<(QuestionSetId, Vec<Question>)> {
    todo!("#80")
}

/// judge 답을 행동으로 바꾼다. `keep_current`를 `is_actionable`보다 먼저 읽는다(이어 가는 입력이 파일 탐색으로 빠지지 않게).
/// 답이 없거나 확신도가 낮거나 `invalid`면 질문별 대체 규칙을 쓴다. `Saturn` 방식이면 확신도 미달 시 행동하지 않는다.
pub fn decide_route(
    answers: Option<&JudgeResponse>,
    thresholds: &Thresholds,
    method: Method,
    revision: ChatRevision,
    settings: SettingsRevision,
) -> RouteDecision {
    todo!("#80")
}

/// judge 답의 형식 검사. 후보 밖 선택, NaN, 확률 누락이면 `Invalid`.
///
/// # Errors
/// 형식이 맞지 않으면 `JudgeError::Invalid`.
pub fn validate(request: &JudgeRequest, response: &JudgeResponse) -> Result<(), JudgeError> {
    todo!("#80")
}
