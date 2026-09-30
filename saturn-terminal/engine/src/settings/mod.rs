//! 설정: 다섯 층(기본값, 사용자, 폴더, 채팅, 실행 `-c`) 병합, 폴더 설정 신뢰, 병합 결과 검사, 스냅샷과 설정 번호, 설정 파일 편집.
//!
//! 설계: docs/design/settings.md, docs/design/judge-key-security.md(키 정보), docs/architecture.md(배치 경로).
//! 규칙:
//! - 뒤 층 값이 앞 층 값보다 우선한다. 표(table)는 키 단위로 합치고, 배열과 값은 통째로 바꾼다.
//! - 폴더 층은 `USER_ONLY`를 바꾸지 못한다. 폴더 파일에 있어도 무시하고 신뢰 창에 무시 항목으로 보인다.
//! - 검사를 통과한 결과만 `store`에 스냅샷으로 남기고 설정 번호를 받는다. 같은 내용이면 기존 번호를 다시 쓴다.
//! - 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다(`SettingsManager::at`).
//! - 판단기 키는 설정에 없다. 사용자 층에 `KeyInfo`(출처와 끝 4자리)만 쓴다.
//! - Saturn 설정은 provider 설정 파일을 바꾸지 않는다. Saturn 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다.
//!
//! 시작 흐름: `rpc` 잠금 → `Store::open`(이관) → `SettingsManager::apply` → judge 시작 확인(설정 번호 확정 뒤).
//! TODO(#49): 설정 키 이름과 기본값

mod edit;
mod layers;
mod manager;
mod trust;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use saturn_core::judges::{Method, Thresholds};
use saturn_core::sessions::context::ContextBudget;
use saturn_protocol::ids::Provider;
use saturn_protocol::state::OnExit;

use crate::secrets::{KeyInfo, StorageMode};
use crate::store::{RetentionPolicy, StoreError};

pub use edit::FileVersion;
pub use layers::{
    USER_ONLY, UserOnly, default_layer, find_folder_config, merge, read_reference, run_layer,
};
pub use manager::{Applied, SettingsManager};
pub use trust::{FolderTrustPrompt, TrustStatus, TrustStore};

/// 사용자 설정 파일 이름(`~/.saturn/config.toml`). 폴더 설정은 `<폴더>/.saturn/config.toml`.
pub const CONFIG_FILE: &str = "config.toml";

/// 설정 오류. 병합 검사 실패는 호출자가 이전 번호로 계속하고 경고한다.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// 설정 파일을 읽거나 쓰지 못했다.
    #[error("failed to access settings file: {path}")]
    Io {
        /// 파일 경로.
        path: PathBuf,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// TOML 문법 오류. `line`은 1부터. 경고 문구 `줄 7: ...`에 쓴다.
    #[error("invalid toml in {path} at line {line}: {message}")]
    Parse {
        /// 파일 경로. 실행 `-c`면 `-c`.
        path: PathBuf,
        /// 줄 번호.
        line: usize,
        /// 파서 메시지.
        message: String,
    },
    /// 병합 결과 검사 실패(모르는 키, 타입 오류, 범위 밖 값). `key`는 `judge.thresholds.keep_current`처럼 점 경로.
    #[error("invalid setting {key}: {reason}")]
    Invalid {
        /// 점 경로 키.
        key: String,
        /// 이유.
        reason: String,
        /// 값이 온 층.
        layer: Layer,
    },
    /// 시작 때 검사가 실패했고 돌아갈 이전 설정 번호도 없다. 실행하지 않는다.
    #[error("no valid settings revision to fall back to")]
    NoPreviousRevision,
    /// 신뢰하지 않은 폴더 설정이다. 신뢰 창을 거친 뒤 다시 병합한다.
    #[error("folder settings not trusted: {path}")]
    Untrusted {
        /// 폴더 설정 경로.
        path: PathBuf,
    },
    /// 쓰려는 파일이 읽은 뒤 바뀌었다. 다시 읽고 다시 고친다.
    #[error("settings file changed since read: {path}")]
    Conflict {
        /// 파일 경로.
        path: PathBuf,
    },
    /// 스냅샷 저장이나 조회 실패.
    #[error("failed to access settings snapshot")]
    Store(#[from] StoreError),
}

/// 설정 층. 순서가 우선순위다(뒤가 이긴다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Layer {
    /// Saturn 안의 기본값.
    Default,
    /// `~/.saturn/config.toml`.
    User,
    /// 작업 폴더에서 git 맨 위까지 올라가며 찾은 `.saturn/config.toml`. 신뢰한 것만.
    Folder,
    /// 채팅마다 둔 값(`store` 채팅 행). 보조 에이전트는 부모 채팅의 값을 쓴다.
    Chat,
    /// 이번 실행의 `-c key=value`.
    Run,
}

/// 병합에 들어간 층 하나. 스냅샷에 층 목록으로 남는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerSource {
    /// 층.
    pub layer: Layer,
    /// 파일 층이면 경로.
    pub path: Option<PathBuf>,
    /// 파일 층이면 읽은 내용의 지문(SHA-256 hex).
    pub fingerprint: Option<String>,
    /// 폴더 층에서 `USER_ONLY`라 무시한 점 경로 키.
    pub ignored: Vec<String>,
}

/// 검사를 통과한 병합 결과. 값은 TOML을 JSON으로 옮긴 표 하나다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    values: serde_json::Value,
}

impl Settings {
    /// 판단 방식. 사용자 전용.
    pub fn method(&self) -> Method {
        todo!("#83")
    }

    /// 질문별 기준값. 사용자 층과 폴더 층에서 바꿀 수 있다. 없는 항목은 `Thresholds::default()`.
    pub fn thresholds(&self) -> Thresholds {
        todo!("#83")
    }

    /// judge 주소. 사용자 전용. 허용 호스트 검사는 `judges`가 한다.
    pub fn judge_endpoint(&self) -> &str {
        todo!("#83")
    }

    /// 사용자 층에 기록한 키 정보(출처와 끝 4자리). 없으면 `None`.
    pub fn key_info(&self) -> Option<KeyInfo> {
        todo!("#83")
    }

    /// 비밀번호 관리자 명령(실행 파일과 인자). 사용자 전용. 없으면 `None`. TODO(#32): 키 이름 `judge.key_command`
    pub fn key_command(&self) -> Option<Vec<String>> {
        todo!("#83")
    }

    /// 키 저장 방식. 기본 `Standard`.
    pub fn storage_mode(&self) -> StorageMode {
        todo!("#83")
    }

    /// 채점 모델. 사용자 전용.
    pub fn grading_model(&self) -> Option<&str> {
        todo!("#83")
    }

    /// 데이터 공유 동의(`consent.share_with_server`). 사용자 전용. 기본 거짓.
    pub fn share_with_server(&self) -> bool {
        todo!("#83")
    }

    /// 자동 정리 정책. 기본 무제한 보존.
    pub fn retention(&self) -> RetentionPolicy {
        todo!("#83")
    }

    /// TUI를 닫을 때 engine이 할 일. 기본 `Background`.
    pub fn on_exit(&self) -> OnExit {
        todo!("#83")
    }

    /// provider별 맥락 기준값.
    pub fn context_budget(&self, provider: Provider) -> ContextBudget {
        todo!("#83")
    }

    /// 점 경로 키의 원값. 모르는 키면 `None`.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        todo!("#83")
    }
}

/// 설정 번호에 붙는 스냅샷. `store`가 저장하고 `digest`로 같은 내용을 찾는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    /// 병합 결과.
    pub settings: Settings,
    /// 들어간 층 순서대로.
    pub layers: Vec<LayerSource>,
}

impl SettingsSnapshot {
    /// 같은 내용 판정 값. 키를 정렬한 `settings` JSON의 SHA-256 hex. 층 목록은 넣지 않는다(값이 같으면 같은 번호).
    pub fn digest(&self) -> String {
        todo!("#83")
    }
}
