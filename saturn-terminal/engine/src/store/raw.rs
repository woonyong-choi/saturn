//! 실행별 원시 기록(provider가 보낸 줄 그대로): 실행 중 이어 쓰기, 끝나면 gzip 압축, 읽을 때 자동 해제.
//!
//! 설계: docs/design/records.md(원시 기록 압축). 해시와 크기는 압축 전 값으로 기록해 압축이 대조 값을 바꾸지 않게 한다.
//! TODO(#82): gzip은 `flate2`(작업 공간 의존성 추가 필요)로 구현한다
//! TODO(#82): 값 미정, 초안 해시 SHA-256 hex

use saturn_protocol::ids::RunId;

use super::{Store, StoreError};

/// 원시 기록 대조 값. 항상 압축 전 바이트로 계산한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDigest {
    /// 압축 전 바이트의 해시(hex).
    pub hash: String,
    /// 압축 전 바이트 수.
    pub size: u64,
}

impl Store {
    /// 실행 중 원시 기록 조각을 압축하지 않고 이어 쓴다. provider가 보낸 줄에 judge 키가 있을 수 없으므로 마스킹하지 않는다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`, 이미 압축한(끝난) 실행이면 `Database`.
    pub async fn append_raw(&self, run: RunId, chunk: &[u8]) -> Result<(), StoreError> {
        todo!("#82")
    }

    /// 끝난 실행의 원시 기록을 압축 전 해시와 크기를 먼저 기록한 뒤 gzip으로 바꿔 한 거래로 저장한다. 이미 압축됐으면 그대로 둔다.
    ///
    /// # Errors
    /// 압축 실패면 `Compression`, 없는 실행이면 `NotFound`.
    pub(crate) async fn compress_run(&self, run: RunId) -> Result<RawDigest, StoreError> {
        todo!("#82")
    }

    /// 원시 기록 전체. 압축된 것은 풀어서 돌려주고, 푼 결과를 기록된 `RawDigest`와 대조한다.
    ///
    /// # Errors
    /// 해제 실패면 `Compression`, 대조가 다르면 `DigestMismatch`, 없는 실행이면 `NotFound`.
    pub async fn read_raw(&self, run: RunId) -> Result<Vec<u8>, StoreError> {
        todo!("#82")
    }

    /// 기록된 압축 전 대조 값. 실행 중이면 지금까지 쓴 바이트로 계산한다.
    ///
    /// # Errors
    /// 없는 실행이면 `NotFound`.
    pub async fn raw_digest(&self, run: RunId) -> Result<RawDigest, StoreError> {
        todo!("#82")
    }
}
