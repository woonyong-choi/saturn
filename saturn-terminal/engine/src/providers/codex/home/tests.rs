use std::os::unix::fs::PermissionsExt;

use super::*;

const USER_CONFIG: &str = r#"model = "gpt-test"
approval_policy = "never"
sandbox_mode = "danger-full-access"
approvals_reviewer = "auto"
cli_auth_credentials_store = "keyring"
profile = "work"

[sandbox_workspace_write]
network_access = true

[shell_environment_policy]
inherit = "all"

[hooks]
pre_tool_use = "x"

[hooks.state]
trusted = true

[projects."/work"]
trust_level = "trusted"
note = "keep"

[profiles.work]
model = "other"
approval_policy = "never"
sandbox_mode = "workspace-write"

[mcp_servers.docs]
command = "docs-server"

[mcp_servers.web]
command = "web-server"

[mcp_servers.off]
command = "off-server"
enabled = false
"#;

struct Fixture {
    _root: tempfile::TempDir,
    saturn_home: PathBuf,
    user_home: PathBuf,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let saturn_home = root.path().join("saturn");
        let user_home = root.path().join("codex");
        std::fs::create_dir_all(user_home.join("rules")).unwrap();
        std::fs::write(user_home.join(CONFIG_FILE), config).unwrap();
        std::fs::write(user_home.join(AUTH_FILE), "{\"token\":\"secret\"}").unwrap();
        std::fs::write(
            user_home.join("rules/default.rules"),
            "prefix_rule(pattern = [\"sort\"], decision = \"allow\")\n",
        )
        .unwrap();
        Self {
            _root: root,
            saturn_home,
            user_home,
        }
    }

    fn prepare(&self, rules: &[Rule]) -> PreparedHome {
        prepare(HomeInput {
            saturn_home: &self.saturn_home,
            user_codex_home: &self.user_home,
            rules,
            questions: true,
        })
        .unwrap()
    }
}

fn rule(tool: PermissionTool, pattern: &str, verdict: Verdict) -> Rule {
    Rule {
        tool,
        pattern: pattern.to_owned(),
        verdict,
    }
}

fn config_of(home: &PreparedHome) -> DocumentMut {
    std::fs::read_to_string(home.path.join(CONFIG_FILE))
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn generated_config_has_no_permission_keys() {
    let fixture = Fixture::new(USER_CONFIG);

    let home = fixture.prepare(&[]);

    let config = config_of(&home);
    for key in DROPPED_KEYS.iter().filter(|key| **key != REVIEWER_KEY) {
        assert!(config.get(key).is_none(), "{key} should be dropped");
    }
    assert_eq!(config[REVIEWER_KEY].as_str(), Some("user"));
    assert!(config["projects"]["/work"].get("trust_level").is_none());
    assert_eq!(config["projects"]["/work"]["note"].as_str(), Some("keep"));
    let profile = &config["profiles"]["work"];
    assert!(profile.get("approval_policy").is_none());
    assert!(profile.get("sandbox_mode").is_none());
    assert_eq!(profile["model"].as_str(), Some("other"));
    assert_eq!(config["model"].as_str(), Some("gpt-test"));
    assert_eq!(
        config["mcp_optional_startup_grace_ms"].as_integer(),
        Some(MCP_STARTUP_GRACE_MS)
    );
}

#[test]
fn config_without_user_file_still_sets_reviewer() {
    let fixture = Fixture::new("");
    std::fs::remove_file(fixture.user_home.join(CONFIG_FILE)).unwrap();

    let home = fixture.prepare(&[]);

    assert_eq!(config_of(&home)[REVIEWER_KEY].as_str(), Some("user"));
    assert!(home.mcp_servers.is_empty());
}

#[test]
fn invalid_user_config_is_an_error() {
    let fixture = Fixture::new("model = = 1");

    let error = prepare(HomeInput {
        saturn_home: &fixture.saturn_home,
        user_codex_home: &fixture.user_home,
        rules: &[],
        questions: true,
    })
    .unwrap_err();

    assert!(matches!(error, HomeError::ParseConfig { .. }));
}

#[test]
fn home_links_login_without_copying_and_leaves_user_files_alone() {
    let fixture = Fixture::new(USER_CONFIG);
    let user_config = std::fs::read_to_string(fixture.user_home.join(CONFIG_FILE)).unwrap();

    let home = fixture.prepare(&[rule(PermissionTool::Shell, "sort *", Verdict::Ask)]);
    let again = fixture.prepare(&[rule(PermissionTool::Shell, "sort *", Verdict::Ask)]);

    let link = home.path.join(AUTH_FILE);
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        fixture.user_home.join(AUTH_FILE)
    );
    assert_eq!(home, again);
    assert_eq!(
        std::fs::read_to_string(fixture.user_home.join(CONFIG_FILE)).unwrap(),
        user_config
    );
    assert_eq!(
        std::fs::read_to_string(fixture.user_home.join("rules/default.rules")).unwrap(),
        "prefix_rule(pattern = [\"sort\"], decision = \"allow\")\n"
    );
}

#[test]
fn home_files_are_private_and_rules_replace_the_user_rules() {
    let fixture = Fixture::new(USER_CONFIG);

    let home = fixture.prepare(&[rule(PermissionTool::Shell, "sort *", Verdict::Ask)]);

    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home.path.join(CONFIG_FILE)), 0o600);
    assert_eq!(mode(&home.path), 0o700);
    assert_eq!(
        std::fs::read_to_string(home.path.join(RULES_FILE)).unwrap(),
        "prefix_rule(pattern = [\"sort\"], decision = \"prompt\")\n"
    );
}

#[test]
fn rule_sets_get_separate_homes() {
    let fixture = Fixture::new("");

    let first = fixture.prepare(&[rule(PermissionTool::Shell, "ls", Verdict::Ask)]);
    let second = fixture.prepare(&[rule(PermissionTool::Shell, "ls", Verdict::Deny)]);

    assert_ne!(first.path, second.path);
    assert!(first.path.starts_with(fixture.saturn_home.join(HOMES_DIR)));
}

#[test]
fn shell_rules_translate_to_execpolicy_decisions() {
    let rules = vec![
        rule(PermissionTool::Shell, "git status *", Verdict::Allow),
        rule(PermissionTool::Shell, "git push *", Verdict::Ask),
        rule(PermissionTool::Shell, "rm *", Verdict::Deny),
        rule(PermissionTool::Shell, "ls", Verdict::Allow),
        rule(PermissionTool::Shell, "make", Verdict::Deny),
        rule(PermissionTool::Shell, "cargo * --release", Verdict::Deny),
        rule(PermissionTool::Shell, "*", Verdict::Allow),
        rule(PermissionTool::Shell, "echo $HOME", Verdict::Deny),
        rule(PermissionTool::Edit, "rm *", Verdict::Deny),
    ];

    let text = execpolicy(&rules);

    assert_eq!(
        text,
        "prefix_rule(pattern = [\"git\", \"status\"], decision = \"allow\")\n\
         prefix_rule(pattern = [\"git\", \"push\"], decision = \"prompt\")\n\
         prefix_rule(pattern = [\"rm\"], decision = \"forbidden\")\n\
         prefix_rule(pattern = [\"make\"], decision = \"forbidden\")\n"
    );
}

#[test]
fn mcp_rules_translate_to_approval_modes_and_disabled_tools() {
    let fixture = Fixture::new(USER_CONFIG);
    let rules = vec![
        rule(PermissionTool::Mcp, "mcp__docs__*", Verdict::Allow),
        rule(PermissionTool::Mcp, "mcp__docs__delete", Verdict::Deny),
        rule(PermissionTool::Mcp, "mcp__docs__write", Verdict::Ask),
        rule(PermissionTool::Mcp, "mcp__web__*", Verdict::Deny),
    ];

    let home = fixture.prepare(&rules);

    let config = config_of(&home);
    let docs = &config["mcp_servers"]["docs"];
    assert_eq!(docs["default_tools_approval_mode"].as_str(), Some("prompt"));
    let disabled: Vec<&str> = docs["disabled_tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool.as_str())
        .collect();
    assert_eq!(disabled, vec!["delete"]);
    assert_eq!(
        docs["tools"]["write"]["approval_mode"].as_str(),
        Some("prompt")
    );
    assert_eq!(
        config["mcp_servers"]["web"]["enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(home.mcp_servers, vec!["docs"]);
}

#[test]
fn mcp_allow_for_a_whole_server_is_approve_and_default_is_prompt() {
    let fixture = Fixture::new(USER_CONFIG);
    let rules = vec![
        rule(PermissionTool::Mcp, "mcp__docs__*", Verdict::Allow),
        rule(PermissionTool::Mcp, "mcp__docs__echo", Verdict::Allow),
    ];

    let home = fixture.prepare(&rules);

    let config = config_of(&home);
    let servers = &config["mcp_servers"];
    assert_eq!(
        servers["docs"]["default_tools_approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(
        servers["docs"]["tools"]["echo"]["approval_mode"].as_str(),
        Some("approve")
    );
    assert_eq!(
        servers["web"]["default_tools_approval_mode"].as_str(),
        Some("prompt")
    );
    assert_eq!(home.mcp_servers, vec!["docs", "web"]);
}

#[test]
fn user_disabled_mcp_server_is_not_waited_for() {
    let fixture = Fixture::new(USER_CONFIG);

    let home = fixture.prepare(&[]);

    assert!(!home.mcp_servers.contains(&"off".to_owned()));
}

#[test]
fn mode_never_enters_the_translation() {
    let fixture = Fixture::new(USER_CONFIG);

    let home = fixture.prepare(&[]);

    let config = config_of(&home);
    assert_eq!(
        config["mcp_servers"]["docs"]["default_tools_approval_mode"].as_str(),
        Some("prompt")
    );
}

#[test]
fn agent_questions_are_on_by_default_and_override_the_user_features() {
    let fixture =
        Fixture::new("[features]\ndefault_mode_request_user_input = false\nother = true\n");

    let home = fixture.prepare(&[]);

    let config = config_of(&home);
    assert_eq!(config["features"][QUESTIONS_FEATURE].as_bool(), Some(true));
    assert_eq!(config["features"]["other"].as_bool(), Some(true));
}

#[test]
fn agent_questions_are_off_in_a_separate_home_when_disabled() {
    let fixture = Fixture::new("[features]\ndefault_mode_request_user_input = true\n");
    let rules = [rule(PermissionTool::Shell, "sort *", Verdict::Ask)];
    let asking = fixture.prepare(&rules);

    let silent = prepare(HomeInput {
        saturn_home: &fixture.saturn_home,
        user_codex_home: &fixture.user_home,
        rules: &rules,
        questions: false,
    })
    .unwrap();

    assert_eq!(
        config_of(&silent)["features"][QUESTIONS_FEATURE].as_bool(),
        Some(false)
    );
    assert_eq!(
        config_of(&asking)["features"][QUESTIONS_FEATURE].as_bool(),
        Some(true)
    );
    assert_ne!(asking.path, silent.path);
    assert_eq!(rules_of_home(&asking.path), rules_of_home(&silent.path));
    assert_eq!(rules_of_home(&asking.path), Some(rules_fingerprint(&rules)));
}

#[test]
fn agent_questions_feature_is_dropped_from_profiles() {
    let fixture =
        Fixture::new("[profiles.work.features]\ndefault_mode_request_user_input = true\n");

    let home = fixture.prepare(&[]);

    assert!(
        config_of(&home)["profiles"]["work"]["features"]
            .get(QUESTIONS_FEATURE)
            .is_none()
    );
}

// #348
#[test]
fn only_read_deny_rules_change_the_rules_fingerprint() {
    let shell = rule(PermissionTool::Shell, "rm *", Verdict::Deny);
    let read = rule(PermissionTool::Read, "/etc/*", Verdict::Allow);
    let deny = rule(PermissionTool::Read, "/etc/*", Verdict::Deny);

    assert_eq!(
        rules_fingerprint(std::slice::from_ref(&shell)),
        rules_fingerprint(&[shell.clone(), read])
    );
    assert_ne!(
        rules_fingerprint(std::slice::from_ref(&shell)),
        rules_fingerprint(&[shell, deny])
    );
}

mod extension_parts {
    use super::*;
    use crate::providers::InjectedPart;
    use saturn_protocol::rpc::ExtensionPartKind;

    const LAYOUT: ExtensionLayout = ExtensionLayout {
        skills_dir: Some("skills"),
        commands: Some(("prompts", "md")),
        mcp_servers: true,
        hooks: false,
    };

    fn write(root: &Path, file: &str, content: &str) -> PathBuf {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn part(kind: ExtensionPartKind, name: &str, source: PathBuf) -> InjectedPart {
        InjectedPart {
            extension: "kit".to_owned(),
            kind,
            name: name.to_owned(),
            source,
        }
    }

    fn kit_parts(root: &Path) -> Vec<InjectedPart> {
        write(root, "kit/skills/commit-helper/SKILL.md", "# commit helper");
        write(root, "kit/commands/review.md", "review the diff");
        let servers = write(
            root,
            "kit/.mcp.json",
            r#"{"mcpServers":{"lint":{"command":"lint","args":["--x"],"env":{"A":"1"},"type":"stdio"},"web":{"command":"mine"},"empty":{}}}"#,
        );
        vec![
            part(
                ExtensionPartKind::Skill,
                "commit-helper",
                root.join("kit/skills/commit-helper"),
            ),
            part(
                ExtensionPartKind::Command,
                "review",
                root.join("kit/commands/review.md"),
            ),
            part(ExtensionPartKind::McpServer, "lint", servers.clone()),
            part(ExtensionPartKind::McpServer, "web", servers.clone()),
            part(ExtensionPartKind::McpServer, "empty", servers),
        ]
    }

    fn prepare_with_kit(
        fixture: &Fixture,
        parts: &[InjectedPart],
        rules: &[Rule],
    ) -> (PreparedHome, Vec<InjectionFailure>) {
        prepare_with(
            HomeInput {
                saturn_home: &fixture.saturn_home,
                user_codex_home: &fixture.user_home,
                rules,
                questions: true,
            },
            &LAYOUT,
            ExtensionInput {
                parts,
                fingerprint: "abc123",
            },
        )
        .unwrap()
    }

    #[test]
    fn skills_and_commands_are_copied_and_servers_follow_the_permission_rules() {
        let fixture = Fixture::new(USER_CONFIG);
        let parts = kit_parts(&fixture._root.path().join("store"));
        let rules = [rule(PermissionTool::Mcp, "mcp__lint__*", Verdict::Allow)];

        let (home, failures) = prepare_with_kit(&fixture, &parts, &rules);

        assert_eq!(
            std::fs::read_to_string(home.path.join("skills/commit-helper/SKILL.md")).unwrap(),
            "# commit helper"
        );
        assert_eq!(
            std::fs::read_to_string(home.path.join("prompts/review.md")).unwrap(),
            "review the diff"
        );
        let config = config_of(&home);
        let lint = &config["mcp_servers"]["lint"];
        assert_eq!(lint["command"].as_str(), Some("lint"));
        assert_eq!(lint["args"][0].as_str(), Some("--x"));
        assert_eq!(lint["env"]["A"].as_str(), Some("1"));
        assert!(lint.get("type").is_none());
        assert_eq!(
            lint["default_tools_approval_mode"].as_str(),
            Some("approve")
        );
        assert!(home.mcp_servers.contains(&"lint".to_owned()));
        let failed: Vec<&str> = failures
            .iter()
            .map(|failure| failure.part.as_str())
            .collect();
        assert_eq!(failed, vec!["web", "empty"]);
    }

    #[test]
    fn connecting_again_with_the_same_extensions_reuses_the_folder_without_failures() {
        let fixture = Fixture::new(USER_CONFIG);
        let parts = kit_parts(&fixture._root.path().join("store"));
        let (first, first_failures) = prepare_with_kit(&fixture, &parts, &[]);

        let (second, second_failures) = prepare_with_kit(&fixture, &parts, &[]);

        let names = |failures: &[InjectionFailure]| -> Vec<String> {
            failures
                .iter()
                .map(|failure| failure.part.clone())
                .collect()
        };
        assert_eq!(second.path, first.path);
        assert_eq!(names(&second_failures), names(&first_failures));
        assert_eq!(names(&second_failures), vec!["web", "empty"]);
        assert_eq!(
            std::fs::read_to_string(second.path.join("skills/commit-helper/SKILL.md")).unwrap(),
            "# commit helper"
        );
        assert_eq!(
            std::fs::read_to_string(second.path.join("prompts/review.md")).unwrap(),
            "review the diff"
        );
    }

    #[test]
    fn a_skill_name_with_different_content_fails_while_the_same_content_does_not() {
        let fixture = Fixture::new(USER_CONFIG);
        let store = fixture._root.path().join("store");
        let mut parts = kit_parts(&store);
        write(
            &store,
            "same/skills/commit-helper/SKILL.md",
            "# commit helper",
        );
        write(&store, "other/skills/commit-helper/SKILL.md", "# other");
        let skill = |extension: &str| InjectedPart {
            extension: extension.to_owned(),
            ..part(
                ExtensionPartKind::Skill,
                "commit-helper",
                store.join(extension).join("skills/commit-helper"),
            )
        };
        parts.extend([skill("same"), skill("other")]);

        let (home, failures) = prepare_with_kit(&fixture, &parts, &[]);

        let conflicts: Vec<(&str, &str)> = failures
            .iter()
            .filter(|failure| failure.part == "commit-helper")
            .map(|failure| (failure.extension.as_str(), failure.reason.as_str()))
            .collect();
        assert_eq!(
            conflicts,
            vec![(
                "other",
                "a skill with this name is already injected with different content"
            )]
        );
        assert_eq!(
            std::fs::read_to_string(home.path.join("skills/commit-helper/SKILL.md")).unwrap(),
            "# commit helper"
        );
    }

    #[test]
    fn the_folder_name_carries_the_extension_fingerprint_after_the_rules_fingerprint() {
        let fixture = Fixture::new(USER_CONFIG);
        let parts = kit_parts(&fixture._root.path().join("store"));

        let (with, _) = prepare_with_kit(&fixture, &parts, &[]);
        let without = fixture.prepare(&[]);

        assert_ne!(with.path, without.path);
        assert!(with.path.to_string_lossy().ends_with("-xabc123"));
        assert_eq!(rules_of_home(&with.path), rules_of_home(&without.path));
        assert!(!without.path.join("skills").exists());
    }

    #[test]
    fn the_user_codex_folder_is_left_as_it_was() {
        let fixture = Fixture::new(USER_CONFIG);
        let parts = kit_parts(&fixture._root.path().join("store"));
        let before = std::fs::read_to_string(fixture.user_home.join(CONFIG_FILE)).unwrap();

        prepare_with_kit(&fixture, &parts, &[]);

        assert_eq!(
            std::fs::read_to_string(fixture.user_home.join(CONFIG_FILE)).unwrap(),
            before
        );
        assert!(!fixture.user_home.join("skills").exists());
        assert!(!fixture.user_home.join("prompts").exists());
    }
}
