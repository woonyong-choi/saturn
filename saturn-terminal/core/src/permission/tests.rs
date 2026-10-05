use super::*;

fn call(tool: PermissionTool, target: &str, paths: &[&str]) -> PermissionCall {
    PermissionCall {
        tool,
        target: target.to_owned(),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        outside_sandbox: false,
        reads_only: false,
    }
}

fn shell(command: &str) -> PermissionCall {
    call(PermissionTool::Shell, command, &[])
}

fn edit(path: &str) -> PermissionCall {
    call(PermissionTool::Edit, "", &[path])
}

fn rule(tool: PermissionTool, pattern: &str, verdict: Verdict) -> Rule {
    Rule {
        tool,
        pattern: pattern.to_owned(),
        verdict,
    }
}

fn policy(mode: Mode, rules: Vec<Rule>) -> Policy {
    Policy {
        mode,
        workdir: PathBuf::from("/work"),
        extra_dirs: Vec::new(),
        rules,
        always: Vec::new(),
    }
}

#[test]
fn mode_edit_allows_edits_inside_added_folders_like_the_workdir() {
    let mut policy = policy(Mode::Edit, Vec::new());
    policy.extra_dirs = vec![PathBuf::from("/shared/lib")];

    assert_eq!(policy.decide(&edit("/shared/lib/src/a.rs")), Verdict::Allow);
    assert_eq!(
        policy.decide(&edit("/shared/lib/../secret/a.rs")),
        Verdict::Ask
    );
    assert_eq!(policy.decide(&edit("/shared/library/a.rs")), Verdict::Ask);
    assert_eq!(
        policy.decide(&edit("/shared/lib/a.rs")),
        Verdict::Allow,
        "an added folder is checked per path, not per mode"
    );
}

#[test]
fn added_folders_do_not_widen_read_only_or_deny_rules() {
    let mut read_only = policy(Mode::ReadOnly, Vec::new());
    read_only.extra_dirs = vec![PathBuf::from("/shared/lib")];
    let mut denied = policy(
        Mode::Edit,
        vec![rule(
            PermissionTool::Edit,
            "/shared/lib/vendor/*",
            Verdict::Deny,
        )],
    );
    denied.extra_dirs = vec![PathBuf::from("/shared/lib")];

    assert_eq!(read_only.decide(&edit("/shared/lib/a.rs")), Verdict::Deny);
    assert_eq!(
        denied.decide(&edit("/shared/lib/vendor/a.rs")),
        Verdict::Deny
    );
}

#[test]
fn mode_default_rules() {
    let inside = edit("/work/src/lib.rs");
    let outside = edit("/etc/hosts");
    let mcp = call(PermissionTool::Mcp, "mcp__s__echo", &[]);
    let subagent = call(PermissionTool::Subagent, "explorer", &[]);
    let table = [
        (
            Mode::ReadOnly,
            [Verdict::Deny, Verdict::Deny, Verdict::Deny, Verdict::Deny],
        ),
        (
            Mode::Ask,
            [Verdict::Ask, Verdict::Ask, Verdict::Ask, Verdict::Ask],
        ),
        (
            Mode::Edit,
            [Verdict::Allow, Verdict::Ask, Verdict::Ask, Verdict::Ask],
        ),
        (
            Mode::Full,
            [
                Verdict::Allow,
                Verdict::Allow,
                Verdict::Allow,
                Verdict::Allow,
            ],
        ),
    ];

    for (mode, [edit_inside, edit_outside, mcp_verdict, subagent_verdict]) in table {
        let policy = policy(mode, Vec::new());

        assert_eq!(policy.decide(&inside), edit_inside, "{mode:?} inside edit");
        assert_eq!(policy.decide(&outside), edit_outside, "{mode:?} outside");
        assert_eq!(policy.decide(&mcp), mcp_verdict, "{mode:?} mcp");
        assert_eq!(
            policy.decide(&subagent),
            subagent_verdict,
            "{mode:?} subagent"
        );
    }
    let shell_by_mode: Vec<Verdict> = [Mode::ReadOnly, Mode::Ask, Mode::Edit, Mode::Full]
        .into_iter()
        .map(|mode| policy(mode, Vec::new()).decide(&shell("touch a.txt")))
        .collect();
    assert_eq!(
        shell_by_mode,
        vec![Verdict::Deny, Verdict::Ask, Verdict::Ask, Verdict::Allow]
    );
    assert_eq!(Mode::DEFAULT, Mode::Edit);
}

#[test]
fn mode_edit_treats_dotdot_escape_as_outside() {
    let policy = policy(Mode::Edit, Vec::new());

    assert_eq!(policy.decide(&edit("src/a.rs")), Verdict::Allow);
    assert_eq!(policy.decide(&edit("/work/../etc/passwd")), Verdict::Ask);
    assert_eq!(policy.decide(&edit("../other/a.rs")), Verdict::Ask);
    assert_eq!(policy.decide(&edit("/workshop/a.rs")), Verdict::Ask);
}

#[test]
fn mode_order_is_read_only_ask_edit_full() {
    assert!(Mode::ReadOnly < Mode::Ask);
    assert!(Mode::Ask < Mode::Edit);
    assert!(Mode::Edit < Mode::Full);
    assert_eq!(Mode::parse("read-only"), Some(Mode::ReadOnly));
    assert_eq!(Mode::parse("plan"), None);
}

#[test]
fn folder_mode_cannot_raise() {
    assert_eq!(Mode::Edit.lowered_by(Mode::ReadOnly), Some(Mode::ReadOnly));
    assert_eq!(Mode::Edit.lowered_by(Mode::Ask), Some(Mode::Ask));
    assert_eq!(Mode::Edit.lowered_by(Mode::Edit), None);
    assert_eq!(Mode::Edit.lowered_by(Mode::Full), None);
    assert_eq!(Mode::Ask.lowered_by(Mode::Full), None);
}

#[test]
fn last_matching_rule_wins() {
    let rules = vec![
        rule(PermissionTool::Shell, "git *", Verdict::Allow),
        rule(PermissionTool::Shell, "git push *", Verdict::Ask),
    ];
    let policy = policy(Mode::Edit, rules);

    assert_eq!(policy.decide(&shell("git status")), Verdict::Allow);
    assert_eq!(policy.decide(&shell("git push origin main")), Verdict::Ask);
}

#[test]
fn rule_allow_overrides_mode_default_deny() {
    let rules = vec![rule(PermissionTool::Shell, "git status *", Verdict::Allow)];
    let policy = policy(Mode::ReadOnly, rules);

    assert_eq!(policy.decide(&shell("git status")), Verdict::Allow);
    assert_eq!(policy.decide(&shell("git commit")), Verdict::Deny);
}

#[test]
fn deny_is_sticky() {
    let rules = vec![
        rule(PermissionTool::Shell, "rm *", Verdict::Deny),
        rule(PermissionTool::Shell, "rm *", Verdict::Allow),
        rule(PermissionTool::Shell, "*", Verdict::Allow),
    ];
    let policy = policy(Mode::Full, rules);

    assert_eq!(policy.decide(&shell("rm -rf build")), Verdict::Deny);
    assert_eq!(policy.decide(&shell("ls")), Verdict::Allow);
}

#[test]
fn deny_beats_always_allow() {
    let mut policy = policy(
        Mode::Edit,
        vec![rule(PermissionTool::Shell, "cargo publish", Verdict::Deny)],
    );
    policy.always = vec![rule(PermissionTool::Shell, "cargo publish", Verdict::Allow)];

    assert_eq!(policy.decide(&shell("cargo publish")), Verdict::Deny);
}

#[test]
fn always_allow_only_lifts_ask() {
    let mut policy = policy(Mode::Edit, Vec::new());
    policy.always = vec![rule(PermissionTool::Shell, "cargo test", Verdict::Allow)];

    assert_eq!(policy.decide(&shell("cargo test")), Verdict::Allow);
    assert_eq!(policy.decide(&shell("cargo test --all")), Verdict::Ask);
    policy.mode = Mode::ReadOnly;
    assert_eq!(policy.decide(&shell("cargo test")), Verdict::Deny);
}

#[test]
fn compound_command_takes_the_strictest_part() {
    let rules = vec![
        rule(PermissionTool::Shell, "git status *", Verdict::Allow),
        rule(PermissionTool::Shell, "rm *", Verdict::Deny),
    ];
    let policy = policy(Mode::Edit, rules);

    assert_eq!(
        policy.decide(&shell("git status && rm -rf x")),
        Verdict::Deny
    );
    assert_eq!(policy.decide(&shell("git status; touch a")), Verdict::Ask);
    assert_eq!(policy.decide(&shell("git status | cat")), Verdict::Ask);
    assert_eq!(policy.decide(&shell("git status")), Verdict::Allow);
}

#[test]
fn command_substitution_is_never_allowed_by_rules_alone() {
    let rules = vec![rule(PermissionTool::Shell, "echo *", Verdict::Allow)];
    let mut policy = policy(Mode::Edit, rules);

    assert_eq!(policy.decide(&shell("echo $(ls)")), Verdict::Ask);
    policy
        .rules
        .push(rule(PermissionTool::Shell, "rm *", Verdict::Deny));
    assert_eq!(policy.decide(&shell("echo $(rm -rf x)")), Verdict::Deny);
    policy.always = vec![rule(
        PermissionTool::Shell,
        &escape("echo $(ls)"),
        Verdict::Allow,
    )];
    assert_eq!(policy.decide(&shell("echo $(ls)")), Verdict::Allow);
}

#[test]
fn edit_rule_matches_absolute_and_relative_paths() {
    let rules = vec![
        rule(PermissionTool::Edit, "secrets/*", Verdict::Deny),
        rule(PermissionTool::Edit, "/work/docs/*", Verdict::Ask),
    ];
    let policy = policy(Mode::Full, rules);

    assert_eq!(policy.decide(&edit("secrets/key")), Verdict::Deny);
    assert_eq!(policy.decide(&edit("/work/secrets/key")), Verdict::Deny);
    assert_eq!(policy.decide(&edit("docs/a.md")), Verdict::Ask);
    assert_eq!(policy.decide(&edit("src/a.rs")), Verdict::Allow);
}

#[test]
fn edit_with_several_paths_takes_the_strictest() {
    let rules = vec![rule(PermissionTool::Edit, "*.lock", Verdict::Deny)];
    let policy = policy(Mode::Edit, rules);
    let patch = call(PermissionTool::Edit, "", &["a.rs", "Cargo.lock"]);

    assert_eq!(policy.decide(&patch), Verdict::Deny);
}

#[test]
fn edit_without_paths_asks_in_edit_mode() {
    let policy = policy(Mode::Edit, Vec::new());

    assert_eq!(
        policy.decide(&call(PermissionTool::Edit, "", &[])),
        Verdict::Ask
    );
}

#[test]
fn mcp_and_subagent_rules_match_their_targets() {
    let rules = vec![
        rule(PermissionTool::Mcp, "mcp__docs__*", Verdict::Allow),
        rule(PermissionTool::Mcp, "mcp__docs__delete", Verdict::Deny),
        rule(PermissionTool::Subagent, "explorer", Verdict::Allow),
    ];
    let policy = policy(Mode::Edit, rules);
    let mcp = |target: &str| call(PermissionTool::Mcp, target, &[]);

    assert_eq!(policy.decide(&mcp("mcp__docs__read")), Verdict::Allow);
    assert_eq!(policy.decide(&mcp("mcp__docs__delete")), Verdict::Deny);
    assert_eq!(policy.decide(&mcp("mcp__web__fetch")), Verdict::Ask);
    assert_eq!(
        policy.decide(&call(PermissionTool::Subagent, "explorer", &[])),
        Verdict::Allow
    );
    assert_eq!(
        policy.decide(&call(PermissionTool::Subagent, "worker", &[])),
        Verdict::Ask
    );
}

#[test]
fn always_rules_cover_only_the_parts_that_asked() {
    let rules = vec![rule(PermissionTool::Shell, "git status *", Verdict::Allow)];
    let policy = policy(Mode::Edit, rules);

    let stored = policy.always_rules(&shell("git status && cargo test * && make"));

    let patterns: Vec<&str> = stored.iter().map(|rule| rule.pattern.as_str()).collect();
    assert_eq!(patterns, vec![r"cargo test \*", "make"]);
    assert!(stored.iter().all(|rule| rule.verdict == Verdict::Allow));
}

#[test]
fn always_rules_for_edit_store_the_absolute_path() {
    let policy = policy(Mode::Edit, Vec::new());

    let stored = policy.always_rules(&edit("../outside/a.rs"));

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].pattern, "/outside/a.rs");
}

#[test]
fn always_rules_for_opaque_command_store_the_whole_command() {
    let policy = policy(Mode::Edit, Vec::new());

    let stored = policy.always_rules(&shell("echo $(ls)"));

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].pattern, "echo $(ls)");
}

#[test]
fn rules_verdict_ignores_mode_and_returns_none_without_a_match() {
    let rules = vec![
        rule(PermissionTool::Shell, "git *", Verdict::Allow),
        rule(PermissionTool::Shell, "git push *", Verdict::Deny),
    ];

    assert_eq!(
        rules_verdict(&rules, PermissionTool::Shell, "git status"),
        Some(Verdict::Allow)
    );
    assert_eq!(
        rules_verdict(&rules, PermissionTool::Shell, "git push x"),
        Some(Verdict::Deny)
    );
    assert_eq!(rules_verdict(&rules, PermissionTool::Shell, "ls"), None);
    assert_eq!(
        rules_verdict(&rules, PermissionTool::Mcp, "git status"),
        None
    );
}

#[test]
fn tool_and_verdict_names_round_trip() {
    for name in ["shell", "edit", "read", "mcp", "subagent"] {
        assert_eq!(tool_name(parse_tool(name).unwrap()), name);
    }
    for name in ["allow", "ask", "deny"] {
        assert_eq!(Verdict::parse(name).unwrap().name(), name);
    }
    assert_eq!(parse_tool("glob"), None);
    assert_eq!(Verdict::parse("maybe"), None);
}

#[test]
fn mode_edit_decides_shell_commands_by_their_read_only_form() {
    let policy = policy(Mode::Edit, Vec::new());
    // (사례, 명령 목록, 예상 판정)
    let cases: [(&str, &[&str], Verdict); 5] = [
        (
            "read-only commands with arguments",
            &[
                "ls",
                "ls -la src",
                "cat Cargo.toml",
                "rg todo src",
                "grep -rn todo .",
                "git status",
                "git diff HEAD~1",
                "git log --oneline",
                r#"grep "a|b" file"#,
                "cat",
            ],
            Verdict::Allow,
        ),
        (
            "commands outside the read-only list",
            &["lsof", "git push", "git statusx", "rm -rf x", "make"],
            Verdict::Ask,
        ),
        (
            "shell syntax mixed into a read-only command",
            &[
                "ls | wc -l",
                "ls > out.txt",
                "cat < in.txt",
                "ls; ls",
                "ls && ls",
                "ls || ls",
                "ls &",
                "ls\nls",
                "cat $(echo a)",
                "cat `echo a`",
                "ls;",
            ],
            Verdict::Ask,
        ),
        (
            "read-only command with a write or exec option",
            &[
                "rg --pre cat foo",
                "rg --pre=cat foo",
                "rg --hostname-bin=/bin/x foo",
                r#"rg --"pre" cat foo"#,
                "git diff --output=out.patch",
                "git diff --output out.patch",
                "git log --output=out.txt",
                "git diff --ext-diff",
                "git log --textconv",
                "git diff --out=out.patch",
            ],
            Verdict::Ask,
        ),
        (
            "read-only command with a safe option",
            &[
                "git diff --no-ext-diff",
                "git log --pretty=oneline",
                "rg --pre-glob '*.gz' foo",
                "git diff --no-textconv HEAD",
            ],
            Verdict::Allow,
        ),
    ];

    for (name, commands, expected) in cases {
        for command in commands {
            assert_eq!(
                policy.decide(&shell(command)),
                expected,
                "{name}: {command}"
            );
        }
    }
}

#[test]
fn read_only_shell_list_yields_to_individual_rules_and_deny_wins() {
    let ask = policy(
        Mode::Edit,
        vec![rule(PermissionTool::Shell, "git log *", Verdict::Ask)],
    );
    let deny = policy(
        Mode::Edit,
        vec![rule(PermissionTool::Shell, "cat *.pem", Verdict::Deny)],
    );

    assert_eq!(ask.decide(&shell("git log")), Verdict::Ask);
    assert_eq!(ask.decide(&shell("git status")), Verdict::Allow);
    assert_eq!(deny.decide(&shell("cat key.pem")), Verdict::Deny);
    assert_eq!(deny.decide(&shell("cat a.txt")), Verdict::Allow);
}

#[test]
fn read_only_shell_list_does_not_change_other_modes() {
    assert_eq!(
        policy(Mode::ReadOnly, Vec::new()).decide(&shell("ls")),
        Verdict::Deny
    );
    assert_eq!(
        policy(Mode::Ask, Vec::new()).decide(&shell("ls")),
        Verdict::Ask
    );
    assert_eq!(
        policy(Mode::Full, Vec::new()).decide(&shell("rm x")),
        Verdict::Allow
    );
}

#[test]
fn mode_edit_asks_for_edits_under_git_internals_but_not_other_dot_files() {
    let mut policy = policy(Mode::Edit, Vec::new());
    policy.extra_dirs = vec![PathBuf::from("/shared/lib")];

    for path in [
        "/work/.git/config",
        ".git/config",
        "/work/sub/.git/hooks/x",
        "/work/.git",
        "/shared/lib/.git/config",
    ] {
        assert_eq!(policy.decide(&edit(path)), Verdict::Ask, "{path}");
    }
    for path in ["/work/.gitignore", "/work/.github/x", "/work/src/.gitkeep"] {
        assert_eq!(policy.decide(&edit(path)), Verdict::Allow, "{path}");
    }
    policy.rules = vec![rule(PermissionTool::Edit, "*/.git/config", Verdict::Allow)];
    assert_eq!(policy.decide(&edit("/work/.git/config")), Verdict::Allow);
}

fn read(path: &str) -> PermissionCall {
    call(PermissionTool::Read, "", &[path])
}

#[test]
fn read_rules_decide_by_path_and_default_to_asking_without_a_rule() {
    let rules = vec![
        rule(PermissionTool::Read, "/etc/*", Verdict::Allow),
        rule(PermissionTool::Read, "/etc/shadow", Verdict::Deny),
    ];
    let policy = policy(Mode::Edit, rules);

    assert_eq!(policy.decide(&read("/etc/hosts")), Verdict::Allow);
    assert_eq!(policy.decide(&read("/etc/shadow")), Verdict::Deny);
    assert_eq!(policy.decide(&read("/var/log/a")), Verdict::Ask);
}

#[test]
fn read_without_a_rule_asks_in_every_mode_but_full() {
    for (mode, expected) in [
        (Mode::ReadOnly, Verdict::Ask),
        (Mode::Ask, Verdict::Ask),
        (Mode::Edit, Verdict::Ask),
        (Mode::Full, Verdict::Allow),
    ] {
        assert_eq!(
            policy(mode, Vec::new()).decide(&read("/etc/hosts")),
            expected
        );
    }
}

#[test]
fn read_rules_do_not_touch_edit_or_shell_calls() {
    let policy = policy(
        Mode::Edit,
        vec![rule(PermissionTool::Read, "*", Verdict::Allow)],
    );

    assert_eq!(policy.decide(&edit("/etc/hosts")), Verdict::Ask);
    assert_eq!(policy.decide(&shell("cat /etc/hosts")), Verdict::Allow);
    assert_eq!(policy.decide(&shell("touch a")), Verdict::Ask);
}

#[test]
fn always_allow_for_a_read_stores_the_path() {
    let policy = policy(Mode::Edit, Vec::new());

    let stored = policy.always_rules(&read("/etc/hosts"));

    assert_eq!(
        stored,
        vec![rule(PermissionTool::Read, "/etc/hosts", Verdict::Allow)]
    );
}

fn provider_read(command: &str, paths: &[&str]) -> PermissionCall {
    PermissionCall {
        reads_only: true,
        ..call(PermissionTool::Shell, command, paths)
    }
}

// #507, #348
#[test]
fn provider_classified_reads_follow_mode_and_read_rules() {
    struct Case {
        name: &'static str,
        mode: Mode,
        rules: Vec<Rule>,
        call: PermissionCall,
        expected: Verdict,
    }
    let case = |name, mode, rules, call, expected| Case {
        name,
        mode,
        rules,
        call,
        expected,
    };
    let secret = |verdict| vec![rule(PermissionTool::Read, "/secret/*", verdict)];
    let sed_main = || provider_read("sed -n 1p src/main.rs", &["src/main.rs"]);
    let mut unmarked = provider_read("sed -n 1p a", &["a"]);
    unmarked.reads_only = false;

    let cases = vec![
        case(
            "read-only mode allows a classified read inside the workdir",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("sed -n 1,240p src/main.rs", &["src/main.rs"]),
            Verdict::Allow,
        ),
        case(
            "a listing without a path reads the workdir",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("ls", &[]),
            Verdict::Allow,
        ),
        case(
            "an absolute path inside the workdir is allowed",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("cat /work/src/a.rs", &["/work/src/a.rs"]),
            Verdict::Allow,
        ),
        case(
            "a read outside the workdir asks",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("cat x", &["/etc/hosts"]),
            Verdict::Ask,
        ),
        case(
            "a read of a parent path asks",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("cat x", &["../x"]),
            Verdict::Ask,
        ),
        case(
            "a read inside git asks",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("cat x", &[".git/config"]),
            Verdict::Ask,
        ),
        case(
            "one outside path among several asks",
            Mode::ReadOnly,
            Vec::new(),
            provider_read("cat x", &["a", "/etc/hosts"]),
            Verdict::Ask,
        ),
        case(
            "a shell sed is not opened by read-only mode",
            Mode::ReadOnly,
            Vec::new(),
            shell("sed -n 1p src/main.rs"),
            Verdict::Deny,
        ),
        case(
            "a shell build is not opened by read-only mode",
            Mode::ReadOnly,
            Vec::new(),
            shell("cargo build"),
            Verdict::Deny,
        ),
        case(
            "a read the provider did not classify stays denied",
            Mode::ReadOnly,
            Vec::new(),
            unmarked,
            Verdict::Deny,
        ),
        case(
            "ask mode keeps asking for a classified read",
            Mode::Ask,
            Vec::new(),
            sed_main(),
            Verdict::Ask,
        ),
        case(
            "edit mode keeps asking for a classified read",
            Mode::Edit,
            Vec::new(),
            sed_main(),
            Verdict::Ask,
        ),
        case(
            "full mode allows a classified read",
            Mode::Full,
            Vec::new(),
            sed_main(),
            Verdict::Allow,
        ),
        case(
            "a shell deny rule beats a classified read in read-only mode",
            Mode::ReadOnly,
            vec![rule(PermissionTool::Shell, "sed *", Verdict::Deny)],
            provider_read("sed -n 1p a", &["a"]),
            Verdict::Deny,
        ),
        case(
            "a read deny rule reaches a classified read",
            Mode::Edit,
            secret(Verdict::Deny),
            provider_read("cat /secret/a.txt", &["/secret/a.txt"]),
            Verdict::Deny,
        ),
        case(
            "a read ask rule reaches a classified read",
            Mode::Edit,
            secret(Verdict::Ask),
            provider_read("cat /secret/a.txt", &["/secret/a.txt"]),
            Verdict::Ask,
        ),
        case(
            "one denied path denies the command",
            Mode::Edit,
            secret(Verdict::Deny),
            provider_read("cat /work/a /secret/a.txt", &["/work/a", "/secret/a.txt"]),
            Verdict::Deny,
        ),
        case(
            "a read rule leaves other paths to the shell rules",
            Mode::Edit,
            secret(Verdict::Deny),
            provider_read("cat /work/a", &["/work/a"]),
            Verdict::Allow,
        ),
        case(
            "a listing without a path is not judged by a read rule",
            Mode::Edit,
            secret(Verdict::Deny),
            provider_read("ls", &[]),
            Verdict::Allow,
        ),
        case(
            "a command the provider did not classify keeps the shell verdict",
            Mode::Edit,
            secret(Verdict::Deny),
            shell("cat /secret/a.txt"),
            Verdict::Allow,
        ),
        case(
            "a read allow rule does not loosen the shell side of the mode",
            Mode::ReadOnly,
            vec![rule(PermissionTool::Read, "/etc/*", Verdict::Allow)],
            provider_read("cat /etc/hosts", &["/etc/hosts"]),
            Verdict::Ask,
        ),
        case(
            "a later read allow overrides an earlier read ask",
            Mode::Edit,
            vec![
                rule(PermissionTool::Read, "/etc/*", Verdict::Ask),
                rule(PermissionTool::Read, "/etc/hosts", Verdict::Allow),
            ],
            provider_read("cat /etc/hosts", &["/etc/hosts"]),
            Verdict::Allow,
        ),
    ];

    for case in cases {
        let policy = policy(case.mode, case.rules);
        assert_eq!(policy.decide(&case.call), case.expected, "{}", case.name);
    }
}

// #348
#[test]
fn always_allow_for_a_read_command_that_a_read_rule_asks_about_sticks() {
    let mut policy = policy(
        Mode::Edit,
        vec![rule(PermissionTool::Read, "/secret/*", Verdict::Ask)],
    );
    let cmd = provider_read("cat /secret/a.txt", &["/secret/a.txt"]);
    assert_eq!(policy.decide(&cmd), Verdict::Ask);

    policy.always = policy.always_rules(&cmd);

    assert_eq!(policy.decide(&cmd), Verdict::Allow);
}
