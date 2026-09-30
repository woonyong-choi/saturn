//! 외부 judge API 연결(`jev` 방식의 기준 judge).
//!
//! 설계: docs/design/judge.md(judge 시작 확인, judge 호출, 오류 처리), docs/design/judge-key-security.md(전송, 출력 마스킹).
//! 전송 규칙:
//! - HTTPS만 쓰고 허용 호스트는 `api.typesafe.ai` 하나다. 주소 검사는 만들 때 한 번 하고 요청마다 다시 본다.
//! - 리다이렉트 때 인증 헤더를 지운다. 허용 호스트 밖으로 가는 리다이렉트는 따르지 않는다.
//! - TLS 검증을 끄는 설정은 두지 않는다.
//! - Authorization 헤더는 어디에도 기록하지 않는다. 키는 요청 직전에 `SecretStore::key`로 빌리고 들고 있지 않는다.
//! - 모델은 버전을 고정한 이름으로 부르고, 별칭이면 응답의 `model`을 기록한다.
//!
//! 재시도 규칙(보내기 전에 확정된 실패만 다시 보낸다):
//! | 실패 | 처리 |
//! |---|---|
//! | 보내기 전 실패(연결 실패, DNS 등) | `RetryPolicy::pre_send_attempts`번까지 다시 보낸다 |
//! | 보낸 뒤 시간 초과 | `JudgeError::TimedOutAfterSend`(`cost-unknown`). 다시 보내지 않는다 |
//! | 속도 제한 | 기다렸다가 다시 보낸다 |
//! | 후보 밖 선택, NaN, 확률 누락 | `JudgeError::Invalid`. 다시 보내지 않는다 |
//!
//! TODO(#88): HTTP 클라이언트 crate(작업 공간 의존성 추가 필요). 만들 때 리다이렉트 정책(`redirect_headers`)과
//! TLS 검증 고정(끄는 옵션을 노출하지 않음)을 건다. 이 파일은 클라이언트 타입을 아직 쓰지 않는다

use std::time::Duration;

use saturn_core::judges::{JudgeClient, JudgeError, JudgeRequest, JudgeResponse, Question};

use super::{JudgeExchange, JudgesError, SharedSecrets};

/// 허용 호스트. judge 키는 이 호스트로만 간다.
pub const ALLOWED_HOST: &str = "api.typesafe.ai";

/// 요청 하나의 크기 한도. 넘으면 나눠 보낸다. 설계는 64K만 정하고 단위(토큰, 바이트)와 1K 크기는 정하지 않았다.
/// TODO(#88): 값 미정, 초안 64 × 1024(단위 미정)
pub const REQUEST_SPLIT_LIMIT: usize = 64 * 1024;

/// `state`와 가장 긴 질문의 합 한도. 넘으면 나눠 보낸다. 단위는 `REQUEST_SPLIT_LIMIT`와 같다.
/// TODO(#88): 값 미정, 초안 32 × 1024(단위 미정)
pub const STATE_SPLIT_LIMIT: usize = 32 * 1024;

/// `choice` 선택지 최대 수. 넘으면 계층 선택으로 나눈다.
pub const MAX_CHOICES: usize = 255;

/// 재시도 설정. 값은 설정 층에서 읽는다. TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// 보내기 전 실패를 다시 보내는 최대 횟수(설정된 횟수).
    pub pre_send_attempts: u32,
    /// 응답을 기다리는 시간. 보낸 뒤 이 시간이 지나면 `TimedOutAfterSend`.
    pub response_timeout: Duration,
    /// 속도 제한 응답이 기다릴 시간을 주지 않을 때 기다리는 시간. TODO(#88): 값 미정
    pub rate_limit_wait: Duration,
}

/// 외부 judge 연결. `JudgeClient`로 engine에만 붙는다.
#[derive(Debug)]
pub struct RemoteJudge {
    /// 검사를 통과한 judge 주소(`https://api.typesafe.ai/...`).
    endpoint: String,
    /// 버전을 고정한 모델 이름.
    model: String,
    /// 키 보관소. 요청 직전에만 키를 빌린다.
    secrets: SharedSecrets,
    /// 재시도 설정.
    retry: RetryPolicy,
    // TODO(#88): HTTP 클라이언트 필드. HTTPS 전용, 리다이렉트 때 인증 헤더 제거, TLS 검증 고정
}

impl RemoteJudge {
    /// 주소를 검사하고 연결을 만든다. 아직 네트워크를 쓰지 않는다.
    ///
    /// # Errors
    /// 주소가 HTTPS가 아니거나 호스트가 `ALLOWED_HOST`가 아니면 `DisallowedEndpoint`.
    pub fn new(
        endpoint: &str,
        model: String,
        secrets: SharedSecrets,
        retry: RetryPolicy,
    ) -> Result<Self, JudgesError> {
        todo!("#88")
    }

    /// 고정한 모델 이름.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// `GET /v1/models`. 키가 맞는지와 고정한 모델이 목록에 있는지 본다.
    ///
    /// # Errors
    /// 인증 실패(키 없음, 거절)는 `Unauthorized`, 연결 실패와 모델 없음은 `NoResponse`.
    pub async fn list_models(&self) -> Result<Vec<String>, JudgeError> {
        todo!("#88")
    }

    /// 요청과 응답 원문을 함께 돌려주는 판단 호출. 흐름:
    /// 1. `split_request`로 크기 한도에 맞게 나누고, 선택지가 255개를 넘는 질문은 `split_choices`로 계층 선택으로 나눈다.
    /// 2. 조각마다 `send_with_retry`로 보낸다. 한 조각이라도 실패하면 그 실패를 결과로 한다.
    /// 3. `merge_responses`로 합친다. 형식 검사는 호출자가 한다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        todo!("#88")
    }

    /// 조각 하나를 보낸다. 보내기 전 실패만 `RetryPolicy::pre_send_attempts`번까지 다시 보내고,
    /// 속도 제한은 기다렸다가 다시 보낸다. 보낸 뒤 시간 초과는 다시 보내지 않는다.
    /// 돌려주는 값은 (보낸 본문, 받은 본문, 결과)다.
    async fn send_with_retry(
        &self,
        request: &JudgeRequest,
    ) -> (String, Option<String>, Result<JudgeResponse, JudgeError>) {
        todo!("#88")
    }

    /// HTTPS POST 한 번. 인증 헤더는 요청 직전에 `SecretStore::key`로 붙이고 기록하지 않는다.
    /// 실패는 보내기 전 실패와 보낸 뒤 실패로 나눠 돌려준다.
    async fn send_once(&self, body: &str) -> Result<String, SendFailure> {
        todo!("#88")
    }
}

impl JudgeClient for RemoteJudge {
    /// `list_models`로 키와 모델을 확인한 뒤 실제 판단 1건을 보낸다.
    async fn check(&self) -> Result<(), JudgeError> {
        todo!("#88")
    }

    /// `exchange`의 결과만 돌려준다.
    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        todo!("#88")
    }
}

/// 전송 한 번의 실패 구분. 재시도 판단에만 쓴다.
#[derive(Debug)]
enum SendFailure {
    /// 보내기 전에 실패했다. 전달되지 않은 것이 확정이라 다시 보내도 된다.
    BeforeSend,
    /// 보낸 뒤 응답 시간 초과. 다시 보내지 않는다.
    TimedOutAfterSend,
    /// 속도 제한. `retry_after`가 있으면 그만큼, 없으면 `RetryPolicy::rate_limit_wait`만큼 기다린다.
    RateLimited { retry_after: Option<Duration> },
    /// 응답은 받았으나 오류 상태다. 본문은 가리기 전 값이다.
    Rejected { status: u16, body: String },
}

/// 주소 검사. `https` 체계이고 호스트가 `ALLOWED_HOST`와 정확히 같아야 한다(하위 도메인, 포트 우회 불가).
///
/// # Errors
/// 조건을 어기면 `DisallowedEndpoint`.
pub(crate) fn validate_endpoint(endpoint: &str) -> Result<(), JudgesError> {
    todo!("#88")
}

/// 리다이렉트를 따를 때 넘길 헤더. `secrets::is_sensitive_header`인 헤더(Authorization 등)를 모두 뺀다.
/// 리다이렉트 대상이 `validate_endpoint`를 통과하지 못하면 호출자는 따르지 않는다.
pub(crate) fn redirect_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    todo!("#88")
}

/// 요청을 나눈다. 전체가 `REQUEST_SPLIT_LIMIT`를 넘거나 `state`와 가장 긴 질문의 합이 `STATE_SPLIT_LIMIT`를 넘으면
/// 질문 세트를 여러 요청에 나눠 담는다. 조각마다 `state`는 그대로 싣는다. 나눌 필요가 없으면 하나를 돌려준다.
pub(crate) fn split_request(request: JudgeRequest) -> Vec<JudgeRequest> {
    todo!("#88")
}

/// 선택지가 `MAX_CHOICES`를 넘는 `choice` 질문을 계층 선택으로 나눈다(묶음 고르기 → 묶음 안에서 고르기).
/// 넘지 않으면 그대로 하나를 돌려준다. TODO(#68): 계층 질문의 묶음 나누기와 2차 질문 방식 미정
pub(crate) fn split_choices(question: &Question) -> Vec<Question> {
    todo!("#88")
}

/// 조각 응답을 질문 id 순서대로 합치고 토큰을 더한다. 계층 선택은 원래 선택지 확률로 되돌린다.
pub(crate) fn merge_responses(request: &JudgeRequest, parts: Vec<JudgeResponse>) -> JudgeResponse {
    todo!("#88")
}
