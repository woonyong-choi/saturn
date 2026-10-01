//! judge 키 보호: 키 입력, 키체인 저장, 자식 환경 변수 제거, 출력 마스킹, PreToolUse 훅 설정.
//! 설계: docs/design/judge-key-security.md

mod env;
mod hook;
mod keys;
mod mask;
mod storage;

use std::path::PathBuf;

pub use env::{CHILD_ENV_DENYLIST, is_denied, scrub, scrub_command};
pub use hook::{HookPolicy, HookVerdict, ToolCall};
pub use keys::{
    JUDGE_KEY_ENV, JudgeKey, KeyInfo, KeyInput, KeySource, acquire, can_prompt, input_order,
};
pub use mask::{Masked, Masker, MaskingWriter, REDACTED, is_sensitive_header};
pub use storage::{
    HARDENED_IDLE_LOCK, HARDENED_MAX_UNLOCK, LOCK_CHECK_INTERVAL, SecretStore, StorageMode,
};

/// 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// 호출자는 숨김 입력을 요청한다.
    #[error("judge key not found")]
    NotFound,
    /// 호출자는 환경 변수와 표준 입력 방식을 안내하고 끝낸다.
    #[error("cannot prompt for judge key in a non-interactive environment")]
    NonInteractive,
    /// 앞뒤 공백과 끝 줄바꿈을 먼저 지운 뒤 판단한다.
    #[error("judge key is empty")]
    Empty,
    #[error("key command failed: {detail}")]
    Command {
        /// 종료 코드와 가린 stderr 첫 줄.
        detail: String,
    },
    /// `BadEncoding`은 `Debug`로 키가 새지 않게 저장된 바이트를 버린 문장으로 바꾼다.
    #[error("keychain operation failed")]
    Keychain(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// 강화 방식에서 session 시작 때 키체인 암호를 다시 받아야 한다.
    #[error("judge key is locked")]
    Locked,
    /// 읽지 않고 멈춘다.
    #[error("key file permission must be 0600: {path}")]
    FilePermission {
        /// 대체 파일 경로.
        path: PathBuf,
    },
    #[error("key io failed")]
    Io(#[from] std::io::Error),
}
