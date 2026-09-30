//! 층 찾기, 층 병합, 사용자 전용 항목 거르기, 병합 결과 검사, 다른 폴더 설정 참고 읽기.
//!
//! 설계: docs/design/settings.md(설정 층, 폴더 층에서 바꿀 수 없는 항목, 병합과 설정 번호).
//! TODO(#83): TOML 파싱은 `toml_edit`(작업 공간 의존성 추가 필요)로 하고 JSON 표로 옮겨 합친다

use std::path::{Path, PathBuf};

use super::{Layer, LayerSource, Settings, SettingsError, SettingsSnapshot};

/// 폴더 층에서 바꿀 수 없는 사용자 전용 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserOnly {
    /// judge 주소.
    JudgeEndpoint,
    /// judge 키 참조(키 정보, 관리자 명령, 저장 방식).
    JudgeKeyRef,
    /// 채점 모델.
    GradingModel,
    /// 데이터 공유 동의.
    DataSharingConsent,
    /// 판단 방식.
    Method,
}

impl UserOnly {
    /// 이 항목의 점 경로 키 접두사. 이 접두사로 시작하는 키는 모두 사용자 전용이다. TODO(#49): 키 이름 확정
    pub fn key_prefix(self) -> &'static str {
        todo!("#83")
    }
}

/// 사용자 전용 항목 전부.
pub const USER_ONLY: &[UserOnly] = &[
    UserOnly::JudgeEndpoint,
    UserOnly::JudgeKeyRef,
    UserOnly::GradingModel,
    UserOnly::DataSharingConsent,
    UserOnly::Method,
];

/// `workdir`에서 부모로 올라가며 `.saturn/config.toml`을 찾아 처음 만난 것을 돌려준다. git 맨 위(`.git`이 있는 폴더)에서 멈춘다.
/// git 저장소가 아니면 `workdir` 하나만 본다. `~/.saturn/config.toml`(사용자 층)은 폴더 층으로 잡지 않는다.
///
/// # Errors
/// 폴더를 읽지 못하면 `Io`.
pub async fn find_folder_config(
    workdir: &Path,
    saturn_home: &Path,
) -> Result<Option<PathBuf>, SettingsError> {
    todo!("#83")
}

/// 층을 순서대로 합치고 검사한다. `layers`는 `(출처, TOML 원문)`이고 `Layer` 순서로 정렬돼 있어야 한다.
/// 폴더 층의 `USER_ONLY` 키는 버리고 `LayerSource::ignored`에 적는다. 표는 키 단위로 합치고 나머지는 뒤 층 값으로 바꾼다.
/// 검사: 모르는 키, 타입 불일치, 기준값 0~1 밖, 되돌릴 수 없는 행동 기준값 0.8 미만이면 실패.
///
/// # Errors
/// 문법 오류면 `Parse`, 검사 실패면 `Invalid`.
pub fn merge(layers: Vec<(LayerSource, String)>) -> Result<SettingsSnapshot, SettingsError> {
    todo!("#83")
}

/// 실행 `-c key=value` 목록을 실행 층 TOML 원문으로 바꾼다. 값은 TOML 값 문법(`"문자열"`, `0.7`, `true`)으로 읽는다.
///
/// # Errors
/// `=`가 없거나 값이 TOML 값이 아니면 `Parse`(경로는 `-c`).
pub fn run_layer(overrides: &[String]) -> Result<String, SettingsError> {
    todo!("#83")
}

/// 기본값 층 원문. 실행 파일에 넣은 TOML.
pub fn default_layer() -> &'static str {
    todo!("#83")
}

/// 작업 폴더가 아닌 폴더의 설정을 참고 자료로만 읽는다. 병합하지 않고 신뢰도 묻지 않으며 원문만 돌려준다.
///
/// # Errors
/// 읽기 실패면 `Io`.
pub async fn read_reference(path: &Path) -> Result<String, SettingsError> {
    todo!("#83")
}

/// 병합 결과에서 사용자 전용 키가 사용자 층(또는 기본값) 값인지 확인한다. 테스트와 디버그 검사용.
pub(crate) fn user_only_from_user_layer(settings: &Settings, layers: &[LayerSource]) -> bool {
    todo!("#83")
}

/// 층 출처 하나를 만든다. 파일 층이면 내용 지문을 함께 계산한다.
pub(crate) fn source(layer: Layer, path: Option<PathBuf>, content: &str) -> LayerSource {
    todo!("#83")
}
