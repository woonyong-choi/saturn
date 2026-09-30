//! 키 저장: macOS 키체인 OS API 직접 저장, OS 저장소가 없으면 0600 파일, 강화 방식의 잠금.
//!
//! 설계: docs/design/judge-key-security.md(키 저장, `security` 명령을 쓰지 않는 이유).
//! `security` 명령은 쓰지 않는다. 그 명령으로 저장하면 명령이 신뢰 앱이 되어 누구나 확인 창 없이 읽는다.
//! TODO(#84): 키체인은 `keyring` crate(작업 공간 의존성 추가 필요)로 OS API를 직접 부른다
//! TODO(#32): 키체인 서비스 이름과 계정 이름

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{JudgeKey, KeySource, SecretsError};

/// 강화 방식: 마지막 사용 뒤 이만큼 쓰지 않으면 잠근다.
pub const HARDENED_IDLE_LOCK: Duration = Duration::from_secs(10 * 60);

/// 강화 방식: 풀린 뒤 사용과 관계없이 이만큼 지나면 잠근다.
pub const HARDENED_MAX_UNLOCK: Duration = Duration::from_secs(12 * 60 * 60);

/// 저장 방식. 사용자 층 설정에서 고른다. TODO(#49): 설정 키 이름과 기본값
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StorageMode {
    /// 기본. 키체인에 OS API로 저장한다.
    #[default]
    Standard,
    /// 신뢰 앱 없는 키체인 항목으로 저장한다. session 시작 때 키체인 암호를 한 번 받고,
    /// `HARDENED_IDLE_LOCK`이나 `HARDENED_MAX_UNLOCK`이 지나면 잠근다.
    Hardened,
}

/// 실제 저장소.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Backend {
    /// macOS 키체인.
    Keychain,
    /// 키체인을 쓸 수 없을 때 권한 0600 파일. 경로는 초안이다(설계에 없음).
    /// TODO(#84): 값 미정, 초안 `~/.saturn/judge.key`
    File(PathBuf),
}

/// judge 키 보관소. engine에 하나. 키를 메모리에 들고 있는 곳은 여기뿐이다.
#[derive(Debug)]
pub struct SecretStore {
    backend: Backend,
    mode: StorageMode,
    /// 이번 실행에 쓰는 키. 관리자 명령과 환경 변수 키도 여기에만 둔다.
    current: Option<(JudgeKey, KeySource)>,
    /// 강화 방식에서 풀린 시각과 마지막 사용 시각.
    unlocked: Option<(Instant, Instant)>,
}

impl SecretStore {
    /// 키체인을 쓸 수 있으면 `Keychain`, 없으면 파일 백엔드(초안 경로 `home/judge.key`)로 연다. 키를 읽지는 않는다.
    pub fn open(home: &Path, mode: StorageMode) -> Self {
        todo!("#84")
    }

    /// 저장된 키를 읽어 `current`에 둔다. 환경 변수가 있으면 그것을 먼저 쓴다(`KeySource::Env`).
    /// 파일 백엔드는 권한이 0600이 아니면 읽지 않는다. 강화 방식이면 잠금 상태에서 `Locked`.
    ///
    /// # Errors
    /// 없으면 `NotFound`, 권한이 틀리면 `FilePermission`, 키체인 실패면 `Keychain`, 잠겼으면 `Locked`.
    pub async fn load(&mut self) -> Result<&JudgeKey, SecretsError> {
        todo!("#84")
    }

    /// 확인을 통과한 키를 보관한다. `Stored`와 `Stdin` 출처만 백엔드에 쓰고, `Env`와 `Command`는 `current`에만 둔다.
    /// 파일 백엔드는 임시 파일을 0600으로 만든 뒤 이름을 바꿔 쓴다. 강화 방식은 신뢰 앱 목록이 빈 항목으로 쓴다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 실패면 `Io`.
    pub async fn save(&mut self, key: JudgeKey, source: KeySource) -> Result<(), SecretsError> {
        todo!("#84")
    }

    /// 저장된 키와 `current`를 지운다. 키가 거부됐거나 사용자가 키를 바꿀 때 쓴다.
    ///
    /// # Errors
    /// 키체인 실패면 `Keychain`, 파일 삭제 실패면 `Io`.
    pub async fn forget(&mut self) -> Result<(), SecretsError> {
        todo!("#84")
    }

    /// 이번 실행의 키. judge 호출 직전에 부르고, 강화 방식이면 잠금 조건을 먼저 확인하고 마지막 사용 시각을 갱신한다.
    ///
    /// # Errors
    /// 없으면 `NotFound`, 잠겼으면 `Locked`.
    pub fn key(&mut self, now: Instant) -> Result<&JudgeKey, SecretsError> {
        todo!("#84")
    }

    /// 강화 방식에서 session 시작 때 키체인 암호로 푼다. 암호는 OS 확인 창이 받는다(Saturn은 암호를 보지 않는다). 표준 방식이면 아무것도 하지 않는다.
    ///
    /// # Errors
    /// 사용자가 거부하면 `Locked`, 키체인 실패면 `Keychain`.
    pub async fn unlock(&mut self, now: Instant) -> Result<(), SecretsError> {
        todo!("#84")
    }

    /// 강화 방식에서 마지막 사용 뒤 `HARDENED_IDLE_LOCK` 또는 풀린 뒤 `HARDENED_MAX_UNLOCK`이 지났으면 `current`를 버리고 잠근다.
    /// 잠갔으면 참. engine 타이머가 주기적으로 부른다.
    /// TODO(#84): 값 미정, 초안 1분 주기
    pub fn lock_if_expired(&mut self, now: Instant) -> bool {
        todo!("#84")
    }

    /// 가림 대상 문자열. 이번 실행의 키가 있으면 그 원문. `Masker`를 만들 때 쓴다.
    pub(crate) fn mask_needles(&self) -> Vec<String> {
        todo!("#84")
    }
}
