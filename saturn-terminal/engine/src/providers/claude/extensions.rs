//! 확장 주입: 공통 도우미가 스킬과 명령을 플러그인 폴더에 놓고, Claude에 맞게 플러그인 설명 파일과 MCP 설정 파일을 만들어
//! `--plugin-dir`과 `--mcp-config`로 넘긴다. 사용자 Claude 설정 파일은 고치지 않는다.
//! 설계: docs/design/extensions.md#주입

use std::path::Path;

use serde_json::{Map, json};

use crate::providers::{
    ExtensionInput, ExtensionLayout, InjectionFailure, collect_definitions, place_files,
};

/// 폴더 이름 `~/.saturn/<이 이름>/<지문>/`. 초안.
const ROOT_DIR: &str = "claude-extensions";

/// 플러그인 이름. 스킬과 명령은 이 이름을 앞에 붙여 부른다. 초안.
const PLUGIN_NAME: &str = "saturn-extensions";

const MANIFEST_DIR: &str = ".claude-plugin";
const MCP_FILE: &str = "mcp.json";

/// 만든 폴더를 가리키는 실행 인자와 주입하지 못한 부분.
#[derive(Debug, Default)]
pub(super) struct Injected {
    pub(super) args: Vec<String>,
    pub(super) failures: Vec<InjectionFailure>,
}

// cost: time O(f + m), heap O(m), stack O(d), io f + m
// vars: f = 복사할 파일 수, m = MCP 서버 수, d = 폴더 깊이
// basis: estimate
/// 부분이 없으면 아무것도 만들지 않는다. 폴더는 지문마다 하나라 같은 지문이면 지우고 다시 쓴다. Claude 형식으로
/// 바꾸는 것은 플러그인 설명 파일과 `mcpServers` 설정 파일뿐이고, 배치와 이름 겹침 처리는 공통 도우미가 한다.
///
/// # Errors
/// 폴더를 만들거나 쓰지 못하면 그 원인.
pub(super) fn inject(
    saturn_home: &Path,
    layout: &ExtensionLayout,
    input: ExtensionInput<'_>,
) -> std::io::Result<Injected> {
    let mut injected = Injected::default();
    if input.parts.is_empty() {
        return Ok(injected);
    }
    let root = saturn_home.join(ROOT_DIR).join(input.fingerprint);
    match std::fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    std::fs::create_dir_all(root.join(MANIFEST_DIR))?;
    let manifest = json!({ "name": PLUGIN_NAME, "description": "Extensions installed in Saturn" });
    std::fs::write(
        root.join(MANIFEST_DIR).join("plugin.json"),
        manifest.to_string(),
    )?;
    let placed = place_files(&root, layout, input.parts);
    let definitions = collect_definitions(layout, input.parts);
    injected.failures.extend(placed.failures);
    injected.failures.extend(definitions.failures);
    if placed.any {
        injected.args.extend([
            "--plugin-dir".to_owned(),
            root.to_string_lossy().into_owned(),
        ]);
    }
    if !definitions.servers.is_empty() {
        let servers: Map<_, _> = definitions
            .servers
            .into_iter()
            .map(|server| (server.name, server.value))
            .collect();
        let file = root.join(MCP_FILE);
        std::fs::write(&file, json!({ "mcpServers": servers }).to_string())?;
        injected.args.extend([
            "--mcp-config".to_owned(),
            file.to_string_lossy().into_owned(),
        ]);
    }
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use saturn_protocol::rpc::ExtensionPartKind;
    use serde_json::Value;

    use super::*;
    use crate::providers::InjectedPart;

    const LAYOUT: ExtensionLayout = ExtensionLayout {
        skills_dir: Some("skills"),
        commands: Some(("commands", "md")),
        mcp_servers: true,
        hooks: false,
    };

    fn write(root: &Path, file: &str, content: &str) -> PathBuf {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn part(extension: &str, kind: ExtensionPartKind, name: &str, source: PathBuf) -> InjectedPart {
        InjectedPart {
            extension: extension.to_owned(),
            kind,
            name: name.to_owned(),
            source,
        }
    }

    /// 원본 폴더와 두 부분씩인 묶음.
    fn parts(store: &Path) -> Vec<InjectedPart> {
        write(
            store,
            "kit/skills/commit-helper/SKILL.md",
            "# commit helper",
        );
        write(store, "kit/skills/commit-helper/notes/a.txt", "note");
        write(store, "kit/commands/review.md", "review the diff");
        let servers = write(
            store,
            "kit/.mcp.json",
            r#"{"mcpServers":{"lint":{"command":"lint","args":["--x"]},"fmt":{"command":"fmt"}}}"#,
        );
        vec![
            part(
                "kit",
                ExtensionPartKind::Skill,
                "commit-helper",
                store.join("kit/skills/commit-helper"),
            ),
            part(
                "kit",
                ExtensionPartKind::Command,
                "review",
                store.join("kit/commands/review.md"),
            ),
            part("kit", ExtensionPartKind::McpServer, "lint", servers.clone()),
            part("kit", ExtensionPartKind::McpServer, "fmt", servers),
        ]
    }

    #[test]
    fn skills_and_commands_go_to_a_plugin_folder_and_servers_to_a_config_file() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let parts = parts(&store);

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "abc123",
            },
        )
        .unwrap();

        let dir = home.join("claude-extensions/abc123");
        assert_eq!(
            injected.args,
            vec![
                "--plugin-dir".to_owned(),
                dir.display().to_string(),
                "--mcp-config".to_owned(),
                dir.join("mcp.json").display().to_string(),
            ]
        );
        assert!(injected.failures.is_empty());
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"], "saturn-extensions");
        assert_eq!(
            std::fs::read_to_string(dir.join("skills/commit-helper/notes/a.txt")).unwrap(),
            "note"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("commands/review.md")).unwrap(),
            "review the diff"
        );
        let servers: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(servers["mcpServers"]["lint"]["args"][0], "--x");
        assert_eq!(servers["mcpServers"]["fmt"]["command"], "fmt");
    }

    #[test]
    fn nothing_is_made_without_parts() {
        let root = tempfile::tempdir().unwrap();

        let injected = inject(root.path(), &LAYOUT, ExtensionInput::default()).unwrap();

        assert!(injected.args.is_empty() && injected.failures.is_empty());
        assert!(!root.path().join(ROOT_DIR).exists());
    }

    #[test]
    fn a_name_used_twice_keeps_the_first_and_fails_the_second_and_hooks_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let mut parts = parts(&store);
        write(&store, "other/skills/commit-helper/SKILL.md", "# other");
        parts.push(part(
            "other",
            ExtensionPartKind::Skill,
            "commit-helper",
            store.join("other/skills/commit-helper"),
        ));
        parts.push(part(
            "other",
            ExtensionPartKind::Hook,
            "PreToolUse",
            store.join("other/hooks.json"),
        ));

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "f",
            },
        )
        .unwrap();

        let failed: Vec<(&str, &str)> = injected
            .failures
            .iter()
            .map(|failure| (failure.extension.as_str(), failure.part.as_str()))
            .collect();
        assert_eq!(
            failed,
            vec![("other", "commit-helper"), ("other", "PreToolUse")]
        );
        assert_eq!(
            std::fs::read_to_string(home.join("claude-extensions/f/skills/commit-helper/SKILL.md"))
                .unwrap(),
            "# commit helper"
        );
    }

    #[test]
    fn a_server_missing_from_its_definition_file_fails_alone() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let mut parts = parts(&store);
        parts.push(part(
            "kit",
            ExtensionPartKind::McpServer,
            "ghost",
            store.join("kit/.mcp.json"),
        ));

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "f",
            },
        )
        .unwrap();

        assert_eq!(injected.failures.len(), 1);
        assert_eq!(injected.failures[0].part, "ghost");
        let servers: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join("claude-extensions/f/mcp.json")).unwrap(),
        )
        .unwrap();
        assert!(servers["mcpServers"].get("ghost").is_none());
        assert!(servers["mcpServers"].get("lint").is_some());
    }

    #[test]
    fn a_server_name_used_by_two_extensions_keeps_the_first() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let mut parts = parts(&store);
        let other = write(
            &store,
            "other/.mcp.json",
            r#"{"mcpServers":{"lint":{"command":"other-lint"}}}"#,
        );
        parts.push(part("other", ExtensionPartKind::McpServer, "lint", other));

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "f",
            },
        )
        .unwrap();

        assert_eq!(injected.failures.len(), 1);
        assert_eq!(injected.failures[0].extension, "other");
        let servers: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join("claude-extensions/f/mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(servers["mcpServers"]["lint"]["command"], "lint");
    }
}
