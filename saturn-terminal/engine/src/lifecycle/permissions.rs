//! 허가 요청 판정 테스트: Saturn 규칙이 허용과 거부는 바로 답하고 묻기만 TUI로 올리며, 항상 허용을 저장한다.

use saturn_core::permission::{Mode, Rule, Verdict};
use saturn_core::providers::ProviderError;
use saturn_protocol::event::{PermissionTool, ProviderEvent};
use saturn_protocol::ids::{AgentId, Provider};
use saturn_protocol::rpc::{ChatNotice, Notification, PermissionAnswer};

use super::support::{Flow, idle_reply, permission, permission_for, turn_completed};
use super::*;
use crate::chat_env::ChatEnv;
use crate::providers::test_support::Call;
use crate::rpc::ClientId;

const ALLOW_CARGO_TEST: &str = "[permission.shell]\n\"cargo test\" = \"allow\"\n";

async fn started(config: &str) -> (Flow, AgentId) {
    let mut flow = Flow::with_config(config, vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    (flow, agent)
}

fn shell(agent: AgentId, request_id: &str, command: &str) -> ProviderEvent {
    permission_for(agent, request_id, PermissionTool::Shell, command, &[])
}

fn answers(flow: &Flow) -> Vec<(String, PermissionAnswer)> {
    flow.fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::AnswerPermission {
                request_id, answer, ..
            } => Some((request_id, answer)),
            _ => None,
        })
        .collect()
}

fn is_asked(notifications: &[Notification]) -> bool {
    notifications
        .iter()
        .any(|notification| matches!(notification, Notification::PermissionRequested { .. }))
}

#[tokio::test]
async fn rule_allow_answers_the_provider_without_asking_the_user() {
    let (mut flow, agent) = started(ALLOW_CARGO_TEST).await;
    let mut client = flow.client().await;

    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::AllowOnce)]
    );
    assert!(flow.engine.flow.permissions.is_empty());
    assert!(!is_asked(&client.window().await));
}

#[tokio::test]
async fn rule_deny_answers_the_provider_without_asking_the_user() {
    let (mut flow, agent) = started("[permission.shell]\n\"rm *\" = \"deny\"\n").await;
    let mut client = flow.client().await;

    flow.claude_event(shell(agent, "r1", "rm -rf build")).await;

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::Deny { note: None })]
    );
    assert!(!is_asked(&client.window().await));
}

#[tokio::test]
async fn rule_ask_goes_to_the_tui_and_waits_for_the_answer() {
    let (mut flow, agent) = started("").await;
    let mut client = flow.client().await;

    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    assert!(answers(&flow).is_empty());
    assert!(is_asked(&client.window().await));
    assert!(flow.engine.flow.permissions.contains_key("r1"));
}

#[tokio::test]
async fn request_without_a_readable_call_goes_to_the_tui_even_in_full_mode() {
    let (mut flow, agent) = started("permission.mode = \"full\"\n").await;
    let mut client = flow.client().await;

    flow.claude_event(permission(agent, "r1")).await;

    assert!(answers(&flow).is_empty());
    assert!(is_asked(&client.window().await));
}

#[tokio::test]
async fn edit_inside_the_workdir_is_allowed_and_outside_is_asked_in_edit_mode() {
    let (mut flow, agent) = started("").await;
    let inside = flow.fixture.workdir.join("src/a.rs");
    let event = |id: &str, path: &str| permission_for(agent, id, PermissionTool::Edit, "", &[path]);

    flow.claude_event(event("inside", &inside.display().to_string()))
        .await;
    flow.claude_event(event("outside", "/etc/hosts")).await;

    assert_eq!(
        answers(&flow),
        vec![("inside".to_owned(), PermissionAnswer::AllowOnce)]
    );
    assert!(flow.engine.flow.permissions.contains_key("outside"));
}

#[tokio::test]
async fn edit_inside_an_added_folder_is_allowed_like_the_workdir_in_edit_mode() {
    let (mut flow, agent) = started("").await;
    let added = flow.fixture.root.path().join("shared-lib");
    std::fs::create_dir_all(&added).unwrap();
    flow.engine
        .chat_dirs
        .entry(flow.chat)
        .or_default()
        .push(added.clone());
    let event = |id: &str, path: &str| permission_for(agent, id, PermissionTool::Edit, "", &[path]);

    flow.claude_event(event(
        "added",
        &added.join("src/a.rs").display().to_string(),
    ))
    .await;
    flow.claude_event(event("sibling", &format!("{}-other/a.rs", added.display())))
        .await;

    assert_eq!(
        answers(&flow),
        vec![("added".to_owned(), PermissionAnswer::AllowOnce)]
    );
    assert!(flow.engine.flow.permissions.contains_key("sibling"));
}

#[tokio::test]
async fn mode_default_rules_apply_when_no_rule_matches() {
    let (mut flow, agent) = started("permission.mode = \"read-only\"\n").await;

    flow.claude_event(shell(agent, "r1", "touch a.txt")).await;

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::Deny { note: None })]
    );
}

#[tokio::test]
async fn run_layer_rules_follow_the_user_rules() {
    let overrides = ["permission.shell={\"cargo test\"=\"ask\"}"];
    let mut flow = Flow::with_setup(ALLOW_CARGO_TEST, &overrides, vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    assert!(answers(&flow).is_empty());
    assert!(flow.engine.flow.permissions.contains_key("r1"));
}

#[tokio::test]
async fn always_allow_is_stored_in_records() {
    let (mut flow, agent) = started("").await;
    let provider_config = flow.fixture.workdir.join(".claude/settings.json");
    std::fs::create_dir_all(provider_config.parent().unwrap()).unwrap();
    std::fs::write(&provider_config, "{\"model\":\"x\"}").unwrap();
    let before = std::fs::read(&provider_config).unwrap();
    flow.claude_event(shell(agent, "r1", "cargo test && make"))
        .await;

    flow.engine
        .answer_permission(ClientId(99), "r1".to_owned(), PermissionAnswer::AllowAlways)
        .await
        .unwrap();

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::AllowOnce)]
    );
    let stored = flow
        .engine
        .store
        .permission_allows(&flow.fixture.workdir)
        .await
        .unwrap();
    let patterns: Vec<&str> = stored.iter().map(|rule| rule.pattern.as_str()).collect();
    assert_eq!(patterns, vec!["cargo test", "make"]);
    assert_eq!(std::fs::read(&provider_config).unwrap(), before);
    flow.claude_event(shell(agent, "r2", "make")).await;
    assert_eq!(
        answers(&flow).last(),
        Some(&("r2".to_owned(), PermissionAnswer::AllowOnce))
    );
    assert!(flow.engine.flow.permissions.is_empty());
}

#[tokio::test]
async fn always_allow_is_kept_per_workdir() {
    let (mut flow, agent) = started("").await;
    flow.claude_event(shell(agent, "r1", "cargo test")).await;
    flow.engine
        .answer_permission(ClientId(99), "r1".to_owned(), PermissionAnswer::AllowAlways)
        .await
        .unwrap();

    let other = flow
        .engine
        .store
        .permission_allows(std::path::Path::new("/somewhere/else"))
        .await
        .unwrap();

    assert!(other.is_empty());
}

#[tokio::test]
async fn deny_rule_beats_a_stored_always_allow() {
    let (mut flow, agent) = started("[permission.shell]\n\"cargo test\" = \"deny\"\n").await;
    flow.engine
        .store
        .add_permission_allow(
            &flow.fixture.workdir,
            &Rule {
                tool: PermissionTool::Shell,
                pattern: "cargo test".to_owned(),
                verdict: Verdict::Allow,
            },
        )
        .await
        .unwrap();

    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::Deny { note: None })]
    );
}

#[tokio::test]
async fn always_allow_for_an_unreadable_request_goes_to_the_provider_as_given() {
    let (mut flow, agent) = started("").await;
    flow.claude_event(permission(agent, "r1")).await;

    flow.engine
        .answer_permission(ClientId(99), "r1".to_owned(), PermissionAnswer::AllowAlways)
        .await
        .unwrap();

    assert_eq!(
        answers(&flow),
        vec![("r1".to_owned(), PermissionAnswer::AllowAlways)]
    );
    assert!(
        flow.engine
            .store
            .permission_allows(&flow.fixture.workdir)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn mode_change_applies_next_request() {
    let (mut flow, agent) = started(ALLOW_CARGO_TEST).await;
    flow.claude_event(shell(agent, "r1", "touch a")).await;
    let opened = flow
        .fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();

    flow.engine
        .set_permission_mode(flow.chat, "read-only")
        .await
        .unwrap();
    flow.claude_event(shell(agent, "r2", "touch b")).await;
    flow.engine
        .set_permission_mode(flow.chat, "full")
        .await
        .unwrap();
    flow.claude_event(shell(agent, "r3", "touch c")).await;

    assert!(flow.engine.flow.permissions.contains_key("r1"));
    assert_eq!(
        answers(&flow),
        vec![
            ("r2".to_owned(), PermissionAnswer::Deny { note: None }),
            ("r3".to_owned(), PermissionAnswer::AllowOnce),
        ]
    );
    let reopened = flow
        .fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .count();
    assert_eq!(opened, reopened);
}

#[tokio::test]
async fn mode_change_keeps_other_chat_layer_values_and_rejects_unknown_modes() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.engine
        .store
        .set_chat_layer(flow.chat, "# note\nrouter.thresholds.injection = 0.75\n")
        .await
        .unwrap();

    flow.engine
        .set_permission_mode(flow.chat, "ask")
        .await
        .unwrap();
    let rejected = flow
        .engine
        .set_permission_mode(flow.chat, "plan")
        .await
        .unwrap_err();

    let layer = flow
        .engine
        .store
        .chat_layer(flow.chat)
        .await
        .unwrap()
        .unwrap();
    assert!(layer.contains("# note"));
    assert!(layer.contains("router.thresholds.injection = 0.75"));
    assert_eq!(
        crate::settings::chat_layer_mode(Some(&layer)),
        Some(Mode::Ask)
    );
    assert!(matches!(
        rejected,
        EngineError::UnknownPermissionMode { .. }
    ));
}

#[tokio::test]
async fn rule_answer_that_the_provider_does_not_take_falls_back_to_the_user() {
    let (mut flow, agent) = started(ALLOW_CARGO_TEST).await;
    flow.fake
        .answer_permission_with([Err(ProviderError::NotSent {
            reason: "write failed".to_owned(),
        })]);

    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    assert!(flow.engine.flow.permissions.contains_key("r1"));
}

#[tokio::test]
async fn turn_end_does_not_leave_rule_answered_requests_behind() {
    let (mut flow, agent) = started(ALLOW_CARGO_TEST).await;
    flow.claude_event(shell(agent, "r1", "cargo test")).await;

    flow.claude_event(turn_completed(agent)).await;

    assert!(flow.engine.flow.permissions.is_empty());
}

#[tokio::test]
async fn codex_launch_builds_a_dedicated_home_from_the_rules() {
    let mut flow = Flow::with_config(
        "[permission.shell]\n\"sort *\" = \"ask\"\n",
        vec![idle_reply(0.95)],
    )
    .await;
    let user_codex = flow.fixture.root.path().join("user-codex");
    std::fs::create_dir_all(&user_codex).unwrap();
    std::fs::write(
        user_codex.join("config.toml"),
        "approval_policy = \"never\"\n",
    )
    .unwrap();
    let env = vec![("CODEX_HOME".to_owned(), user_codex.display().to_string())];
    flow.engine
        .chats
        .insert(flow.chat, ChatEnv::new(flow.fixture.workdir.clone(), env));
    let revision = flow.engine.settings.current().unwrap();

    let spec = flow
        .engine
        .launch_spec(Provider::Codex, flow.chat, revision)
        .await
        .unwrap();

    let home = spec.permission.codex_home.unwrap();
    assert!(home.starts_with(flow.fixture.options.home.join("codex-home")));
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("approvals_reviewer = \"user\""));
    assert!(!config.contains("approval_policy"));
    let rules = std::fs::read_to_string(home.join("rules/default.rules")).unwrap();
    assert!(rules.contains("decision = \"prompt\""));
    assert_eq!(
        std::fs::read_to_string(user_codex.join("config.toml")).unwrap(),
        "approval_policy = \"never\"\n"
    );
}

async fn permission_of_first_input(config: &str) -> saturn_core::queue::Permission {
    let mut flow = Flow::with_config(config, vec![idle_reply(0.95)]).await;
    let input = flow.submit("look around").await;
    flow.record(input).permission
}

#[tokio::test]
async fn read_only_mode_accepts_inputs_as_read_only() {
    use saturn_core::queue::Permission;

    assert_eq!(
        permission_of_first_input("[permission]\nmode = \"read-only\"\n").await,
        Permission::ReadOnly
    );
    assert_eq!(permission_of_first_input("").await, Permission::Write);
    assert_eq!(
        permission_of_first_input("[permission]\nmode = \"ask\"\n").await,
        Permission::Write
    );
}

#[tokio::test]
async fn read_only_mode_with_an_allow_rule_keeps_inputs_as_write() {
    let config = "[permission]\nmode = \"read-only\"\n[permission.edit]\n\"src/*\" = \"allow\"\n";

    assert_eq!(
        permission_of_first_input(config).await,
        saturn_core::queue::Permission::Write
    );
}

#[tokio::test]
async fn chat_layer_mode_decides_the_input_permission() {
    let mut flow = Flow::with_config("", vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.engine
        .set_permission_mode(flow.chat, "read-only")
        .await
        .unwrap();

    let input = flow.submit("look around").await;

    assert_eq!(
        flow.record(input).permission,
        saturn_core::queue::Permission::ReadOnly
    );
}

#[tokio::test]
async fn changed_codex_rules_mark_the_connection_stale_until_they_match_again() {
    let mut flow = Flow::new(Vec::new()).await;
    let mut client = flow.client().await;
    let revision = flow.engine.settings.current().unwrap();
    let current = crate::providers::rules_fingerprint(
        &flow
            .engine
            .settings
            .at(&flow.engine.store, revision)
            .await
            .unwrap()
            .permission()
            .rules,
    );

    flow.engine.note_rules_revision(flow.chat, revision).await;
    assert!(flow.engine.flow.rules_stale.is_empty());

    flow.engine
        .flow
        .rules_of_connection
        .insert(flow.chat, "older-rules".to_owned());
    flow.engine.note_rules_revision(flow.chat, revision).await;
    assert!(flow.engine.flow.rules_stale.contains(&flow.chat));
    client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::PermissionsChanged,
                ..
            } => Some(()),
            _ => None,
        })
        .await;

    flow.engine
        .flow
        .rules_of_connection
        .insert(flow.chat, current);
    flow.engine.note_rules_revision(flow.chat, revision).await;
    assert!(flow.engine.flow.rules_stale.is_empty());
}

#[tokio::test]
async fn stale_codex_connection_restarts_after_the_turn_ends_and_reopens_the_session() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.add_provider(Provider::Codex);
    let mut client = flow.client().await;
    flow.engine.switch_provider(flow.chat, Provider::Codex);
    flow.submit("write the cache module").await;
    let agent = flow.agent();
    flow.engine
        .flow
        .rules_of_connection
        .insert(flow.chat, "older-rules".to_owned());
    flow.engine.flow.rules_stale.insert(flow.chat);

    flow.event(Provider::Codex, turn_completed(agent)).await;

    assert!(
        !flow
            .engine
            .providers
            .contains_key(&(flow.chat, Provider::Codex))
    );
    assert!(flow.engine.flow.live.is_empty());
    assert!(flow.engine.flow.rules_stale.is_empty());
    assert!(flow.engine.flow.rules_of_connection.is_empty());
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::ProviderRestarted { provider },
                ..
            } => Some(*provider),
            _ => None,
        })
        .await;
    assert_eq!(notice, Provider::Codex);

    let reconnected = flow.add_provider(Provider::Codex);
    flow.submit("add the tests").await;

    let resumed: Vec<_> = reconnected
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { resume, .. } => Some(resume),
            _ => None,
        })
        .collect();
    assert_eq!(
        resumed,
        vec![Some(saturn_protocol::ids::ProviderSessionId(
            "fake-session-1".to_owned()
        ))]
    );
}

#[tokio::test]
async fn stale_codex_connection_waits_while_the_chat_is_running() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.add_provider(Provider::Codex);
    flow.engine.switch_provider(flow.chat, Provider::Codex);
    flow.submit("write the cache module").await;
    flow.engine.flow.rules_stale.insert(flow.chat);

    flow.engine.restart_stale_codex(flow.chat).await;

    assert!(
        flow.engine
            .providers
            .contains_key(&(flow.chat, Provider::Codex))
    );
    assert!(flow.engine.flow.rules_stale.contains(&flow.chat));
}
