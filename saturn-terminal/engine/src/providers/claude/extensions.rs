//! 확장 주입: 공통 도우미가 스킬과 명령을 플러그인 폴더에 놓고, Claude에 맞게 플러그인 설명 파일과 MCP 설정 파일, 훅 파일을
//! 만들어 `--plugin-dir`과 `--mcp-config`로 넘긴다. 사용자 Claude 설정 파일과 Saturn 소유 훅이 든 `--settings`는 건드리지
//! 않는다.
//! 설계: docs/design/extensions.md#주입

use std::path::Path;

use saturn_protocol::rpc::ExtensionPartKind;
use serde_json::{Map, Value, json};

use crate::providers::{
    Definition, ExtensionInput, ExtensionLayout, InjectedPart, InjectionFailure,
    collect_definitions, place_files,
};

/// 폴더 이름 `~/.saturn/<이 이름>/<지문>/`. 초안.
const ROOT_DIR: &str = "claude-extensions";

/// 플러그인 이름. 스킬과 명령은 이 이름을 앞에 붙여 부른다. 초안.
const PLUGIN_NAME: &str = "saturn-extensions";

const MANIFEST_DIR: &str = ".claude-plugin";
const MCP_FILE: &str = "mcp.json";
/// 플러그인 폴더에서 Claude가 훅을 찾는 위치.
const HOOKS_FILE: &str = "hooks/hooks.json";

/// 훅 명령이 확장 폴더를 가리키는 Claude의 변수. 원본은 플러그인 폴더에 복사하지 않으므로 원본 폴더로 바꿔 넣는다.
const PLUGIN_ROOT_VAR: &str = "${CLAUDE_PLUGIN_ROOT}";

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
    let hooks_file = write_hooks(
        &root,
        &definitions.hooks,
        input.parts,
        &mut injected.failures,
    )?;
    if placed.any || hooks_file {
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

/// 훅 정의를 이벤트마다 이어 플러그인의 `hooks/hooks.json`에 쓴다. 이 파일은 Saturn 소유 PreToolUse 훅이 든 실행별
/// `--settings`와 따로라서 Saturn 훅을 바꾸지 않고, Claude는 같은 이벤트의 훅을 모두 부르며 한 훅이라도 막으면 막는다.
/// 훅이 없으면 파일을 만들지 않고 `false`.
fn write_hooks(
    root: &Path,
    hooks: &[Definition],
    parts: &[InjectedPart],
    failures: &mut Vec<InjectionFailure>,
) -> std::io::Result<bool> {
    if hooks.is_empty() {
        return Ok(false);
    }
    let mut events: Map<String, Value> = Map::new();
    for hook in hooks {
        let origin = parts
            .iter()
            .find(|part| {
                part.kind == ExtensionPartKind::Hook
                    && part.extension == hook.extension
                    && part.name == hook.name
            })
            .and_then(|part| part.source.parent()?.parent());
        let mut text = hook.value.to_string();
        if let Some(dir) = origin {
            let escaped = Value::from(dir.to_string_lossy().into_owned()).to_string();
            text = text.replace(PLUGIN_ROOT_VAR, escaped.trim_matches('"'));
        }
        let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&text) else {
            failures.push(InjectionFailure {
                extension: hook.extension.clone(),
                part: hook.name.clone(),
                reason: "the hook definition is not a list of matchers".to_owned(),
            });
            continue;
        };
        if let Value::Array(all) = events.entry(hook.name.clone()).or_insert_with(|| json!([])) {
            all.extend(entries);
        }
    }
    if events.is_empty() {
        return Ok(false);
    }
    let file = root.join(HOOKS_FILE);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, json!({ "hooks": events }).to_string())?;
    Ok(true)
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
        hooks: true,
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
    fn a_name_used_twice_keeps_the_first_and_fails_the_second() {
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
        assert_eq!(failed, vec![("other", "commit-helper")]);
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

    #[test]
    fn hooks_of_every_extension_go_to_the_plugin_hook_file_and_not_to_the_saturn_settings() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let mut parts = Vec::new();
        for (extension, command) in [
            ("first", "${CLAUDE_PLUGIN_ROOT}/hooks/check.sh"),
            ("second", "echo second"),
        ] {
            let file = write(
                &store,
                &format!("{extension}/hooks/hooks.json"),
                &json!({"hooks": {"PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": command}]}
                ]}})
                .to_string(),
            );
            parts.push(part(extension, ExtensionPartKind::Hook, "PreToolUse", file));
        }

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "h",
            },
        )
        .unwrap();

        let dir = home.join("claude-extensions/h");
        assert_eq!(
            injected.args,
            vec!["--plugin-dir".to_owned(), dir.display().to_string()]
        );
        assert!(injected.failures.is_empty());
        let file: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("hooks/hooks.json")).unwrap())
                .unwrap();
        let entries = file["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0]["hooks"][0]["command"],
            format!("{}/hooks/check.sh", store.join("first").display())
        );
        assert_eq!(entries[1]["hooks"][0]["command"], "echo second");
    }

    #[test]
    fn a_hook_that_is_not_a_list_fails_alone() {
        let root = tempfile::tempdir().unwrap();
        let (home, store) = (root.path().join("saturn"), root.path().join("store"));
        let file = write(
            &store,
            "odd/hooks/hooks.json",
            r#"{"hooks":{"PreToolUse":{"matcher":"Bash"}}}"#,
        );
        let parts = vec![part("odd", ExtensionPartKind::Hook, "PreToolUse", file)];

        let injected = inject(
            &home,
            &LAYOUT,
            ExtensionInput {
                parts: &parts,
                fingerprint: "o",
            },
        )
        .unwrap();

        assert!(injected.args.is_empty());
        assert_eq!(injected.failures.len(), 1);
        assert!(!home.join("claude-extensions/o/hooks/hooks.json").exists());
    }
}
