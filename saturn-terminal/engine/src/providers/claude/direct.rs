//! 사용자가 Claude Code에 직접 설치한 스킬, 명령, MCP 서버, 플러그인을 읽는다. 사용자 폴더는 읽기만 하고, 설정 파일에서는
//! 필요한 키 이름과 MCP 서버 정의만 꺼내며 내용은 로그나 화면에 내지 않는다.
//! 설계: docs/design/extensions.md#provider에-직접-설치한-것

use std::ffi::OsString;
use std::path::Path;

use saturn_protocol::rpc::DirectKind;
use serde_json::Value;

use super::config::user_folder;
use crate::providers::{
    DIRECT_INSTALL_LIMIT, DirectInstall, DirectOrigin, scan_command_files, scan_skill_folders,
};

/// 사용자 폴더 안의 위치. 초안.
const PLUGINS_FILE: &str = "plugins/installed_plugins.json";

// cost: time O(n + f), heap O(n + f), stack O(1), io n + f
// vars: n = 폴더 항목 수, f = 설정 파일 크기
// basis: estimate
/// `config::user_folder`가 가리키는 사용자 Claude 폴더에서 읽은 항목. 폴더를 정할 수 없으면 비어 있다.
pub(super) fn read(env: &[(OsString, OsString)]) -> Vec<DirectInstall> {
    let Some(user) = user_folder(env) else {
        return Vec::new();
    };
    let mut found = scan_skill_folders(&user.dir.join("skills"));
    found.extend(scan_command_files(&user.dir.join("commands"), "md"));
    found.extend(mcp_servers(&user.state_file));
    found.extend(plugins(&user.dir.join(PLUGINS_FILE)));
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
        let names = |env: &[(OsString, OsString)]| -> Vec<(DirectKind, String)> {
            read(env)
                .into_iter()
                .map(|item| (item.kind, item.name))
                .collect()
        };

        assert_eq!(
            names(&env),
            vec![
                (DirectKind::Skill, "commit-helper".to_owned()),
                (DirectKind::Command, "review".to_owned()),
                (DirectKind::McpServer, "lint".to_owned()),
                (DirectKind::Plugin, "kit@market".to_owned()),
            ]
        );

        // CLAUDE_CONFIG_DIR가 있으면 홈 폴더 대신 그 폴더(MCP 상태 파일 포함)만 읽는다. 빈 값은 없는 것으로 본다
        let custom = tempfile::tempdir().unwrap();
        write(custom.path(), "skills/other/SKILL.md", "# skill");
        write(
            custom.path(),
            ".claude.json",
            r#"{"mcpServers":{"remote":{"command":"remote"}}}"#,
        );
        let mut with_custom = env.clone();
        with_custom.push((
            "CLAUDE_CONFIG_DIR".into(),
            custom.path().as_os_str().to_owned(),
        ));
        assert_eq!(
            names(&with_custom),
            vec![
                (DirectKind::Skill, "other".to_owned()),
                (DirectKind::McpServer, "remote".to_owned()),
            ]
        );
        let mut with_empty = env.clone();
        with_empty.push(("CLAUDE_CONFIG_DIR".into(), OsString::new()));
        assert_eq!(names(&with_empty), names(&env));
        // 상대 경로는 폴더를 정하지 못하고, 없는 폴더는 항목이 없다
        let mut with_relative = env.clone();
        with_relative.push(("CLAUDE_CONFIG_DIR".into(), "rel".into()));
        assert!(names(&with_relative).is_empty());
        let mut with_missing = env;
        with_missing.push((
            "CLAUDE_CONFIG_DIR".into(),
            custom.path().join("none").into_os_string(),
        ));
        assert!(names(&with_missing).is_empty());
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
