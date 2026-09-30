//! judge 키 보호: 키 입력, 키체인 저장, 자식 환경 변수 제거, 출력 마스킹, Saturn 소유 PreToolUse 훅 설정.
//!
//! 설계: docs/design/judge-key-security.md, docs/architecture.md(불변 조건).
//! 규칙:
//! - 키는 숨김 입력, 표준 입력, 환경 변수, 비밀번호 관리자 명령 네 방법으로만 받는다. 명령 인자로 받지 않는다.
//! - 키는 SQLite 기록 저장소, 로그, 영수증, 설정 파일에 저장하지 않는다. 설정에는 출처와 끝 4자리(`KeyInfo`)만.
//! - judge는 engine만 부른다. Authorization 헤더는 어디에도 기록하지 않는다.
//! - 키와 일치하는 문자열은 로그, 오류, 디버그 출력, 판단 기록 저장 전에 가린다.
//!
//! 흐름: judge 시작 확인 실패 → `keys::acquire`로 키 받기 → `judges`가 `GET /v1/models`로 확인하고 모델 버전 고정
//! → 성공하면 `SecretStore::save`(관리자 명령 키는 저장하지 않음) → `settings`가 `KeyInfo`를 사용자 설정에 기록.
//! 자식 실행: engine → `Supervisor` 넘길 때 `env::scrub`, `Supervisor` → provider 넘길 때 다시 `env::scrub`.

mod env;
mod hook;
mod keys;
mod mask;
mod storage;

use std::path::PathBuf;

pub use env::{CHILD_ENV_DENYLIST, is_denied, scrub, scrub_command};
pub use hook::{HookPolicy, HookVerdict, ToolCall};
pub use keys::{JUDGE_KEY_ENV, JudgeKey, KeyInfo, KeyInput, KeySource, acquire, can_prompt};
pub use mask::{Masked, Masker, MaskingWriter, is_sensitive_header};
pub use storage::{HARDENED_IDLE_LOCK, HARDENED_MAX_UNLOCK, SecretStore, StorageMode};

/// 키 입력, 저장, 조회 오류. 메시지와 원인 어디에도 키 문자열을 넣지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// 저장된 키도, 환경 변수도, 관리자 명령 설정도 없다. 호출자는 숨김 입력을 요청한다.
    #[error("judge key not found")]
    NotFound,
    /// 입력할 수 없는 환경(파이프, CI)이라 묻지 못한다. 호출자는 환경 변수와 표준 입력 방식을 안내하고 끝낸다.
    #[error("cannot prompt for judge key in a non-interactive environment")]
    NonInteractive,
    /// 받은 값이 비었거나 공백뿐이다(앞뒤 공백과 끝 줄바꿈은 먼저 지운다).
    #[error("judge key is empty")]
    Empty,
    /// 비밀번호 관리자 명령을 실행하지 못했거나 0이 아닌 코드로 끝났다. stderr는 가린 뒤 첫 줄만 남긴다.
    #[error("key command failed: {detail}")]
    Command {
        /// 종료 코드와 가린 stderr 첫 줄.
        detail: String,
    },
    /// OS 키체인 호출 실패. TODO(#84): 원인을 `keyring::Error`로 바꾼다
    #[error("keychain operation failed")]
    Keychain(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// 강화 방식에서 잠금 상태다. session 시작 때 키체인 암호를 다시 받아야 한다.
    #[error("judge key is locked")]
    Locked,
    /// 대체 파일의 권한이 0600이 아니다. 읽지 않고 멈춘다.
    #[error("key file permission must be 0600: {path}")]
    FilePermission {
        /// 대체 파일 경로.
        path: PathBuf,
    },
    /// 대체 파일이나 표준 입력 I/O 실패.
    #[error("key io failed")]
    Io(#[from] std::io::Error),
}
