//! 사용자가 Claude Code에 직접 설치한 스킬, 명령, MCP 서버, 플러그인을 읽는다. 사용자 폴더는 읽기만 하고, 설정 파일에서는
//! 필요한 키 이름과 MCP 서버 정의만 꺼내며 내용은 로그나 화면에 내지 않는다.
//! 설계: docs/design/extensions.md#provider에-직접-설치한-것

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use saturn_protocol::rpc::DirectKind;
use serde_json::Value;

use crate::providers::{
    DIRECT_INSTALL_LIMIT, DirectInstall, DirectOrigin, scan_command_files, scan_skill_folders,
};

/// 사용자 폴더 안의 위치. 초안.
const USER_DIR: &str = ".claude";
const PLUGINS_FILE: &str = "plugins/installed_plugins.json";
/// 사용자 범위 MCP 서버가 든 파일. 홈 폴더 바로 아래에 있다.
const USER_STATE_FILE: &str = ".claude.json";

// cost: time O(n + f), heap O(n + f), stack O(1), io n + f
// vars: n = 폴더 항목 수, f = 설정 파일 크기
// basis: estimate
/// `env`의 `HOME` 아래 사용자 Claude 폴더에서 읽은 항목. `HOME`이 없으면 비어 있다.
pub(super) fn read(env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
    let Some(home) = env
        .iter()
        .find(|(key, _)| key == "HOME")
        .map(|(_, value)| PathBuf::from(value))
    else {
        return Vec::new();
    };
    let user = home.join(USER_DIR);
    let mut found = scan_skill_folders(&user.join("skills"));
    found.extend(scan_command_files(&user.join("commands"), "md"));
    found.extend(mcp_servers(&home.join(USER_STATE_FILE)));
    found.extend(plugins(&user.join(PLUGINS_FILE)));
    found.truncate(DIRECT_INSTALL_LIMIT);
    found
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn mcp_servers(file: &Path) -> Vec<DirectInstall> {
    let Some(Value::Object(servers)) =
        read_json(file).and_then(|mut all| all.get_mut("mcpServers").map(Value::take))
    else {
        return Vec::new();
    };
    servers
        .into_iter()
        .map(|(name, value)| DirectInstall {
            kind: DirectKind::McpServer,
            name,
            origin: DirectOrigin::Server(value),
        })
        .collect()
}

/// 설치한 플러그인의 이름만 읽는다. 키는 `이름@마켓플레이스`다.
fn plugins(file: &Path) -> Vec<DirectInstall> {
    let Some(Value::Object(plugins)) =
        read_json(file).and_then(|mut all| all.get_mut("plugins").map(Value::take))
    else {
        return Vec::new();
    };
    plugins
        .into_iter()
        .map(|(name, _)| DirectInstall {
            kind: DirectKind::Plugin,
            name,
            origin: DirectOrigin::TrackedOnly,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, file: &str, content: &str) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn skills_commands_servers_and_plugins_are_read_from_a_fake_home() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            ".claude/skills/commit-helper/SKILL.md",
            "# skill",
        );
        write(home.path(), ".claude/skills/not-a-skill/readme.txt", "x");
        write(home.path(), ".claude/commands/review.md", "review");
        write(
            home.path(),
            ".claude.json",
            r#"{"mcpServers":{"lint":{"command":"lint"}},"projects":{"/x":{}}}"#,
        );
        write(
            home.path(),
            ".claude/plugins/installed_plugins.json",
            r#"{"version":2,"plugins":{"kit@market":[{"scope":"user"}]}}"#,
        );
        let env = vec![("HOME".into(), home.path().as_os_str().to_owned())];

        let found: Vec<(DirectKind, String)> = read(&env)
            .into_iter()
            .map(|item| (item.kind, item.name))
            .collect();

        assert_eq!(
            found,
            vec![
                (DirectKind::Skill, "commit-helper".to_owned()),
                (DirectKind::Command, "review".to_owned()),
                (DirectKind::McpServer, "lint".to_owned()),
                (DirectKind::Plugin, "kit@market".to_owned()),
            ]
        );
    }

    #[test]
    fn nothing_is_found_without_a_home_or_when_the_files_are_broken() {
        assert!(read(&[]).is_empty());
        let home = tempfile::tempdir().unwrap();
        write(home.path(), ".claude.json", "{ not json");
        let env = vec![("HOME".into(), home.path().as_os_str().to_owned())];

        assert!(read(&env).is_empty());
    }
}
