//! engine이 발급하고 TUI는 그대로 돌려주는 식별자.

use std::sync::{Mutex, PoisonError};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 여러 provider session이 한 채팅에 이어진다.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct ChatId(pub u64);

/// 접수 때 발급한다.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct InputId(pub u64);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct TaskId(pub u64);

/// 끝난 작업의 글자는 비어 있는 가장 앞 글자로 재사용한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct TaskLabel(pub char);

/// Saturn이 띄운 메인 또는 보조 에이전트. provider subagent는 `SubagentId`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct AgentId(pub u64);

/// provider가 준 id 문자열 그대로.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct SubagentId(pub String);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct SessionId(pub u64);

/// Codex `thread_id`, Claude session id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, TS)]
pub struct ProviderSessionId(pub String);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct RunId(pub u64);

/// 채팅 트리 전체 기록의 연속 번호.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
    TS,
)]
pub struct LedgerSeq(pub u64);

/// 입력은 접수 때 고정한 번호로 끝까지 처리한다.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct SettingsRevision(pub u64);

/// router 결과를 적용하기 직전에 비교한다(CAS).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct ChatRevision(pub u64);

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct JudgmentId(pub u64);

/// provider는 닫힌 목록이 아니라 열린 id 글자로 식별한다. id는 소문자 영문, 숫자, `-`만 쓴다(1~32자).
/// 공통 코드는 id를 불투명한 글자로 저장하고 전달하고 같은지 비교만 한다.
///
/// 값은 프로세스 안에서 한 번만 만들어 두고 계속 쓰는 글자라 `Copy`다. 서로 다른 id는 `MAX_PROVIDER_IDS`개까지만
/// 받는다. 글자를 영구히 잡아 두므로 엉뚱한 입력이 메모리를 늘리지 못하게 하기 위해서다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, JsonSchema, TS)]
#[schemars(with = "String")]
#[ts(as = "String")]
pub struct Provider(&'static str);

/// 서로 다른 provider id를 한 프로세스에서 받는 최대 개수.
pub const MAX_PROVIDER_IDS: usize = 64;

/// provider id를 만들지 못한 이유.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderIdError {
    #[error("provider id should be 1 to 32 characters of a-z, 0-9 and '-'")]
    Malformed,
    #[error("too many distinct provider ids")]
    TooMany,
}

const fn is_well_formed(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
            return false;
        }
        index += 1;
    }
    true
}

impl Provider {
    /// 코드에 적은 id 글자로 상수를 만든다. 형식이 틀리면 상수를 계산하는 컴파일 때 실패한다.
    /// 실행 중에 받은 글자에는 `parse`를 쓴다.
    pub const fn from_static(id: &'static str) -> Self {
        assert!(is_well_formed(id), "provider id should be well formed");
        Self(id)
    }

    /// 형식이 맞는 id 글자를 받는다.
    ///
    /// # Errors
    /// 형식이 틀리면 `Malformed`, 서로 다른 id가 `MAX_PROVIDER_IDS`개를 넘으면 `TooMany`.
    pub fn parse(text: &str) -> Result<Self, ProviderIdError> {
        static KNOWN: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
        if !is_well_formed(text) {
            return Err(ProviderIdError::Malformed);
        }
        let mut known = KNOWN.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(id) = known.iter().find(|id| **id == text) {
            return Ok(Self(id));
        }
        if known.len() >= MAX_PROVIDER_IDS {
            return Err(ProviderIdError::TooMany);
        }
        let id: &'static str = Box::leak(text.to_owned().into_boxed_str());
        known.push(id);
        Ok(Self(id))
    }

    /// 기록 저장소와 JSON에 남은 값을 읽는다. 옛 값 `Codex`, `Claude`처럼 대소문자가 섞여 있어도 같은 id로 읽는다.
    ///
    /// # Errors
    /// `parse`와 같다.
    pub fn from_stored(text: &str) -> Result<Self, ProviderIdError> {
        Self::parse(&text.to_ascii_lowercase())
    }

    /// 형식이 맞는 id 글자인지. 값은 만들지 않는다.
    pub const fn is_well_formed(text: &str) -> bool {
        is_well_formed(text)
    }

    pub fn as_str(self) -> &'static str {
        self.0
    }
}

impl Serialize for Provider {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Provider {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<str>::deserialize(deserializer)?;
        Self::from_stored(&text).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Provider({})", self.0)
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_one_to_thirty_two_lowercase_characters() {
        assert!(Provider::parse("codex").is_ok());
        assert!(Provider::parse("my-agent-2").is_ok());
        for bad in ["", "Codex", "a/b", "a b", "a_b", &"x".repeat(33)] {
            assert_eq!(
                Provider::parse(bad),
                Err(ProviderIdError::Malformed),
                "{bad}"
            );
        }
    }

    #[test]
    fn same_text_gives_equal_ids() {
        assert_eq!(Provider::parse("codex"), Ok(Provider::from_static("codex")));
        assert_ne!(
            Provider::from_static("codex"),
            Provider::from_static("claude")
        );
    }

    #[test]
    fn old_stored_values_read_as_the_same_id() {
        let old: Provider = serde_json::from_str("\"Codex\"").unwrap();
        let new: Provider = serde_json::from_str("\"codex\"").unwrap();

        assert_eq!(old, new);
        assert_eq!(serde_json::to_string(&old).unwrap(), "\"codex\"");
        assert!(serde_json::from_str::<Provider>("\"a/b\"").is_err());
    }
}
