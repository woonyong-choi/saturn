//! Codex 전용 `CODEX_HOME`: 사용자 설정에서 권한 키를 빼고 Saturn 규칙을 번역해 매 실행마다 만든다.
//! 설계: docs/design/permissions.md#codex-구성

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, symlink};
use std::path::{Path, PathBuf};

use saturn_core::permission::{
    PermissionTool, Rule, Verdict, is_literal, literal_prefix, rules_verdict, tool_name,
};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::store::sha256_hex;

/// 전용 폴더들이 들어가는 `~/.saturn` 아래 폴더 이름. 초안.
const HOMES_DIR: &str = "codex-home";

const CONFIG_FILE: &str = "config.toml";

const AUTH_FILE: &str = "auth.json";

/// execpolicy 규칙 파일. `CODEX_HOME` 아래 위치는 실험 5에서 확인했다.
const RULES_FILE: &str = "rules/default.rules";

/// 자동 검토자가 Saturn 앞에서 판단하지 못하게 사용자가 직접 답하도록 둔다.
const REVIEWER_KEY: &str = "approvals_reviewer";

/// 늦게 뜨는 MCP 서버의 도구가 첫 턴에서 빠지지 않게 늘린 유예(ms). 실험 8에서 12000으로 확인했다. 초안.
const MCP_STARTUP_GRACE_MS: i64 = 12_000;

/// 폴더 이름에 쓰는 규칙 지문의 글자 수. 초안.
const HOME_NAME_LEN: usize = 16;

/// 사용자 설정에서 옮기지 않는 최상위 키. 권한을 정하거나 로그인 저장 방식을 바꾸는 키다.
const DROPPED_KEYS: &[&str] = &[
    "approval_policy",
    "sandbox_mode",
    "sandbox_workspace_write",
    REVIEWER_KEY,
    "hooks",
    "rules",
    "shell_environment_policy",
    "cli_auth_credentials_store",
    "default_permissions",
    "permissions",
];

/// 프로필과 프로젝트 표 안에서 옮기지 않는 키.
const DROPPED_PROFILE_KEYS: &[&str] = &[
    "approval_policy",
    "sandbox_mode",
    "sandbox_workspace_write",
    REVIEWER_KEY,
    "hooks",
    "rules",
    "shell_environment_policy",
    "default_permissions",
    "permissions",
];

#[derive(Debug, thiserror::Error)]
pub enum HomeError {
    #[error("failed to read codex config: {path}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("codex config is not valid toml: {path}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },
    #[error("failed to write codex home file: {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 만든 `CODEX_HOME`과, 첫 턴 전에 준비를 확인할 MCP 서버.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedHome {
    pub path: PathBuf,
    pub mcp_servers: Vec<String>,
}

/// 사용자 폴더는 읽기만 한다.
#[derive(Debug, Clone, Copy)]
pub struct HomeInput<'a> {
    pub saturn_home: &'a Path,
    pub user_codex_home: &'a Path,
    pub rules: &'a [Rule],
}

// cost: time O(c + r·m), heap O(c), stack O(1), alloc c, io 6
// vars: c = 사용자 설정 크기, r = 규칙 수, m = MCP 서버 수
// basis: estimate
/// 규칙 집합마다 하나인 폴더를 만들고 `config.toml`과 규칙 파일은 매번 다시 쓴다. 로그인은 사용자 `auth.json`을
/// 가리키는 링크로 두고 복사하지 않는다.
///
/// # Errors
/// 사용자 설정을 읽거나 해석하지 못하면 `ReadConfig`와 `ParseConfig`, 파일을 쓰지 못하면 `Write`.
pub fn prepare(input: HomeInput<'_>) -> Result<PreparedHome, HomeError> {
    let mut doc = read_user_config(&input.user_codex_home.join(CONFIG_FILE))?;
    drop_permission_keys(&mut doc);
    let mcp_servers = translate_mcp(&mut doc, input.rules);
    doc[REVIEWER_KEY] = value("user");
    doc["mcp_optional_startup_grace_ms"] = value(MCP_STARTUP_GRACE_MS);
    let policy = execpolicy(input.rules);
    let path = input
        .saturn_home
        .join(HOMES_DIR)
        .join(rules_fingerprint(input.rules));
    create_private_dir(&path.join("rules")).map_err(write_error(&path))?;
    write_private(&path.join(CONFIG_FILE), doc.to_string().as_bytes())?;
    write_private(&path.join(RULES_FILE), policy.as_bytes())?;
    link_login(input.user_codex_home, &path)?;
    Ok(PreparedHome { path, mcp_servers })
}

// cost: time O(c), heap O(c), stack O(1), alloc 2, io 1
// vars: c = 사용자 설정 크기
// basis: estimate
fn read_user_config(path: &Path) -> Result<DocumentMut, HomeError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(HomeError::ReadConfig {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    content.parse().map_err(|source| HomeError::ParseConfig {
        path: path.to_path_buf(),
        source,
    })
}

// cost: time O(k + p), heap O(1), stack O(1)
// vars: k = 최상위 키 수, p = 프로필과 프로젝트 수
// basis: estimate
/// 권한을 정하는 키를 모두 뺀다. 모델, MCP 서버 같은 권한 밖 설정은 그대로 둔다.
fn drop_permission_keys(doc: &mut DocumentMut) {
    let root = doc.as_table_mut();
    for key in DROPPED_KEYS {
        root.remove(key);
    }
    for section in ["profiles", "projects"] {
        let Some(entries) = root.get_mut(section).and_then(Item::as_table_like_mut) else {
            continue;
        };
        for (_, entry) in entries.iter_mut() {
            let Some(table) = entry.as_table_like_mut() else {
                continue;
            };
            table.remove("trust_level");
            for key in DROPPED_PROFILE_KEYS {
                table.remove(key);
            }
        }
    }
}

/// 서버가 다루는 도구 이름 앞부분.
fn mcp_prefix(server: &str) -> String {
    format!("mcp__{server}__")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stance {
    Approve,
    Prompt,
    Disable,
}

// cost: time O(r·p·t), heap O(r), stack O(1), alloc 3
// vars: r = 규칙 수, p = 패턴 글자 수, t = 서버 이름 글자 수
// basis: estimate
/// 모드는 보지 않고 개별 규칙만으로 서버 기본 태도를 정한다. 모드는 engine이 요청마다 답으로 적용한다.
fn stance(rules: &[Rule], server: &str) -> Stance {
    let prefix = mcp_prefix(server);
    let overlapping: Vec<&Rule> = rules
        .iter()
        .filter(|rule| rule.tool == PermissionTool::Mcp && rule.may_match_prefix(&prefix))
        .collect();
    let probe = format!("{prefix}\u{0}");
    match rules_verdict(rules, PermissionTool::Mcp, &probe) {
        Some(Verdict::Allow)
            if overlapping
                .iter()
                .all(|rule| rule.verdict == Verdict::Allow) =>
        {
            Stance::Approve
        }
        Some(Verdict::Deny)
            if overlapping
                .iter()
                .all(|rule| rule.verdict != Verdict::Allow) =>
        {
            Stance::Disable
        }
        _ => Stance::Prompt,
    }
}

// cost: time O(m·r), heap O(m + t), stack O(1), alloc m
// vars: m = MCP 서버 수, r = 규칙 수, t = 이름이 정해진 도구 수
// basis: estimate
/// 허용은 `approve`, 묻기는 `prompt`, 거부는 서버 끄기나 `disabled_tools`로 옮기고, 준비를 기다릴 서버 이름을 돌려준다.
fn translate_mcp(doc: &mut DocumentMut, rules: &[Rule]) -> Vec<String> {
    let mut waiting = Vec::new();
    let Some(servers) = doc.get_mut("mcp_servers").and_then(Item::as_table_like_mut) else {
        return waiting;
    };
    for (key, entry) in servers.iter_mut() {
        let name = key.get().to_owned();
        let Some(server) = entry.as_table_like_mut() else {
            continue;
        };
        let is_enabled = server.get("enabled").and_then(Item::as_bool) != Some(false);
        match stance(rules, &name) {
            Stance::Disable => {
                server.insert("enabled", value(false));
            }
            stance => {
                let mode = if stance == Stance::Approve {
                    "approve"
                } else {
                    "prompt"
                };
                server.insert("default_tools_approval_mode", value(mode));
                apply_named_tools(server, rules, &name);
                if is_enabled {
                    waiting.push(name);
                }
            }
        }
    }
    waiting
}

// cost: time O(r·p·t), heap O(r), stack O(1), alloc r
// vars: r = 규칙 수, p = 패턴 글자 수, t = 도구 이름 글자 수
// basis: estimate
/// 규칙 패턴에 이름이 정해진 도구(`mcp__서버__도구`)는 도구별 승인과 `disabled_tools`로 옮긴다.
fn apply_named_tools(server: &mut dyn toml_edit::TableLike, rules: &[Rule], name: &str) {
    let prefix = mcp_prefix(name);
    let mut disabled = Array::new();
    for rule in rules {
        let named = (rule.tool == PermissionTool::Mcp && is_literal(&rule.pattern))
            .then(|| literal_prefix(&rule.pattern))
            .and_then(|target| target.strip_prefix(&prefix).map(str::to_owned));
        let Some(tool) = named.filter(|tool| !tool.is_empty()) else {
            continue;
        };
        match rules_verdict(rules, PermissionTool::Mcp, &format!("{prefix}{tool}")) {
            Some(Verdict::Deny) => disabled.push(tool),
            Some(verdict) => set_tool_mode(server, &tool, verdict),
            None => {}
        }
    }
    if !disabled.is_empty() {
        server.insert("disabled_tools", value(disabled));
    }
}

fn set_tool_mode(server: &mut dyn toml_edit::TableLike, tool: &str, verdict: Verdict) {
    let mode = if verdict == Verdict::Allow {
        "approve"
    } else {
        "prompt"
    };
    let tools = server
        .entry("tools")
        .or_insert_with(|| Item::Table(Table::new()));
    let Some(tools) = tools.as_table_like_mut() else {
        return;
    };
    let entry = tools
        .entry(tool)
        .or_insert_with(|| Item::Table(Table::new()));
    if let Some(entry) = entry.as_table_like_mut() {
        entry.insert("approval_mode", value(mode));
    }
}

// cost: time O(r·c), heap O(r·c), stack O(1), alloc r
// vars: r = 규칙 수, c = 패턴 글자 수
// basis: estimate
/// 셸 규칙을 `prefix_rule`로 옮긴다. 글자 일치 규칙은 접두사 일치로만 옮길 수 있어, 허용은 `명령 *` 모양만 옮기고
/// 묻기와 거부는 접두사 일치로 더 엄하게 옮긴다. 옮길 수 없는 패턴은 건너뛰고 engine이 요청마다 답한다.
fn execpolicy(rules: &[Rule]) -> String {
    let mut text = String::new();
    for rule in rules
        .iter()
        .filter(|rule| rule.tool == PermissionTool::Shell)
    {
        let Some((tokens, is_prefix)) = prefix_tokens(&rule.pattern) else {
            continue;
        };
        let decision = match rule.verdict {
            Verdict::Allow if is_prefix => "allow",
            Verdict::Allow => continue,
            Verdict::Ask => "prompt",
            Verdict::Deny => "forbidden",
        };
        let quoted: Vec<String> = tokens.iter().map(|token| quote(token)).collect();
        let _ = writeln!(
            text,
            "prefix_rule(pattern = [{}], decision = \"{decision}\")",
            quoted.join(", ")
        );
    }
    text
}

// cost: time O(p), heap O(p), stack O(1), alloc w
// vars: p = 패턴 글자 수, w = 낱말 수
// basis: estimate
/// 낱말 목록과 `명령 *` 모양인지. 셸 문법이 든 패턴은 `None`.
fn prefix_tokens(pattern: &str) -> Option<(Vec<String>, bool)> {
    let (body, is_prefix) = match pattern.strip_suffix(" *") {
        Some(body) => (body, true),
        None if is_literal(pattern) => (pattern, false),
        None => return None,
    };
    let is_plain = |token: &str| {
        !token.chars().any(|c| {
            matches!(
                c,
                '*' | '\\' | '\'' | '"' | ';' | '&' | '|' | '<' | '>' | '(' | ')' | '$' | '`'
            )
        })
    };
    let tokens: Vec<String> = body.split_whitespace().map(str::to_owned).collect();
    (!tokens.is_empty() && tokens.iter().all(|token| is_plain(token)))
        .then_some((tokens, is_prefix))
}

fn quote(token: &str) -> String {
    serde_json::to_string(token).expect("string should serialize as json")
}

// cost: time O(r·p), heap O(r·p), stack O(1), alloc r
// vars: r = 규칙 수, p = 패턴 글자 수
// basis: estimate
/// 규칙 목록 전체의 지문. 규칙이 같은 채팅은 같은 폴더를 쓴다.
pub fn rules_fingerprint(rules: &[Rule]) -> String {
    let mut text = String::new();
    for rule in rules {
        let _ = writeln!(
            text,
            "{}\t{}\t{}",
            tool_name(rule.tool),
            rule.verdict.name(),
            rule.pattern
        );
    }
    let mut digest = sha256_hex(text.as_bytes());
    digest.truncate(HOME_NAME_LEN);
    digest
}

// cost: time O(d), heap O(1), stack O(1), io d
// vars: d = 경로 깊이
// basis: estimate
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}

/// 같은 폴더의 임시 파일에 쓴 뒤 이름을 바꿔 갈아 끼워, 다른 app-server가 반쯤 쓴 파일을 읽지 않게 한다.
fn write_private(path: &Path, body: &[u8]) -> Result<(), HomeError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let write = || -> std::io::Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        file.write_all(body)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok(())
    };
    write().map_err(write_error(path))
}

// cost: time O(1), heap O(1), stack O(1), io 4
// basis: estimate
/// 사용자 `auth.json`이 있으면 같은 곳을 가리키는 링크를 둔다. 없으면 로그인 전이라 링크를 만들지 않는다.
fn link_login(user_codex_home: &Path, home: &Path) -> Result<(), HomeError> {
    let source = user_codex_home.join(AUTH_FILE);
    let target = home.join(AUTH_FILE);
    if !source.exists() {
        return Ok(());
    }
    if std::fs::read_link(&target).is_ok_and(|current| current == source) {
        return Ok(());
    }
    let relink = || -> std::io::Result<()> {
        match std::fs::remove_file(&target) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        symlink(&source, &target)
    };
    relink().map_err(write_error(&target))
}

fn write_error(path: &Path) -> impl FnOnce(std::io::Error) -> HomeError + '_ {
    move |source| HomeError::Write {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests;
