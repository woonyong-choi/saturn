//! 출력 마스킹: judge 키와 일치하는 문자열을 로그, 오류, 디버그 출력, 판단 기록 저장 전에 가린다. Authorization 헤더는 기록하지 않는다.
//!
//! 설계: docs/design/judge-key-security.md(출력 마스킹, 키를 저장하지 않는 곳).
//! 가린 자리는 치환 문자열로 바꾼다(끝 4자리도 남기지 않는다). 치환 문자열과 헤더 목록은 초안이다(설계에 없음).
//! TODO(#84): 값 미정, 초안 치환 문자열 `[redacted]`, 헤더 `authorization`, `proxy-authorization`, `x-api-key`

use std::io::Write;

/// 가린 뒤의 문자열. `Masker::mask`로만 만든다. `store::NewJudgment`는 원문을 이 타입으로만 받는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked(String);

impl Masked {
    /// 가린 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn assume_masked(text: &str) -> Self {
        Self(text.to_owned())
    }
}

/// 가림 대상 목록. `SecretStore::mask_needles`로 만들고, 키가 바뀌면 새로 만든다. 대상이 키 원문이라 `Debug`는 개수만 보인다.
#[derive(Clone, Default)]
pub struct Masker {
    needles: Vec<String>,
}

impl Masker {
    /// 대상 목록으로 만든다. 빈 문자열은 넣지 않는다.
    pub fn new(needles: Vec<String>) -> Self {
        todo!("#84")
    }

    /// `text`의 모든 대상 문자열을 치환 문자열(초안 `[redacted]`)로 바꾼다. 긴 대상부터 바꿔 겹친 대상이 남지 않게 한다.
    /// `Authorization: ...`, `x-api-key: ...` 헤더 줄은 값 전체를 가린다.
    pub fn mask(&self, text: &str) -> Masked {
        todo!("#84")
    }
}

impl std::fmt::Debug for Masker {
    /// `Masker { needles: 1 }`처럼 개수만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!("#84")
    }
}

/// 로그 출력 감싸개. 줄바꿈까지 모았다가 `Masker::mask`를 거쳐 안쪽에 쓴다(키가 두 번의 쓰기로 나뉘어도 가린다).
/// `tracing` 구독자의 writer와 오류 출력(stderr)에 쓴다.
#[derive(Debug)]
pub struct MaskingWriter<W: Write> {
    inner: W,
    masker: Masker,
    line: Vec<u8>,
}

impl<W: Write> MaskingWriter<W> {
    /// `inner`를 감싼다.
    pub fn new(inner: W, masker: Masker) -> Self {
        Self {
            inner,
            masker,
            line: Vec::new(),
        }
    }
}

impl<W: Write> Write for MaskingWriter<W> {
    /// 줄바꿈이 나올 때마다 그 줄을 가려서 쓴다. 남은 조각은 버퍼에 둔다.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        todo!("#84")
    }

    /// 버퍼에 남은 조각도 가려서 쓰고 안쪽을 비운다.
    fn flush(&mut self) -> std::io::Result<()> {
        todo!("#84")
    }
}

/// 기록하면 안 되는 HTTP 헤더인지. 설계는 Authorization만 정한다. 초안 목록 `authorization`, `proxy-authorization`, `x-api-key`(대소문자 무시)면 참.
/// judge 요청과 응답을 기록할 때 이 헤더는 이름만 남기고 값을 버린다.
pub fn is_sensitive_header(name: &str) -> bool {
    todo!("#84")
}
