//! #294: 생성한 훅 명령을 그대로 실행해 키 보호 훅의 입출력과 종료 코드를 확인한다.
//! 설계: docs/design/router-key-security.md

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("fixture should work");
        std::fs::create_dir_all(dir.path().join(".saturn")).expect("fixture should work");
        Self { dir }
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    /// 엔진이 Claude에 넘기는 설정에서 훅 명령 문자열을 꺼낸다.
    fn hook_command(&self) -> String {
        let bin = Path::new(env!("CARGO_BIN_EXE_saturn-engine"));
        let settings = saturn_engine::pre_tool_use_hook_settings(
            &self.home().join(".saturn"),
            self.home(),
            bin,
        );
        settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("fixture should work")
            .to_owned()
    }

    fn run(&self, stdin: &str) -> Output {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(self.hook_command())
            .env_clear()
            .env("HOME", self.home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fixture should work");
        child
            .stdin
            .take()
            .expect("fixture should work")
            .write_all(stdin.as_bytes())
            .expect("fixture should work");
        child.wait_with_output().expect("fixture should work")
    }

    fn call(&self, tool: &str, tool_input: Value) -> Output {
        let input = json!({
            "hook_event_name": "PreToolUse",
            "cwd": self.home(),
            "tool_name": tool,
            "tool_input": tool_input,
        });
        self.run(&input.to_string())
    }
}

#[test]
fn hook_command_denies_key_store_access() {
    let fixture = Fixture::new();
    let cases = [
        (
            "Bash",
            json!({ "command": "security find-generic-password -s saturn -w" }),
        ),
        ("Bash", json!({ "command": "cat ~/.saturn/router.key" })),
        (
            "Read",
            json!({ "file_path": fixture.home().join(".saturn/router.key") }),
        ),
        ("Read", json!({ "file_path": ".saturn/router.key" })),
        (
            "Grep",
            json!({ "pattern": "x", "path": fixture.home().join("Library/Keychains") }),
        ),
    ];

    for (tool, tool_input) in cases {
        let output = fixture.call(tool, tool_input.clone());

        assert_eq!(output.status.code(), Some(0), "{tool} {tool_input}");
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        let decision = &reply["hookSpecificOutput"];
        assert_eq!(decision["hookEventName"], "PreToolUse");
        assert_eq!(
            decision["permissionDecision"], "deny",
            "{tool} {tool_input}"
        );
        assert!(decision["permissionDecisionReason"].as_str().is_some());
    }
}

#[test]
fn hook_command_allows_ordinary_calls_without_output() {
    let fixture = Fixture::new();
    let cases = [
        ("Bash", json!({ "command": "cargo test -p saturn-engine" })),
        (
            "Read",
            json!({ "file_path": fixture.home().join(".saturn/config.toml") }),
        ),
        ("mcp__server__tool", json!({ "anything": 1 })),
    ];

    for (tool, tool_input) in cases {
        let output = fixture.call(tool, tool_input.clone());

        assert_eq!(output.status.code(), Some(0), "{tool} {tool_input}");
        assert!(output.stdout.is_empty(), "{tool} {tool_input}");
    }
}

#[test]
fn hook_command_blocks_unreadable_input_with_exit_code_2() {
    let fixture = Fixture::new();

    for input in ["not json", "{}"] {
        let output = fixture.run(input);

        assert_eq!(output.status.code(), Some(2), "{input}");
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn hook_command_leaves_saturn_home_untouched() {
    let fixture = Fixture::new();

    let output = fixture.call("Bash", json!({ "command": "ls" }));

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    let entries = std::fs::read_dir(fixture.home().join(".saturn")).unwrap();
    assert_eq!(entries.count(), 0);
}

/// 작업 폴더와 더한 폴더를 훅 명령 인자로 넘겨 실행한다. 폴더는 고정 시험 폴더 밑에 만든다.
struct ScopedFixture {
    fixture: Fixture,
    work: std::path::PathBuf,
    added: std::path::PathBuf,
    outside: std::path::PathBuf,
}

impl ScopedFixture {
    fn new() -> Self {
        let fixture = Fixture::new();
        let root = fixture.home().canonicalize().expect("fixture should work");
        let (work, added, outside) = (root.join("work"), root.join("added"), root.join("outside"));
        for dir in [&work, &added, &outside] {
            std::fs::create_dir_all(dir).expect("fixture should work");
        }
        Self {
            fixture,
            work,
            added,
            outside,
        }
    }

    fn decision(&self, tool: &str, tool_input: Value) -> Option<String> {
        let settings = saturn_engine::with_read_scope(
            saturn_engine::pre_tool_use_hook_settings(
                &self.fixture.home().join(".saturn"),
                self.fixture.home(),
                Path::new(env!("CARGO_BIN_EXE_saturn-engine")),
            ),
            &self.work,
            std::slice::from_ref(&self.added),
        );
        let command = settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("fixture should work")
            .to_owned();
        let input = json!({
            "hook_event_name": "PreToolUse",
            "cwd": self.work,
            "tool_name": tool,
            "tool_input": tool_input,
        });
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .env_clear()
            .env("HOME", self.fixture.home())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("fixture should work");
        child
            .stdin
            .take()
            .expect("fixture should work")
            .write_all(input.to_string().as_bytes())
            .expect("fixture should work");
        let output = child.wait_with_output().expect("fixture should work");
        assert_eq!(output.status.code(), Some(0));
        if output.stdout.is_empty() {
            return None;
        }
        let value: Value = serde_json::from_slice(&output.stdout).expect("hook output is JSON");
        Some(
            value["hookSpecificOutput"]["permissionDecision"]
                .as_str()
                .expect("decision should exist")
                .to_owned(),
        )
    }
}

// #348
#[test]
fn hook_command_asks_for_a_read_outside_the_working_folders() {
    let fixture = ScopedFixture::new();
    let outside_file = fixture.outside.join("secret.txt");
    let cases = [
        ("Read", json!({ "file_path": outside_file })),
        ("Read", json!({ "file_path": "../outside/secret.txt" })),
        ("NotebookRead", json!({ "notebook_path": outside_file })),
        ("LS", json!({ "path": fixture.outside })),
        ("Grep", json!({ "pattern": "x", "path": fixture.outside })),
        (
            "Glob",
            json!({ "pattern": "*.txt", "path": fixture.outside }),
        ),
        (
            "Glob",
            json!({ "pattern": format!("{}/*.txt", fixture.outside.display()) }),
        ),
    ];

    for (tool, tool_input) in cases {
        assert_eq!(
            fixture.decision(tool, tool_input.clone()).as_deref(),
            Some("ask"),
            "{tool} {tool_input}"
        );
    }
}

// #348
#[test]
fn hook_command_leaves_reads_inside_the_working_folders_to_the_provider() {
    let fixture = ScopedFixture::new();
    let cases = [
        ("Read", json!({ "file_path": fixture.work.join("a.txt") })),
        ("Read", json!({ "file_path": "src/a.txt" })),
        (
            "Read",
            json!({ "file_path": fixture.added.join("sub/a.txt") }),
        ),
        ("Grep", json!({ "pattern": "x" })),
        ("Glob", json!({ "pattern": "**/*.rs" })),
        ("LS", json!({ "path": fixture.added })),
        (
            "Glob",
            json!({ "pattern": format!("{}/**/*.rs", fixture.added.display()) }),
        ),
        ("Bash", json!({ "command": "cat ../outside/secret.txt" })),
        (
            "Write",
            json!({ "file_path": fixture.outside.join("a.txt") }),
        ),
    ];

    for (tool, tool_input) in cases {
        assert_eq!(
            fixture.decision(tool, tool_input.clone()),
            None,
            "{tool} {tool_input}"
        );
    }
}

// #348
#[test]
fn hook_command_resolves_links_and_dots_before_judging_a_read() {
    let fixture = ScopedFixture::new();
    std::os::unix::fs::symlink(&fixture.outside, fixture.work.join("link"))
        .expect("fixture should work");

    let through_link = fixture.decision(
        "Read",
        json!({ "file_path": fixture.work.join("link/secret.txt") }),
    );
    let dotted = fixture.decision(
        "Read",
        json!({ "file_path": fixture.work.join("a/../../outside/secret.txt") }),
    );
    let inside_dotted = fixture.decision(
        "Read",
        json!({ "file_path": fixture.work.join("a/../b.txt") }),
    );

    assert_eq!(through_link.as_deref(), Some("ask"));
    assert_eq!(dotted.as_deref(), Some("ask"));
    assert_eq!(inside_dotted, None);
}

// #348
#[test]
fn key_protection_deny_wins_over_the_outside_read_ask() {
    let fixture = ScopedFixture::new();
    let key_file = fixture.fixture.home().join(".saturn/router.key");

    let decision = fixture.decision("Read", json!({ "file_path": key_file }));

    assert_eq!(decision.as_deref(), Some("deny"));
}
