//! 설정 적용 흐름: 층 읽기 → 폴더 신뢰 확인 → 병합과 검사 → 스냅샷 저장과 설정 번호. 입력별 고정 번호 조회.
//!
//! 설계: docs/design/settings.md(병합과 설정 번호, 입력마다 설정 번호 고정, 오류 처리).
//! 검사 실패: 이전 번호를 유지하고 경고 `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`를 `Notification::SettingsApplied`로 보낸다.
//! 시작 때 검사 실패이고 이전 번호도 없으면 실행하지 않는다.

use std::path::{Path, PathBuf};

use saturn_protocol::ids::{ChatId, SettingsRevision};

use super::{Settings, SettingsError, TrustStatus, TrustStore};
use crate::store::Store;

/// 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// 이번에 쓰는 설정 번호. 검사 실패면 이전 번호.
    pub revision: SettingsRevision,
    /// 검사 실패나 무시한 사용자 전용 키가 있을 때 한 줄 경고.
    pub warning: Option<String>,
}

/// 설정 관리자. engine에 하나. 작업 폴더와 실행 `-c`는 engine 시작 때 정한다.
#[derive(Debug)]
pub struct SettingsManager {
    /// `~/.saturn`.
    home: PathBuf,
    /// 작업 폴더. 폴더 층 검색의 시작점.
    workdir: PathBuf,
    /// 실행 `-c key=value` 원문.
    run_overrides: Vec<String>,
    /// 폴더 신뢰 기록.
    trust: TrustStore,
    /// 마지막으로 적용한 번호. 검사 실패 때 돌아갈 곳.
    current: Option<SettingsRevision>,
}

impl SettingsManager {
    /// 관리자를 만든다. 신뢰 기록을 읽고, `store.latest_settings_revision()`을 `current`로 둔다.
    ///
    /// # Errors
    /// 신뢰 기록 읽기 실패면 `Io`/`Parse`, 스냅샷 조회 실패면 `Store`.
    pub async fn new(
        home: PathBuf,
        workdir: PathBuf,
        run_overrides: Vec<String>,
        store: &Store,
    ) -> Result<Self, SettingsError> {
        todo!("#83")
    }

    /// 폴더 설정의 신뢰 상태. 폴더 설정이 없으면 `None`. `Unknown`·`Changed`면 호출자가 TUI 신뢰 창을 열고 답을 `trust_folder`로 넘긴다.
    ///
    /// # Errors
    /// 파일 읽기 실패면 `Io`.
    pub async fn folder_status(&self) -> Result<Option<(PathBuf, TrustStatus)>, SettingsError> {
        todo!("#83")
    }

    /// 신뢰 창에서 `y`를 확정했을 때 경로와 지문을 기록한다.
    ///
    /// # Errors
    /// 기록 쓰기 실패면 `Io`.
    pub async fn trust_folder(
        &mut self,
        path: &Path,
        fingerprint: &str,
    ) -> Result<(), SettingsError> {
        todo!("#83")
    }

    /// 병합하고 적용한다. 순서: 기본값 → 사용자 → 폴더(신뢰한 것만, 아니면 `Untrusted`) → `chat`의 채팅 층(`store.chat_layer`)
    /// → 실행 `-c` → `merge` 검사 → `store.save_settings_snapshot`(같은 내용이면 기존 번호) → `mark_settings_applied`.
    /// 검사가 실패하면 `current`가 있으면 그 번호와 경고를 돌려주고, 없으면 `NoPreviousRevision`.
    /// 보조 에이전트도 부모의 `chat`을 넘긴다(채팅 층 상속).
    ///
    /// # Errors
    /// 폴더 설정 미신뢰면 `Untrusted`, 이전 번호 없이 검사 실패면 `NoPreviousRevision`, 저장 실패면 `Store`.
    pub async fn apply(
        &mut self,
        store: &Store,
        chat: Option<ChatId>,
    ) -> Result<Applied, SettingsError> {
        todo!("#83")
    }

    /// 지금 쓰는 설정 번호. 입력 접수 때 이 값을 `NewInput::settings`로 고정한다. 한 번도 적용하지 않았으면 `None`.
    pub fn current(&self) -> Option<SettingsRevision> {
        self.current
    }

    /// 고정한 번호의 설정. provider 실행과 judge 호출은 입력의 번호로 이것을 불러 쓴다(처리 중 설정이 바뀌어도 같은 값).
    ///
    /// # Errors
    /// 없는 번호면 `Store`.
    pub async fn at(
        &self,
        store: &Store,
        revision: SettingsRevision,
    ) -> Result<Settings, SettingsError> {
        todo!("#83")
    }

    /// 설정 파일이 바뀌었는지 본다(사용자 파일, 폴더 파일의 수정 시각과 지문). 바뀌었으면 호출자가 다음 입력 접수 전에 `apply`한다.
    ///
    /// # Errors
    /// 파일 읽기 실패면 `Io`.
    pub async fn changed(&self) -> Result<bool, SettingsError> {
        todo!("#83")
    }
}
