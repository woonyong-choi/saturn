//! 층 찾기, 층 병합, 사용자 전용 항목 거르기, 병합 결과 검사, 다른 폴더 설정 참고 읽기.
//!
//! 설계: docs/design/settings.md(설정 층, 폴더 층에서 바꿀 수 없는 항목, 병합과 설정 번호).
//! TOML은 `toml_edit`로 읽어 JSON 표로 옮긴 뒤 합친다. 키 이름, 기본값, 검사 표는 초안이다(TODO(#49)).

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{CONFIG_FILE, Layer, LayerSource, Settings, SettingsError, SettingsSnapshot};
use crate::store::sha256_hex;

/// 폴더 설정 폴더 이름. `<폴더>/.saturn/config.toml`.
const FOLDER_DIR: &str = ".saturn";

/// 실행 층 오류의 경로 표시.
const RUN_LAYER_PATH: &str = "-c";

/// 되돌릴 수 없는 행동의 기준값. 0.8 미만으로 둘 수 없다(docs/design/judge-training.md). 목록은 초안이다(설계는 원칙만 정함).
const IRREVERSIBLE_THRESHOLDS: &[&str] = &[
    "judge.thresholds.keep_current",
    "judge.thresholds.resume_held",
];

/// 되돌릴 수 없는 행동 기준값의 최저값.
const IRREVERSIBLE_MIN: f64 = 0.8;

/// 기본값 층. 질문별 기준값은 docs/design/judge.md 표, 맥락 창 크기는 결정 기록의 확인 값이다.
/// 키 이름과 나머지 값은 초안이다(TODO(#49), 맥락 기준값은 #7 실측 전 초안).
const DEFAULT_LAYER: &str = r#"# Saturn 기본값
on_exit = "background"

[judge]
method = "jev"
endpoint = "https://api.typesafe.ai"
model = "jev-1.13.0"

[judge.key]
storage = "standard"

[judge.thresholds]
keep_current = 0.8
is_actionable = 0.7
min_confidence = 0.6
resume_held = 0.85
file_present = 0.7
file_absent = 0.35
context_gate = 0.3
compact_keep = 0.5
injection = 0.7
progressing = 0.2
feedback_cause = 0.7

[consent]
share_with_server = false

[context]
safety_percent = 70

[context.codex]
t_abs = 200000
window = 272000
cache_read = 0.1
cache_write = 1.0
cache_ttl_secs = 300

[context.claude]
t_abs = 200000
window = 1000000
cache_read = 0.1
cache_write = 1.25
cache_ttl_secs = 300
"#;

/// 값 검사 종류.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// 문자열.
    Text,
    /// 정해진 문자열 중 하나.
    OneOf(&'static [&'static str]),
    /// 문자열 배열.
    TextList,
    /// 참·거짓.
    Flag,
    /// 0~1 실수.
    Unit,
    /// 0 이상 실수.
    NonNegative,
    /// 1 이상 정수.
    Positive,
    /// 0~100 정수.
    Percent,
}

/// 아는 키와 값 종류. 이 밖의 키는 모르는 키로 검사에 실패한다(초안, TODO(#49)).
const SCHEMA: &[(&str, Kind)] = &[
    ("on_exit", Kind::OneOf(&["background", "stop", "ask"])),
    ("judge.method", Kind::OneOf(&["jev", "saturn", "collect"])),
    ("judge.endpoint", Kind::Text),
    (
        "judge.key.info.source",
        Kind::OneOf(&["Stored", "Stdin", "Env", "Command"]),
    ),
    ("judge.key.info.last4", Kind::Text),
    ("judge.key.command", Kind::TextList),
    ("judge.key.storage", Kind::OneOf(&["standard", "hardened"])),
    ("judge.thresholds.keep_current", Kind::Unit),
    ("judge.thresholds.is_actionable", Kind::Unit),
    ("judge.thresholds.min_confidence", Kind::Unit),
    ("judge.thresholds.resume_held", Kind::Unit),
    ("judge.thresholds.file_present", Kind::Unit),
    ("judge.thresholds.file_absent", Kind::Unit),
    ("judge.thresholds.context_gate", Kind::Unit),
    ("judge.thresholds.compact_keep", Kind::Unit),
    ("judge.thresholds.injection", Kind::Unit),
    ("judge.thresholds.progressing", Kind::Unit),
    ("judge.thresholds.feedback_cause", Kind::Unit),
    ("judge.model", Kind::Text),
    ("judge.local.endpoint", Kind::Text),
    ("judge.local.version", Kind::Text),
    ("judge.skip_check", Kind::Flag),
    ("grading.model", Kind::Text),
    ("consent.share_with_server", Kind::Flag),
    ("retention.max_age_days", Kind::Positive),
    ("context.safety_percent", Kind::Percent),
    ("context.codex.t_abs", Kind::Positive),
    ("context.codex.window", Kind::Positive),
    ("context.codex.cache_read", Kind::NonNegative),
    ("context.codex.cache_write", Kind::NonNegative),
    ("context.codex.cache_ttl_secs", Kind::Positive),
    ("context.claude.t_abs", Kind::Positive),
    ("context.claude.window", Kind::Positive),
    ("context.claude.cache_read", Kind::NonNegative),
    ("context.claude.cache_write", Kind::NonNegative),
    ("context.claude.cache_ttl_secs", Kind::Positive),
];

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
    /// 이 항목의 점 경로 키 접두사. 이 접두사와 같거나 `접두사.`로 시작하는 키는 모두 사용자 전용이다.
    /// 키 이름은 초안이다(TODO(#49)).
    pub fn key_prefix(self) -> &'static str {
        match self {
            Self::JudgeEndpoint => "judge.endpoint",
            Self::JudgeKeyRef => "judge.key",
            Self::GradingModel => "grading.model",
            Self::DataSharingConsent => "consent",
            Self::Method => "judge.method",
        }
    }

    /// 점 경로 키가 이 항목에 속하는지.
    fn covers(self, key: &str) -> bool {
        let prefix = self.key_prefix();
        key == prefix
            || key
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
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
    let home = canonical(saturn_home);
    let git_root = workdir
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf);
    let last = git_root.unwrap_or_else(|| workdir.to_path_buf());
    for dir in workdir.ancestors() {
        let folder = dir.join(FOLDER_DIR);
        let candidate = folder.join(CONFIG_FILE);
        let is_user_layer = canonical(&folder) == home;
        let exists = candidate.try_exists().map_err(|source| SettingsError::Io {
            path: candidate.clone(),
            source,
        })?;
        if exists && !is_user_layer {
            return Ok(Some(candidate));
        }
        if dir == last {
            break;
        }
    }
    Ok(None)
}

/// 층을 순서대로 합치고 검사한다. `layers`는 `(출처, TOML 원문)`이고 `Layer` 순서로 정렬돼 있어야 한다.
/// 폴더 층의 `USER_ONLY` 키는 버리고 `LayerSource::ignored`에 적는다. 표는 키 단위로 합치고 나머지는 뒤 층 값으로 바꾼다.
/// 검사: 모르는 키, 타입 불일치, 기준값 0~1 밖, 되돌릴 수 없는 행동 기준값 0.8 미만이면 실패.
///
/// # Errors
/// 문법 오류면 `Parse`, 검사 실패면 `Invalid`.
pub fn merge(mut layers: Vec<(LayerSource, String)>) -> Result<SettingsSnapshot, SettingsError> {
    layers.sort_by_key(|(source, _)| source.layer);
    let mut parsed = Vec::with_capacity(layers.len());
    for (mut source, content) in layers {
        let mut values = parse_toml(&content, &layer_path(&source))?;
        if source.layer == Layer::Folder {
            source.ignored = strip_user_only(&mut values);
        }
        parsed.push((source, values));
    }
    let mut merged = Map::new();
    for (_, values) in &parsed {
        merge_into(&mut merged, values);
    }
    if let Err((key, reason)) = validate(&merged) {
        let layer = origin_layer(&parsed, &key);
        return Err(SettingsError::Invalid { key, reason, layer });
    }
    Ok(SettingsSnapshot {
        settings: Settings {
            values: Value::Object(merged),
        },
        layers: parsed.into_iter().map(|(source, _)| source).collect(),
    })
}

/// 실행 `-c key=value` 목록을 실행 층 TOML 원문으로 바꾼다. 값은 TOML 값 문법(`"문자열"`, `0.7`, `true`)으로 읽는다.
/// 같은 키가 여러 번 오면 뒤 값이 이긴다.
///
/// # Errors
/// `=`가 없거나 값이 TOML 값이 아니면 `Parse`(경로는 `-c`).
pub fn run_layer(overrides: &[String]) -> Result<String, SettingsError> {
    let mut doc = toml_edit::DocumentMut::new();
    for (index, item) in overrides.iter().enumerate() {
        let parse_error = |message: String| SettingsError::Parse {
            path: PathBuf::from(RUN_LAYER_PATH),
            line: index + 1,
            message,
        };
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| parse_error(format!("expected key=value: {item}")))?;
        let keys = parse_key(key.trim()).map_err(parse_error)?;
        let value: toml_edit::Value = value
            .trim()
            .parse()
            .map_err(|error: toml_edit::TomlError| parse_error(error.message().to_owned()))?;
        set_path(doc.as_table_mut(), &keys, value);
    }
    Ok(doc.to_string())
}

/// 기본값 층 원문. 실행 파일에 넣은 TOML.
pub fn default_layer() -> &'static str {
    DEFAULT_LAYER
}

/// 작업 폴더가 아닌 폴더의 설정을 참고 자료로만 읽는다. 병합하지 않고 신뢰도 묻지 않으며 원문만 돌려준다.
///
/// # Errors
/// 읽기 실패면 `Io`.
pub async fn read_reference(path: &Path) -> Result<String, SettingsError> {
    std::fs::read_to_string(path).map_err(|source| SettingsError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// 병합 결과에서 사용자 전용 키가 사용자 층(또는 기본값) 값인지 확인한다. 테스트와 디버그 검사용.
/// `merge`는 폴더 층의 사용자 전용 키를 병합 전에 버리므로, 폴더 파일의 사용자 전용 키가 모두 그 층의 `ignored`에
/// 있으면 병합 결과의 사용자 전용 값은 폴더에서 오지 않았다. 폴더 파일을 다시 읽지 못하면 거짓.
pub(crate) fn user_only_from_user_layer(_settings: &Settings, layers: &[LayerSource]) -> bool {
    layers
        .iter()
        .filter(|source| source.layer == Layer::Folder)
        .all(|source| {
            let Some(path) = &source.path else {
                return false;
            };
            let Ok(content) = std::fs::read_to_string(path) else {
                return false;
            };
            let Ok(values) = parse_toml(&content, path) else {
                return false;
            };
            let mut keys = Vec::new();
            leaf_keys(&values, "", &mut keys);
            keys.iter()
                .filter(|key| is_user_only(key))
                .all(|key| source.ignored.contains(key))
        })
}

/// 층 출처 하나를 만든다. 파일 층이면 내용 지문을 함께 계산한다.
pub(crate) fn source(layer: Layer, path: Option<PathBuf>, content: &str) -> LayerSource {
    let fingerprint = path.as_ref().map(|_| fingerprint(content));
    LayerSource {
        layer,
        path,
        fingerprint,
        ignored: Vec::new(),
    }
}

/// 내용 지문. SHA-256 hex(초안, 설계는 지문만 정함).
pub(crate) fn fingerprint(content: &str) -> String {
    sha256_hex(content.as_bytes())
}

/// 사용자 전용 키인지.
pub(crate) fn is_user_only(key: &str) -> bool {
    USER_ONLY.iter().any(|item| item.covers(key))
}

/// TOML 원문을 JSON 표로. 오류 줄은 1부터.
///
/// # Errors
/// 문법 오류면 `Parse`.
pub(crate) fn parse_toml(content: &str, path: &Path) -> Result<Value, SettingsError> {
    let doc: toml_edit::DocumentMut =
        content
            .parse()
            .map_err(|error: toml_edit::TomlError| SettingsError::Parse {
                path: path.to_path_buf(),
                line: error
                    .span()
                    .map(|span| {
                        content[..span.start.min(content.len())]
                            .matches('\n')
                            .count()
                            + 1
                    })
                    .unwrap_or(1),
                message: error.message().to_owned(),
            })?;
    Ok(table_to_json(doc.as_table()))
}

/// 표 안 모든 값의 점 경로 키.
pub(crate) fn leaf_keys(value: &Value, prefix: &str, out: &mut Vec<String>) {
    let Value::Object(map) = value else {
        if !prefix.is_empty() {
            out.push(prefix.to_owned());
        }
        return;
    };
    for (key, child) in map {
        let path = join_key(prefix, key);
        leaf_keys(child, &path, out);
    }
}

/// 점 경로 키로 값 찾기.
pub(crate) fn get_path<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

/// 점 경로 키를 `toml_edit` 키 목록으로.
pub(crate) fn parse_key(key: &str) -> Result<Vec<toml_edit::Key>, String> {
    toml_edit::Key::parse(key).map_err(|error| error.message().to_owned())
}

/// 표 안 점 경로에 값을 넣는다. 중간 표가 없거나 표가 아니면 새 표로 바꾼다. 주석과 다른 키는 그대로 둔다.
pub(crate) fn set_path(
    table: &mut toml_edit::Table,
    keys: &[toml_edit::Key],
    value: toml_edit::Value,
) {
    let Some((last, parents)) = keys.split_last() else {
        return;
    };
    let mut current = table;
    for key in parents {
        let entry = current.entry(key.get()).or_insert_with(|| {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            toml_edit::Item::Table(table)
        });
        if !entry.is_table_like() {
            let mut table = toml_edit::Table::new();
            table.set_implicit(true);
            *entry = toml_edit::Item::Table(table);
        }
        if let Some(inline) = entry.as_inline_table().cloned() {
            *entry = toml_edit::Item::Table(inline.into_table());
        }
        current = entry
            .as_table_mut()
            .expect("entry should be a table after conversion");
    }
    match current.get_mut(last.get()) {
        Some(item) if item.is_value() => {
            let decor = item
                .as_value()
                .map(|old| old.decor().clone())
                .unwrap_or_default();
            let mut value = value;
            *value.decor_mut() = decor;
            *item = toml_edit::Item::Value(value);
        }
        _ => {
            current.insert(last.get(), toml_edit::Item::Value(value));
        }
    }
}

/// 오류 메시지에 쓸 층 경로. 파일 층은 파일 경로, 나머지는 층 이름.
fn layer_path(source: &LayerSource) -> PathBuf {
    match (&source.path, source.layer) {
        (Some(path), _) => path.clone(),
        (None, Layer::Run) => PathBuf::from(RUN_LAYER_PATH),
        (None, Layer::Default) => PathBuf::from("default"),
        (None, Layer::User) => PathBuf::from("user"),
        (None, Layer::Folder) => PathBuf::from("folder"),
        (None, Layer::Chat) => PathBuf::from("chat"),
    }
}

/// 사용자 전용 키를 지우고 지운 키를 돌려준다.
fn strip_user_only(values: &mut Value) -> Vec<String> {
    let mut keys = Vec::new();
    leaf_keys(values, "", &mut keys);
    let ignored: Vec<String> = keys.into_iter().filter(|key| is_user_only(key)).collect();
    for key in &ignored {
        remove_path(values, key);
    }
    for item in USER_ONLY {
        remove_path(values, item.key_prefix());
    }
    ignored
}

/// 점 경로 키 값을 지운다. 비게 된 표는 남겨 둔다(병합에 영향이 없다).
fn remove_path(value: &mut Value, key: &str) {
    let Some((parent, last)) = key.rsplit_once('.') else {
        if let Value::Object(map) = value {
            map.remove(key);
        }
        return;
    };
    let target = parent.split('.').try_fold(value, |current, segment| {
        current.as_object_mut()?.get_mut(segment)
    });
    if let Some(Value::Object(map)) = target {
        map.remove(last);
    }
}

/// 뒤 층 값을 앞 층 위에 합친다. 표는 키 단위로, 나머지는 통째로 바꾼다.
fn merge_into(base: &mut Map<String, Value>, layer: &Value) {
    let Value::Object(layer) = layer else {
        return;
    };
    for (key, value) in layer {
        match (base.get_mut(key), value) {
            (Some(Value::Object(existing)), Value::Object(_)) => merge_into(existing, value),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

/// 병합 결과 검사. 실패하면 점 경로 키와 이유.
fn validate(merged: &Map<String, Value>) -> Result<(), (String, String)> {
    let mut keys = Vec::new();
    leaf_keys(&Value::Object(merged.clone()), "", &mut keys);
    for key in &keys {
        let value = get_path_map(merged, key).expect("leaf key should exist in merged table");
        let Some(kind) = SCHEMA
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, kind)| *kind)
        else {
            let is_table = SCHEMA.iter().any(|(name, _)| {
                name.strip_prefix(key.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
            });
            let reason = if is_table {
                "expected a table"
            } else {
                "unknown key"
            };
            return Err((key.clone(), reason.to_owned()));
        };
        check_kind(kind, value).map_err(|reason| (key.clone(), reason))?;
    }
    for key in IRREVERSIBLE_THRESHOLDS {
        let value = get_path_map(merged, key).and_then(Value::as_f64);
        if value.is_some_and(|value| value < IRREVERSIBLE_MIN) {
            return Err((
                (*key).to_owned(),
                format!("irreversible action threshold should be at least {IRREVERSIBLE_MIN}"),
            ));
        }
    }
    Ok(())
}

/// 값 종류 검사.
fn check_kind(kind: Kind, value: &Value) -> Result<(), String> {
    let ok = match kind {
        Kind::Text => value.is_string(),
        Kind::OneOf(options) => {
            let text = value.as_str().ok_or("expected a string")?;
            if !options.contains(&text) {
                return Err(format!("expected one of {}", options.join(", ")));
            }
            true
        }
        Kind::TextList => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        Kind::Flag => value.is_boolean(),
        Kind::Unit => {
            let number = value.as_f64().ok_or("expected a number")?;
            if !(0.0..=1.0).contains(&number) {
                return Err("expected a value between 0 and 1".to_owned());
            }
            true
        }
        Kind::NonNegative => value.as_f64().is_some_and(|number| number >= 0.0),
        Kind::Positive => value.as_u64().is_some_and(|number| number >= 1),
        Kind::Percent => value.as_u64().is_some_and(|number| number <= 100),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("expected {}", kind_name(kind)))
    }
}

/// 오류 메시지의 종류 이름.
fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text | Kind::OneOf(_) => "a string",
        Kind::TextList => "an array of strings",
        Kind::Flag => "a boolean",
        Kind::Unit => "a number between 0 and 1",
        Kind::NonNegative => "a non-negative number",
        Kind::Positive => "a positive integer",
        Kind::Percent => "an integer between 0 and 100",
    }
}

/// 검사에 실패한 키를 마지막으로 정한 층.
fn origin_layer(parsed: &[(LayerSource, Value)], key: &str) -> Layer {
    parsed
        .iter()
        .rev()
        .find(|(_, values)| get_path(values, key).is_some())
        .map_or(Layer::Default, |(source, _)| source.layer)
}

fn get_path_map<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    let (first, rest) = match key.split_once('.') {
        Some((first, rest)) => (first, Some(rest)),
        None => (key, None),
    };
    let value = map.get(first)?;
    match rest {
        Some(rest) => get_path(value, rest),
        None => Some(value),
    }
}

fn join_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

fn table_to_json(table: &dyn toml_edit::TableLike) -> Value {
    let map = table
        .iter()
        .filter_map(|(key, item)| item_to_json(item).map(|value| (key.to_owned(), value)))
        .collect();
    Value::Object(map)
}

fn item_to_json(item: &toml_edit::Item) -> Option<Value> {
    match item {
        toml_edit::Item::None => None,
        toml_edit::Item::Value(value) => Some(value_to_json(value)),
        toml_edit::Item::Table(table) => Some(table_to_json(table)),
        toml_edit::Item::ArrayOfTables(tables) => Some(Value::Array(
            tables.iter().map(|table| table_to_json(table)).collect(),
        )),
    }
}

fn value_to_json(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(text) => Value::String(text.value().clone()),
        toml_edit::Value::Integer(number) => Value::from(*number.value()),
        toml_edit::Value::Float(number) => serde_json::Number::from_f64(*number.value())
            .map_or_else(|| Value::String(number.value().to_string()), Value::Number),
        toml_edit::Value::Boolean(flag) => Value::Bool(*flag.value()),
        toml_edit::Value::Datetime(datetime) => Value::String(datetime.value().to_string()),
        toml_edit::Value::Array(items) => Value::Array(items.iter().map(value_to_json).collect()),
        toml_edit::Value::InlineTable(table) => table_to_json(table),
    }
}

/// 심볼릭 링크를 푼 경로. 없는 경로면 그대로.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(layer: Layer, content: &str) -> (LayerSource, String) {
        let path = matches!(layer, Layer::User | Layer::Folder)
            .then(|| PathBuf::from(format!("/{layer:?}/.saturn/config.toml")));
        (source(layer, path, content), content.to_owned())
    }

    #[test]
    fn later_layer_wins_in_order() {
        let key = "judge.thresholds.is_actionable";
        let layers = vec![
            layer(Layer::Run, &run_layer(&[format!("{key}=0.95")]).unwrap()),
            layer(Layer::Default, default_layer()),
            layer(Layer::User, "[judge.thresholds]\nis_actionable = 0.71\n"),
            layer(Layer::Folder, "[judge.thresholds]\nis_actionable = 0.72\n"),
            layer(Layer::Chat, "judge.thresholds.is_actionable = 0.73\n"),
        ];

        let snapshot = merge(layers).unwrap();

        assert_eq!(snapshot.settings.thresholds().is_actionable, 0.95);
        let order: Vec<Layer> = snapshot.layers.iter().map(|source| source.layer).collect();
        assert_eq!(
            order,
            vec![
                Layer::Default,
                Layer::User,
                Layer::Folder,
                Layer::Chat,
                Layer::Run
            ]
        );
        assert_eq!(snapshot.settings.thresholds().keep_current, 0.8);
    }

    #[test]
    fn folder_cannot_change_user_only_items() {
        let folder = "on_exit = \"ask\"\n\
            [judge]\nmethod = \"saturn\"\nendpoint = \"https://evil.example\"\n\
            [judge.key]\ncommand = [\"steal\"]\n\
            [grading]\nmodel = \"other\"\n\
            [consent]\nshare_with_server = true\n";
        let layers = vec![
            layer(Layer::Default, default_layer()),
            layer(Layer::User, "[grading]\nmodel = \"mine\"\n"),
            layer(Layer::Folder, folder),
        ];

        let snapshot = merge(layers).unwrap();

        let settings = &snapshot.settings;
        assert_eq!(settings.method(), saturn_core::judges::Method::Jev);
        assert_eq!(settings.judge_endpoint(), "https://api.typesafe.ai");
        assert_eq!(settings.key_command(), None);
        assert_eq!(settings.grading_model(), Some("mine"));
        assert!(!settings.share_with_server());
        assert_eq!(settings.on_exit(), saturn_protocol::state::OnExit::Ask);
        let mut ignored = snapshot.layers[2].ignored.clone();
        ignored.sort();
        assert_eq!(
            ignored,
            vec![
                "consent.share_with_server",
                "grading.model",
                "judge.endpoint",
                "judge.key.command",
                "judge.method",
            ]
        );
    }

    #[test]
    fn validation_rejects_bad_values_with_origin_layer() {
        let cases = [
            ("unknown_key = 1\n", "unknown_key"),
            (
                "judge.thresholds.injection = 1.5\n",
                "judge.thresholds.injection",
            ),
            (
                "judge.thresholds.keep_current = 0.75\n",
                "judge.thresholds.keep_current",
            ),
            ("judge.thresholds = 0.5\n", "judge.thresholds"),
            ("on_exit = \"later\"\n", "on_exit"),
            ("context.safety_percent = 120\n", "context.safety_percent"),
        ];
        for (content, expected) in cases {
            let layers = vec![
                layer(Layer::Default, default_layer()),
                layer(Layer::Chat, content),
            ];

            let error = merge(layers).unwrap_err();

            let SettingsError::Invalid { key, layer, .. } = error else {
                panic!("{content} should be invalid");
            };
            assert_eq!(key, expected);
            assert_eq!(layer, Layer::Chat);
        }
    }

    #[test]
    fn parse_error_reports_line() {
        let layers = vec![
            layer(Layer::Default, default_layer()),
            layer(Layer::Folder, "[judge]\n\n\nbroken = = 1\n"),
        ];

        let error = merge(layers).unwrap_err();

        assert!(
            matches!(error, SettingsError::Parse { line: 4, .. }),
            "{error:?}"
        );
    }

    #[test]
    fn run_layer_parses_values_and_rejects_bad_input() {
        let text = run_layer(&[
            "judge.thresholds.injection=0.9".to_owned(),
            "judge.thresholds.injection = 0.95".to_owned(),
            "on_exit=\"ask\"".to_owned(),
        ])
        .unwrap();
        let values = parse_toml(&text, Path::new("-c")).unwrap();

        assert_eq!(
            get_path(&values, "judge.thresholds.injection"),
            Some(&Value::from(0.95))
        );
        assert_eq!(get_path(&values, "on_exit"), Some(&Value::from("ask")));
        for bad in ["no_equals", "key=not a value"] {
            let error = run_layer(&[bad.to_owned()]).unwrap_err();
            assert!(matches!(error, SettingsError::Parse { .. }), "{bad}");
        }
    }

    #[test]
    fn default_layer_is_valid_and_matches_design_thresholds() {
        let snapshot = merge(vec![layer(Layer::Default, default_layer())]).unwrap();

        let thresholds = snapshot.settings.thresholds();
        assert_eq!(thresholds.keep_current, 0.8);
        assert_eq!(thresholds.resume_held, 0.85);
        assert_eq!(thresholds.file_relevant, (0.7, 0.35));
        assert_eq!(snapshot.settings.retention().max_age, None);
        let budget = snapshot
            .settings
            .context_budget(saturn_protocol::ids::Provider::Claude);
        assert_eq!(budget.window, 1_000_000);
    }

    #[test]
    fn user_only_prefix_matches_whole_segments() {
        assert!(is_user_only("judge.key.info.last4"));
        assert!(is_user_only("consent.share_with_server"));
        assert!(!is_user_only("judge.keys"));
        assert!(!is_user_only("judge.thresholds.keep_current"));
    }

    #[tokio::test]
    async fn folder_config_search_stops_at_git_root() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let nested = repo.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let home = root.path().join("home").join(".saturn");
        std::fs::create_dir_all(&home).unwrap();
        let outside = root.path().join(FOLDER_DIR);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join(CONFIG_FILE), "").unwrap();

        assert_eq!(find_folder_config(&nested, &home).await.unwrap(), None);
        let repo_config = repo.join(FOLDER_DIR).join(CONFIG_FILE);
        std::fs::create_dir_all(repo_config.parent().unwrap()).unwrap();
        std::fs::write(&repo_config, "").unwrap();

        assert_eq!(
            find_folder_config(&nested, &home).await.unwrap(),
            Some(repo_config)
        );
    }

    #[tokio::test]
    async fn folder_config_search_skips_user_layer_and_non_git_parents() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join(FOLDER_DIR);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(CONFIG_FILE), "").unwrap();
        let workdir = root.path().join("work");
        std::fs::create_dir_all(&workdir).unwrap();

        assert_eq!(find_folder_config(root.path(), &home).await.unwrap(), None);
        assert_eq!(find_folder_config(&workdir, &home).await.unwrap(), None);
    }

    #[tokio::test]
    async fn reference_read_returns_raw_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "# note\nx = 1\n").unwrap();

        assert_eq!(read_reference(&path).await.unwrap(), "# note\nx = 1\n");
    }
}
