//! 판단 기록: judge 호출마다 보낸 원문, 받은 원문, 질문별 답, 비용, 시간, 질문 버전, 설정 번호. 전용 정리와 JSONL 내보내기.
//!
//! 설계: docs/design/records.md(판단 기록, 판단 기록 정리와 내보내기), docs/design/judge.md(판단 기록).
//! 판단 방식(`Method`)과 관계없이 전부 남긴다. 원문은 `secrets::Masker`가 가린 `Masked`로만 받는다.
//! `/record off` 채팅이면 저장하지 않는다. 일반 정리(`prune`)는 판단 기록을 지우지 않는다.

use std::path::Path;
use std::time::{Duration, SystemTime};

use saturn_core::judges::{Answer, Method, QuestionSetId};
use saturn_protocol::ids::{ChatId, InputId, JudgmentId, SettingsRevision};

use super::{Store, StoreError};
use crate::secrets::Masked;

/// 판단 호출 결과 분류. core가 정본이다.
pub use saturn_core::judges::JudgmentOutcome;

/// 저장할 판단 한 건.
#[derive(Debug, Clone)]
pub struct NewJudgment {
    /// 채팅. `/record off`인지 이것으로 본다.
    pub chat: ChatId,
    /// 판단 대상 입력. 입력과 무관한 호출(`compact`, `loop` 등)은 `None`.
    pub input: Option<InputId>,
    /// 판단 방식.
    pub method: Method,
    /// judge 이름과 출처(중립 이름). 예: `jev`, `saturn-local`.
    pub judge: String,
    /// 요청 모델과 응답이 보고한 실제 모델.
    pub model: (String, Option<String>),
    /// 물은 질문 세트와 버전(`route@3.1`). 버전이 바뀐 뒤 옛 기록을 다시 해석하는 데 쓴다.
    pub question_sets: Vec<QuestionSetId>,
    /// 판단 때 쓴 설정 번호.
    pub settings: SettingsRevision,
    /// 보낸 원문(요청 본문). 비밀값을 가린 뒤.
    pub sent: Masked,
    /// 받은 원문(응답 본문). 없으면 `None`. 비밀값을 가린 뒤.
    pub received: Option<Masked>,
    /// 질문 id별 답. 답이 없으면 빈 목록.
    pub answers: Vec<(String, Answer)>,
    /// 대체 규칙을 적용한 질문 id와 사유.
    pub fallbacks: Vec<(String, String)>,
    /// 입력 토큰, 출력 토큰. 보고되지 않았거나 `CostUnknown`이면 `None`(NULL).
    pub tokens: Option<(u64, u64)>,
    /// 요청 시작 시각.
    pub started_at: SystemTime,
    /// 걸린 시간. 응답이 없으면 포기할 때까지의 시간.
    pub elapsed: Duration,
    /// 결과 분류.
    pub outcome: JudgmentOutcome,
    /// 판단 때 judge 버전(모델, 보정값, 질문별 목표 틀림 비율 묶음). 기준값 조정 계산을 다시 하는 데 쓴다.
    pub judge_version: String,
    /// 질문 id별 그때 기준값.
    pub thresholds: Vec<(String, f64)>,
    /// 피드백 질문을 한 확률 q. 계산 때 1/q로 가중한다.
    pub asked_with: Option<f64>,
}

/// 판단 기록 전용 정리 요청. 일반 정리와 같이 `yes`가 거짓이면 미리보기만 한다.
#[derive(Debug, Clone)]
pub struct JudgmentPruneRequest {
    /// 이 시각보다 이른 기록만. `None`이면 전부.
    pub before: Option<SystemTime>,
    /// 이 채팅만. `None`이면 모든 채팅.
    pub chat: Option<ChatId>,
    /// 참이면 지우고, 거짓이면 대상만 센다.
    pub yes: bool,
}

impl Store {
    /// 판단 기록 한 건을 쓴다. 채팅이 `/record off`면 쓰지 않고 `None`을 돌려준다. 보고되지 않은 값은 NULL.
    ///
    /// # Errors
    /// 쓰기 실패면 `Database`.
    pub async fn record_judgment(
        &self,
        judgment: &NewJudgment,
    ) -> Result<Option<JudgmentId>, StoreError> {
        todo!("#82")
    }

    /// 판단 기록 전용 정리. 일반 정리와 같은 삭제 절차(`secure_delete`, `wal_checkpoint(TRUNCATE)`, `VACUUM`)를 따른다.
    /// 지운 건수를 돌려준다. `yes`가 거짓이면 지우지 않고 지울 건수만 돌려준다. 삭제 흔적은 남기지 않는다(채팅 삭제가 아니다).
    ///
    /// # Errors
    /// 삭제나 정리 실패면 `Database`.
    pub async fn prune_judgments(&self, request: &JudgmentPruneRequest) -> Result<u64, StoreError> {
        todo!("#82")
    }

    /// 판단 기록을 `path`에 JSONL로 쓴다. 한 줄에 한 건, 채점하지 않은 기록도 모두 포함한다. 쓴 건수를 돌려준다.
    /// 필드: id, chat, input, method, judge, model, question_sets(`name@major.minor`), settings, sent, received, answers,
    /// fallbacks, tokens(없으면 null), started_at(unix 밀리초), elapsed_ms, outcome. 파일은 새로 만들고 권한 0600.
    ///
    /// # Errors
    /// 파일 쓰기 실패면 `Export`, 직렬화 실패면 `Json`.
    pub async fn export_judgments(&self, path: &Path) -> Result<u64, StoreError> {
        todo!("#82")
    }
}
