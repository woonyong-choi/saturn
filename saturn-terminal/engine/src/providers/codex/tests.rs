use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use saturn_core::permission::{Mode, Policy, Rule, Verdict};
use saturn_protocol::event::{
    Activity, LineChange, PermissionCall, PermissionTool, ToolCategory, ToolDetail, UsageReport,
    UsageScope,
};
use saturn_protocol::ids::{ChatId, SettingsRevision, SubagentId};
use saturn_protocol::input::InputValue;

use super::config::{default_args, read_user_config, scan_user_config};
use super::convert::{
    activity_of, convert_notification, detail_of, exit_code_of, model_info, tool_output,
};
use super::home::{HomeInput, prepare as prepare_codex_home};
use super::threads::remove_thread_tree;
use super::*;
use crate::Masker;
use crate::events::{next_arrival, start_queued_turn};
use crate::providers::mask_values;
use crate::providers::{
    LaunchSpec, PermissionLaunch, ProviderConnection, SaturnDefaults, UserProviderConfig,
};

/// 받은 요청에 schema 모양 그대로 응답한다.
const FAKE_APP_SERVER: &str = r#"#!/usr/bin/perl
use strict; use warnings; use JSON::PP; use IO::Handle;
$| = 1;
my $json = JSON::PP->new->canonical;
sub out { print $json->encode($_[0]), "\n"; }
sub note { out({ method => $_[0], params => $_[1] }); }
my $active = "";
my $mcp_polls = 0;
sub rules_text {
  my $home = $ENV{CODEX_HOME} // "";
  open(my $file, "<", "$home/rules/default.rules") or return "";
  local $/;
  my $text = <$file>;
  return $text // "";
}
my %gates = (
  "gate-file-known" => ["srv-file2", "item/fileChange/requestApproval", { itemId => "item_f", reason => "write file" }],
  "run-sort" => [21, "item/commandExecution/requestApproval", { itemId => "item_s", command => "sort a.txt", reason => "needs approval" }],
  "gate-command" => [7, "item/commandExecution/requestApproval", { itemId => "item_g", command => "touch a.txt", reason => "needs write", availableDecisions => ["accept", "acceptForSession", "decline"] }],
  "gate-command-once-only" => [8, "item/commandExecution/requestApproval", { itemId => "item_g", command => "ls", availableDecisions => ["accept", "cancel"] }],
  "gate-file" => ["srv-file", "item/fileChange/requestApproval", { itemId => "item_g", reason => "write file" }],
  "gate-mcp" => [0, "mcpServer/elicitation/request", { serverName => "probe", mode => "form", message => "Allow the probe MCP server to run tool \"echo\"?", _meta => { codex_approval_kind => "mcp_tool_call" }, requestedSchema => { type => "object", properties => {} } }],
  "gate-permissions" => [9, "item/permissions/requestApproval", { itemId => "item_g", reason => "needs network", permissions => { network => { enabled => JSON::PP::true } } }],
  "gate-legacy" => ["legacy-1", "execCommandApproval", { conversationId => "thr_main", command => ["ls"], reason => "legacy" }],
  "gate-elicit-form" => [41, "mcpServer/elicitation/request", { serverName => "probe", mode => "form", message => "실험 입력을 작성하라", requestedSchema => { type => "object", properties => { choice => { type => "string", title => "단일 선택", enum => ["a", "b"] }, count => { type => "integer", title => "횟수" }, enabled => { type => "boolean", title => "사용" }, tags => { type => "array", title => "다중 선택", items => { type => "string", enum => ["x", "y"] } }, title => { type => "string", title => "제목" } }, required => ["title", "count", "enabled", "choice", "tags"] } }],
  "gate-elicit-url" => [42, "mcpServer/elicitation/request", { serverName => "probe", mode => "url", message => "URL을 열어라", url => "https://example.invalid/experiment-252", elicitationId => "experiment-252-url" }],
  "gate-user-input" => [43, "item/tool/requestUserInput", { itemId => "call_1", isBlocking => JSON::PP::false, autoResolutionMs => undef, questions => [ { id => "preference", header => "선호 확인", question => "무엇을 먼저 할까요?", isOther => JSON::PP::true, isSecret => JSON::PP::false, options => [ { label => "코드 변경", description => "구현을 고칩니다" }, { label => "설계 검토", description => "문서를 검토합니다" } ] } ] }],
);
while (my $line = <STDIN>) {
  my $m = eval { $json->decode($line) } or next;
  my $method = $m->{method} // "";
  my $id = $m->{id};
  next unless defined $id;
  if ($method eq "" && exists $m->{result}) {
    note("item/agentMessage/delta", { threadId => "thr_main", turnId => "turn_g", itemId => "mg", delta => "answer:" . $json->encode({ id => $m->{id}, result => $m->{result} }) });
    note("turn/completed", { threadId => "thr_main", turn => { id => "turn_g", status => "completed", items => [] } });
    next;
  }
  my $p = $m->{params} // {};
  my $tid = $p->{threadId} // "thr_main";
  if (($ENV{FAKE_CALL_LOG} // "") ne "") {
    open(my $log, ">>", $ENV{FAKE_CALL_LOG}) or die;
    print $log "$method $tid\n";
    close $log;
  }
  if ($method eq "initialize") {
    out({ id => $id, result => { userAgent => ($ENV{FAKE_USER_AGENT} // "fake/0.158.0"), platformFamily => "unix", platformOs => "macos", codexHome => ($ENV{CODEX_HOME} // "/fake") } });
  } elsif ($method eq "skills/list") {
    out({ id => $id, result => { data => [ { cwd => "/w", errors => [], skills => [
      { name => "lint", description => "Run the linter", shortDescription => "lint it", enabled => JSON::PP::true, path => "/skills/lint/SKILL.md", scope => "user" },
      { name => "off", description => "disabled", enabled => JSON::PP::false, path => "/skills/off/SKILL.md", scope => "user" } ] } ] } });
  } elsif ($method eq "thread/start" || $method eq "thread/resume") {
    select(undef, undef, undef, $ENV{FAKE_SLOW_OPEN_MS} / 1000) if ($ENV{FAKE_SLOW_OPEN_MS} // "") ne "";
    my $model = $p->{model} // "";
    if (($ENV{FAKE_REQUIRE_ADD_DIR} // "") ne "") {
      my $roots = $p->{config}{sandbox_workspace_write}{writable_roots} // [];
      if (($roots->[0] // "") ne $ENV{FAKE_REQUIRE_ADD_DIR}) {
        out({ id => $id, error => { code => -32000, message => "added folders are missing" } });
        next;
      }
    }
    if (($ENV{FAKE_REQUIRE_MCP} // "") ne "" && $mcp_polls < 3) {
      out({ id => $id, error => { code => -32000, message => "mcp tools are not ready" } });
      next;
    }
    my $policy = $model eq "wrong-policy" ? "on-request" : ($p->{approvalPolicy} // "on-request");
    my $asked = $p->{sandbox} // "";
    my $sandbox = $model eq "wrong-sandbox" ? { type => "dangerFullAccess" } : ($asked eq "read-only" ? { type => "readOnly" } : { type => "workspaceWrite" });
    out({ id => $id, result => { thread => { id => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" }, model => "gpt-test", modelProvider => "openai", approvalPolicy => $policy, approvalsReviewer => "user", sandbox => $sandbox, cwd => "/w" } });
    # 표준 입력을 읽지 않는 provider. 파이프가 가득 차면 쓰는 쪽이 막힌다
    select(undef, undef, undef, $ENV{FAKE_STOP_READING_SECS}) if ($ENV{FAKE_STOP_READING_SECS} // "") ne "";
  } elsif ($method eq "mcpServerStatus/list") {
    $mcp_polls++;
    my $ready = $mcp_polls >= 3 && ($ENV{FAKE_MCP_STUCK} // "") eq "";
    my @servers = ({ name => "docs", runtimeStatus => undef,
      serverInfo => ($ready ? { name => "docs", title => undef, version => "1" } : undef),
      tools => ($ready ? { echo => { name => "echo", inputSchema => { type => "object" } } } : {}), toolsError => undef });
    push @servers, { name => $ENV{FAKE_MCP_FAILED}, runtimeStatus => undef, serverInfo => undef, tools => {},
      toolsError => "MCP startup failed: No such file or directory (os error 2)" } if ($ENV{FAKE_MCP_FAILED} // "") ne "";
    out({ id => $id, result => { data => \@servers, nextCursor => undef } });
  } elsif ($method eq "turn/start") {
    my $first = $p->{input}[0];
    next if ($first->{text} // "") eq "stall";
    if (($first->{text} // "") eq "crash") {
      out({ id => $id, result => { turn => { id => "turn_x", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_x", status => "inProgress", items => [] } });
      exit 1;
    }
    if (($first->{text} // "") eq "merging-packet") {
      # 진행 중인 턴에 `turn/start`가 오면 같은 턴에 합쳐 `turn/completed`를 하나만 보낸다(codex-cli 0.158.0 실측 동작)
      out({ id => $id, result => { turn => { id => "turn_m", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_m", status => "inProgress", items => [] } });
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_m", itemId => "m1", delta => "packet" });
      select(undef, undef, undef, 0.5);
      STDIN->blocking(0);
      my $next = <STDIN>;
      STDIN->blocking(1);
      if (defined $next) {
        my $merged = eval { $json->decode($next) } // {};
        out({ id => $merged->{id}, result => { turn => { id => "turn_m", status => "inProgress", items => [] } } });
        note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_m", itemId => "m2", delta => "merged:" . ($merged->{params}{input}[0]{text} // "") });
      }
      note("turn/completed", { threadId => $tid, turn => { id => "turn_m", status => "completed", items => [] } });
      next;
    }
    if (($first->{text} // "") eq "quick") {
      out({ id => $id, result => { turn => { id => "turn_q", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_q", status => "inProgress", items => [] } });
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_q", itemId => "m3", delta => "quick-answer" });
      note("turn/completed", { threadId => $tid, turn => { id => "turn_q", status => "completed", items => [] } });
      next;
    }
    my $gate = $gates{$first->{text} // ""};
    if ($gate && ($first->{text} eq "run-sort") && rules_text() !~ /pattern = \["sort"\], decision = "prompt"/) {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_g", itemId => "mg", delta => "ran-without-asking" });
      note("turn/completed", { threadId => $tid, turn => { id => "turn_g", status => "completed", items => [] } });
      next;
    }
    if (($first->{text} // "") eq "gate-child") {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      note("thread/started", { thread => { id => "thr_child", parentThreadId => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" } });
      out({ id => 31, method => "item/commandExecution/requestApproval", params => { threadId => "thr_child", turnId => "turn_c", itemId => "item_c", command => "rm -rf build", reason => "child needs write" } });
      next;
    }
    if ($gate) {
      out({ id => $id, result => { turn => { id => "turn_g", status => "inProgress", items => [] } } });
      note("turn/started", { threadId => $tid, turn => { id => "turn_g", status => "inProgress", items => [] } });
      if ($first->{text} eq "gate-file-known") {
        note("item/started", { threadId => $tid, turnId => "turn_g", startedAtMs => 1, item => { type => "fileChange", id => "item_f", status => "inProgress", changes => [ { path => "src/a.rs", kind => { type => "update" }, diff => "" } ] } });
      }
      out({ id => $gate->[0], method => $gate->[1], params => { threadId => $tid, turnId => "turn_g", %{ $gate->[2] } } });
      next;
    }
    $active = "turn_1";
    out({ id => $id, result => { turn => { id => "turn_1", status => "inProgress", items => [] } } });
    if ($first->{type} eq "skill") {
      note("item/agentMessage/delta", { threadId => $tid, turnId => "turn_1", itemId => "m0", delta => "skill:$first->{name}:$first->{path}" });
      next;
    }
    note("turn/started", { threadId => $tid, turn => { id => "turn_1", status => "inProgress", items => [] } });
    note("thread/started", { thread => { id => "thr_child", parentThreadId => $tid, sessionId => "s1", preview => "", turns => [], cliVersion => "0.158.0", createdAt => 1, updatedAt => 1, ephemeral => JSON::PP::false, modelProvider => "openai" } });
    note("turn/started", { threadId => "thr_child", turn => { id => "turn_c", status => "inProgress", items => [] } });
    note("item/agentMessage/delta", { threadId => "thr_child", turnId => "turn_c", itemId => "m1", delta => "looking" });
    note("item/started", { threadId => $tid, turnId => "turn_1", startedAtMs => 1, item => { type => "commandExecution", id => "item_1", command => "cargo test", cwd => "/w", status => "inProgress", commandActions => [ { type => "unknown", command => "cargo test" } ] } });
    note("item/completed", { threadId => $tid, turnId => "turn_1", completedAtMs => 2, item => { type => "commandExecution", id => "item_1", command => "cargo test", cwd => "/w", status => "completed", aggregatedOutput => "ok", exitCode => 0, commandActions => [] } });
    out({ id => "srv-1", method => "item/commandExecution/requestApproval", params => { threadId => $tid, turnId => "turn_1", itemId => "item_2", command => "rm -rf build", reason => "needs write", startedAtMs => 3 } });
    note("thread/tokenUsage/updated", { threadId => $tid, turnId => "turn_1", tokenUsage => { modelContextWindow => 272000,
      total => { inputTokens => 1200, cachedInputTokens => 200, outputTokens => 50, reasoningOutputTokens => 10, totalTokens => 1250 },
      last => { inputTokens => 900, cachedInputTokens => 100, outputTokens => 40, reasoningOutputTokens => 5, totalTokens => 940 } } });
    note("turn/completed", { threadId => "thr_child", turn => { id => "turn_c", status => "completed", items => [] } });
  } elsif ($method eq "turn/steer") {
    if ($p->{input}[0]{text} eq "late" || $p->{expectedTurnId} ne $active) {
      out({ id => $id, error => { code => -32600, message => "no active turn to steer" } });
    } else {
      out({ id => $id, result => { turnId => $active } });
    }
  } elsif ($method eq "turn/interrupt") {
    next if ($ENV{FAKE_INTERRUPT_SILENT} // "") ne "";
    out({ id => $id, result => {} });
    note("turn/completed", { threadId => $tid, turn => { id => $p->{turnId}, status => "interrupted", items => [] } });
    $active = "";
  } elsif ($method eq "thread/compact/start" || $method eq "thread/unsubscribe" || $method eq "thread/archive" || $method eq "review/start") {
    out({ id => $id, result => {} });
  } else {
    out({ id => $id, error => { code => -32601, message => "method not found: $method" } });
  }
}
"#;

pub(crate) fn launch(dir: &Path, env: Vec<(std::ffi::OsString, std::ffi::OsString)>) -> LaunchSpec {
    let program = dir.join("fake-codex");
    std::fs::write(&program, FAKE_APP_SERVER).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    LaunchSpec {
        provider: crate::providers::test_support::CODEX,
        program,
        workdir: dir.to_path_buf(),
        settings: SettingsRevision(1),
        user_config: UserProviderConfig::default(),
        defaults: SaturnDefaults {
            auto_compact_tokens: Some(180_000),
        },
        env,
        hook_settings: None,
        permission: PermissionLaunch::default(),
        masker: Masker::new(Vec::new()),
    }
}

fn spec(dir: &Path) -> SessionSpec {
    SessionSpec {
        agent: AgentId(7),
        workdir: dir.to_path_buf(),
        model: None,
        settings: SettingsRevision(1),
        resume: None,
        packet: None,
        add_dirs: Vec::new(),
        interrupted_children: Vec::new(),
    }
}

async fn take(client: &mut CodexClient, count: usize) -> Vec<ProviderEvent> {
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

async fn start(dir: &Path) -> (CodexClient, SessionHandle) {
    let mut client = CodexClient::start(launch(dir, Vec::new()), Supervisor::new())
        .await
        .unwrap();
    let handle = client.open_session(spec(dir)).await.unwrap();
    (client, handle)
}

fn expected_turn_events(agent: AgentId) -> Vec<ProviderEvent> {
    let child = Some(SubagentId("thr_child".to_owned()));
    vec![
        ProviderEvent::SubagentStarted {
            agent,
            subagent: SubagentId("thr_child".to_owned()),
            parent: None,
        },
        ProviderEvent::Text {
            agent,
            subagent: child.clone(),
            text: "looking".to_owned(),
        },
        ProviderEvent::ToolCall {
            agent,
            subagent: None,
            call_id: "item_1".to_owned(),
            activity: Activity::RunningCommand {
                command: "cargo test".to_owned(),
            },
            detail: ToolDetail {
                category: ToolCategory::TestRun,
                ..ToolDetail::default()
            },
        },
        ProviderEvent::ToolResult {
            agent,
            subagent: None,
            call_id: "item_1".to_owned(),
            output: "ok".to_owned(),
            exit_code: Some(0),
        },
        ProviderEvent::PermissionRequested {
            agent,
            request_id: "srv-1".to_owned(),
            summary: "run command: rm -rf build".to_owned(),
            reason: "needs write".to_owned(),
            call: Some(PermissionCall {
                tool: PermissionTool::Shell,
                target: "rm -rf build".to_owned(),
                paths: Vec::new(),
            }),
        },
        ProviderEvent::Usage(UsageReport {
            agent,
            subagent: None,
            model: Some("gpt-test".to_owned()),
            scope: UsageScope::ThreadCumulative,
            input: Some(1000),
            cache_read: Some(200),
            cache_write: None,
            output: Some(50),
            reasoning: Some(10),
        }),
        ProviderEvent::SubagentEnded {
            agent,
            subagent: SubagentId("thr_child".to_owned()),
        },
    ]
}

#[tokio::test]
async fn turn_events_are_converted_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session.clone();
    assert_eq!(main.0, "thr_main");
    assert!(!handle.steer_verified);
    assert_eq!(
        client.applied_settings(&main),
        Some(AppliedSettings {
            model: Some("gpt-test".to_owned()),
            permission: Some("untrusted".to_owned()),
        })
    );

    client.send_turn(&main, "fix the build").await.unwrap();
    let events = take(&mut client, 7).await;

    let agent = AgentId(7);
    assert_eq!(events, expected_turn_events(agent));

    client.steer(&main, "also run clippy").await.unwrap();
    client
        .interrupt(
            &main,
            InterruptTarget::Subagent(SubagentId("gone".to_owned())),
        )
        .await
        .unwrap();
    client
        .interrupt(&main, InterruptTarget::Main)
        .await
        .unwrap();
    let end = take(&mut client, 2).await;
    assert_eq!(
        end,
        vec![
            ProviderEvent::ContextSize {
                agent,
                tokens: Some(940),
            },
            ProviderEvent::TurnCompleted {
                agent,
                origin: TurnOrigin::User,
            },
        ]
    );
    assert!(matches!(
        client.steer(&main, "after end").await,
        Err(ProviderError::NoActiveTurn)
    ));
}

/// `text` 턴이 승인 요청에서 멈추게 한 뒤, 답이 없는 동안 턴이 멈춰 있는지 확인하고 답한다.
/// 올라온 요청과 가짜 app-server가 받은 응답(`id`, `result`)을 돌려준다.
async fn answer_gate(text: &str, answer: PermissionAnswer) -> (ProviderEvent, Value) {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, text).await.unwrap();
    let requested = take(&mut client, 1).await.remove(0);
    let ProviderEvent::PermissionRequested { request_id, .. } = &requested else {
        panic!("expected a permission request, got {requested:?}");
    };
    let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
    assert!(stalled.is_err(), "turn should wait for the answer");

    client
        .answer_permission(&main, request_id, answer)
        .await
        .unwrap();
    let resumed = take(&mut client, 3).await;

    let ProviderEvent::Text { text, .. } = &resumed[0] else {
        panic!(
            "expected the turn to continue with text, got {:?}",
            resumed[0]
        );
    };
    assert!(matches!(resumed[2], ProviderEvent::TurnCompleted { .. }));
    let received = text.strip_prefix("answer:").expect("answer prefix");
    (requested, serde_json::from_str(received).unwrap())
}

fn deny() -> PermissionAnswer {
    PermissionAnswer::Deny { note: None }
}

#[tokio::test]
async fn command_approval_is_answered_with_the_same_numeric_request_id() {
    let (requested, received) = answer_gate("gate-command", PermissionAnswer::AllowOnce).await;

    assert_eq!(
        requested,
        ProviderEvent::PermissionRequested {
            agent: AgentId(7),
            request_id: "7".to_owned(),
            summary: "run command: touch a.txt".to_owned(),
            reason: "needs write".to_owned(),
            call: Some(PermissionCall {
                tool: PermissionTool::Shell,
                target: "touch a.txt".to_owned(),
                paths: Vec::new(),
            }),
        }
    );
    assert_eq!(
        received,
        json!({ "id": 7, "result": { "decision": "accept" } })
    );
}

#[tokio::test]
async fn command_decisions_follow_the_answer_and_the_available_list() {
    let always = answer_gate("gate-command", PermissionAnswer::AllowAlways)
        .await
        .1;
    let listed_without_session =
        answer_gate("gate-command-once-only", PermissionAnswer::AllowAlways)
            .await
            .1;
    let denied = answer_gate("gate-command", deny()).await.1;
    let denied_without_decline_listed = answer_gate("gate-command-once-only", deny()).await.1;

    assert_eq!(always["result"], json!({ "decision": "acceptForSession" }));
    assert_eq!(listed_without_session["id"], 8);
    assert_eq!(
        listed_without_session["result"],
        json!({ "decision": "accept" })
    );
    assert_eq!(denied["result"], json!({ "decision": "decline" }));
    assert_eq!(
        denied_without_decline_listed["result"],
        json!({ "decision": "decline" })
    );
}

#[tokio::test]
async fn file_change_approval_keeps_a_string_request_id() {
    let (requested, received) = answer_gate("gate-file", PermissionAnswer::AllowOnce).await;

    assert!(matches!(
        requested,
        ProviderEvent::PermissionRequested { ref request_id, ref summary, .. }
            if request_id == "srv-file" && summary == "change files"
    ));
    assert_eq!(
        received,
        json!({ "id": "srv-file", "result": { "decision": "accept" } })
    );
}

#[tokio::test]
async fn mcp_tool_approval_is_answered_with_an_elicitation_action() {
    let (requested, allowed) = answer_gate("gate-mcp", PermissionAnswer::AllowOnce).await;
    let always = answer_gate("gate-mcp", PermissionAnswer::AllowAlways)
        .await
        .1;
    let denied = answer_gate("gate-mcp", deny()).await.1;

    assert!(matches!(
        requested,
        ProviderEvent::PermissionRequested { ref request_id, ref summary, .. }
            if request_id == "0" && summary.contains("run tool")
    ));
    assert_eq!(
        allowed,
        json!({ "id": 0, "result": { "action": "accept", "content": {} } })
    );
    assert_eq!(always, allowed);
    assert_eq!(denied["result"], json!({ "action": "decline" }));
}

#[tokio::test]
async fn permission_grant_approval_returns_the_requested_permissions() {
    let once = answer_gate("gate-permissions", PermissionAnswer::AllowOnce)
        .await
        .1;
    let always = answer_gate("gate-permissions", PermissionAnswer::AllowAlways)
        .await
        .1;
    let denied = answer_gate("gate-permissions", deny()).await.1;

    let network = json!({ "network": { "enabled": true } });
    assert_eq!(
        once["result"],
        json!({ "permissions": network, "scope": "turn" })
    );
    assert_eq!(
        always["result"],
        json!({ "permissions": network, "scope": "session" })
    );
    assert_eq!(
        denied["result"],
        json!({ "permissions": {}, "scope": "turn" })
    );
}

#[tokio::test]
async fn legacy_approvals_use_review_decisions() {
    let once = answer_gate("gate-legacy", PermissionAnswer::AllowOnce)
        .await
        .1;
    let always = answer_gate("gate-legacy", PermissionAnswer::AllowAlways)
        .await
        .1;
    let denied = answer_gate("gate-legacy", deny()).await.1;

    assert_eq!(once["id"], "legacy-1");
    assert_eq!(once["result"], json!({ "decision": "approved" }));
    assert_eq!(
        always["result"],
        json!({ "decision": "approved_for_session" })
    );
    assert_eq!(denied["result"], json!({ "decision": "denied" }));
}

/// `text` 턴이 입력 요청에서 멈추게 한 뒤, 답이 없는 동안 턴이 멈춰 있는지 확인하고 답한다.
/// 올라온 요청과 가짜 app-server가 받은 응답(`id`, `result`)을 돌려준다.
async fn answer_input_gate(text: &str, answer: InputAnswer) -> (ProviderEvent, Value) {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, text).await.unwrap();
    let requested = take(&mut client, 1).await.remove(0);
    let ProviderEvent::InputRequested { request_id, .. } = &requested else {
        panic!("expected an input request, got {requested:?}");
    };
    let stalled = tokio::time::timeout(Duration::from_millis(300), client.next_event()).await;
    assert!(stalled.is_err(), "turn should wait for the answer");

    client
        .answer_input(&main, request_id, answer)
        .await
        .unwrap();
    let resumed = take(&mut client, 3).await;

    let ProviderEvent::Text { text, .. } = &resumed[0] else {
        panic!(
            "expected the turn to continue with text, got {:?}",
            resumed[0]
        );
    };
    let received = text.strip_prefix("answer:").expect("answer prefix");
    (requested, serde_json::from_str(received).unwrap())
}

#[tokio::test]
async fn elicitation_form_round_trip_answers_accept_with_content() {
    let submit = InputAnswer::Submit {
        values: vec![
            ("title".to_owned(), InputValue::Text("Saturn".to_owned())),
            ("count".to_owned(), InputValue::Integer(3)),
            ("enabled".to_owned(), InputValue::Boolean(true)),
            (
                "choice".to_owned(),
                InputValue::Selected(vec!["a".to_owned()]),
            ),
            (
                "tags".to_owned(),
                InputValue::Selected(vec!["x".to_owned(), "y".to_owned()]),
            ),
        ],
    };

    let (requested, received) = answer_input_gate("gate-elicit-form", submit).await;

    let ProviderEvent::InputRequested { request, .. } = requested else {
        unreachable!("answer_input_gate returns an input request");
    };
    assert_eq!(request.message, "실험 입력을 작성하라");
    assert_eq!(request.fields.len(), 5);
    assert_eq!(request.fields[0].id, "title");
    assert_eq!(received["id"], json!(41));
    assert_eq!(
        received["result"],
        json!({ "action": "accept", "content": {
            "title": "Saturn", "count": 3, "enabled": true, "choice": "a", "tags": ["x", "y"]
        } })
    );
}

#[tokio::test]
async fn elicitation_form_round_trip_answers_decline_and_cancel() {
    let declined = answer_input_gate("gate-elicit-form", InputAnswer::Decline)
        .await
        .1;
    let cancelled = answer_input_gate("gate-elicit-form", InputAnswer::Cancel)
        .await
        .1;

    assert_eq!(declined["result"], json!({ "action": "decline" }));
    assert_eq!(cancelled["result"], json!({ "action": "cancel" }));
    assert_eq!(declined["id"], json!(41));
}

#[tokio::test]
async fn elicitation_url_round_trip_answers_accept_decline_and_cancel() {
    let accepted = answer_input_gate(
        "gate-elicit-url",
        InputAnswer::Submit { values: Vec::new() },
    )
    .await;
    let declined = answer_input_gate("gate-elicit-url", InputAnswer::Decline)
        .await
        .1;
    let cancelled = answer_input_gate("gate-elicit-url", InputAnswer::Cancel)
        .await
        .1;

    let ProviderEvent::InputRequested { request, .. } = &accepted.0 else {
        unreachable!("answer_input_gate returns an input request");
    };
    assert_eq!(
        request.url.as_deref(),
        Some("https://example.invalid/experiment-252")
    );
    assert!(request.fields.is_empty());
    assert_eq!(accepted.1["result"], json!({ "action": "accept" }));
    assert_eq!(declined["result"], json!({ "action": "decline" }));
    assert_eq!(cancelled["result"], json!({ "action": "cancel" }));
}

#[tokio::test]
async fn elicitation_user_input_round_trip_answers_by_question_id() {
    let submit = InputAnswer::Submit {
        values: vec![(
            "preference".to_owned(),
            InputValue::Selected(vec!["설계 검토".to_owned()]),
        )],
    };

    let (requested, received) = answer_input_gate("gate-user-input", submit).await;
    let cancelled = answer_input_gate("gate-user-input", InputAnswer::Cancel)
        .await
        .1;

    let ProviderEvent::InputRequested { request, .. } = requested else {
        unreachable!("answer_input_gate returns an input request");
    };
    assert_eq!(request.fields[0].id, "preference");
    assert_eq!(received["id"], json!(43));
    assert_eq!(
        received["result"],
        json!({ "answers": { "preference": { "answers": ["설계 검토"] } } })
    );
    assert_eq!(cancelled["result"], json!({ "answers": {} }));
}

#[tokio::test]
async fn elicitation_input_and_permission_answers_do_not_cross() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, "gate-elicit-url").await.unwrap();
    take(&mut client, 1).await;

    let as_permission = client
        .answer_permission(&main, "42", PermissionAnswer::AllowOnce)
        .await;
    let unknown = client.answer_input(&main, "99", InputAnswer::Cancel).await;
    let as_input = client.answer_input(&main, "42", InputAnswer::Cancel).await;
    let again = client.answer_input(&main, "42", InputAnswer::Cancel).await;

    assert!(matches!(as_permission, Err(ProviderError::NotSent { .. })));
    assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
    assert!(as_input.is_ok());
    assert!(matches!(again, Err(ProviderError::NotSent { .. })));
}

#[tokio::test]
async fn elicitation_approval_is_still_a_permission_request() {
    let (requested, _) = answer_gate("gate-mcp", PermissionAnswer::AllowOnce).await;

    assert!(matches!(
        requested,
        ProviderEvent::PermissionRequested { .. }
    ));
}

#[tokio::test]
async fn answering_an_unknown_or_answered_request_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, "gate-command").await.unwrap();
    take(&mut client, 1).await;

    let unknown = client
        .answer_permission(&main, "99", PermissionAnswer::AllowOnce)
        .await;
    client
        .answer_permission(&main, "7", PermissionAnswer::AllowOnce)
        .await
        .unwrap();
    let again = client
        .answer_permission(&main, "7", PermissionAnswer::AllowOnce)
        .await;

    assert!(matches!(unknown, Err(ProviderError::NotSent { .. })));
    assert!(matches!(again, Err(ProviderError::NotSent { .. })));
}

#[tokio::test]
async fn rejected_steer_is_no_active_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, "start").await.unwrap();

    let error = client.steer(&main, "late").await.unwrap_err();

    assert!(matches!(error, ProviderError::NoActiveTurn));
    assert!(matches!(
        client.steer(&main, "again").await,
        Err(ProviderError::NoActiveTurn)
    ));
}

#[tokio::test]
async fn commands_compact_skills_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;

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
    client.compact(&main).await.unwrap();
    client.send_turn(&main, "/compact").await.unwrap();
    client.send_turn(&main, "/lint src").await.unwrap();
    let skill = take(&mut client, 1).await;
    assert_eq!(
        skill,
        vec![ProviderEvent::Text {
            agent: AgentId(7),
            subagent: None,
            text: "skill:lint:/skills/lint/SKILL.md".to_owned(),
        }]
    );

    client.close_session(&main).await.unwrap();

    assert!(matches!(
        client.send_turn(&main, "hi").await,
        Err(ProviderError::NotSent { .. })
    ));
    assert!(client.applied_settings(&main).is_none());
}

#[tokio::test]
async fn stream_end_mid_turn_reports_lost() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;

    client
        .send_turn(&handle.provider_session, "crash")
        .await
        .unwrap();
    let events = take(&mut client, 1).await;

    assert_eq!(
        events,
        vec![ProviderEvent::StreamLost { agent: AgentId(7) }]
    );
    let closed = tokio::time::timeout(Duration::from_secs(5), client.next_event())
        .await
        .unwrap();
    assert_eq!(closed, None);
    assert!(matches!(
        client.compact(&handle.provider_session).await,
        Err(ProviderError::NotSent { .. })
    ));
}

#[tokio::test]
async fn missing_program_is_connection_lost() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), Vec::new());
    launch.program = PathBuf::from("/nonexistent/codex");

    let error = CodexClient::start(launch, Supervisor::new())
        .await
        .unwrap_err();

    assert!(matches!(error, ProviderError::ConnectionLost));
}

#[test]
fn defaults_respect_user_config() {
    let dir = tempfile::tempdir().unwrap();
    let launch = launch(dir.path(), Vec::new());

    let none = default_args(UserProviderConfig::default(), &launch);
    let compact_set = default_args(
        UserProviderConfig {
            has_auto_compact: true,
        },
        &launch,
    );

    assert_eq!(none, vec!["-c", "model_auto_compact_token_limit=180000"]);
    assert!(compact_set.is_empty());
}

#[test]
fn user_config_reads_root_and_selected_profile() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "profile = \"fast\"\n# approval_policy = \"never\"\n[profiles.slow]\nsandbox_mode = \"read-only\"\n\
         [profiles.fast]\nmodel_auto_compact_token_limit = 9000\n",
    )
    .unwrap();
    let env = vec![("CODEX_HOME".into(), home.clone().into_os_string())];

    let found = read_user_config(&launch(dir.path(), env));

    assert_eq!(
        found,
        UserProviderConfig {
            has_auto_compact: true,
        }
    );
    assert_eq!(
        scan_user_config("approval_policy = \"never\"\n"),
        UserProviderConfig::default()
    );
    assert_eq!(
        read_user_config(&launch(dir.path(), Vec::new())),
        UserProviderConfig::default()
    );
}

#[test]
fn model_list_entries_skip_hidden_models_and_fall_back_to_the_id() {
    let visible = json!({ "id": "a", "model": "gpt-a", "displayName": "GPT A", "hidden": false });
    let hidden = json!({ "id": "b", "model": "gpt-b", "hidden": true });
    let unnamed = json!({ "id": "c" });

    let infos: Vec<(String, String)> = [visible, hidden, unnamed]
        .iter()
        .filter_map(model_info)
        .map(|info| (info.choice.model, info.name))
        .collect();

    assert_eq!(
        infos,
        vec![
            ("gpt-a".to_owned(), "GPT A".to_owned()),
            ("c".to_owned(), "c".to_owned())
        ]
    );
}

#[test]
fn nested_child_threads_point_to_parent_subagent() {
    let mut threads = HashMap::new();
    let main = ProviderSessionId("main".to_owned());
    threads.insert(
        main,
        ThreadState::new(AgentId(1), None, AppliedSettings::default()),
    );
    let started =
        |id: &str, parent: &str| json!({ "thread": { "id": id, "parentThreadId": parent } });

    let first = convert_notification(&mut threads, "thread/started", &started("a", "main"));
    let second = convert_notification(&mut threads, "thread/started", &started("b", "a"));
    let unknown = convert_notification(&mut threads, "thread/started", &started("c", "zzz"));

    assert_eq!(
        second,
        vec![ProviderEvent::SubagentStarted {
            agent: AgentId(1),
            subagent: SubagentId("b".to_owned()),
            parent: Some(SubagentId("a".to_owned())),
        }]
    );
    assert_eq!(first.len(), 1);
    assert!(unknown.is_empty());
    remove_thread_tree(&mut threads, &ProviderSessionId("a".to_owned()));
    assert_eq!(threads.len(), 1);
}

#[test]
fn read_only_commands_are_reading() {
    let item = json!({
        "type": "commandExecution",
        "command": "rg foo",
        "commandActions": [{ "type": "search", "command": "rg foo" }],
    });

    assert_eq!(activity_of(&item), Some(Activity::ReadingFile));
    assert_eq!(
        activity_of(&json!({ "type": "fileChange" })),
        Some(Activity::EditingFile)
    );
    assert_eq!(activity_of(&json!({ "type": "agentMessage" })), None);
    assert!(is_no_active_turn(&json!({ "message": "No active turn" })));
    assert!(!is_no_active_turn(&json!({ "message": "invalid input" })));
}

#[test]
fn activity_of_wrapped_command_is_inner_command() {
    let item = json!({
        "type": "commandExecution",
        "command": "/bin/zsh -lc 'python3 scripts/check.py'",
        "commandActions": [{ "type": "unknown", "command": "python3 scripts/check.py" }],
    });

    assert_eq!(
        activity_of(&item),
        Some(Activity::RunningCommand {
            command: "python3 scripts/check.py".to_owned()
        })
    );
}

#[test]
fn detail_of_reasoning_is_not_a_candidate() {
    let detail = detail_of(&json!({ "type": "reasoning", "summary": [] }));

    assert_eq!(detail.category, ToolCategory::Reasoning);
    assert!(!detail.category.is_candidate());
}

#[test]
fn detail_of_read_command_keeps_action_paths() {
    let item = json!({
        "type": "commandExecution",
        "command": "/bin/zsh -lc 'cat notes/a.txt'",
        "commandActions": [
            { "type": "read", "command": "cat notes/a.txt", "name": "a.txt", "path": "/work/notes/a.txt" }
        ],
    });

    let detail = detail_of(&item);

    assert_eq!(detail.category, ToolCategory::FileRead);
    assert_eq!(detail.paths, vec!["/work/notes/a.txt".to_owned()]);
}

#[test]
fn detail_of_test_command_is_test_run_without_paths() {
    let item = json!({
        "type": "commandExecution",
        "command": "/bin/zsh -lc 'python3 -m unittest tests.test_rules'",
        "commandActions": [{ "type": "unknown", "command": "python3 -m unittest tests.test_rules" }],
    });

    let detail = detail_of(&item);

    assert_eq!(detail.category, ToolCategory::TestRun);
    assert!(detail.paths.is_empty());
}

#[test]
fn detail_of_file_change_counts_diff_lines() {
    let item = json!({
        "type": "fileChange",
        "changes": [{
            "path": "/work/config/app.cfg",
            "kind": { "type": "update", "move_path": null },
            "diff": "@@ -1,4 +1,4 @@\n name=demo\n-mode=draft\n-retries=1\n+mode=harbor\n+retries=tundra\n level=2\n",
        }],
    });

    let detail = detail_of(&item);

    assert_eq!(detail.category, ToolCategory::FileEdit);
    assert_eq!(detail.paths, vec!["/work/config/app.cfg".to_owned()]);
    assert_eq!(
        detail.changed,
        Some(LineChange {
            added: 2,
            removed: 2
        })
    );
}

#[test]
fn detail_of_file_change_added_file_counts_whole_text() {
    let item = json!({
        "type": "fileChange",
        "changes": [{ "path": "/work/new.txt", "kind": { "type": "add" }, "diff": "a\nb\nc\n" }],
    });

    let changed = detail_of(&item).changed;

    assert_eq!(
        changed,
        Some(LineChange {
            added: 3,
            removed: 0
        })
    );
}

#[test]
fn tool_output_file_change_is_path_and_diff() {
    let item = json!({
        "type": "fileChange",
        "changes": [{ "path": "/work/a.cfg", "kind": { "type": "update" }, "diff": "-a\n+b\n" }],
    });

    assert_eq!(tool_output(&item).as_deref(), Some("/work/a.cfg\n-a\n+b\n"));
}

#[test]
fn exit_code_of_command_reads_code_and_ignores_other_items() {
    let failed = json!({ "type": "commandExecution", "exitCode": 3 });
    let running = json!({ "type": "commandExecution", "exitCode": null });
    let patch = json!({ "type": "fileChange", "exitCode": 1 });

    assert_eq!(exit_code_of(&failed), Some(3));
    assert_eq!(exit_code_of(&running), None);
    assert_eq!(exit_code_of(&patch), None);
}

#[test]
fn app_server_values_hide_router_key_before_routing() {
    let key = "sk-secret-1234";
    let mut message = json!({
        "id": 1,
        "error": { "message": format!("bad key {key}") },
        "params": { "item": { "aggregatedOutput": [key] } },
    });

    mask_values(&mut message, &Masker::new(vec![key.to_owned()]));

    assert!(!message.to_string().contains(key));
    assert_eq!(message["error"]["message"], "bad key [redacted]");
}

fn rule(tool: PermissionTool, pattern: &str, verdict: Verdict) -> Rule {
    Rule {
        tool,
        pattern: pattern.to_owned(),
        verdict,
    }
}

fn policy(rules: Vec<Rule>) -> Policy {
    Policy {
        mode: Mode::Edit,
        workdir: PathBuf::from("/work"),
        extra_dirs: Vec::new(),
        rules,
        always: Vec::new(),
    }
}

/// `text` 턴이 올린 승인 요청을 Saturn 규칙으로 판정한다. 허용과 거부는 그 답을 보내 가짜 app-server가 받은
/// 응답(`id`, `result`)을 함께 돌려주고, 묻기는 답하지 않고 `None`을 돌려준다.
async fn answer_by_rules(text: &str, policy: &Policy) -> (Verdict, Option<Value>) {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;
    let main = handle.provider_session;
    client.send_turn(&main, text).await.unwrap();
    let requested = loop {
        let event = take(&mut client, 1).await.remove(0);
        if matches!(event, ProviderEvent::PermissionRequested { .. }) {
            break event;
        }
    };
    let ProviderEvent::PermissionRequested {
        request_id, call, ..
    } = &requested
    else {
        unreachable!("loop should stop at a permission request");
    };
    let call = call.as_ref().expect("request should be readable");
    let verdict = policy.decide(call);
    let answer = match verdict {
        Verdict::Allow => PermissionAnswer::AllowOnce,
        Verdict::Deny => deny(),
        Verdict::Ask => return (verdict, None),
    };
    client
        .answer_permission(&main, request_id, answer)
        .await
        .unwrap();
    let resumed = take(&mut client, 3).await;
    let ProviderEvent::Text { text, .. } = &resumed[0] else {
        panic!("expected text after the answer, got {:?}", resumed[0]);
    };
    let received = text.strip_prefix("answer:").expect("answer prefix");
    (verdict, Some(serde_json::from_str(received).unwrap()))
}

#[tokio::test]
async fn permission_shell() {
    let allow = policy(vec![rule(PermissionTool::Shell, "touch *", Verdict::Allow)]);
    let denied = policy(vec![rule(PermissionTool::Shell, "touch *", Verdict::Deny)]);

    let allowed = answer_by_rules("gate-command", &allow).await;
    let refused = answer_by_rules("gate-command", &denied).await;
    let asked = answer_by_rules("gate-command", &policy(Vec::new())).await;

    assert_eq!(
        allowed,
        (
            Verdict::Allow,
            Some(json!({ "id": 7, "result": { "decision": "accept" } }))
        )
    );
    assert_eq!(
        refused,
        (
            Verdict::Deny,
            Some(json!({ "id": 7, "result": { "decision": "decline" } }))
        )
    );
    assert_eq!(asked, (Verdict::Ask, None));
}

#[tokio::test]
async fn permission_edit() {
    let denied = policy(vec![rule(PermissionTool::Edit, "src/*", Verdict::Deny)]);

    let inside = answer_by_rules("gate-file-known", &policy(Vec::new())).await;
    let refused = answer_by_rules("gate-file-known", &denied).await;
    let unknown_path = answer_by_rules("gate-file", &policy(Vec::new())).await;

    assert_eq!(
        inside,
        (
            Verdict::Allow,
            Some(json!({ "id": "srv-file2", "result": { "decision": "accept" } }))
        )
    );
    assert_eq!(
        refused,
        (
            Verdict::Deny,
            Some(json!({ "id": "srv-file2", "result": { "decision": "decline" } }))
        )
    );
    assert_eq!(unknown_path, (Verdict::Ask, None));
}

#[tokio::test]
async fn permission_subagent() {
    let allow = policy(vec![rule(PermissionTool::Shell, "rm *", Verdict::Allow)]);
    let denied = policy(vec![rule(PermissionTool::Shell, "rm *", Verdict::Deny)]);

    let allowed = answer_by_rules("gate-child", &allow).await;
    let refused = answer_by_rules("gate-child", &denied).await;

    assert_eq!(
        allowed,
        (
            Verdict::Allow,
            Some(json!({ "id": 31, "result": { "decision": "accept" } }))
        )
    );
    assert_eq!(
        refused,
        (
            Verdict::Deny,
            Some(json!({ "id": 31, "result": { "decision": "decline" } }))
        )
    );
}

#[tokio::test]
async fn permission_subagent_request_belongs_to_the_parent_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, handle) = start(dir.path()).await;

    client
        .send_turn(&handle.provider_session, "gate-child")
        .await
        .unwrap();
    let events = take(&mut client, 2).await;

    assert!(matches!(
        events[0],
        ProviderEvent::SubagentStarted {
            agent: AgentId(7),
            ..
        }
    ));
    assert!(matches!(
        &events[1],
        ProviderEvent::PermissionRequested { agent: AgentId(7), call: Some(call), .. }
            if call.target == "rm -rf build"
    ));
}

#[tokio::test]
async fn permission_mcp() {
    let allow = policy(vec![rule(
        PermissionTool::Mcp,
        "mcp__probe__echo",
        Verdict::Allow,
    )]);
    let denied = policy(vec![rule(
        PermissionTool::Mcp,
        "mcp__probe__*",
        Verdict::Deny,
    )]);

    let allowed = answer_by_rules("gate-mcp", &allow).await;
    let refused = answer_by_rules("gate-mcp", &denied).await;
    let asked = answer_by_rules("gate-mcp", &policy(Vec::new())).await;

    assert_eq!(
        allowed,
        (
            Verdict::Allow,
            Some(json!({ "id": 0, "result": { "action": "accept", "content": {} } }))
        )
    );
    assert_eq!(
        refused,
        (
            Verdict::Deny,
            Some(json!({ "id": 0, "result": { "action": "decline" } }))
        )
    );
    assert_eq!(asked, (Verdict::Ask, None));
}

#[tokio::test]
async fn startup_checks_applied_policy_whatever_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let newer = vec![("FAKE_USER_AGENT".into(), "fake/0.200.0".into())];

    let mut client = CodexClient::start(launch(dir.path(), newer), Supervisor::new())
        .await
        .unwrap();

    assert!(client.open_session(spec(dir.path())).await.is_ok());
    for (model, expected) in [
        ("wrong-policy", "approval policy"),
        ("wrong-sandbox", "sandbox"),
    ] {
        let mut client = CodexClient::start(launch(dir.path(), Vec::new()), Supervisor::new())
            .await
            .unwrap();
        let mut wrong = spec(dir.path());
        wrong.model = Some(model.to_owned());

        let error = client.open_session(wrong).await.unwrap_err();

        assert!(matches!(
            error,
            ProviderError::NotSent { ref reason } if reason.contains(expected)
        ));
        let thread = ProviderSessionId("thr_main".to_owned());
        assert!(client.applied_settings(&thread).is_none());
    }
    let (client, handle) = start(dir.path()).await;
    let applied = client.applied_settings(&handle.provider_session).unwrap();
    assert_eq!(applied.permission.as_deref(), Some("untrusted"));
}

#[tokio::test]
async fn add_dir_goes_to_the_thread_config_when_a_session_opens() {
    let dir = tempfile::tempdir().unwrap();
    let env = || vec![("FAKE_REQUIRE_ADD_DIR".into(), "/extra".into())];
    let mut with_dir = spec(dir.path());
    with_dir.add_dirs = vec![PathBuf::from("/extra")];
    let mut resumed = with_dir.clone();
    resumed.resume = Some(ProviderSessionId("thr_main".to_owned()));
    let mut client = CodexClient::start(launch(dir.path(), env()), Supervisor::new())
        .await
        .unwrap();

    let opened = client.open_session(with_dir).await;
    let reopened = client.open_session(resumed).await;
    let without = client.open_session(spec(dir.path())).await;

    assert!(opened.is_ok());
    assert!(reopened.is_ok());
    assert!(matches!(
        without,
        Err(ProviderError::NotSent { ref reason }) if reason.contains("added folders")
    ));
}

/// 패킷 턴 중에 온 입력은 줄 섰다가 자기 완료를 받는다(#318). 엔진처럼 `next_arrival`로 읽을 때 그 입력의 전송 응답이
/// 늦어도(첫 poll이 `Pending`) 완료 이벤트와 그 입력을 잃지 않는다(#324).
#[tokio::test]
async fn a_turn_sent_during_the_packet_turn_gets_its_own_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = CodexClient::start(launch(dir.path(), Vec::new()), Supervisor::new())
        .await
        .unwrap();
    let mut packet_spec = spec(dir.path());
    packet_spec.packet = Some("merging-packet".to_owned());
    let handle = client.open_session(packet_spec).await.unwrap();
    client
        .send_turn(&handle.provider_session, "quick")
        .await
        .unwrap();
    let mut providers = HashMap::from([(
        (ChatId(1), crate::providers::test_support::CODEX),
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
        ProviderEvent::Text { text, .. } if text == "quick-answer"
    )));
    assert!(!format!("{events:?}").contains("merged:"));
}

#[tokio::test]
async fn interrupt_drops_the_turns_waiting_behind_the_running_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = CodexClient::start(launch(dir.path(), Vec::new()), Supervisor::new())
        .await
        .unwrap();
    let mut packet_spec = spec(dir.path());
    packet_spec.packet = Some("merging-packet".to_owned());
    let handle = client.open_session(packet_spec).await.unwrap();
    client
        .send_turn(&handle.provider_session, "quick")
        .await
        .unwrap();

    client
        .interrupt(&handle.provider_session, InterruptTarget::Main)
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(1500), client.next_event()).await
    {
        events.push(event);
    }

    assert!(!events.is_empty());
    assert!(
        !format!("{events:?}").contains("quick-answer"),
        "the waiting turn must not be sent after the interrupt: {events:?}"
    );
}

#[tokio::test]
async fn first_turn_waits_for_mcp_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), vec![("FAKE_REQUIRE_MCP".into(), "1".into())]);
    launch.permission.mcp_servers = vec!["docs".to_owned()];
    let mut client = CodexClient::start(launch, Supervisor::new()).await.unwrap();

    let handle = client.open_session(spec(dir.path())).await.unwrap();

    client
        .send_turn(&handle.provider_session, "hello")
        .await
        .unwrap();
}

#[tokio::test]
async fn first_turn_is_sent_when_a_server_failed_to_start() {
    let dir = tempfile::tempdir().unwrap();
    let env = vec![
        ("FAKE_REQUIRE_MCP".into(), "1".into()),
        ("FAKE_MCP_FAILED".into(), "broken".into()),
    ];
    let mut launch = launch(dir.path(), env);
    launch.permission.mcp_servers = vec!["docs".to_owned(), "broken".to_owned()];
    let mut client = CodexClient::start(launch, Supervisor::new()).await.unwrap();

    let handle = client.open_session(spec(dir.path())).await.unwrap();

    client
        .send_turn(&handle.provider_session, "hello")
        .await
        .unwrap();
}

#[tokio::test]
async fn unavailable_mcp_server_is_reported_once_after_the_session_opens() {
    let dir = tempfile::tempdir().unwrap();
    let env = vec![
        ("FAKE_REQUIRE_MCP".into(), "1".into()),
        ("FAKE_MCP_FAILED".into(), "broken".into()),
    ];
    let mut launch = launch(dir.path(), env);
    launch.permission.mcp_servers = vec!["docs".to_owned(), "broken".to_owned()];
    let mut client = CodexClient::start(launch, Supervisor::new()).await.unwrap();

    client.open_session(spec(dir.path())).await.unwrap();
    client.open_session(spec(dir.path())).await.unwrap();

    let event = tokio::time::timeout(Duration::from_secs(5), client.next_event()).await;
    let text = format!("{event:?}");
    assert!(text.contains("McpUnavailable"), "{text}");
    assert!(text.contains("broken failed to start"), "{text}");
    assert!(!text.contains("docs"), "{text}");
    let more = tokio::time::timeout(Duration::from_millis(200), client.next_event()).await;
    assert!(
        more.is_err() || !format!("{more:?}").contains("McpUnavailable"),
        "reported twice: {more:?}"
    );
}

#[tokio::test]
async fn ready_mcp_servers_are_not_reported() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), vec![("FAKE_REQUIRE_MCP".into(), "1".into())]);
    launch.permission.mcp_servers = vec!["docs".to_owned()];
    let mut client = CodexClient::start(launch, Supervisor::new()).await.unwrap();

    client.open_session(spec(dir.path())).await.unwrap();

    let next = tokio::time::timeout(Duration::from_millis(200), client.next_event()).await;
    assert!(
        next.is_err() || !format!("{next:?}").contains("McpUnavailable"),
        "{next:?}"
    );
}

#[tokio::test]
async fn first_turn_is_sent_when_a_server_stays_unknown_past_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut launch = launch(dir.path(), vec![("FAKE_MCP_STUCK".into(), "1".into())]);
    launch.permission.mcp_servers = vec!["docs".to_owned()];
    let mut client = CodexClient::start(launch, Supervisor::new())
        .await
        .unwrap()
        .with_mcp_ready_timeout(Duration::from_millis(10));

    let handle = client.open_session(spec(dir.path())).await.unwrap();

    client
        .send_turn(&handle.provider_session, "hello")
        .await
        .unwrap();
}

#[tokio::test]
async fn codex_home_ignores_user_rules() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("user-codex");
    std::fs::create_dir_all(user.join("rules")).unwrap();
    std::fs::write(
        user.join("rules/default.rules"),
        "prefix_rule(pattern = [\"sort\"], decision = \"allow\")\n",
    )
    .unwrap();
    let prepared = prepare_codex_home(HomeInput {
        saturn_home: &dir.path().join("saturn"),
        user_codex_home: &user,
        rules: &[rule(PermissionTool::Shell, "sort *", Verdict::Ask)],
        questions: true,
    })
    .unwrap();
    let env = vec![("CODEX_HOME".into(), user.clone().into_os_string())];
    let mut dedicated = launch(dir.path(), env.clone());
    dedicated.permission.env = vec![("CODEX_HOME".into(), prepared.path.clone().into_os_string())];
    let mut with_user_home = CodexClient::start(launch(dir.path(), env), Supervisor::new())
        .await
        .unwrap();
    let mut with_saturn_home = CodexClient::start(dedicated, Supervisor::new())
        .await
        .unwrap();
    let mut sessions = Vec::new();
    for client in [&mut with_user_home, &mut with_saturn_home] {
        sessions.push(client.open_session(spec(dir.path())).await.unwrap());
    }

    with_user_home
        .send_turn(&sessions[0].provider_session, "run-sort")
        .await
        .unwrap();
    with_saturn_home
        .send_turn(&sessions[1].provider_session, "run-sort")
        .await
        .unwrap();
    let user_rules_ran = take(&mut with_user_home, 1).await.remove(0);
    let saturn_rules_asked = take(&mut with_saturn_home, 1).await.remove(0);

    assert!(matches!(
        user_rules_ran,
        ProviderEvent::Text { ref text, .. } if text == "ran-without-asking"
    ));
    assert!(matches!(
        saturn_rules_asked,
        ProviderEvent::PermissionRequested { .. }
    ));
}

fn text_of(events: &[ProviderEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn messages_of_one_turn_stay_apart_when_the_message_item_changes() {
    let mut threads = HashMap::new();
    threads.insert(
        ProviderSessionId("main".to_owned()),
        ThreadState::new(AgentId(1), None, AppliedSettings::default()),
    );
    let mut delta = |item: &str, text: &str| {
        convert_notification(
            &mut threads,
            "item/agentMessage/delta",
            &json!({ "threadId": "main", "itemId": item, "delta": text }),
        )
    };

    let mut events = delta("m1", "읽어 보겠습니다.");
    events.extend(delta("m1", " 잠시만요."));
    events.extend(delta("m2", "마지막 줄입니다."));

    assert_eq!(
        text_of(&events),
        "읽어 보겠습니다. 잠시만요.\n\n마지막 줄입니다."
    );
}

#[test]
fn a_new_turn_does_not_start_with_a_separator() {
    let mut threads = HashMap::new();
    threads.insert(
        ProviderSessionId("main".to_owned()),
        ThreadState::new(AgentId(1), None, AppliedSettings::default()),
    );
    let mut notify = |method: &str, params: serde_json::Value| {
        convert_notification(&mut threads, method, &params)
    };
    notify(
        "item/agentMessage/delta",
        json!({ "threadId": "main", "itemId": "m1", "delta": "첫 턴" }),
    );
    notify(
        "turn/completed",
        json!({ "threadId": "main", "turn": { "id": "t1" } }),
    );
    notify(
        "turn/started",
        json!({ "threadId": "main", "turn": { "id": "t2" } }),
    );

    let events = notify(
        "item/agentMessage/delta",
        json!({ "threadId": "main", "itemId": "m2", "delta": "둘째 턴" }),
    );

    assert_eq!(text_of(&events), "둘째 턴");
}

#[test]
fn a_delta_without_an_item_id_is_passed_through_unchanged() {
    let mut threads = HashMap::new();
    threads.insert(
        ProviderSessionId("main".to_owned()),
        ThreadState::new(AgentId(1), None, AppliedSettings::default()),
    );

    let events = convert_notification(
        &mut threads,
        "item/agentMessage/delta",
        &json!({ "threadId": "main", "delta": "조각" }),
    );

    assert_eq!(text_of(&events), "조각");
}

#[test]
fn file_change_paths_are_remembered_from_the_item_start() {
    let mut threads = HashMap::new();
    let main = ProviderSessionId("main".to_owned());
    threads.insert(
        main,
        ThreadState::new(AgentId(1), None, AppliedSettings::default()),
    );
    let started = json!({
        "threadId": "main",
        "item": { "type": "fileChange", "id": "item_f", "changes": [{ "path": "a.rs", "diff": "" }] },
    });

    convert_notification(&mut threads, "item/started", &started);

    let state = threads.values().next().unwrap();
    assert_eq!(state.file_changes["item_f"], vec!["a.rs"]);
}

#[test]
fn rejected_turn_tells_context_overflow_from_other_rejections() {
    let overflow = json!({
        "message": "This model's maximum context length is 128000 tokens",
        "data": { "codexErrorInfo": "ContextWindowExceeded" },
    });

    assert!(matches!(
        rejected_turn(&overflow),
        ProviderError::ContextExceeded {
            limit_tokens: Some(128_000)
        }
    ));
    assert!(matches!(
        rejected_turn(&json!({ "message": "context_length_exceeded" })),
        ProviderError::ContextExceeded { limit_tokens: None }
    ));
    assert!(matches!(
        rejected_turn(&json!({ "message": "invalid input" })),
        ProviderError::NotSent { .. }
    ));
}

#[tokio::test]
async fn a_silent_request_fails_alone_and_leaves_the_connection_usable() {
    let dir = tempfile::tempdir().unwrap();
    let mut client = CodexClient::start(launch(dir.path(), Vec::new()), Supervisor::new())
        .await
        .unwrap()
        .with_reply_timeouts(Duration::from_millis(300), Duration::from_secs(5));
    let main = client
        .open_session(spec(dir.path()))
        .await
        .unwrap()
        .provider_session;

    let silent = client.send_turn(&main, "stall").await;

    assert!(matches!(silent, Err(ProviderError::Unknown)), "{silent:?}");
    assert!(
        lock(&client.pending).is_empty(),
        "the abandoned request should be forgotten"
    );
    client.send_turn(&main, "quick").await.unwrap();
    assert!(!take(&mut client, 3).await.is_empty());
}

#[tokio::test]
async fn a_silent_interrupt_reports_the_lost_connection_instead_of_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let env = vec![("FAKE_INTERRUPT_SILENT".into(), "1".into())];
    let mut client = CodexClient::start(launch(dir.path(), env), Supervisor::new())
        .await
        .unwrap()
        .with_reply_timeouts(Duration::from_millis(300), Duration::from_secs(5));
    let main = client
        .open_session(spec(dir.path()))
        .await
        .unwrap()
        .provider_session;
    client.send_turn(&main, "fix the build").await.unwrap();

    let interrupted = client.interrupt(&main, InterruptTarget::Main).await;

    assert!(
        matches!(interrupted, Err(ProviderError::ConnectionLost)),
        "{interrupted:?}"
    );
}

#[tokio::test]
async fn a_slow_open_reply_is_waited_for_longer_than_a_quick_one() {
    let dir = tempfile::tempdir().unwrap();
    let env = vec![("FAKE_SLOW_OPEN_MS".into(), "1500".into())];
    let mut client = CodexClient::start(launch(dir.path(), env), Supervisor::new())
        .await
        .unwrap()
        .with_reply_timeouts(Duration::from_millis(300), Duration::from_secs(5));

    let opened = client.open_session(spec(dir.path())).await;

    assert!(opened.is_ok(), "{opened:?}");
}

fn thread_calls(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("thread/"))
        .map(str::to_owned)
        .collect()
}

// #66: 끊긴 자식 thread는 부모를 다시 열기 전에 보관하고 구독을 끊으며, 부모 thread는 건드리지 않는다
#[tokio::test]
async fn interrupted_children_are_cleaned_before_the_parent_is_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let env = vec![("FAKE_CALL_LOG".into(), log.clone().into_os_string())];
    let mut client = CodexClient::start(launch(dir.path(), env), Supervisor::new())
        .await
        .unwrap();
    let mut reopened = spec(dir.path());
    reopened.resume = Some(ProviderSessionId("thr_main".to_owned()));
    reopened.interrupted_children = vec![
        SubagentId("thr_child".to_owned()),
        SubagentId("thr_grandchild".to_owned()),
    ];

    client.open_session(reopened).await.unwrap();

    assert_eq!(
        thread_calls(&log),
        vec![
            "thread/archive thr_child",
            "thread/unsubscribe thr_child",
            "thread/archive thr_grandchild",
            "thread/unsubscribe thr_grandchild",
            "thread/resume thr_main",
        ]
    );
}

// #66: 끊긴 자식이 없으면 부모를 여는 요청만 보낸다
#[tokio::test]
async fn resume_without_interrupted_children_cleans_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("calls.log");
    let env = vec![("FAKE_CALL_LOG".into(), log.clone().into_os_string())];
    let mut client = CodexClient::start(launch(dir.path(), env), Supervisor::new())
        .await
        .unwrap();
    let mut reopened = spec(dir.path());
    reopened.resume = Some(ProviderSessionId("thr_main".to_owned()));

    client.open_session(reopened).await.unwrap();

    assert_eq!(thread_calls(&log), vec!["thread/resume thr_main"]);
}
