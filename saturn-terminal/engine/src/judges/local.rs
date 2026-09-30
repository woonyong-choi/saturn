//! 로컬 Saturn 모델 연결(`saturn` 방식).
//!
//! 설계: docs/design/judge.md(판단 방식, judge 시작 확인), docs/design/judge-training.md(judge 버전).
//! 시작 확인은 모델 로드나 로컬 API 서버 응답으로 한다. 키를 쓰지 않는다.
//! `saturn` 방식에서 확신도가 기준보다 낮으면 행동하지 않는 규칙은 `saturn_core::judges::decide_route`가 맡는다.
//! TODO(#43): 모델 형식(공개 체크포인트 이어 학습, 항목별 yes/no 확률)과 실행 방식이 정해지면 `LocalSource`를 확정한다

use std::path::PathBuf;

use saturn_core::judges::{JudgeClient, JudgeError, JudgeRequest, JudgeResponse};

use super::JudgeExchange;

/// 로컬 모델을 어디서 부르는지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalSource {
    /// 승격된 모델 파일. engine이 불러 쓴다. 경로는 judge 버전의 모델.
    Model {
        /// 모델 경로.
        path: PathBuf,
    },
    /// 이미 떠 있는 로컬 API 서버. TODO(#88): 허용 주소(루프백만 등)와 설정 키 미정
    Server {
        /// 서버 주소.
        endpoint: String,
    },
}

/// 로컬 Saturn 모델 연결. `JudgeClient`로 engine에만 붙는다.
#[derive(Debug)]
pub struct LocalJudge {
    /// 모델 위치.
    source: LocalSource,
    /// 지금 쓰는 judge 버전 이름. 판단 기록에 남긴다.
    version: String,
}

impl LocalJudge {
    /// 연결을 만든다. 아직 모델을 불러오지 않는다.
    pub fn new(source: LocalSource, version: String) -> Self {
        Self { source, version }
    }

    /// 지금 쓰는 judge 버전.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// 요청과 응답 원문을 함께 돌려주는 판단 호출. 원문은 모델에 넣은 본문과 모델 출력이다.
    pub async fn exchange(&self, request: JudgeRequest) -> JudgeExchange {
        todo!("#88")
    }
}

impl JudgeClient for LocalJudge {
    /// `Model`이면 모델을 불러오고, `Server`면 서버 응답을 확인한다. 둘 다 실제 판단 1건을 돌려 본다.
    async fn check(&self) -> Result<(), JudgeError> {
        todo!("#88")
    }

    /// `exchange`의 결과만 돌려준다.
    async fn judge(&self, request: JudgeRequest) -> Result<JudgeResponse, JudgeError> {
        todo!("#88")
    }
}
