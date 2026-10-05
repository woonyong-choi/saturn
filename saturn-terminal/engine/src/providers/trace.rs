//! provider 원시 메시지 관측 기록(`debug.provider_events`). 연결이 받은 메시지의 모양(방법 이름, 순서, thread와 요청 ID,
//! 필드 이름)만 engine 로그 옆 파일에 한 줄씩 남기고 값은 남기지 않는다.
//! 설계: docs/design/providers-and-sessions.md#원시-이벤트-관측-기록

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use chrono::{Duration, Local, NaiveDate, Utc};
use saturn_protocol::ids::{ChatId, Provider};
use serde_json::{Map, Value, json};

use crate::secrets::Masker;

const FILE_PREFIX: &str = "provider-events-";
const FILE_SUFFIX: &str = ".log";
const DATE_FORMAT: &str = "%Y-%m-%d";

/// 이 일수보다 오래된 날짜의 파일을 지운다. engine 로그와 같은 보관 기간이다.
const KEEP_DAYS: i64 = 30;

/// 메시지 한 줄에 남기는 필드 이름 수의 상한. 넘으면 `truncated`를 적는다.
const MAX_FIELDS: usize = 300;

/// 필드 이름을 따라 내려가는 깊이의 상한.
const MAX_DEPTH: usize = 8;

/// 이름에 쓸 수 있는 최대 글자 수. 방법 이름과 필드 이름 모두에 적용한다.
const MAX_NAME_LEN: usize = 80;

/// ID 값의 최대 글자 수. 넘는 값은 ID로 보지 않고 남기지 않는다.
const MAX_ID_LEN: usize = 128;

/// 메시지가 연결에서 어떤 자리로 왔는지. 어댑터가 provider의 형식에 맞춰 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// provider가 보낸 요청(승인 요청 등). 응답을 기다린다.
    Request,
    Notification,
    /// 우리 요청에 대한 응답.
    Response,
    /// 줄 단위 스트림의 한 줄. 요청과 알림을 가르지 않는 provider가 쓴다.
    Line,
}

impl Frame {
    fn name(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Notification => "notification",
            Self::Response => "response",
            Self::Line => "line",
        }
    }
}

/// 연결 하나가 받은 메시지를 기록하는 손잡이. 꺼져 있으면 아무것도 하지 않고 파일도 만들지 않는다.
/// 켜고 끄는 값은 채팅마다 둔 하나를 연결들이 함께 보므로, 설정을 바꾸면 연결을 다시 열지 않아도 바로 따른다.
#[derive(Debug, Clone, Default)]
pub struct ProviderTrace {
    link: Option<Arc<Link>>,
}

#[derive(Debug)]
struct Link {
    sink: Arc<Sink>,
    enabled: Arc<AtomicBool>,
    provider: Provider,
    /// 같은 engine에서 연결마다 다르다. 한 파일에서 여러 연결의 줄이 섞여도 가를 수 있다.
    connection: u64,
    masker: Masker,
}

impl ProviderTrace {
    /// 기록하지 않는 손잡이.
    pub fn off() -> Self {
        Self::default()
    }

    pub fn is_enabled(&self) -> bool {
        self.link
            .as_ref()
            .is_some_and(|link| link.enabled.load(Ordering::Relaxed))
    }

    /// `kind`는 어댑터가 정한 방법 이름이고, 이름으로 쓸 수 없는 글자가 있으면 `?`로 적는다. `message`는 이미 router 키를
    /// 가린 값이어도 다시 가린다. 쓰기에 실패해도 연결을 막지 않는다.
    pub fn record(&self, frame: Frame, kind: &str, message: &Value) {
        let Some(link) = self.link.as_ref().filter(|link| link.is_on()) else {
            return;
        };
        let shape = Shape::of(message);
        let masker = &link.masker;
        let mask = |text: &str| masker.mask(text).as_str().to_owned();
        let ids: Map<String, Value> = shape
            .ids
            .iter()
            .map(|(path, values)| {
                let values: Vec<String> = values.iter().map(|value| mask(value)).collect();
                (mask(path), json!(values))
            })
            .collect();
        let fields: Vec<String> = shape.fields.iter().map(|field| mask(field)).collect();
        let mut line = json!({
            "at": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "provider": link.provider.to_string(),
            "connection": link.connection,
            "frame": frame.name(),
            "kind": mask(&safe_kind(kind)),
            "ids": ids,
            "fields": fields,
        });
        if shape.truncated {
            line["truncated"] = Value::Bool(true);
        }
        if let Err(error) = link.sink.append(&line) {
            tracing::debug!(kind = ?error.kind(), "failed to write the provider event trace");
        }
    }
}

impl Link {
    fn is_on(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// engine 하나가 가지고 연결마다 손잡이를 나눠 주는 곳. 켜고 끄는 값은 채팅마다 하나다.
#[derive(Debug)]
pub(crate) struct TraceHub {
    sink: Arc<Sink>,
    flags: Mutex<HashMap<ChatId, Arc<AtomicBool>>>,
    connections: AtomicU64,
}

impl TraceHub {
    /// `home` 아래 로그 폴더에 쓴다. 켜기 전에는 폴더도 파일도 만들지 않는다.
    pub(crate) fn new(home: &Path) -> Self {
        Self {
            sink: Arc::new(Sink::new(home.join(saturn_protocol::home::LOG_DIR))),
            flags: Mutex::new(HashMap::new()),
            connections: AtomicU64::new(0),
        }
    }

    /// 채팅의 켜짐 값을 바꾼다. 이미 열린 연결의 손잡이도 바로 따른다.
    pub(crate) fn set_enabled(&self, chat: ChatId, on: bool) {
        self.flag_of(chat).store(on, Ordering::Relaxed);
    }

    /// 새 연결의 손잡이.
    pub(crate) fn link(&self, chat: ChatId, provider: Provider, masker: &Masker) -> ProviderTrace {
        ProviderTrace {
            link: Some(Arc::new(Link {
                sink: Arc::clone(&self.sink),
                enabled: self.flag_of(chat),
                provider,
                connection: self.connections.fetch_add(1, Ordering::Relaxed) + 1,
                masker: masker.clone(),
            })),
        }
    }

    fn flag_of(&self, chat: ChatId) -> Arc<AtomicBool> {
        let mut flags = self.flags.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(flags.entry(chat).or_default())
    }
}

/// 날짜별 파일 하나에 줄 순서대로 쓴다. 파일은 처음 쓸 때 만든다.
#[derive(Debug)]
struct Sink {
    dir: PathBuf,
    state: Mutex<SinkState>,
}

#[derive(Debug, Default)]
struct SinkState {
    next_seq: u64,
    open: Option<(NaiveDate, File)>,
}

impl Sink {
    fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            state: Mutex::new(SinkState::default()),
        }
    }

    /// 줄 번호는 쓰는 순서와 같다. 번호를 정하고 쓰는 일을 한 잠금 안에서 해서 파일의 줄 순서와 번호 순서가 어긋나지 않는다.
    fn append(&self, line: &Value) -> std::io::Result<()> {
        self.append_on(Local::now().date_naive(), line)
    }

    fn append_on(&self, today: NaiveDate, line: &Value) -> std::io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.open.as_ref().is_none_or(|(date, _)| *date != today) {
            std::fs::create_dir_all(&self.dir)?;
            // 정리 실패가 기록을 막지 않게 한다. 지우지 못한 파일은 다음 날짜 변경 때 다시 지운다
            let _ = remove_expired_files(&self.dir, today);
            state.open = Some((today, open_file(&self.dir, today)?));
        }
        state.next_seq += 1;
        let seq = state.next_seq;
        let mut numbered = Map::new();
        numbered.insert("seq".to_owned(), Value::from(seq));
        if let Value::Object(fields) = line {
            numbered.extend(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        let mut text = Value::Object(numbered).to_string();
        text.push('\n');
        match state.open.as_mut() {
            Some((_, file)) => file.write_all(text.as_bytes()),
            None => Ok(()),
        }
    }
}

fn file_name(date: NaiveDate) -> String {
    format!("{FILE_PREFIX}{}{FILE_SUFFIX}", date.format(DATE_FORMAT))
}

fn date_of(name: &str) -> Option<NaiveDate> {
    let text = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
    let date = NaiveDate::parse_from_str(text, DATE_FORMAT).ok()?;
    (file_name(date) == name).then_some(date)
}

fn open_file(dir: &Path, date: NaiveDate) -> std::io::Result<File> {
    let path = dir.join(file_name(date));
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)?;
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 로그 폴더의 파일 수
// basis: estimate
/// `today`보다 `KEEP_DAYS`일 넘게 앞선 날짜의 관측 기록 파일만 지운다.
fn remove_expired_files(dir: &Path, today: NaiveDate) -> std::io::Result<()> {
    let oldest_kept = today - Duration::days(KEEP_DAYS);
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let is_expired = entry
            .file_name()
            .to_str()
            .and_then(date_of)
            .is_some_and(|date| date < oldest_kept);
        if is_expired {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// 메시지에서 뽑은 모양. 값은 ID 필드의 글자뿐이다.
#[derive(Debug, Default, PartialEq, Eq)]
struct Shape {
    /// ID 필드의 경로와 값.
    ids: BTreeMap<String, Vec<String>>,
    /// 필드 이름 경로. 배열 칸은 `[]`로 적는다.
    fields: BTreeSet<String>,
    truncated: bool,
}

impl Shape {
    fn of(message: &Value) -> Self {
        let mut shape = Self::default();
        shape.walk(message, "", 0);
        shape
    }

    fn walk(&mut self, value: &Value, path: &str, depth: usize) {
        if depth >= MAX_DEPTH {
            return;
        }
        match value {
            Value::Object(fields) => {
                for (key, child) in fields {
                    let name = safe_field(key);
                    let child_path = if path.is_empty() {
                        name
                    } else {
                        format!("{path}.{name}")
                    };
                    if self.fields.len() >= MAX_FIELDS {
                        self.truncated = true;
                        return;
                    }
                    self.fields.insert(child_path.clone());
                    if is_id_key(key) {
                        self.take_ids(child, &child_path);
                    }
                    self.walk(child, &child_path, depth + 1);
                }
            }
            Value::Array(items) => {
                let item_path = format!("{path}[]");
                for item in items {
                    self.walk(item, &item_path, depth + 1);
                }
            }
            _ => {}
        }
    }

    /// ID 필드의 값이 글자나 숫자이거나 그 배열일 때만 남긴다.
    fn take_ids(&mut self, value: &Value, path: &str) {
        let values: Vec<String> = match value {
            Value::String(text) => vec![text.clone()],
            Value::Number(number) => vec![number.to_string()],
            Value::Array(items) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let values: Vec<String> = values
            .into_iter()
            .filter(|text| !text.is_empty() && text.len() <= MAX_ID_LEN)
            .collect();
        if !values.is_empty() {
            self.ids.entry(path.to_owned()).or_default().extend(values);
        }
    }
}

/// `id`, `...Id`, `...Ids`, `..._id`, `..._ids`.
fn is_id_key(key: &str) -> bool {
    key == "id"
        || ["Id", "Ids", "_id", "_ids"]
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

/// 식별자 모양의 필드 이름만 그대로 쓴다. 값이 이름 자리에 들어온 경우(경로나 ID를 키로 쓴 표)를 걸러 `*`로 적는다.
/// 영문자나 밑줄로 시작하고 영문자, 숫자, 밑줄뿐이며 숫자가 6개 이어지지 않는 이름이다.
fn safe_field(key: &str) -> String {
    let mut chars = key.chars();
    let starts_well = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_');
    let is_plain = key.len() <= MAX_NAME_LEN
        && starts_well
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && !has_digit_run(key, 6);
    if is_plain {
        key.to_owned()
    } else {
        "*".to_owned()
    }
}

fn has_digit_run(text: &str, length: usize) -> bool {
    let mut run = 0;
    for ch in text.chars() {
        run = if ch.is_ascii_digit() { run + 1 } else { 0 };
        if run >= length {
            return true;
        }
    }
    false
}

/// 방법 이름에 쓰는 글자(`/`, `.`, `:`, `_`, `-`와 영문자, 숫자)만 있는 짧은 이름이면 그대로, 아니면 `?`.
fn safe_kind(kind: &str) -> String {
    let is_plain = !kind.is_empty()
        && kind.len() <= MAX_NAME_LEN
        && kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | ':' | '_' | '-'));
    if is_plain {
        kind.to_owned()
    } else {
        "?".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::test_support::CODEX;

    fn lines_of(dir: &Path) -> Vec<Value> {
        let logs = dir.join("logs");
        let Ok(entries) = std::fs::read_dir(&logs) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(FILE_PREFIX))
            })
            .collect();
        files.sort();
        files
            .iter()
            .flat_map(|path| {
                std::fs::read_to_string(path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn text_of(dir: &Path) -> String {
        let logs = dir.join("logs");
        let Ok(entries) = std::fs::read_dir(&logs) else {
            return String::new();
        };
        entries
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect()
    }

    fn started(dir: &Path, masker: &Masker, on: bool) -> (TraceHub, ProviderTrace) {
        let hub = TraceHub::new(dir);
        hub.set_enabled(ChatId(1), on);
        let trace = hub.link(ChatId(1), CODEX, masker);
        (hub, trace)
    }

    fn thread_closed(thread: &str) -> Value {
        json!({ "method": "thread/closed", "params": { "threadId": thread } })
    }

    #[test]
    fn off_writes_nothing_and_creates_no_file() {
        let home = tempfile::tempdir().unwrap();
        let (_hub, trace) = started(home.path(), &Masker::default(), false);

        trace.record(Frame::Notification, "thread/closed", &thread_closed("t-1"));
        ProviderTrace::off().record(Frame::Line, "x", &json!({}));

        assert!(!trace.is_enabled());
        assert!(!home.path().join("logs").exists());
    }

    #[test]
    fn on_records_order_ids_and_field_names_without_values() {
        let home = tempfile::tempdir().unwrap();
        let (_hub, trace) = started(home.path(), &Masker::default(), true);
        let started_child = json!({
            "method": "thread/started",
            "params": { "thread": { "id": "child-1", "cwd": "/Users/someone/secret-project" } },
        });
        let approval = json!({
            "id": 7,
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "child-1",
                "command": "cat /etc/passwd",
                "item": { "receiverThreadIds": ["child-1", "child-2"] },
            },
        });

        trace.record(Frame::Notification, "thread/started", &started_child);
        trace.record(
            Frame::Request,
            "item/commandExecution/requestApproval",
            &approval,
        );
        trace.record(
            Frame::Notification,
            "thread/closed",
            &thread_closed("child-1"),
        );

        let lines = lines_of(home.path());
        assert_eq!(lines.len(), 3);
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "thread/started",
                "item/commandExecution/requestApproval",
                "thread/closed"
            ]
        );
        let seqs: Vec<u64> = lines.iter().map(|l| l["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2, 3]);
        assert_eq!(lines[0]["provider"], "codex");
        assert_eq!(lines[1]["frame"], "request");
        assert_eq!(lines[0]["ids"]["params.thread.id"], json!(["child-1"]));
        assert_eq!(lines[1]["ids"]["id"], json!(["7"]));
        assert_eq!(
            lines[1]["ids"]["params.item.receiverThreadIds"],
            json!(["child-1", "child-2"])
        );
        let fields = lines[1]["fields"].as_array().unwrap();
        assert!(fields.contains(&json!("params.command")));
        assert!(fields.contains(&json!("params.item.receiverThreadIds")));
        let text = text_of(home.path());
        for value in ["/Users/someone", "secret-project", "/etc/passwd", "cat "] {
            assert!(!text.contains(value), "{value} leaked into the trace");
        }
    }

    #[test]
    fn router_key_does_not_appear_even_in_names_and_ids() {
        let home = tempfile::tempdir().unwrap();
        let masker = Masker::new(vec!["sk_live_abc123".to_owned()]);
        let (_hub, trace) = started(home.path(), &masker, true);
        let message = json!({
            "method": "item/started",
            "params": {
                "threadId": "thread-sk_live_abc123",
                "sk_live_abc123": "value",
                "text": "uses sk_live_abc123 here",
            },
        });

        trace.record(Frame::Notification, "item/started", &message);
        trace.record(Frame::Notification, "sk_live_abc123", &message);

        let text = text_of(home.path());
        assert!(!text.contains("sk_live_abc123"));
        assert!(text.contains("[redacted]"));
        assert!(!text.contains("uses "));
    }

    #[test]
    fn data_used_as_a_key_is_not_written_as_a_field_name() {
        let message = json!({
            "params": {
                "changes": {
                    "/Users/someone/project/file.rs": { "kind": "update" },
                    "019a2b3c-0000-1111-2222-333344445555": { "kind": "add" },
                    "123456789": { "kind": "delete" },
                },
            },
        });

        let shape = Shape::of(&message);

        assert!(shape.fields.contains("params.changes.*.kind"));
        assert!(!shape.fields.iter().any(|field| field.contains("Users")));
        assert!(!shape.fields.iter().any(|field| field.contains("019a")));
        assert!(!shape.fields.iter().any(|field| field.contains("123456789")));
    }

    #[test]
    fn unusable_kind_is_written_as_a_placeholder() {
        let home = tempfile::tempdir().unwrap();
        let (_hub, trace) = started(home.path(), &Masker::default(), true);

        trace.record(Frame::Line, "has space and /path/to/file", &json!({}));
        trace.record(Frame::Line, "system/init", &json!({}));

        let lines = lines_of(home.path());
        assert_eq!(lines[0]["kind"], "?");
        assert_eq!(lines[1]["kind"], "system/init");
    }

    #[test]
    fn switching_the_flag_applies_to_open_connections_at_once() {
        let home = tempfile::tempdir().unwrap();
        let (hub, trace) = started(home.path(), &Masker::default(), false);
        trace.record(Frame::Line, "before", &json!({}));

        hub.set_enabled(ChatId(1), true);
        trace.record(Frame::Line, "while-on", &json!({}));
        hub.set_enabled(ChatId(1), false);
        trace.record(Frame::Line, "after", &json!({}));

        let lines = lines_of(home.path());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["kind"], "while-on");
    }

    #[test]
    fn connections_are_told_apart_and_share_one_order() {
        let home = tempfile::tempdir().unwrap();
        let hub = TraceHub::new(home.path());
        hub.set_enabled(ChatId(1), true);
        hub.set_enabled(ChatId(2), true);
        let first = hub.link(ChatId(1), CODEX, &Masker::default());
        let second = hub.link(ChatId(2), CODEX, &Masker::default());

        first.record(Frame::Line, "a", &json!({}));
        second.record(Frame::Line, "b", &json!({}));
        first.record(Frame::Line, "c", &json!({}));

        let lines = lines_of(home.path());
        let order: Vec<(u64, u64)> = lines
            .iter()
            .map(|l| {
                (
                    l["seq"].as_u64().unwrap(),
                    l["connection"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(order, [(1, 1), (2, 2), (3, 1)]);
    }

    #[test]
    fn file_is_owner_only_and_old_files_are_removed() {
        let home = tempfile::tempdir().unwrap();
        let logs = home.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let old = logs.join(file_name(today - Duration::days(KEEP_DAYS + 1)));
        let kept = logs.join(file_name(today - Duration::days(KEEP_DAYS)));
        let engine_log = logs.join("engine-2020-01-01.log");
        for path in [&old, &kept, &engine_log] {
            std::fs::write(path, "x").unwrap();
        }
        let sink = Sink::new(logs.clone());

        sink.append_on(today, &json!({ "kind": "x" })).unwrap();

        assert!(!old.exists());
        assert!(kept.exists());
        assert!(
            engine_log.exists(),
            "engine log files are not ours to remove"
        );
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(logs.join(file_name(today)))
                .unwrap()
                .permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn field_count_is_capped() {
        let fields: Map<String, Value> = (0..(MAX_FIELDS + 50))
            .map(|index| {
                (
                    format!("field{}", "a".repeat(index % 7)) + &index.to_string(),
                    json!(1),
                )
            })
            .collect();
        let shape = Shape::of(&Value::Object(fields));

        assert!(shape.truncated);
        assert_eq!(shape.fields.len(), MAX_FIELDS);
    }
}
