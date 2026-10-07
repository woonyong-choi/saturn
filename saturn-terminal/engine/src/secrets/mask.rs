//! 출력 마스킹: router 키와 일치하는 문자열을 로그, 오류, 디버그 출력, 판단 기록 저장 전에 가린다.
//! 설계: docs/design/router-key-security.md

#[cfg(test)]
use std::io::Write;

use crate::passes::TOKEN_PREFIX;

/// 끝 4자리도 남기지 않는다. 초안 값.
pub(crate) const REDACTED: &str = "[redacted]";

/// 초안 목록(설계는 Authorization만 정함).
const SENSITIVE_HEADERS: &[&str] = &["authorization", "proxy-authorization", "x-api-key"];

/// `Masker::mask`로만 만든다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked(String);

impl Masked {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn assume_masked(text: &str) -> Self {
        Self(text.to_owned())
    }
}

/// 하위 접속 출입증은 키가 아니지만 채팅 범위의 권한이라 로그와 오류에 글자를 남기지 않는다. 발급한 토큰마다 대상을 늘리지
/// 않고, 토큰의 모양(앞부분과 16진수)으로 가린다. 회수된 토큰도 가린다.
fn mask_passes(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(TOKEN_PREFIX) {
        masked.push_str(&rest[..at]);
        let after = &rest[at + TOKEN_PREFIX.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_hexdigit).count();
        masked.push_str(REDACTED);
        rest = &after[digits..];
    }
    masked.push_str(rest);
    masked
}

/// 대상이 키 원문이라 `Debug`는 개수만 보인다.
#[derive(Clone, Default)]
pub struct Masker {
    needles: Vec<String>,
}

impl Masker {
    /// 빈 문자열은 넣지 않는다.
    pub fn new(needles: Vec<String>) -> Self {
        let mut needles: Vec<String> = needles
            .into_iter()
            .filter(|needle| !needle.is_empty())
            .collect();
        needles.sort_by_key(|needle| std::cmp::Reverse(needle.len()));
        needles.dedup();
        Self { needles }
    }

    /// 긴 대상부터 바꿔 겹친 대상이 남지 않게 한다.
    pub fn mask(&self, text: &str) -> Masked {
        let mut masked = text.to_owned();
        for needle in &self.needles {
            if masked.contains(needle.as_str()) {
                masked = masked.replace(needle.as_str(), REDACTED);
            }
        }
        if masked.contains(TOKEN_PREFIX) {
            masked = mask_passes(&masked);
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
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Masker")
            .field("needles", &self.needles.len())
            .finish()
    }
}

/// 키가 두 번의 쓰기로 나뉘어도 가리도록 줄바꿈까지 모았다가 쓴다.
#[cfg(test)]
pub(crate) struct MaskingWriter<W: Write> {
    inner: W,
    masker: Masker,
    line: Vec<u8>,
}

#[cfg(test)]
impl<W: Write> MaskingWriter<W> {
    pub(crate) fn new(inner: W, masker: Masker) -> Self {
        Self {
            inner,
            masker,
            line: Vec::new(),
        }
    }
}

#[cfg(test)]
impl<W: Write> Write for MaskingWriter<W> {
    /// 남은 조각은 버퍼에 둔다.
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.line.extend_from_slice(buf);
        while let Some(end) = self.line.iter().position(|byte| *byte == b'\n') {
            let rest = self.line.split_off(end + 1);
            let line = std::mem::replace(&mut self.line, rest);
            self.write_masked(&line)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
impl<W: Write> std::fmt::Debug for MaskingWriter<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MaskingWriter")
            .field("buffered_bytes", &self.line.len())
            .finish()
    }
}

#[cfg(test)]
impl<W: Write> MaskingWriter<W> {
    /// 키는 ASCII라 깨진 UTF-8 바이트를 바꾼 뒤 가려도 맞는다.
    fn write_masked(&mut self, line: &[u8]) -> std::io::Result<()> {
        let text = String::from_utf8_lossy(line);
        self.inner
            .write_all(self.masker.mask(&text).as_str().as_bytes())
    }
}

/// 줄 끝 줄바꿈은 그대로 둔다.
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

/// 대소문자를 무시한다.
pub(crate) fn is_sensitive_header(name: &str) -> bool {
    SENSITIVE_HEADERS
        .iter()
        .any(|header| header.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-test-0123456789abcdef";

    #[test]
    fn pass_tokens_are_masked_by_shape() {
        let masker = Masker::new(Vec::new());
        let token = format!("{TOKEN_PREFIX}{}", "ab01".repeat(16));

        let masked = masker.mask(&format!("env SATURN_PASS={token} and {token}."));

        assert_eq!(
            masked.as_str(),
            "env SATURN_PASS=[redacted] and [redacted]."
        );
    }

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
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();

        let out = String::from_utf8(writer.inner).unwrap();
        assert_eq!(out, "first [redacted] end\nsecond [redacted]\n");
    }

    #[test]
    fn flush_and_debug_do_not_expose_partial_key() {
        let mut writer = MaskingWriter::new(Vec::new(), Masker::new(vec![KEY.to_owned()]));
        writer.write_all(&KEY.as_bytes()[..8]).unwrap();
        writer.flush().unwrap();

        assert!(writer.inner.is_empty());
        assert!(!format!("{writer:?}").contains(&KEY[..8]));

        writer.write_all(&KEY.as_bytes()[8..]).unwrap();
        writer.write_all(b"\n").unwrap();
        assert_eq!(writer.inner, b"[redacted]\n");
    }
}
