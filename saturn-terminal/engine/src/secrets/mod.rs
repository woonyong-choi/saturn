//! router 키 보호: 키 입력, 키체인 저장, 자식 환경 변수 제거, 출력 마스킹, PreToolUse 훅 설정.
//! 설계: docs/design/router-key-security.md

mod env;
mod hook;
mod keys;
mod mask;
mod storage;

use std::path::PathBuf;

pub(crate) use env::{scrub, scrub_command};
pub use hook::pre_tool_use_hook_settings;
pub(crate) use hook::{HookPolicy, HookVerdict, ToolCall};
pub(crate) use keys::{
    KeyInfo, KeyInput, KeySource, ROUTER_KEY_ENV, RouterKey, acquire, input_order,
};
pub use mask::Masker;
#[cfg(test)]
pub(crate) use mask::REDACTED;
pub(crate) use mask::{Masked, is_sensitive_header};
pub(crate) use storage::{SecretStore, StorageMode};

/// 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// 호출자는 숨김 입력을 요청한다.
    #[error("router key not found")]
    NotFound,
    /// 호출자는 `SATURN_KEY` 환경 변수와 `router.key.command` 설정 방법을 안내하고 끝낸다.
    #[error("cannot prompt for router key in a non-interactive environment")]
    NonInteractive,
    /// 앞뒤 공백과 끝 줄바꿈을 먼저 지운 뒤 판단한다.
    #[error("router key is empty")]
    Empty,
    #[error("key command failed: {detail}")]
    Command {
        /// 종료 코드만 담는다.
        detail: String,
    },
    /// 원인 객체에 키가 담길 수 있어 오류 종류만 남긴다.
    #[error("keychain operation failed")]
    Keychain,
    /// 강화 방식에서 session 시작 때 키체인 암호를 다시 받아야 한다.
    #[error("router key is locked")]
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
