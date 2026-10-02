//! provider 이벤트 테스트: 기록 뒤에만 화면과 상태에 반영하고, 허가 요청이 TUI와 provider 사이를 오간다.

use saturn_core::providers::ProviderError;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, LedgerSeq, Provider};
use saturn_protocol::rpc::{Notification, PermissionAnswer};
use saturn_protocol::state::{EffectScope, TaskState};

use super::support::{
    Flow, context_size, idle_reply, permission, text, tool_read, tool_result, turn_completed,
};
use super::*;
use crate::providers::test_support::Call;
use crate::rpc::ClientId;

#[tokio::test]
async fn event_is_recorded_before_it_reaches_the_screen_and_the_state() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.engine.store.deny_writes().await;

    let refused = flow
        .engine
        .on_provider_event(Provider::Claude, text(agent, "working"))
        .await;

    assert!(matches!(refused, Err(EngineError::Store(_))));
    assert!(client.window().await.is_empty());
    assert_eq!(flow.engine.agents.status(agent), None);
    flow.engine.store.allow_writes().await;
    flow.claude_event(text(agent, "working")).await;
    let seen = client
        .until(|notification| match notification {
            Notification::TaskEvent { event, .. } => Some(event.clone()),
            _ => None,
        })
        .await;
    assert_eq!(seen, text(agent, "working"));
    let recorded = flow
        .engine
        .store
        .events_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].1, text(agent, "working"));
}

#[tokio::test]
async fn events_get_one_number_each_in_arrival_order() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();

    flow.claude_event(text(agent, "one")).await;
    flow.claude_event(tool_read(agent, "c1", "src/lib.rs"))
        .await;
    flow.claude_event(tool_result(agent, "c1", "body")).await;

    let recorded = flow
        .engine
        .store
        .events_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    let numbers: Vec<u64> = recorded.iter().map(|(seq, _)| seq.0).collect();
    assert_eq!(numbers, vec![1, 2, 3]);
}

#[tokio::test]
async fn event_from_the_provider_pump_reaches_the_engine_loop() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    let (fake, chat) = (flow.fake.clone(), flow.chat);

    let seen = drive(&mut flow.engine, async {
        fake.emit(text(agent, "from the pump"));
        client
            .until(|notification| match notification {
                Notification::TaskEvent { event, .. } => Some(event.clone()),
                _ => None,
            })
            .await
    })
    .await;

    assert_eq!(seen, text(agent, "from the pump"));
    let recorded = flow
        .engine
        .store
        .events_since(chat, LedgerSeq(0))
        .await
        .unwrap();
    assert_eq!(recorded.len(), 1);
}

#[tokio::test]
async fn event_of_an_agent_without_a_session_is_dropped() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;

    flow.claude_event(text(AgentId(999), "stray")).await;

    let recorded = flow
        .engine
        .store
        .events_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    assert!(recorded.is_empty());
}

#[tokio::test]
async fn usage_is_recorded_as_a_usage_row_and_not_as_a_ledger_event() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let report = saturn_protocol::event::UsageReport {
        agent,
        subagent: None,
        model: Some("model".to_owned()),
        scope: saturn_protocol::event::UsageScope::MainTurn,
        input: Some(10),
        cache_read: None,
        cache_write: None,
        output: Some(5),
        reasoning: None,
    };

    flow.claude_event(ProviderEvent::Usage(report)).await;

    let recorded = flow
        .engine
        .store
        .events_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    assert!(recorded.is_empty());
    let rows = flow
        .engine
        .store
        .usage_rows(saturn_protocol::rpc::UsageRange::Chat, Some(flow.chat))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn output_after_the_turn_ended_starts_a_run_without_input() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;
    assert!(
        flow.engine
            .store
            .unfinished_runs()
            .await
            .unwrap()
            .is_empty()
    );

    flow.claude_event(text(agent, "woke up")).await;

    let runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].input, None);
    assert!(flow.engine.chat_is_running(flow.chat));
}

#[tokio::test]
async fn lost_stream_records_unobserved_and_needs_a_check() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.claude_event(ProviderEvent::StreamLost { agent }).await;

    let state = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!(state, TaskState::NeedsCheck);
    let runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].effect_scope, EffectScope::Unobserved);
    assert!(!flow.engine.flow.live.contains_key(&agent));
}

#[tokio::test]
async fn context_size_goes_to_the_screen_with_the_threshold() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.claude_event(context_size(agent, 1_234)).await;

    let (tokens, threshold) = client
        .until(|notification| match notification {
            Notification::ContextSize {
                tokens, threshold, ..
            } => Some((*tokens, *threshold)),
            _ => None,
        })
        .await;
    assert_eq!(tokens, Some(1_234));
    assert!(threshold > 1_234);
}

#[tokio::test]
async fn permission_request_reaches_the_tui_and_the_answer_reaches_the_provider() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(permission(agent, "req-1")).await;

    let (request_id, provider) = client
        .until(|notification| match notification {
            Notification::PermissionRequested {
                request_id,
                provider,
                ..
            } => Some((request_id.clone(), *provider)),
            _ => None,
        })
        .await;
    let waiting = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!((request_id.as_str(), provider), ("req-1", Provider::Claude));
    assert_eq!(waiting, TaskState::AwaitingPermission);

    flow.engine
        .answer_permission(
            ClientId(99),
            "req-1".to_owned(),
            PermissionAnswer::AllowOnce,
        )
        .await
        .unwrap();

    assert!(flow.fake.calls().iter().any(|call| matches!(
        call,
        Call::AnswerPermission { request_id, answer: PermissionAnswer::AllowOnce, .. } if request_id == "req-1"
    )));
    let resumed = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!(resumed, TaskState::Running);
    assert!(flow.engine.flow.permissions.is_empty());
}

#[tokio::test]
async fn answer_for_a_request_nobody_asked_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;

    let error = flow
        .engine
        .answer_permission(ClientId(99), "nope".to_owned(), PermissionAnswer::AllowOnce)
        .await
        .unwrap_err();

    assert!(matches!(error, EngineError::UnexpectedAnswer { .. }));
}

#[tokio::test]
async fn answer_the_provider_did_not_take_keeps_the_request_for_another_try() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(permission(agent, "req-1")).await;
    flow.fake
        .answer_permission_with([Err(ProviderError::NotSent {
            reason: "write failed".to_owned(),
        })]);
    let deny = PermissionAnswer::Deny { note: None };

    let first = flow
        .engine
        .answer_permission(ClientId(99), "req-1".to_owned(), deny.clone())
        .await;
    let second = flow
        .engine
        .answer_permission(ClientId(99), "req-1".to_owned(), deny)
        .await;

    assert!(matches!(first, Err(EngineError::Provider(_))));
    assert!(second.is_ok());
    assert!(flow.engine.flow.permissions.is_empty());
}

#[tokio::test]
async fn turn_end_withdraws_requests_nobody_answered() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(permission(agent, "req-1")).await;

    flow.claude_event(turn_completed(agent)).await;

    let resolved = client
        .until(|notification| match notification {
            Notification::PermissionResolved { request_id } => Some(request_id.clone()),
            _ => None,
        })
        .await;
    assert_eq!(resolved, "req-1");
    assert!(flow.engine.flow.permissions.is_empty());
}
