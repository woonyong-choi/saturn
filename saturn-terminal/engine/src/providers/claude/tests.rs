use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use saturn_core::permission::{Mode, Policy, Rule, Verdict};
use saturn_protocol::event::{
    Activity, LineChange, LineRange, PermissionCall, PermissionTool, ToolCategory, ToolDetail,
    TurnOrigin, UsageReport, UsageScope,
};
use saturn_protocol::ids::{ChatId, SettingsRevision};
use saturn_protocol::input::InputValue;

use super::config::{default_args, read_user_config, with_ask_tools};
use super::convert::{cache_ttl, detail_of, permission_call, shell_exit_code};
use super::*;
use crate::events::{next_arrival, start_queued_turn};
use crate::providers::{PermissionLaunch, ProviderConnection, SaturnDefaults};
use crate::secrets::Masker;

/// 파이프 버퍼(64KiB)보다 커서 쓰는 도중 막히는 턴 크기. fake가 `big:<길이>`로 답한다.
const BIG_TURN_BYTES: usize = 400_000;

/// 받은 사용자 메시지 글에 따라 정해진 줄을 낸다.
const FAKE_CLAUDE: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP; use IO::Handle;
$| = 1;
my $json = JSON::PP->new->canonical;
my %arg; for (my $i = 0; $i < @ARGV; $i++) { $arg{$ARGV[$i]} = $ARGV[$i + 1] if $ARGV[$i] =~ /^--/; }
exit 3 if ($arg{"--resume"} // "") eq "missing";
my $sid = $arg{"--resume"} // $arg{"--session-id"};
my $model = "claude-test";
my $init = 0;
sub out { print $json->encode($_[0]), "\n"; }
sub init { out({ type => "system", subtype => "init", session_id => $sid, model => $model, permissionMode => "acceptEdits", cwd => "/w", tools => ["Bash"], slash_commands => ["compact", "review", "clear", "lint"], skills => ["lint"] }); }
sub assistant { my ($content, $parent, $usage) = @_; out({ type => "assistant", session_id => $sid, parent_tool_use_id => $parent, message => { role => "assistant", model => $model, content => $content, usage => $usage // { input_tokens => 10, cache_read_input_tokens => 1000, cache_creation_input_tokens => 200, output_tokens => 5 } } }); }
sub tool_result { my ($id, $text, $parent) = @_; out({ type => "user", session_id => $sid, parent_tool_use_id => $parent, message => { role => "user", content => [ { type => "tool_result", tool_use_id => $id, content => $text } ] } }); }
sub result { out({ type => "result", subtype => $_[0] // "success", is_error => JSON::PP::false, session_id => $sid, usage => { input_tokens => 30, cache_read_input_tokens => 2000, cache_creation_input_tokens => 200, output_tokens => 40 } }); }
sub failed_result { my ($subtype, $is_error, %extra) = @_; out({ type => "result", subtype => $subtype, is_error => $is_error ? JSON::PP::true : JSON::PP::false, session_id => $sid, usage => { input_tokens => 30, cache_read_input_tokens => 2000, cache_creation_input_tokens => 200, output_tokens => 40 }, %extra }); }
while (my $line = <STDIN>) {
  my $m = eval { $json->decode($line) } or next;
  if ($m->{type} eq "control_request") {
    out({ type => "control_response", response => { subtype => "success", request_id => $m->{request_id}, response => {} } });
    result("error_during_execution");
    next;
  }
  if ($m->{type} eq "control_response") {
    assistant([ { type => "text", text => "answer:" . $json->encode($m->{response}) } ], undef);
    result();
    next;
  }
  my $text = $m->{message}{content}[0]{text};
  if (!$init) { init(); $init = 1; }
  if ($text eq "hello") {
    assistant([ { type => "text", text => "hi" }, { type => "tool_use", id => "toolu_bash", name => "Bash", input => { command => "ls -la" } } ], undef);
    tool_result("toolu_bash", "total 0", undef);
    assistant([ { type => "tool_use", id => "toolu_task", name => "Task", input => { prompt => "look" } } ], undef);
    assistant([ { type => "tool_use", id => "toolu_read", name => "Read", input => { file_path => "/w/a.rs" } } ], "toolu_task", { input_tokens => 1 });
    tool_result("toolu_read", [ { type => "text", text => "fn main" } ], "toolu_task");
    tool_result("toolu_task", "found it", undef);
    assistant([ { type => "text", text => "done" } ], undef, { input_tokens => 20, cache_read_input_tokens => 3000, cache_creation_input_tokens => 100, output_tokens => 7 });
    result();
  } elsif ($text eq "merging-packet") {
    # 진행 중인 턴에 다음 사용자 메시지가 오면 합쳐서 `result`를 하나만 낸다(Claude Code 실측 동작)
    assistant([ { type => "text", text => "packet" } ], undef);
    select(undef, undef, undef, 0.5);
    STDIN->blocking(0);
    my $next = <STDIN>;
    STDIN->blocking(1);
    if (defined $next) {
      my $merged = eval { $json->decode($next) } // {};
      assistant([ { type => "text", text => "merged:" . ($merged->{message}{content}[0]{text} // "") } ], undef);
    }
    result();
  } elsif (length($text) > 100000) {
    assistant([ { type => "text", text => "big:" . length($text) } ], undef);
    result();
  } elsif ($text eq "wait") {
    assistant([ { type => "text", text => "working" } ], undef);
  } elsif ($text eq "env-check") {
    assistant([ { type => "text", text => "env:" . ($ENV{CLAUDE_CODE_RESUME_INTERRUPTED_TURN} // "unset") . ":" . ($ENV{KEEP_ME} // "none") } ], undef);
    result();
  } elsif ($text eq "more") {
    assistant([ { type => "text", text => "got more" } ], undef);
    result();
  } elsif ($text eq "ask") {
    out({ type => "control_request", request_id => "perm-1", request => { subtype => "can_use_tool", tool_name => "Bash", input => { command => "rm -rf build" }, decision_reason => "outside workdir" } });
  } elsif ($text eq "ask-user") {
    out({ type => "control_request", request_id => "ask-1", request => { subtype => "can_use_tool", tool_name => "AskUserQuestion", display_name => "AskUserQuestion", input => { questions => [ { question => "What is the name of your project?", header => "Project name", options => [ { label => "Saturn WT", description => "The Saturn Waterfall Testing project" }, { label => "Other project", description => "A different project name" } ], multiSelect => JSON::PP::false } ] }, tool_use_id => "toolu_ask", requires_user_interaction => JSON::PP::true } });
  } elsif ($text eq "secret") {
    assistant([ { type => "text", text => "sk-secret-1234" } ], undef);
    result();
  } elsif ($text eq "too-long") {
    # 맥락 초과: `success` 종류이지만 `is_error`가 참이고 `terminal_reason`에 이유가 있다(Claude Code 2.1.288 설치본의 구조)
    assistant([ { type => "text", text => "working" } ], undef);
    failed_result("success", 1, terminal_reason => "prompt_too_long", result => "Prompt is too long");
  } elsif ($text eq "max-turns") {
    failed_result("error_max_turns", 0, terminal_reason => "max_turns", errors => []);
  } elsif ($text eq "/compact") {
    $model = "claude-other";
    init();
    result();
  } elsif ($text eq "orphan") {
    assistant([ { type => "tool_use", id => "toolu_bg", name => "Agent", input => {} } ], undef);
    result();
    exit 0;
  } elsif ($text eq "crash") {
    exit 1;
  }
}
"#;

fn launch(dir: &Path, env: Vec<(OsString, OsString)>) -> LaunchSpec {
    let program = dir.join("fake-claude");
    std::fs::write(&program, FAKE_CLAUDE).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    // 새로 만든 실행 파일의 첫 실행은 macOS 검사로 수백 ms 늦다. 재개 실패 판정 시간에 걸리지 않게 한 번 미리 실행한다
    let warmed = std::process::Command::new(&program)
        .args(["--resume", "missing"])
        .status()
        .unwrap();
    assert_eq!(warmed.code(), Some(3));
    LaunchSpec {
        provider: crate::providers::test_support::CLAUDE,
        program,
        workdir: dir.to_path_buf(),
        settings: SettingsRevision(1),
        user_config: UserProviderConfig::default(),
        defaults: SaturnDefaults {
            auto_compact_tokens: Some(60_000),
        },
        env,
        hook_settings: Some(json!({ "hooks": { "PreToolUse": [] } })),
        key_deny_read: Vec::new(),
        permission: PermissionLaunch::default(),
        masker: Masker::new(Vec::new()),
    }
}

fn spec(dir: &Path, resume: Option<&str>) -> SessionSpec {
    SessionSpec {
        agent: AgentId(3),
        workdir: dir.to_path_buf(),
        model: Some("sonnet".to_owned()),
        settings: SettingsRevision(1),
        resume: resume.map(|id| ProviderSessionId(id.to_owned())),
        packet: None,
        add_dirs: Vec::new(),
        interrupted_children: Vec::new(),
    }
}

async fn take(client: &mut ClaudeClient, count: usize) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    for _ in 0..count {
        let event = tokio::time::timeout(Duration::from_secs(5), client.next_event())
            .await
            .expect("event should arrive")
            .expect("stream should be open");
        events.push(event);
    }
    events
}

#[test]
fn cache_window_without_api_key_is_one_hour() {
    let init = json!({ "type": "system", "subtype": "init", "apiKeySource": "none" });

    assert_eq!(cache_ttl(&init), Some(Duration::from_secs(3600)));
}

#[test]
fn cache_window_with_api_key_is_five_minutes_and_missing_source_is_unknown() {
    let keyed = json!({ "apiKeySource": "user" });
    let missing = json!({ "type": "system", "subtype": "init" });

    assert_eq!(cache_ttl(&keyed), Some(Duration::from_secs(300)));
    assert_eq!(cache_ttl(&missing), None);
}

#[test]
fn detail_of_edit_counts_changed_lines_and_keeps_path() {
    let input = json!({
        "file_path": "/w/app.cfg",
        "old_string": "mode=draft\nretries=1",
        "new_string": "mode=harbor\nretries=tundra"
    });

    let detail = detail_of("Edit", &input);

    assert_eq!(detail.category, ToolCategory::FileEdit);
    assert_eq!(detail.paths, vec!["/w/app.cfg".to_owned()]);
    assert_eq!(
        detail.changed,
        Some(LineChange {
            added: 2,
            removed: 2
        })
    );
}

#[test]
fn detail_of_multi_edit_sums_every_edit() {
    let input = json!({
        "file_path": "/w/a.rs",
        "edits": [
            { "old_string": "a", "new_string": "b\nc" },
            { "old_string": "x\ny", "new_string": "z" }
        ]
    });

    let changed = detail_of("MultiEdit", &input).changed;

    assert_eq!(
        changed,
        Some(LineChange {
            added: 3,
            removed: 3
        })
    );
}

#[test]
fn detail_of_write_has_no_line_change() {
    let input = json!({ "file_path": "/w/a.rs", "content": "fn main() {}" });

    let detail = detail_of("Write", &input);

    assert_eq!(detail.category, ToolCategory::FileEdit);
    assert_eq!(detail.changed, None);
}

#[test]
fn detail_of_read_range_needs_offset_and_limit() {
    let ranged = json!({ "file_path": "/w/a.rs", "offset": 10, "limit": 5 });
    let partial = json!({ "file_path": "/w/a.rs", "limit": 5 });

    assert_eq!(
        detail_of("Read", &ranged).read_lines,
        Some(LineRange {
            first: 10,
            last: 14
        })
    );
    assert_eq!(detail_of("Read", &partial).read_lines, None);
}

#[test]
fn detail_of_test_command_is_test_run() {
    let input = json!({ "command": "python3 -m unittest tests.test_rules" });

    assert_eq!(detail_of("Bash", &input).category, ToolCategory::TestRun);
}

#[test]
fn shell_exit_code_reads_prefix_only_for_errors() {
    assert_eq!(shell_exit_code(false, "ok"), Some(0));
    assert_eq!(shell_exit_code(true, "Exit code 3\nboom"), Some(3));
    assert_eq!(shell_exit_code(true, "interrupted"), None);
}

#[tokio::test]
async fn stdout_hides_router_key_before_emitting_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = launch(dir.path(), Vec::new());
    config.masker = Masker::new(vec!["sk-secret-1234".to_owned()]);
    let mut client = ClaudeClient::new(config, Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;

    client.send_turn(&session, "secret").await.unwrap();
    let events = take(&mut client, 4).await;

    assert!(matches!(&events[0], ProviderEvent::Text { text, .. } if text == "[redacted]"));
    assert!(!format!("{events:?}").contains("sk-secret-1234"));
    client.close_session(&session).await.unwrap();
}

fn expected_tree_events(agent: AgentId) -> Vec<ProviderEvent> {
    let task = SubagentId("toolu_task".to_owned());
    vec![
        ProviderEvent::Text {
            agent,
            subagent: None,
            text: "hi".to_owned(),
        },
        ProviderEvent::ToolCall {
            agent,
            subagent: None,
            call_id: "toolu_bash".to_owned(),
            activity: Activity::RunningCommand {
                command: "ls -la".to_owned(),
            },
            detail: ToolDetail {
                category: ToolCategory::Shell,
                ..ToolDetail::default()
            },
        },
        ProviderEvent::ToolResult {
            agent,
            subagent: None,
            call_id: "toolu_bash".to_owned(),
            output: "total 0".to_owned(),
            exit_code: Some(0),
        },
        ProviderEvent::SubagentStarted {
            agent,
            subagent: task.clone(),
            parent: None,
        },
        ProviderEvent::ToolCall {
            agent,
            subagent: Some(task.clone()),
            call_id: "toolu_read".to_owned(),
            activity: Activity::ReadingFile,
            detail: ToolDetail {
                category: ToolCategory::FileRead,
                paths: vec!["/w/a.rs".to_owned()],
                ..ToolDetail::default()
            },
        },
        ProviderEvent::ToolResult {
            agent,
            subagent: Some(task.clone()),
            call_id: "toolu_read".to_owned(),
            output: "fn main".to_owned(),
            exit_code: None,
        },
        ProviderEvent::SubagentEnded {
            agent,
            subagent: task,
        },
        ProviderEvent::Text {
            agent,
            subagent: None,
            text: "done".to_owned(),
        },
        ProviderEvent::Usage(UsageReport {
            agent,
            subagent: None,
            model: Some("claude-test".to_owned()),
            scope: UsageScope::MainTurn,
            input: Some(30),
            cache_read: Some(2000),
            cache_write: Some(200),
            output: Some(40),
            reasoning: None,
        }),
        ProviderEvent::ContextSize {
            agent,
            tokens: Some(3120),
        },
        ProviderEvent::TurnCompleted {
            agent,
            origin: TurnOrigin::User,
        },
    ]
}

#[tokio::test]
async fn stream_input_converts_tree_and_usage() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let handle = client.open_session(spec(dir.path(), None)).await.unwrap();
    let session = handle.provider_session.clone();
    assert_eq!(session.0.len(), 36);
    assert!(!handle.steer_verified);

    client.send_turn(&session, "hello").await.unwrap();
    let events = take(&mut client, 11).await;

    assert_eq!(events, expected_tree_events(AgentId(3)));
    assert_eq!(
        client.applied_settings(&session),
        Some(AppliedSettings {
            model: Some("claude-test".to_owned()),
            permission: Some("acceptEdits".to_owned()),
        })
    );
    let commands: Vec<(String, bool)> = client
        .commands()
        .into_iter()
        .map(|command| (command.name, command.is_skill))
        .collect();
    assert_eq!(
        commands,
        vec![
            ("compact".to_owned(), false),
            ("review".to_owned(), false),
            ("lint".to_owned(), true),
        ]
    );
    client.close_session(&session).await.unwrap();
    assert!(client.process_group(&session).is_none());
}

#[tokio::test]
async fn steer_needs_active_turn_and_interrupt_waits_for_response() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    let agent = AgentId(3);

    assert!(matches!(
        client.steer(&session, "early").await,
        Err(ProviderError::NoActiveTurn)
    ));
    client.send_turn(&session, "wait").await.unwrap();
    assert_eq!(
        take(&mut client, 1).await,
        vec![ProviderEvent::Text {
            agent,
            subagent: None,
            text: "working".to_owned()
        }]
    );
    assert!(matches!(
        client.compact(&session).await,
        Err(ProviderError::NotSent { .. })
    ));
    client.steer(&session, "more").await.unwrap();
    let steered = take(&mut client, 4).await;
    assert_eq!(
        steered[3],
        ProviderEvent::TurnCompleted {
            agent,
            origin: TurnOrigin::User
        }
    );

    client.send_turn(&session, "wait").await.unwrap();
    take(&mut client, 1).await;
    client
        .interrupt(
            &session,
            InterruptTarget::Subagent(SubagentId("x".to_owned())),
        )
        .await
        .unwrap();
    client
        .interrupt(&session, InterruptTarget::Main)
        .await
        .unwrap();
    let stopped = take(&mut client, 3).await;
    assert_eq!(
        stopped[2],
        ProviderEvent::TurnCompleted {
            agent,
            origin: TurnOrigin::User
        }
    );

    client.compact(&session).await.unwrap();
    let compacted = take(&mut client, 4).await;
    assert_eq!(
        compacted[0],
        ProviderEvent::SettingsApplied {
            agent,
            values: vec![
                ("model".to_owned(), "claude-other".to_owned()),
                ("permission_mode".to_owned(), "acceptEdits".to_owned()),
            ],
        }
    );
    client.close_session(&session).await.unwrap();
}

/// 패킷 턴 중에 온 입력은 줄 섰다가 자기 완료를 받는다(#318). 엔진처럼 `next_arrival`로 읽을 때 그 입력을 쓰는 중에
/// 막혀(파이프가 가득 참) 첫 poll이 `Pending`이어도 완료 이벤트와 그 입력을 잃지 않는다(#324).
#[tokio::test]
async fn a_turn_sent_during_the_packet_turn_gets_its_own_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let mut packet_spec = spec(dir.path(), None);
    packet_spec.packet = Some("merging-packet".to_owned());
    let session = client
        .open_session(packet_spec)
        .await
        .unwrap()
        .provider_session;
    let big = "x".repeat(BIG_TURN_BYTES);
    client.send_turn(&session, &big).await.unwrap();
    let mut providers = HashMap::from([(
        (ChatId(1), crate::providers::test_support::CLAUDE),
        ProviderConnection::new(super::adapter::ID, client),
    )]);

    let mut events = Vec::new();
    while events
        .iter()
        .filter(|event| matches!(event, ProviderEvent::TurnCompleted { .. }))
        .count()
        < 2
    {
        assert!(events.len() < 12, "{events:?}");
        let arrival = tokio::time::timeout(Duration::from_secs(5), next_arrival(&mut providers))
            .await
            .expect("event should arrive");
        if let Some(ProviderEvent::TurnCompleted { agent, .. }) = &arrival.event {
            start_queued_turn(&mut providers, arrival.chat, arrival.provider, *agent).await;
        }
        events.extend(arrival.event);
    }

    assert!(events.iter().any(|event| matches!(
        event,
        ProviderEvent::Text { text, .. } if *text == format!("big:{BIG_TURN_BYTES}")
    )));
    assert!(!format!("{events:?}").contains("merged:"));
}

#[tokio::test]
async fn interrupt_drops_the_turns_waiting_behind_the_running_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;

    client.send_turn(&session, "wait").await.unwrap();
    take(&mut client, 1).await;
    client.send_turn(&session, "more").await.unwrap();
    client
        .interrupt(&session, InterruptTarget::Main)
        .await
        .unwrap();
    let events = take(&mut client, 3).await;

    assert!(matches!(events[2], ProviderEvent::TurnCompleted { .. }));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), client.next_event())
            .await
            .is_err(),
        "the waiting turn must not be sent after the interrupt"
    );
    client.close_session(&session).await.unwrap();
}

#[tokio::test]
async fn permission_request_and_stream_loss() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    let agent = AgentId(3);

    client.send_turn(&session, "ask").await.unwrap();
    assert_eq!(
        take(&mut client, 1).await,
        vec![ProviderEvent::PermissionRequested {
            agent,
            request_id: "perm-1".to_owned(),
            summary: "Bash: rm -rf build".to_owned(),
            reason: "outside workdir".to_owned(),
            call: Some(PermissionCall {
                tool: PermissionTool::Shell,
                target: "rm -rf build".to_owned(),
                paths: Vec::new(),
            }),
        }]
    );
    client.steer(&session, "crash").await.unwrap();
    assert_eq!(
        take(&mut client, 1).await,
        vec![ProviderEvent::StreamLost { agent }]
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(matches!(
        client.send_turn(&session, "again").await,
        Err(ProviderError::NotSent { .. })
    ));
    client.close_session(&session).await.unwrap();
}

/// `ask`로 허가 요청을 받고, 답이 없는 동안 턴이 멈춰 있는지 확인한 뒤 답해서 가짜 provider가 받은 응답을 돌려준다.
async fn answer_ask(answer: PermissionAnswer) -> Value {
    answer_ask_by(|_| answer).await
}

/// 올라온 요청을 보고 `decide`가 정한 답을 보낸다.
async fn answer_ask_by(decide: impl FnOnce(&ProviderEvent) -> PermissionAnswer) -> Value {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    client.send_turn(&session, "ask").await.unwrap();
    let requested = take(&mut client, 1).await;
    assert!(matches!(
        &requested[0],
        ProviderEvent::PermissionRequested { request_id, .. } if request_id == "perm-1"
    ));
    let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
    assert!(stalled.is_err(), "turn should wait for the answer");

    client
        .answer_permission(&session, "perm-1", decide(&requested[0]))
        .await
        .unwrap();
    let resumed = take(&mut client, 4).await;

    let ProviderEvent::Text { text, .. } = &resumed[0] else {
        panic!(
            "expected the provider to continue with text, got {:?}",
            resumed[0]
        );
    };
    assert!(matches!(resumed[3], ProviderEvent::TurnCompleted { .. }));
    client.close_session(&session).await.unwrap();
    let received = text.strip_prefix("answer:").expect("answer prefix");
    serde_json::from_str(received).unwrap()
}

#[tokio::test]
async fn allow_once_answers_can_use_tool_with_the_request_input() {
    let received = answer_ask(PermissionAnswer::AllowOnce).await;

    assert_eq!(
        received,
        json!({
            "subtype": "success",
            "request_id": "perm-1",
            "response": {
                "behavior": "allow",
                "updatedInput": { "command": "rm -rf build" },
            },
        })
    );
}

/// 요청의 호출을 Saturn 규칙으로 판정해 `allow`와 `deny`로 답한다.
#[tokio::test]
async fn permission_rules() {
    let policy = |verdict| Policy {
        mode: Mode::Edit,
        workdir: PathBuf::from("/work"),
        extra_dirs: Vec::new(),
        rules: vec![Rule {
            tool: PermissionTool::Shell,
            pattern: "rm *".to_owned(),
            verdict,
        }],
        always: Vec::new(),
    };
    let answer_by = |policy: Policy| {
        answer_ask_by(move |event| {
            let ProviderEvent::PermissionRequested {
                call: Some(call), ..
            } = event
            else {
                panic!("expected a readable permission request, got {event:?}");
            };
            match policy.decide(call) {
                Verdict::Allow => PermissionAnswer::AllowOnce,
                Verdict::Deny => PermissionAnswer::Deny { note: None },
                Verdict::Ask => panic!("rule should decide this call"),
            }
        })
    };

    let allowed = answer_by(policy(Verdict::Allow)).await;
    let denied = answer_by(policy(Verdict::Deny)).await;

    assert_eq!(allowed["response"]["behavior"], "allow");
    assert_eq!(
        allowed["response"]["updatedInput"],
        json!({ "command": "rm -rf build" })
    );
    assert_eq!(
        denied["response"],
        json!({ "behavior": "deny", "message": DENY_MESSAGE })
    );
}

#[tokio::test]
async fn allow_always_is_sent_as_allow_until_a_session_rule_value_is_measured() {
    let always = answer_ask(PermissionAnswer::AllowAlways).await;
    let once = answer_ask(PermissionAnswer::AllowOnce).await;

    assert_eq!(always, once);
}

#[tokio::test]
async fn deny_answers_can_use_tool_without_the_note() {
    let received = answer_ask(PermissionAnswer::Deny {
        note: Some("do it differently".to_owned()),
    })
    .await;

    assert_eq!(
        received["response"],
        json!({ "behavior": "deny", "message": DENY_MESSAGE })
    );
    assert_eq!(received["request_id"], "perm-1");
}

/// `ask-user`로 질문 요청을 받고, 답이 없는 동안 턴이 멈춰 있는지 확인한 뒤 답해서 가짜 provider가 받은 응답을
/// 돌려준다.
async fn answer_ask_user(answer: InputAnswer) -> (ProviderEvent, Value) {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    client.send_turn(&session, "ask-user").await.unwrap();
    let requested = take(&mut client, 1).await.remove(0);
    let ProviderEvent::InputRequested { request_id, .. } = &requested else {
        panic!("expected an input request, got {requested:?}");
    };
    let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
    assert!(stalled.is_err(), "turn should wait for the answer");

    client
        .answer_input(&session, request_id, answer)
        .await
        .unwrap();
    let resumed = take(&mut client, 4).await;

    let ProviderEvent::Text { text, .. } = &resumed[0] else {
        panic!(
            "expected the provider to continue with text, got {:?}",
            resumed[0]
        );
    };
    client.close_session(&session).await.unwrap();
    let received = text.strip_prefix("answer:").expect("answer prefix");
    (requested, serde_json::from_str(received).unwrap())
}

#[tokio::test]
async fn elicitation_ask_user_question_round_trip_returns_answers_in_updated_input() {
    let question = "What is the name of your project?";
    let submit = InputAnswer::Submit {
        values: vec![(
            question.to_owned(),
            InputValue::Selected(vec!["Saturn WT".to_owned()]),
        )],
    };

    let (requested, received) = answer_ask_user(submit).await;

    let ProviderEvent::InputRequested {
        request_id,
        request,
        ..
    } = requested
    else {
        unreachable!("answer_ask_user returns an input request");
    };
    assert_eq!(request_id, "ask-1");
    assert_eq!(request.fields[0].id, question);
    assert_eq!(received["request_id"], "ask-1");
    assert_eq!(received["response"]["behavior"], "allow");
    assert_eq!(
        received["response"]["updatedInput"]["answers"],
        json!({ question: "Saturn WT" })
    );
    assert_eq!(
        received["response"]["updatedInput"]["questions"][0]["header"],
        "Project name"
    );
}

#[tokio::test]
async fn elicitation_ask_user_question_decline_and_cancel_deny_the_call() {
    for answer in [InputAnswer::Decline, InputAnswer::Cancel] {
        let (_, received) = answer_ask_user(answer).await;

        assert_eq!(received["response"]["behavior"], "deny");
        assert_eq!(received["request_id"], "ask-1");
    }
}

#[tokio::test]
async fn elicitation_ask_user_question_is_not_answered_as_a_permission() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    client.send_turn(&session, "ask-user").await.unwrap();
    take(&mut client, 1).await;

    let as_permission = client
        .answer_permission(&session, "ask-1", PermissionAnswer::AllowOnce)
        .await;
    let unknown = client
        .answer_input(&session, "other", InputAnswer::Cancel)
        .await;
    let again = {
        client
            .answer_input(&session, "ask-1", InputAnswer::Cancel)
            .await
            .unwrap();
        client
            .answer_input(&session, "ask-1", InputAnswer::Cancel)
            .await
    };

    assert!(matches!(as_permission, Err(ProviderError::NotSent { .. })));
    assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
    assert!(matches!(again, Err(ProviderError::NotSent { .. })));
    client.close_session(&session).await.unwrap();
}

#[tokio::test]
async fn answering_an_unknown_or_answered_request_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    client.send_turn(&session, "ask").await.unwrap();
    take(&mut client, 1).await;

    let unknown = client
        .answer_permission(&session, "other", PermissionAnswer::AllowOnce)
        .await;
    client
        .answer_permission(&session, "perm-1", PermissionAnswer::AllowOnce)
        .await
        .unwrap();
    let again = client
        .answer_permission(&session, "perm-1", PermissionAnswer::AllowOnce)
        .await;

    assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
    assert!(matches!(again, Err(ProviderError::NotSent { .. })));
    client.close_session(&session).await.unwrap();
}

#[tokio::test]
async fn subagent_without_result_is_stream_loss() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;

    client.send_turn(&session, "orphan").await.unwrap();
    let events = take(&mut client, 5).await;

    assert_eq!(events[4], ProviderEvent::StreamLost { agent: AgentId(3) });
    client.close_session(&session).await.unwrap();
}

#[tokio::test]
async fn error_result_is_not_a_completed_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    let agent = AgentId(3);

    // `is_error`가 참인 `success` 결과(맥락 초과)
    client.send_turn(&session, "too-long").await.unwrap();
    let too_long = take(&mut client, 4).await;
    assert!(matches!(too_long[1], ProviderEvent::Usage(_)));
    assert_eq!(too_long[3], ProviderEvent::StreamLost { agent });

    // 오류 종류(`error_max_turns`)의 결과. `is_error`가 거짓이어도 성공이 아니다
    client.send_turn(&session, "max-turns").await.unwrap();
    let max_turns = take(&mut client, 3).await;
    assert_eq!(max_turns[2], ProviderEvent::StreamLost { agent });
    assert!(
        tokio::time::timeout(Duration::from_millis(300), client.next_event())
            .await
            .is_err(),
        "a failed turn must not also report completion or a second loss"
    );
    client.close_session(&session).await.unwrap();
}

#[tokio::test]
async fn reopening_a_session_still_linked_closes_the_old_process() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new())
        .with_resume_settle(Duration::from_millis(100));
    let first = client
        .open_session(spec(dir.path(), None))
        .await
        .unwrap()
        .provider_session;
    let old_group = client.process_group(&first).unwrap();

    let again = client
        .open_session(spec(dir.path(), Some(&first.0)))
        .await
        .unwrap()
        .provider_session;

    assert_eq!(again, first);
    assert_ne!(client.process_group(&again), Some(old_group));
    client.close_session(&again).await.unwrap();
}

#[tokio::test]
async fn resume_failure_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    // 실패 쪽은 종료를 기다리는 시간만 넉넉히, 성공 쪽은 살아 있는지만 보므로 짧게 둔다
    let mut failing = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new())
        .with_resume_settle(Duration::from_secs(30));
    let mut client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new())
        .with_resume_settle(Duration::from_millis(100));

    let error = failing
        .open_session(spec(dir.path(), Some("missing")))
        .await
        .unwrap_err();
    let resumed = client
        .open_session(spec(dir.path(), Some("kept-session")))
        .await
        .unwrap();

    assert!(matches!(error, ProviderError::NotSent { .. }));
    assert_eq!(resumed.provider_session.0, "kept-session");
    client
        .close_session(&resumed.provider_session)
        .await
        .unwrap();
}

#[test]
fn launch_args_add_defaults_and_hook_settings() {
    let dir = tempfile::tempdir().unwrap();
    let client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());

    let args = client.launch_args(&spec(dir.path(), None), &SessionArg::New("id-1".to_owned()));

    assert_eq!(
        args,
        vec![
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--session-id",
            "id-1",
            "--model",
            "sonnet",
            "--permission-prompt-tool",
            "stdio",
            "--autocompact",
            "100000",
            "--settings",
            "{\"hooks\":{\"PreToolUse\":[]},\"permissions\":{\"ask\":[\"Bash\",\"Edit\",\"MultiEdit\",\"Write\",\"NotebookEdit\",\"Task\",\"Agent\",\"mcp__*\"]},\"sandbox\":{\"allowUnsandboxedCommands\":false,\"enabled\":true,\"failIfUnavailable\":true,\"filesystem\":{\"denyRead\":[]}}}",
        ]
    );
}

fn key_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Users/u/.saturn/router.key"),
        PathBuf::from("/Users/u/Library/Keychains"),
        PathBuf::from("/Library/Keychains"),
    ]
}

fn settings_arg(args: &[String]) -> Value {
    let at = args.iter().position(|arg| arg == "--settings").unwrap();
    serde_json::from_str(&args[at + 1]).unwrap()
}

#[test]
fn launch_args_enable_the_bash_sandbox_and_deny_reading_key_stores() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), Vec::new());
    launch.key_deny_read = key_paths();
    let client = ClaudeClient::new(launch, Supervisor::new());

    let args = client.launch_args(&spec(dir.path(), None), &SessionArg::New("id-1".to_owned()));

    let sandbox = &settings_arg(&args)["sandbox"];
    assert_eq!(sandbox["enabled"], json!(true));
    assert_eq!(sandbox["allowUnsandboxedCommands"], json!(false));
    assert_eq!(sandbox["failIfUnavailable"], json!(true));
    assert_eq!(
        sandbox["filesystem"]["denyRead"],
        json!([
            "/Users/u/.saturn/router.key",
            "/Users/u/Library/Keychains",
            "/Library/Keychains"
        ])
    );
}

#[test]
fn the_key_sandbox_stays_on_in_full_mode_and_keeps_the_other_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), Vec::new());
    launch.key_deny_read = key_paths();
    // 모드 `full`은 에이전트 질문을 끈다. 그 launch에도 샌드박스가 그대로 들어가야 한다
    launch.permission.questions_disabled = true;
    let client = ClaudeClient::new(launch, Supervisor::new());

    let args = client.launch_args(&spec(dir.path(), None), &SessionArg::New("id-1".to_owned()));

    assert!(args.iter().any(|arg| arg == "--disallowedTools"));
    let settings = settings_arg(&args);
    assert_eq!(settings["sandbox"]["enabled"], json!(true));
    assert_eq!(
        settings["sandbox"]["allowUnsandboxedCommands"],
        json!(false)
    );
    assert_eq!(settings["sandbox"]["failIfUnavailable"], json!(true));
    assert_eq!(
        settings["sandbox"]["filesystem"]["denyRead"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(settings["hooks"], json!({ "PreToolUse": [] }));
    assert_eq!(settings["permissions"]["ask"], json!(ASK_TOOLS));
}

#[test]
fn agent_questions_are_asked_by_default_and_disallowed_in_full_mode() {
    let dir = tempfile::tempdir().unwrap();
    let asking = launch(dir.path(), Vec::new());
    let mut silent = launch(dir.path(), Vec::new());
    silent.permission.questions_disabled = true;
    let found = UserProviderConfig::default();

    let asking_args = default_args(found, &asking);
    let silent_args = default_args(found, &silent);

    assert!(!asking_args.iter().any(|arg| arg == "--disallowedTools"));
    let at = silent_args
        .iter()
        .position(|arg| arg == "--disallowedTools")
        .expect("full mode should pass --disallowedTools");
    assert_eq!(silent_args[at + 1], "AskUserQuestion");
}

#[test]
fn extension_arguments_come_after_the_defaults_and_before_the_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut with_extensions = launch(dir.path(), Vec::new());
    with_extensions.permission.extra_args = vec!["--plugin-dir".to_owned(), "/x".to_owned()];
    let client = ClaudeClient::new(with_extensions, Supervisor::new());
    let args = client.launch_args(
        &spec(dir.path(), None),
        &SessionArg::New("11111111-1111-4111-8111-111111111111".to_owned()),
    );

    let at = args
        .iter()
        .position(|arg| arg == "--plugin-dir")
        .expect("the extension arguments should reach claude");
    assert_eq!(args[at + 1], "/x");
    assert!(args.iter().position(|arg| arg == "--settings").unwrap() > at);
}

#[test]
fn default_model_is_not_passed_to_claude() {
    let dir = tempfile::tempdir().unwrap();
    let client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let mut default_model = spec(dir.path(), None);
    default_model.model = Some(DEFAULT_MODEL.to_owned());

    let args = client.launch_args(&default_model, &SessionArg::New("id-1".to_owned()));

    assert!(!args.iter().any(|arg| arg == "--model"));
}

#[test]
fn add_dir_follows_the_model_as_one_variadic_flag_before_the_permission_args() {
    let dir = tempfile::tempdir().unwrap();
    let client = ClaudeClient::new(launch(dir.path(), Vec::new()), Supervisor::new());
    let mut with_dirs = spec(dir.path(), None);
    with_dirs.add_dirs = vec![PathBuf::from("/shared/lib"), PathBuf::from("/docs")];

    let args = client.launch_args(&with_dirs, &SessionArg::New("id-1".to_owned()));
    let without = client.launch_args(&spec(dir.path(), None), &SessionArg::New("id-1".to_owned()));

    let at = args.iter().position(|arg| arg == "--add-dir").unwrap();
    assert_eq!(
        args[at..at + 4],
        [
            "--add-dir",
            "/shared/lib",
            "/docs",
            "--permission-prompt-tool"
        ]
    );
    assert!(!without.contains(&"--add-dir".to_owned()));
}

#[test]
fn ask_settings_merge_into_hook_settings_and_stand_alone() {
    let merged = with_ask_tools(json!({ "hooks": { "PreToolUse": [] } }));
    let alone = with_ask_tools(Value::Null);

    assert_eq!(merged["hooks"], json!({ "PreToolUse": [] }));
    assert_eq!(merged["permissions"]["ask"], json!(ASK_TOOLS));
    assert_eq!(alone, json!({ "permissions": { "ask": ASK_TOOLS } }));
}

#[test]
fn permission_call_reads_rule_tools_and_leaves_the_rest_to_the_user() {
    let call = |tool: &str, input: Value| permission_call(tool, &input);

    assert_eq!(
        call("Bash", json!({ "command": "cargo test" })),
        Some(PermissionCall {
            tool: PermissionTool::Shell,
            target: "cargo test".to_owned(),
            paths: Vec::new(),
        })
    );
    assert_eq!(
        call("Edit", json!({ "file_path": "/work/a.rs" }))
            .unwrap()
            .paths,
        vec!["/work/a.rs"]
    );
    assert_eq!(
        call("NotebookEdit", json!({ "notebook_path": "/work/a.ipynb" }))
            .unwrap()
            .paths,
        vec!["/work/a.ipynb"]
    );
    assert_eq!(
        call("Task", json!({ "subagent_type": "explorer" })).unwrap(),
        PermissionCall {
            tool: PermissionTool::Subagent,
            target: "explorer".to_owned(),
            paths: Vec::new(),
        }
    );
    assert_eq!(
        call("mcp__docs__read", json!({})).unwrap().target,
        "mcp__docs__read"
    );
    assert_eq!(call("WebFetch", json!({ "url": "https://x" })), None);
}

#[test]
fn permission_call_reads_file_reading_tools_as_read_calls() {
    let call = |tool: &str, input: Value| permission_call(tool, &input);
    let read = |path: &str| PermissionCall {
        tool: PermissionTool::Read,
        target: String::new(),
        paths: vec![path.to_owned()],
    };

    assert_eq!(
        call("Read", json!({ "file_path": "/etc/hosts" })),
        Some(read("/etc/hosts"))
    );
    assert_eq!(
        call("Grep", json!({ "pattern": "x", "path": "/etc" })),
        Some(read("/etc"))
    );
    assert_eq!(call("LS", json!({ "path": "/etc" })), Some(read("/etc")));
    assert_eq!(
        call("Glob", json!({ "pattern": "*.rs" })).unwrap().paths,
        Vec::<String>::new()
    );
}

#[test]
fn user_settings_suppress_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(
        home.join(".claude").join("settings.json"),
        r#"{"permissions":{"defaultMode":"plan"}}"#,
    )
    .unwrap();
    let env = vec![
        ("HOME".into(), home.into_os_string()),
        ("DISABLE_COMPACT".into(), "1".into()),
    ];
    let launch = launch(dir.path(), env);

    let found = read_user_config(&launch);

    assert_eq!(
        found,
        UserProviderConfig {
            has_auto_compact: true,
        }
    );
    assert_eq!(
        default_args(found, &launch),
        vec!["--permission-prompt-tool", "stdio"]
    );
    let mut big = launch.clone();
    big.defaults.auto_compact_tokens = Some(5_000_000);
    assert_eq!(
        default_args(
            UserProviderConfig {
                has_auto_compact: false
            },
            &big
        ),
        vec![
            "--permission-prompt-tool",
            "stdio",
            "--autocompact",
            "1000000"
        ]
    );
}

#[test]
fn session_uuid_is_version_four() {
    let id = new_session_uuid().unwrap();

    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4");
    assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
}

// #66: 끊긴 턴을 자동으로 이어 가는 환경 변수는 실행 환경에서 빼고, 다른 변수는 그대로 둔다
#[tokio::test]
async fn resume_interrupted_turn_variable_is_not_passed_to_claude() {
    let dir = tempfile::tempdir().unwrap();
    let env = vec![
        (
            OsString::from("CLAUDE_CODE_RESUME_INTERRUPTED_TURN"),
            OsString::from("1"),
        ),
        (OsString::from("KEEP_ME"), OsString::from("yes")),
    ];
    let mut client = ClaudeClient::new(launch(dir.path(), env), Supervisor::new());
    let session = client
        .open_session(spec(dir.path(), Some("resumed-session")))
        .await
        .unwrap()
        .provider_session;

    client.send_turn(&session, "env-check").await.unwrap();
    let events = take(&mut client, 1).await;

    assert!(matches!(&events[0], ProviderEvent::Text { text, .. } if text == "env:unset:yes"));
    client.close_session(&session).await.unwrap();
}
