//! 묶음 나누기: 설치한 확장 폴더를 스킬, MCP 서버, 명령, 훅 부분으로 나눈다.
//! 설계: docs/design/extensions.md#묶음-나누기

use std::path::Path;

use saturn_protocol::rpc::ExtensionPartKind;
use serde_json::Value;

use super::ExtensionError;

/// 스킬 폴더의 정의 파일 이름.
const SKILL_FILE: &str = "SKILL.md";
const SKILLS_DIR: &str = "skills";
const COMMANDS_DIR: &str = "commands";
const HOOKS_FILE: &str = "hooks/hooks.json";
/// MCP 서버 정의 파일. 앞에 있는 것을 쓴다.
const MCP_FILES: [&str; 2] = [".mcp.json", "mcp.json"];

/// 나눈 부분 하나. `path`는 확장 폴더 안의 상대 경로다. 스킬은 폴더(루트면 `.`), 명령은 파일, MCP 서버와 훅은
/// 정의가 든 JSON 파일이고 `name`이 그 안의 키다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartSpec {
    pub(crate) kind: ExtensionPartKind,
    pub(crate) name: String,
    pub(crate) path: String,
}

// cost: time O(f), heap O(p), stack O(1), io f
// vars: f = 폴더 안 파일 수, p = 부분 수
// basis: estimate
/// `root` 폴더를 부분으로 나눈다. 루트에 `SKILL.md`가 있으면 확장 자체가 스킬 하나이고 `name`이 스킬 이름이다.
/// 이름이 같은 종류 안에서 겹치지 않도록 정렬한 순서로 돌려준다.
///
/// # Errors
/// 읽지 못하면 `Read`, JSON 정의가 깨졌으면 `InvalidDefinition`, 부분이 하나도 없으면 `NoParts`.
pub(crate) fn split(root: &Path, name: &str) -> Result<Vec<PartSpec>, ExtensionError> {
    let mut parts = Vec::new();
    if root.join(SKILL_FILE).is_file() {
        parts.push(PartSpec {
            kind: ExtensionPartKind::Skill,
            name: name.to_owned(),
            path: ".".to_owned(),
        });
    }
    for dir in sorted_entries(&root.join(SKILLS_DIR))? {
        if dir.is_dir()
            && dir.join(SKILL_FILE).is_file()
            && let Some(skill) = file_name(&dir)
        {
            parts.push(PartSpec {
                kind: ExtensionPartKind::Skill,
                name: skill.clone(),
                path: format!("{SKILLS_DIR}/{skill}"),
            });
        }
    }
    for file in sorted_entries(&root.join(COMMANDS_DIR))? {
        if file.is_file()
            && file.extension().is_some_and(|extension| extension == "md")
            && let Some(stem) = file.file_stem().and_then(|stem| stem.to_str())
            && let Some(full) = file_name(&file)
        {
            parts.push(PartSpec {
                kind: ExtensionPartKind::Command,
                name: stem.to_owned(),
                path: format!("{COMMANDS_DIR}/{full}"),
            });
        }
    }
    if let Some(file) = MCP_FILES.iter().find(|file| root.join(file).is_file()) {
        for key in keys_under(root, file, "mcpServers")? {
            parts.push(PartSpec {
                kind: ExtensionPartKind::McpServer,
                name: key,
                path: (*file).to_owned(),
            });
        }
    }
    if root.join(HOOKS_FILE).is_file() {
        for key in keys_under(root, HOOKS_FILE, "hooks")? {
            parts.push(PartSpec {
                kind: ExtensionPartKind::Hook,
                name: key,
                path: HOOKS_FILE.to_owned(),
            });
        }
    }
    if parts.is_empty() {
        return Err(ExtensionError::NoParts);
    }
    Ok(parts)
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// 폴더가 없으면 빈 목록이다.
fn sorted_entries(dir: &Path) -> Result<Vec<std::path::PathBuf>, ExtensionError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(read_error(dir, source)),
    };
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| read_error(dir, source))?;
    paths.sort();
    Ok(paths)
}

/// JSON 파일의 `section` 객체 키를 정렬한 순서로 돌려준다. `section`이 없으면 빈 목록이다.
fn keys_under(root: &Path, file: &str, section: &str) -> Result<Vec<String>, ExtensionError> {
    let path = root.join(file);
    let text = std::fs::read_to_string(&path).map_err(|source| read_error(&path, source))?;
    let invalid = |reason: String| ExtensionError::InvalidDefinition {
        file: file.to_owned(),
        reason,
    };
    let value: Value = serde_json::from_str(&text).map_err(|error| invalid(error.to_string()))?;
    let Some(inner) = value.get(section) else {
        return Ok(Vec::new());
    };
    let object = inner
        .as_object()
        .ok_or_else(|| invalid(format!("`{section}` should be an object")))?;
    let mut keys: Vec<String> = object.keys().cloned().collect();
    keys.sort();
    Ok(keys)
}

fn read_error(path: &Path, source: std::io::Error) -> ExtensionError {
    ExtensionError::Read {
        path: path.display().to_string(),
        source,
    }
}
