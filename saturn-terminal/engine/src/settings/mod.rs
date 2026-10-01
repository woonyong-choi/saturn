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
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use saturn_core::judges::{Method, Thresholds};
use saturn_core::sessions::context::ContextBudget;
use saturn_protocol::ids::Provider;
use saturn_protocol::state::OnExit;

use crate::secrets::{KeyInfo, StorageMode};
use crate::store::{RetentionPolicy, StoreError, sha256_hex};

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
    /// 파일 층이면 읽은 내용의 지문(hex). 해시 SHA-256은 초안이다(설계는 지문만 정함).
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
        match self.text("judge.method") {
            "saturn" => Method::Saturn,
            "collect" => Method::Collect,
            _ => Method::Jev,
        }
    }

    /// 질문별 기준값. 사용자 층과 폴더 층에서 바꿀 수 있다. 없는 항목은 기본값 층 값(docs/design/judge.md 표)이다.
    pub fn thresholds(&self) -> Thresholds {
        let value = |name: &str| self.number(&format!("judge.thresholds.{name}"));
        Thresholds {
            keep_current: value("keep_current"),
            is_actionable: value("is_actionable"),
            min_confidence: value("min_confidence"),
            resume_held: value("resume_held"),
            file_relevant: (value("file_present"), value("file_absent")),
            context_gate: value("context_gate"),
            compact_keep: value("compact_keep"),
            injection: value("injection"),
            progressing: value("progressing"),
            feedback_cause: value("feedback_cause"),
        }
    }

    /// judge 주소. 사용자 전용. 허용 호스트 검사는 `judges`가 한다.
    pub fn judge_endpoint(&self) -> &str {
        self.text("judge.endpoint")
    }

    /// 사용자 층에 기록한 키 정보(출처와 끝 4자리). 없으면 `None`.
    pub fn key_info(&self) -> Option<KeyInfo> {
        serde_json::from_value(self.get("judge.key.info")?.clone()).ok()
    }

    /// 비밀번호 관리자 명령(실행 파일과 인자). 사용자 전용. 없으면 `None`. 키 이름 `judge.key.command`는 초안이다.
    /// TODO(#32): 키 이름 확정
    pub fn key_command(&self) -> Option<Vec<String>> {
        let items = self.get("judge.key.command")?.as_array()?;
        let command: Vec<String> = items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect();
        (!command.is_empty()).then_some(command)
    }

    /// 키 저장 방식. 기본 `Standard`.
    pub fn storage_mode(&self) -> StorageMode {
        match self.text("judge.key.storage") {
            "hardened" => StorageMode::Hardened,
            _ => StorageMode::Standard,
        }
    }

    /// 채점 모델. 사용자 전용.
    pub fn grading_model(&self) -> Option<&str> {
        self.get("grading.model")?.as_str()
    }

    /// 데이터 공유 동의(`consent.share_with_server`). 사용자 전용. 기본 거짓.
    pub fn share_with_server(&self) -> bool {
        self.lookup("consent.share_with_server")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 자동 정리 정책. 기본 무제한 보존. `retention.max_age_days`(초안)가 있으면 그 일수.
    pub fn retention(&self) -> RetentionPolicy {
        let days = self.get("retention.max_age_days").and_then(Value::as_u64);
        RetentionPolicy {
            max_age: days.map(|days| Duration::from_secs(days.saturating_mul(24 * 60 * 60))),
        }
    }

    /// TUI를 닫을 때 engine이 할 일. 기본 `Background`.
    pub fn on_exit(&self) -> OnExit {
        match self.text("on_exit") {
            "stop" => OnExit::Stop,
            "ask" => OnExit::Ask,
            _ => OnExit::Background,
        }
    }

    /// provider별 맥락 기준값. 키는 `context.<codex|claude>.*`, 안전 비율은 두 provider 공통 `context.safety_percent`(초안).
    pub fn context_budget(&self, provider: Provider) -> ContextBudget {
        let section = match provider {
            Provider::Codex => "context.codex",
            Provider::Claude => "context.claude",
        };
        let integer = |key: &str| {
            self.lookup(key)
                .and_then(Value::as_u64)
                .expect("default layer should define every context integer")
        };
        ContextBudget {
            t_abs: integer(&format!("{section}.t_abs")),
            safety_percent: u8::try_from(integer("context.safety_percent")).unwrap_or(100),
            window: integer(&format!("{section}.window")),
            cache_read: self.number(&format!("{section}.cache_read")),
            cache_write: self.number(&format!("{section}.cache_write")),
            cache_ttl: Duration::from_secs(integer(&format!("{section}.cache_ttl_secs"))),
        }
    }

    /// 점 경로 키의 원값. 모르는 키면 `None`.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        layers::get_path(&self.values, key)
    }

    /// 이 값에 없으면 기본값 층 값. 옛 스냅샷에 없던 키도 기본값으로 읽는다.
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key).or_else(|| layers::get_path(defaults(), key))
    }

    /// 기본값 층에 반드시 있는 문자열 키.
    fn text(&self, key: &str) -> &str {
        self.lookup(key)
            .and_then(Value::as_str)
            .expect("default layer should define every string setting")
    }

    /// 기본값 층에 반드시 있는 실수 키.
    fn number(&self, key: &str) -> f64 {
        self.lookup(key)
            .and_then(Value::as_f64)
            .expect("default layer should define every numeric setting")
    }
}

/// 기본값 층을 JSON 표로 한 번만 읽는다.
fn defaults() -> &'static Value {
    static DEFAULTS: OnceLock<Value> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        layers::parse_toml(default_layer(), std::path::Path::new("default"))
            .expect("default layer should be valid toml")
    })
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
    /// 같은 내용 판정 값. 키를 정렬한 `settings` JSON의 해시 hex. 층 목록은 넣지 않는다(값이 같으면 같은 번호).
    /// 해시 SHA-256은 초안이다(설계에 없음).
    pub fn digest(&self) -> String {
        let sorted = serde_json::to_string(&self.settings.values)
            .expect("settings json should serialize because it holds only json values");
        sha256_hex(sorted.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(content: &str, layers: Vec<LayerSource>) -> SettingsSnapshot {
        let mut sources = vec![(
            layers::source(Layer::Default, None, default_layer()),
            default_layer().to_owned(),
        )];
        sources.push((
            layers::source(Layer::Chat, None, content),
            content.to_owned(),
        ));
        let mut merged = merge(sources).unwrap();
        merged.layers = layers;
        merged
    }

    #[test]
    fn digest_ignores_layer_list_and_key_order() {
        let a = snapshot(
            "on_exit = \"ask\"\nretention.max_age_days = 3\n",
            Vec::new(),
        );
        let b = snapshot(
            "retention.max_age_days = 3\non_exit = \"ask\"\n",
            vec![layers::source(Layer::User, Some(PathBuf::from("/u")), "x")],
        );
        let c = snapshot("on_exit = \"stop\"\n", Vec::new());

        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), c.digest());
        assert_eq!(a.digest().len(), 64);
        assert_eq!(
            a.settings.retention().max_age,
            Some(Duration::from_secs(3 * 24 * 60 * 60))
        );
    }

    #[test]
    fn user_only_check_sees_ignored_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".saturn").join(CONFIG_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = "[judge]\nmethod = \"saturn\"\n";
        std::fs::write(&path, content).unwrap();
        let sources = vec![
            (
                layers::source(Layer::Default, None, default_layer()),
                default_layer().to_owned(),
            ),
            (
                layers::source(Layer::Folder, Some(path), content),
                content.to_owned(),
            ),
        ];

        let snapshot = merge(sources).unwrap();

        assert!(layers::user_only_from_user_layer(
            &snapshot.settings,
            &snapshot.layers
        ));
        let mut forged = snapshot.layers.clone();
        forged[1].ignored.clear();
        assert!(!layers::user_only_from_user_layer(
            &snapshot.settings,
            &forged
        ));
    }
}
