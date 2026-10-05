//! 완료 검사 근거 테스트: 끝난 작업에 근거를 붙여 알리고, 다시 붙어도 같은 줄이 보이며, 입력을 다시 보내지 않는다.
//! 설계: docs/design/providers-and-sessions.md#완료-검사-근거

use std::path::Path;

use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory, ToolDetail};
use saturn_protocol::ids::AgentId;
use saturn_protocol::rpc::{ChatNotice, Notification};
use saturn_protocol::state::{CompletionEvidence, EvidenceState, UnverifiedReason};

use super::support::{Flow, idle_reply, tool_result, turn_completed};
use crate::providers::test_support::Call;

fn command_call(agent: AgentId, id: &str, command: &str) -> ProviderEvent {
    ProviderEvent::ToolCall {
        agent,
        subagent: None,
        call_id: id.to_owned(),
        activity: Activity::RunningCommand {
            command: command.to_owned(),
        },
        detail: ToolDetail::default(),
    }
}

fn edit_call(agent: AgentId, id: &str, path: &Path) -> ProviderEvent {
    ProviderEvent::ToolCall {
        agent,
        subagent: None,
        call_id: id.to_owned(),
        activity: Activity::EditingFile,
        detail: ToolDetail {
            category: ToolCategory::FileEdit,
            paths: vec![path.to_string_lossy().into_owned()],
            ..ToolDetail::default()
        },
    }
}

fn exit(agent: AgentId, id: &str, code: i32) -> ProviderEvent {
    match tool_result(agent, id, "") {
        ProviderEvent::ToolResult {
            agent,
            subagent,
            call_id,
            output,
            ..
        } => ProviderEvent::ToolResult {
            agent,
            subagent,
            call_id,
            output,
            exit_code: Some(code),
        },
        other => other,
    }
}

fn evidence_of(notifications: &[Notification]) -> Vec<CompletionEvidence> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::CompletionEvidence { evidence },
                ..
            } => Some(evidence.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_check_after_the_last_edit_is_shown_with_its_event_and_survives_reattaching() {
    let mut flow = Flow::with_config(
        "[completion]\nchecks = [\"cargo test\"]\n",
        vec![idle_reply(0.95)],
    )
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    let file = flow.fixture.workdir.canonicalize().unwrap().join("lib.rs");

    std::fs::write(&file, "fn main() {}").unwrap();
    flow.claude_event(edit_call(agent, "edit-1", &file)).await;
    flow.claude_event(tool_result(agent, "edit-1", "")).await;
    flow.claude_event(command_call(agent, "test-1", "cargo test --workspace"))
        .await;
    flow.claude_event(exit(agent, "test-1", 0)).await;
    flow.claude_event(turn_completed(agent)).await;

    let live = client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::CompletionEvidence { evidence },
                task: Some(_),
                ..
            } => Some(evidence.clone()),
            _ => None,
        })
        .await;
    assert_eq!(live.state, EvidenceState::Verified);
    assert_eq!(live.reason, None);
    assert_eq!(live.events.len(), 1);

    let (_second, greeting) = flow.attach().await;
    let replayed: Vec<CompletionEvidence> = greeting
        .iter()
        .flat_map(|notification| match notification {
            Notification::HistoryChunk { entries, .. } => evidence_of(entries),
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(replayed, vec![live]);
    let sent: Vec<String> = flow
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SendTurn { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec!["fix the build"]);
}

#[tokio::test]
async fn an_edit_without_a_configured_check_is_unverified_and_a_clean_run_is_not_applicable() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("look around").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(turn_completed(agent)).await;
    let clean = client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::CompletionEvidence { evidence },
                ..
            } => Some(evidence.clone()),
            _ => None,
        })
        .await;
    assert_eq!(clean.state, EvidenceState::NotApplicable);

    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    let file = flow.fixture.workdir.canonicalize().unwrap().join("lib.rs");
    std::fs::write(&file, "fn main() {}").unwrap();
    flow.claude_event(edit_call(agent, "edit-1", &file)).await;
    flow.claude_event(turn_completed(agent)).await;
    let edited = client
        .until(|notification| match notification {
            Notification::ChatNotice {
                notice: ChatNotice::CompletionEvidence { evidence },
                ..
            } => Some(evidence.clone()),
            _ => None,
        })
        .await;
    assert_eq!(edited.state, EvidenceState::Unverified);
    assert_eq!(edited.reason, Some(UnverifiedReason::NotChecked));
}
