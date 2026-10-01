//! 설정: 다섯 층 병합, 폴더 설정 신뢰, 병합 결과 검사, 스냅샷과 설정 번호, 설정 파일 편집.
//! 설계: docs/design/settings.md
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

/// 사용자 층은 `~/.saturn/`, 폴더 층은 `<폴더>/.saturn/` 아래 파일 이름.
pub const CONFIG_FILE: &str = "config.toml";

/// 병합 검사 실패면 호출자가 이전 번호로 계속하고 경고한다.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("failed to access settings file: {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid toml in {path} at line {line}: {message}")]
    Parse {
        /// 실행 `-c`면 `-c`.
        path: PathBuf,
        /// 1부터.
        line: usize,
        message: String,
    },
    /// 모르는 키, 타입 오류, 범위 밖 값.
    #[error("invalid setting {key}: {reason}")]
    Invalid {
        /// 점 경로. 예: `judge.thresholds.keep_current`.
        key: String,
        reason: String,
        /// 값이 온 층.
        layer: Layer,
    },
    /// 시작 때 검사가 실패했고 돌아갈 이전 설정 번호도 없어 실행하지 않는다.
    #[error("no valid settings revision to fall back to")]
    NoPreviousRevision,
    /// 신뢰 창을 거친 뒤 다시 병합한다.
    #[error("folder settings not trusted: {path}")]
    Untrusted {
        /// 폴더 설정 파일 경로.
        path: PathBuf,
    },
    /// 다시 읽고 다시 고친다.
    #[error("settings file changed since read: {path}")]
    Conflict {
        /// 사용자 파일이나 폴더 파일 경로.
        path: PathBuf,
    },
    #[error("failed to access settings snapshot")]
    Store(#[from] StoreError),
}

/// 순서가 우선순위다(뒤가 이긴다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Layer {
    Default,
    /// `~/.saturn/config.toml`.
    User,
    /// 작업 폴더에서 git 맨 위까지 올라가며 찾은 `.saturn/config.toml`. 신뢰한 것만.
    Folder,
    /// 보조 에이전트는 부모 채팅의 값을 쓴다.
    Chat,
    /// 이번 실행의 `-c key=value`.
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerSource {
    pub layer: Layer,
    pub path: Option<PathBuf>,
    /// 파일 층이면 읽은 내용의 SHA-256 hex. 해시 종류는 초안이다.
    pub fingerprint: Option<String>,
    /// 폴더 층에서 `USER_ONLY`라 무시한 점 경로 키.
    pub ignored: Vec<String>,
}

/// 값은 TOML을 JSON으로 옮긴 표 하나다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    values: serde_json::Value,
}

impl Settings {
    /// 사용자 전용.
    pub fn method(&self) -> Method {
        match self.text("judge.method") {
            "saturn" => Method::Saturn,
            "collect" => Method::Collect,
            _ => Method::Jev,
        }
    }

    /// 없는 항목은 기본값 층 값이다.
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

    /// 사용자 전용. 허용 호스트 검사는 `judges`가 한다.
    pub fn judge_endpoint(&self) -> &str {
        self.text("judge.endpoint")
    }

    /// 출처와 끝 4자리만 담는다.
    pub fn key_info(&self) -> Option<KeyInfo> {
        serde_json::from_value(self.get("judge.key.info")?.clone()).ok()
    }

    /// 사용자 전용.
    /// TODO(#32): 키 이름 `judge.key.command`(초안) 확정
    pub fn key_command(&self) -> Option<Vec<String>> {
        let items = self.get("judge.key.command")?.as_array()?;
        let command: Vec<String> = items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect();
        (!command.is_empty()).then_some(command)
    }

    pub fn storage_mode(&self) -> StorageMode {
        match self.text("judge.key.storage") {
            "hardened" => StorageMode::Hardened,
            _ => StorageMode::Standard,
        }
    }

    /// 사용자 전용.
    pub fn grading_model(&self) -> Option<&str> {
        self.get("grading.model")?.as_str()
    }

    /// 사용자 전용. 기본 거짓.
    pub fn share_with_server(&self) -> bool {
        self.lookup("consent.share_with_server")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 기본 무제한 보존.
    pub fn retention(&self) -> RetentionPolicy {
        let days = self.get("retention.max_age_days").and_then(Value::as_u64);
        RetentionPolicy {
            max_age: days.map(|days| Duration::from_secs(days.saturating_mul(24 * 60 * 60))),
        }
    }

    pub fn on_exit(&self) -> OnExit {
        match self.text("on_exit") {
            "stop" => OnExit::Stop,
            "ask" => OnExit::Ask,
            _ => OnExit::Background,
        }
    }

    /// 안전 비율 `context.safety_percent`(초안)는 두 provider 공통이다.
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

    /// 모르는 키면 `None`.
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        layers::get_path(&self.values, key)
    }

    /// 옛 스냅샷에 없던 키도 기본값 층 값으로 읽는다.
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key).or_else(|| layers::get_path(defaults(), key))
    }

    /// 기본값 층에 반드시 있는 키만 받는다.
    fn text(&self, key: &str) -> &str {
        self.lookup(key)
            .and_then(Value::as_str)
            .expect("default layer should define every string setting")
    }

    /// 기본값 층에 반드시 있는 키만 받는다.
    fn number(&self, key: &str) -> f64 {
        self.lookup(key)
            .and_then(Value::as_f64)
            .expect("default layer should define every numeric setting")
    }
}

fn defaults() -> &'static Value {
    static DEFAULTS: OnceLock<Value> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        layers::parse_toml(default_layer(), std::path::Path::new("default"))
            .expect("default layer should be valid toml")
    })
}

/// `store`가 저장하고 `digest`로 같은 내용을 찾는다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub settings: Settings,
    pub layers: Vec<LayerSource>,
}

impl SettingsSnapshot {
    /// 값이 같으면 같은 번호를 쓰도록 층 목록은 넣지 않는다. 해시 SHA-256은 초안이다.
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
