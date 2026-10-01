//! 출력 마스킹: judge 키와 일치하는 문자열을 로그, 오류, 디버그 출력, 판단 기록 저장 전에 가린다. Authorization 헤더는 기록하지 않는다.
//!
//! 설계: docs/design/judge-key-security.md(출력 마스킹, 키를 저장하지 않는 곳).
//! 가린 자리는 치환 문자열로 바꾼다(끝 4자리도 남기지 않는다). 치환 문자열 `[redacted]`와 헤더 목록
//! `authorization`, `proxy-authorization`, `x-api-key`는 초안이다(설계에 없음).

use std::io::Write;

/// 가린 자리에 넣는 문자열.
pub const REDACTED: &str = "[redacted]";

/// 기록하면 안 되는 헤더 이름(소문자).
const SENSITIVE_HEADERS: &[&str] = &["authorization", "proxy-authorization", "x-api-key"];

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
        let mut needles: Vec<String> = needles
            .into_iter()
            .filter(|needle| !needle.is_empty())
            .collect();
        needles.sort_by_key(|needle| std::cmp::Reverse(needle.len()));
        needles.dedup();
        Self { needles }
    }

    /// `text`의 모든 대상 문자열을 치환 문자열(초안 `[redacted]`)로 바꾼다. 긴 대상부터 바꿔 겹친 대상이 남지 않게 한다.
    /// `Authorization: ...`, `x-api-key: ...` 헤더 줄은 값 전체를 가린다.
    pub fn mask(&self, text: &str) -> Masked {
        let mut masked = text.to_owned();
        for needle in &self.needles {
            if masked.contains(needle.as_str()) {
                masked = masked.replace(needle.as_str(), REDACTED);
            }
        }
        if SENSITIVE_HEADERS
            .iter()
            .any(|header| masked.to_ascii_lowercase().contains(header))
        {
            masked = masked.split_inclusive('\n').map(mask_header_line).collect();
        }
        Masked(masked)
    }
}

impl std::fmt::Debug for Masker {
    /// `Masker { needles: 1 }`처럼 개수만 쓴다.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Masker")
            .field("needles", &self.needles.len())
            .finish()
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
        self.line.extend_from_slice(buf);
        while let Some(end) = self.line.iter().position(|byte| *byte == b'\n') {
            let rest = self.line.split_off(end + 1);
            let line = std::mem::replace(&mut self.line, rest);
            self.write_masked(&line)?;
        }
        Ok(buf.len())
    }

    /// 버퍼에 남은 조각도 가려서 쓰고 안쪽을 비운다.
    fn flush(&mut self) -> std::io::Result<()> {
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.write_masked(&line)?;
        }
        self.inner.flush()
    }
}

impl<W: Write> MaskingWriter<W> {
    /// 한 줄을 가려서 안쪽에 쓴다. UTF-8이 아니면 깨진 바이트를 바꾼 뒤 가린다(키는 ASCII라 가림은 그대로 맞는다).
    fn write_masked(&mut self, line: &[u8]) -> std::io::Result<()> {
        let text = String::from_utf8_lossy(line);
        self.inner
            .write_all(self.masker.mask(&text).as_str().as_bytes())
    }
}

/// `Authorization: ...` 같은 헤더 줄이면 이름만 남기고 값을 가린다. 줄 끝 줄바꿈은 그대로 둔다.
fn mask_header_line(line: &str) -> String {
    let Some((name, _)) = line.split_once(':') else {
        return line.to_owned();
    };
    if !is_sensitive_header(name.trim()) {
        return line.to_owned();
    }
    let ending = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    format!("{name}: {REDACTED}{ending}")
}

/// 기록하면 안 되는 HTTP 헤더인지. 설계는 Authorization만 정한다. 초안 목록 `authorization`, `proxy-authorization`, `x-api-key`(대소문자 무시)면 참.
/// judge 요청과 응답을 기록할 때 이 헤더는 이름만 남기고 값을 버린다.
pub fn is_sensitive_header(name: &str) -> bool {
    SENSITIVE_HEADERS
        .iter()
        .any(|header| header.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-test-0123456789abcdef";

    #[test]
    fn mask_replaces_every_key_occurrence() {
        let masker = Masker::new(vec![KEY.to_owned(), String::new()]);

        let masked = masker.mask(&format!("error: bad key {KEY} (again {KEY})"));

        assert_eq!(
            masked.as_str(),
            "error: bad key [redacted] (again [redacted])"
        );
        assert!(!masked.as_str().contains("cdef"));
    }

    #[test]
    fn longer_needles_are_masked_first() {
        let masker = Masker::new(vec!["abc".to_owned(), "abcdef".to_owned()]);

        assert_eq!(
            masker.mask("x abcdef y abc").as_str(),
            "x [redacted] y [redacted]"
        );
    }

    #[test]
    fn sensitive_header_values_are_hidden() {
        let masker = Masker::new(Vec::new());

        let masked = masker
            .mask("POST /v1\r\nAuthorization: Bearer abc\r\nX-Api-Key: zzz\r\nAccept: json\r\n");

        assert_eq!(
            masked.as_str(),
            "POST /v1\r\nAuthorization: [redacted]\r\nX-Api-Key: [redacted]\r\nAccept: json\r\n"
        );
        assert!(is_sensitive_header("PROXY-AUTHORIZATION"));
        assert!(!is_sensitive_header("content-type"));
    }

    #[test]
    fn debug_shows_only_needle_count() {
        let masker = Masker::new(vec![KEY.to_owned()]);

        let debug = format!("{masker:?}");

        assert_eq!(debug, "Masker { needles: 1 }");
    }

    #[test]
    fn writer_masks_key_split_across_writes() {
        let masker = Masker::new(vec![KEY.to_owned()]);
        let mut writer = MaskingWriter::new(Vec::new(), masker);

        writer
            .write_all(format!("first {}", &KEY[..6]).as_bytes())
            .unwrap();
        writer
            .write_all(format!("{} end\nsecond {KEY}", &KEY[6..]).as_bytes())
            .unwrap();
        writer.flush().unwrap();

        let out = String::from_utf8(writer.inner).unwrap();
        assert_eq!(out, "first [redacted] end\nsecond [redacted]");
    }
}
