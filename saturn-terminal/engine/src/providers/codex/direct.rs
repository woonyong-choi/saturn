//! 사용자가 Codex에 직접 설치한 스킬, 명령(프롬프트), MCP 서버를 읽는다. 사용자 `~/.codex`는 읽기만 하고, 설정 파일에서는
//! MCP 서버 정의의 주입 가능한 키만 꺼내며 내용은 로그나 화면에 내지 않는다.
//! 설계: docs/design/extensions.md#provider에-직접-설치한-것

use std::ffi::OsString;

use saturn_protocol::rpc::DirectKind;
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item};

use super::config::user_folder;
use crate::providers::{
    DIRECT_INSTALL_LIMIT, DirectInstall, DirectOrigin, scan_command_files, scan_skill_folders,
};

/// 확장의 MCP 서버 정의로 옮길 수 있는 키.
const SERVER_KEYS: [&str; 5] = ["command", "args", "env", "cwd", "url"];

// cost: time O(n + f), heap O(n + f), stack O(1), io n + f
// vars: n = 폴더 항목 수, f = 설정 파일 크기
// basis: estimate
/// 환경의 `CODEX_HOME`, 없으면 `HOME/.codex`에서 읽은 항목. 둘 다 없으면 비어 있다.
pub(super) fn read(env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
    let Some(home) = user_folder(env) else {
        return Vec::new();
    };
    let mut found = scan_skill_folders(&home.join("skills"));
    found.extend(scan_command_files(&home.join("prompts"), "md"));
    found.extend(mcp_servers(&home.join("config.toml")));
    found.truncate(DIRECT_INSTALL_LIMIT);
    found
}

fn mcp_servers(file: &std::path::Path) -> Vec<DirectInstall> {
    let Some(doc) = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
    else {
        return Vec::new();
    };
    let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table_like) else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|(name, item)| {
            let table = item.as_table_like()?;
            let mut definition = Map::new();
            for key in SERVER_KEYS {
                if let Some(item) = table.get(key) {
                    definition.insert(key.to_owned(), item_json(item));
                }
            }
            Some(DirectInstall {
                kind: DirectKind::McpServer,
                name: name.to_owned(),
                origin: DirectOrigin::Server(Value::Object(definition)),
            })
        })
        .collect()
}

fn item_json(item: &Item) -> Value {
    match item {
        Item::Value(value) => to_json(value),
        Item::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, item)| (key.to_owned(), item_json(item)))
                .collect(),
        ),
        Item::ArrayOfTables(tables) => Value::Array(
            tables
                .iter()
                .map(|table| item_json(&Item::Table(table.clone())))
                .collect(),
        ),
        Item::None => Value::Null,
    }
}

fn to_json(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(text) => Value::from(text.value().as_str()),
        toml_edit::Value::Integer(number) => Value::from(*number.value()),
        toml_edit::Value::Float(number) => Value::from(*number.value()),
        toml_edit::Value::Boolean(flag) => Value::from(*flag.value()),
        toml_edit::Value::Datetime(date) => Value::from(date.value().to_string()),
        toml_edit::Value::Array(items) => Value::Array(items.iter().map(to_json).collect()),
        toml_edit::Value::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn write(root: &Path, file: &str, content: &str) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn skills_prompts_and_servers_are_read_and_builtin_skills_are_skipped() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            ".codex/skills/commit-helper/SKILL.md",
            "# skill",
        );
        write(
            home.path(),
            ".codex/skills/.system/builtin/SKILL.md",
            "# builtin",
        );
        write(home.path(), ".codex/prompts/review.md", "review");
        write(
            home.path(),
            ".codex/config.toml",
            "model = \"x\"\n[mcp_servers.lint]\ncommand = \"lint\"\nargs = [\"--x\"]\nstartup_timeout_sec = 5\n[mcp_servers.lint.env]\nKEY = \"v\"\n",
        );
        let env = vec![("HOME".into(), home.path().as_os_str().to_owned())];

        let found = read(&env);

        let names: Vec<(DirectKind, &str)> = found
            .iter()
            .map(|item| (item.kind, item.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                (DirectKind::Skill, "commit-helper"),
                (DirectKind::Command, "review"),
                (DirectKind::McpServer, "lint"),
            ]
        );
        let DirectOrigin::Server(server) = &found[2].origin else {
            panic!("expected a server definition");
        };
        assert_eq!(server["command"], "lint");
        assert_eq!(server["args"][0], "--x");
        assert_eq!(server["env"]["KEY"], "v");
        assert!(server.get("startup_timeout_sec").is_none());
    }

    #[test]
    fn the_codex_home_variable_wins_over_the_home_folder() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), ".codex/skills/a/SKILL.md", "# a");
        write(home.path(), "other/skills/b/SKILL.md", "# b");
        let env = vec![
            ("HOME".into(), home.path().as_os_str().to_owned()),
            (
                "CODEX_HOME".into(),
                home.path().join("other").into_os_string(),
            ),
        ];

        let names: Vec<String> = read(&env).into_iter().map(|item| item.name).collect();

        assert_eq!(names, vec!["b".to_owned()]);

        // 빈 값은 없는 것으로 보고, 상대 경로는 폴더를 정하지 못해 비어 있다
        let mut empty = env.clone();
        empty[1].1 = OsString::new();
        assert_eq!(read(&empty).len(), 1);
        let mut relative = env;
        relative[1].1 = "other".into();
        assert!(read(&relative).is_empty());
    }
}
