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
fn home_does_not_delete_a_non_symlink_auth_file() {
    let fixture = Fixture::new(USER_CONFIG);
    let home = fixture.prepare(&[]);
    let target = home.path.join(AUTH_FILE);
    std::fs::remove_file(&target).unwrap();
    std::fs::write(&target, "keep").unwrap();

    let error = prepare(HomeInput {
        saturn_home: &fixture.saturn_home,
        user_codex_home: &fixture.user_home,
        rules: &[],
    })
    .unwrap_err();

    assert!(matches!(error, HomeError::Write { .. }));
    assert_eq!(std::fs::read_to_string(target).unwrap(), "keep");
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
