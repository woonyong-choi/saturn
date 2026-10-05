//! 설정: 다섯 층 병합, 폴더 설정 신뢰, 병합 결과 검사, 스냅샷과 설정 번호, 설정 파일 편집.
//! 설계: docs/design/settings.md

mod edit;
mod layers;
mod manager;
#[cfg(test)]
mod names_tests;
mod permission;
mod trust;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use saturn_core::routers::{Method, Thresholds};
use saturn_core::sessions::context::{ContextBudget, DEFAULT_CACHE_TTL};
use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::ModelMode;

use crate::providers::ContextDefaults;
use saturn_protocol::state::OnExit;

use crate::secrets::{KeyInfo, StorageMode};
use crate::store::{RetentionPolicy, StoreError, sha256_hex};

pub(crate) use layers::default_layer;
pub(crate) use manager::{Applied, FileFingerprints, SettingsManager};
pub(crate) use permission::{PermissionSettings, chat_layer_mode, with_chat_layer_mode};
pub(crate) use trust::{FolderTrustPrompt, TrustStatus, TrustStore};

/// 모든 provider가 같은 맥락 기본값. `window`와 `cache_write`는 어댑터 설명자가 알린다.
const DEFAULT_T_ABS: u64 = 200_000;
const DEFAULT_CACHE_READ: f64 = 0.1;

/// 사용자 층은 `~/.saturn/`, 폴더 층은 `<폴더>/.saturn/` 아래 파일 이름.
pub(crate) const CONFIG_FILE: &str = "config.toml";

/// 병합 검사 실패면 호출자가 이전 번호로 계속하고 경고한다.
/// 맥락 정리를 누가 맡을지 정하는 `context.mode`. `Provider`면 `sessions`는 compaction을 판정하지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextMode {
    Saturn,
    Provider,
}

/// 화면 방식 `tui.screen`. `auto`는 터미널이면 전체 화면, 아니면 plain이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Auto,
    Full,
    Plain,
}

impl Screen {
    /// 설정 값 이름.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Plain => "plain",
        }
    }
}

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
        /// 점 경로. 예: `router.thresholds.keep_current`.
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
pub(crate) struct LayerSource {
    pub layer: Layer,
    pub path: Option<PathBuf>,
    /// 파일 층이면 읽은 내용의 SHA-256 hex. 해시 종류는 초안이다.
    pub fingerprint: Option<String>,
    /// 폴더 층에서 `USER_ONLY`라 무시한 점 경로 키.
    pub ignored: Vec<String>,
    /// 옛 이름으로 적혀 새 이름으로 읽은 `(옛 이름, 새 이름)`. 옛 스냅샷에는 없다.
    #[serde(default)]
    pub renamed: Vec<(String, String)>,
}

/// 값은 TOML을 JSON으로 옮긴 표 하나다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    values: serde_json::Value,
}

impl Settings {
    /// 사용자 전용. 옛 스냅샷의 `router.method`도 읽는다.
    pub(crate) fn method(&self) -> Method {
        match self.text_or_old("router.mode", "router.method") {
            "saturn" => Method::Saturn,
            "collect" => Method::Collect,
            _ => Method::Jev,
        }
    }

    /// 없는 항목은 기본값 층 값이다.
    pub(crate) fn thresholds(&self) -> Thresholds {
        let value = |name: &str| self.number(&format!("router.thresholds.{name}"));
        Thresholds {
            keep_current: value("keep_current"),
            is_actionable: value("is_actionable"),
            min_confidence: value("min_confidence"),
            resume_held: value("resume_held"),
            file_relevant: (value("file_present"), value("file_absent")),
            context_gate: value("context_gate"),
            injection: value("injection"),
            progressing: value("progressing"),
            feedback_cause: value("feedback_cause"),
            is_constraint: value("is_constraint"),
            constraint_ask: value("constraint_ask"),
            constraint_release: value("constraint_release"),
        }
    }

    /// 사용자 전용. 허용 호스트 검사는 `routers`가 한다.
    pub(crate) fn router_endpoint(&self) -> &str {
        self.text("router.endpoint")
    }

    /// 출처와 끝 4자리만 담는다.
    pub(crate) fn key_info(&self) -> Option<KeyInfo> {
        serde_json::from_value(self.get("router.key.info")?.clone()).ok()
    }

    /// 사용자 전용.
    pub(crate) fn key_command(&self) -> Option<Vec<String>> {
        let items = self.get("router.key.command")?.as_array()?;
        let command: Vec<String> = items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect();
        (!command.is_empty()).then_some(command)
    }

    pub(crate) fn storage_mode(&self) -> StorageMode {
        match self.text("router.key.storage") {
            "hardened" => StorageMode::Hardened,
            _ => StorageMode::Standard,
        }
    }

    /// 완료 검사 근거로 인정할 검사 명령. 없으면 빈 목록이다.
    pub(crate) fn completion_checks(&self) -> Vec<String> {
        self.get("completion.checks")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 사용자 전용.
    pub(crate) fn grading_model(&self) -> Option<&str> {
        self.get("grading.model")?.as_str()
    }

    /// 사용자 전용. 기본 거짓.
    pub(crate) fn share_with_server(&self) -> bool {
        self.lookup("consent.share_with_server")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 사용자 전용. router 판단으로 지속 제약을 자동 등록하는지. 기본 거짓.
    pub(crate) fn constraint_auto_apply(&self) -> bool {
        self.lookup("constraint.auto_apply")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 사용자 전용. 모델 선택 그림자 판단을 켰는지. 실험 옵션이라 기본 거짓이고, 켜도 실제 선택은 바뀌지 않는다.
    pub(crate) fn shadow_model_selection(&self) -> bool {
        self.lookup("router.shadow.model_selection")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 기본 무제한 보존.
    pub(crate) fn retention(&self) -> RetentionPolicy {
        let days = self.get("retention.max_age_days").and_then(Value::as_u64);
        RetentionPolicy {
            max_age: days.map(|days| Duration::from_secs(days.saturating_mul(24 * 60 * 60))),
            auto_prune: self
                .get("retention.auto_prune")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// 모드와 개별 규칙. 옛 스냅샷에 없으면 기본 모드와 빈 규칙이다.
    pub(crate) fn permission(&self) -> PermissionSettings {
        permission::from_value(self.get(permission::KEY))
    }

    /// TUI 키 묶음 이름 `tui.keymap`.
    pub(crate) fn keymap(&self) -> &str {
        self.text("tui.keymap")
    }

    /// 하위 접속 상한 `child.*`. 사용자 전용이고 없는 키는 기본값이다.
    pub(crate) fn child_limits(&self) -> saturn_core::passes::PassLimits {
        let defaults = saturn_core::passes::PassLimits::DEFAULT;
        let number = |key: &str, fallback: u32| {
            self.get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(fallback)
        };
        saturn_core::passes::PassLimits {
            max_depth: number("child.max_depth", defaults.max_depth),
            max_concurrent: number("child.max_concurrent", defaults.max_concurrent),
            max_total: number("child.max_total", defaults.max_total),
        }
    }

    /// 옛 스냅샷의 `on_exit`도 읽는다.
    pub(crate) fn on_exit(&self) -> OnExit {
        match self.text_or_old("tui.on_exit", "on_exit") {
            "stop" => OnExit::Stop,
            "ask" => OnExit::Ask,
            _ => OnExit::Background,
        }
    }

    /// 화면 방식 `tui.screen`. 모르는 값은 검사에서 걸러져 `auto`로 본다.
    pub(crate) fn screen(&self) -> Screen {
        match self.text("tui.screen") {
            "full" => Screen::Full,
            "plain" => Screen::Plain,
            _ => Screen::Auto,
        }
    }

    /// 작업이 끝났을 때 알림 `notify.on_done`. 기본 거짓. 알림 동작은 #151이 정한다.
    #[cfg_attr(not(test), expect(dead_code, reason = "알림 동작은 #151"))]
    pub(crate) fn notify_on_done(&self) -> bool {
        self.lookup("notify.on_done")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// 모델 선택 방식 `model.mode`. 옛 스냅샷에 없거나 모르는 값이면 오토다.
    pub(crate) fn model_mode(&self) -> ModelMode {
        match self.get("model.mode").and_then(Value::as_str) {
            Some("manual") => ModelMode::Manual,
            _ => ModelMode::Auto,
        }
    }

    /// 기본 모델 `model.default`의 원문(`<provider>/<model>`). 고르지 않았으면 `None`.
    pub(crate) fn model_default(&self) -> Option<&str> {
        self.get("model.default")?.as_str()
    }

    /// 정리 모드 `context.mode`. 모르는 값은 검사에서 걸러져 기본값(`saturn`)으로 본다.
    pub(crate) fn context_mode(&self) -> ContextMode {
        match self.text("context.mode") {
            "provider" => ContextMode::Provider,
            _ => ContextMode::Saturn,
        }
    }

    /// 안전 비율 `context.safety_percent`(초안)는 모든 provider 공통이다. provider 값은 `provider.<id>.context.*`이고,
    /// 옛 스냅샷의 `context.<id>.*`도 읽는다. 설정에 없으면 `window`와 `cache_write`는 어댑터 설명자의 값(`defaults`),
    /// `t_abs`와 `cache_read`는 모든 provider 공통 기본값이다.
    pub(crate) fn context_budget(
        &self,
        provider: Provider,
        defaults: ContextDefaults,
    ) -> ContextBudget {
        let integer = |name: &str, fallback: u64| {
            self.provider_context(provider, name)
                .and_then(Value::as_u64)
                .unwrap_or(fallback)
        };
        let number = |name: &str, fallback: f64| {
            self.provider_context(provider, name)
                .and_then(Value::as_f64)
                .unwrap_or(fallback)
        };
        ContextBudget {
            t_abs: integer("t_abs", DEFAULT_T_ABS),
            safety_percent: u8::try_from(
                self.lookup("context.safety_percent")
                    .and_then(Value::as_u64)
                    .expect("default layer should define the safety percent"),
            )
            .unwrap_or(100),
            window: integer("window", defaults.window),
            cache_read: number("cache_read", DEFAULT_CACHE_READ),
            cache_write: number("cache_write", defaults.cache_write),
            cache_ttl: DEFAULT_CACHE_TTL,
            packet_hard_percent: self.packet_hard_percent(),
            item_cap_percent: self.positive("context.item_cap_percent"),
            constraint_slot_percent: self.positive("context.constraint_slot_percent"),
            rrf_k: u32::try_from(self.whole("context.select.rrf_k")).unwrap_or(u32::MAX),
        }
    }

    /// 옛 스냅샷에 `context.packet_hard_divisor`만 있으면 `100 / 나눗수`로 읽는다.
    fn packet_hard_percent(&self) -> u64 {
        let old = self
            .get("context.packet_hard_divisor")
            .and_then(layers::divisor_to_percent)
            .and_then(|value| value.as_u64());
        self.get("context.packet_hard_percent")
            .and_then(Value::as_u64)
            .or(old)
            .unwrap_or_else(|| self.whole("context.packet_hard_percent"))
            .clamp(1, 100)
    }

    /// 새 키, 옛 스냅샷의 옛 키, 기본값 층 순서로 찾는다.
    fn provider_context(&self, provider: Provider, name: &str) -> Option<&Value> {
        self.get(&format!("provider.{provider}.context.{name}"))
            .or_else(|| self.get(&format!("context.{provider}.{name}")))
    }

    /// 모르는 키면 `None`.
    pub(crate) fn get(&self, key: &str) -> Option<&serde_json::Value> {
        layers::get_path(&self.values, key)
    }

    /// 옛 스냅샷에 없던 키도 기본값 층 값으로 읽는다.
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key).or_else(|| layers::get_path(defaults(), key))
    }

    /// 새 키, 옛 스냅샷의 옛 키, 기본값 층 순서로 찾는다.
    fn text_or_old(&self, key: &str, old: &str) -> &str {
        self.get(key)
            .or_else(|| self.get(old))
            .and_then(Value::as_str)
            .unwrap_or_else(|| self.text(key))
    }

    /// 기본값 층에 반드시 있는 키만 받는다.
    fn text(&self, key: &str) -> &str {
        self.lookup(key)
            .and_then(Value::as_str)
            .expect("default layer should define every string setting")
    }

    /// 기본값 층에 반드시 있는 키만 받는다.
    fn whole(&self, key: &str) -> u64 {
        self.lookup(key)
            .and_then(Value::as_u64)
            .expect("default layer should define every integer setting")
    }

    /// 기본값 층에 반드시 있는 1 이상 정수 키만 받는다.
    fn positive(&self, key: &str) -> u64 {
        self.whole(key).max(1)
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
pub(crate) struct SettingsSnapshot {
    pub settings: Settings,
    pub layers: Vec<LayerSource>,
}

impl SettingsSnapshot {
    /// 값이 같으면 같은 번호를 쓰도록 층 목록은 넣지 않는다. 해시 SHA-256은 초안이다.
    pub(crate) fn digest(&self) -> String {
        let sorted = serde_json::to_string(&self.settings.values)
            .expect("settings json should serialize because it holds only json values");
        sha256_hex(sorted.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::layers::merge;
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
            "tui.on_exit = \"ask\"\nretention.max_age_days = 3\n",
            Vec::new(),
        );
        let b = snapshot(
            "retention.max_age_days = 3\ntui.on_exit = \"ask\"\n",
            vec![layers::source(Layer::User, Some(PathBuf::from("/u")), "x")],
        );
        let c = snapshot("tui.on_exit = \"stop\"\n", Vec::new());

        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), c.digest());
        assert_eq!(a.digest().len(), 64);
        assert_eq!(
            a.settings.retention().max_age,
            Some(Duration::from_secs(3 * 24 * 60 * 60))
        );
    }

    #[test]
    fn context_mode_defaults_to_saturn_and_reads_provider() {
        let default = snapshot("", Vec::new());
        let provider = snapshot("[context]\nmode = \"provider\"\n", Vec::new());

        assert_eq!(default.settings.context_mode(), ContextMode::Saturn);
        assert_eq!(provider.settings.context_mode(), ContextMode::Provider);
    }

    #[test]
    fn context_mode_rejects_unknown_values() {
        let sources = vec![
            (
                layers::source(Layer::Default, None, default_layer()),
                default_layer().to_owned(),
            ),
            (
                layers::source(Layer::Chat, None, "[context]\nmode = \"auto\"\n"),
                "[context]\nmode = \"auto\"\n".to_owned(),
            ),
        ];

        assert!(merge(sources).is_err());
    }
}
