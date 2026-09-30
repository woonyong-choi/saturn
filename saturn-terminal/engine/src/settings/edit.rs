//! 명령으로 설정 파일 고치기: 읽은 버전을 확인한 뒤 쓰고, 주석과 순서를 보존한다.
//!
//! 설계: docs/design/settings.md(설정 파일 편집). 판단기 키 자체는 쓰지 않는다. 키는 `KeyInfo`(출처와 끝 4자리)만 쓴다.
//! TODO(#83): 편집은 `toml_edit::DocumentMut`(작업 공간 의존성 추가 필요)로 해 주석과 서식을 그대로 둔다

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{SettingsError, SettingsManager};
use crate::secrets::KeyInfo;

/// 읽은 순간의 파일 버전. 쓰기 직전에 다시 재서 다르면 쓰지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    /// 파일 경로.
    pub path: PathBuf,
    /// 내용 지문(SHA-256 hex). 파일이 없었으면 `None`.
    pub fingerprint: Option<String>,
    /// 수정 시각. 지문 비교 전에 빠른 확인용.
    pub modified: Option<SystemTime>,
}

impl SettingsManager {
    /// 파일을 읽고 버전을 함께 돌려준다.
    ///
    /// # Errors
    /// 읽기 실패면 `Io`(없는 파일은 빈 내용과 `fingerprint: None`).
    pub async fn read_for_edit(&self, path: &Path) -> Result<(String, FileVersion), SettingsError> {
        todo!("#83")
    }

    /// 점 경로 키 하나를 `value`(TOML 값 문법)로 바꾼다. 흐름: 읽기 → `toml_edit`로 그 키만 고치기 → 쓰기 직전 버전 비교
    /// → 같으면 임시 파일에 쓰고 이름 바꾸기. 주석, 빈 줄, 키 순서는 그대로 둔다. 사용자 파일과 폴더 파일만 고친다.
    ///
    /// # Errors
    /// 읽은 뒤 파일이 바뀌었으면 `Conflict`, 값이 TOML이 아니면 `Parse`, 쓰기 실패면 `Io`.
    pub async fn set_value(
        &self,
        path: &Path,
        key: &str,
        value: &str,
        read: &FileVersion,
    ) -> Result<(), SettingsError> {
        todo!("#83")
    }

    /// 사용자 층에 판단기 키 정보(출처와 끝 4자리)를 쓴다. 키 원문은 받지 않는다. `set_value`와 같은 버전 확인을 거친다.
    ///
    /// # Errors
    /// `set_value`와 같다.
    pub async fn record_key_info(&self, info: &KeyInfo) -> Result<(), SettingsError> {
        todo!("#83")
    }
}
