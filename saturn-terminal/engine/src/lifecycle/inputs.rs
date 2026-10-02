//! provider 입력 요청 테스트: 요청이 TUI와 provider 사이를 오가고, 답이 없는 동안 작업이 기다린다.

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::Provider;
use saturn_protocol::input::{InputAnswer, InputValue};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::TaskState;

use super::support::{Flow, idle_reply, input_request, turn_completed};
use crate::EngineError;
use crate::providers::test_support::Call;
use crate::rpc::ClientId;

fn typed(text: &str) -> InputAnswer {
    InputAnswer::Submit {
        values: vec![("q".to_owned(), InputValue::Text(text.to_owned()))],
    }
}

#[tokio::test]
async fn elicitation_request_reaches_the_tui_and_the_answer_reaches_the_provider() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(input_request(agent, "ask-1")).await;

    let (request_id, provider, title) = client
        .until(|notification| match notification {
            Notification::InputRequested {
                request_id,
                provider,
                request,
                ..
            } => Some((
                request_id.clone(),
                *provider,
                request.fields[0].title.clone(),
            )),
            _ => None,
        })
        .await;
    let waiting = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!(
        (request_id.as_str(), provider, title.as_str()),
        ("ask-1", Provider::Claude, "Which?")
    );
    assert_eq!(waiting, TaskState::AwaitingInput);

    flow.engine
        .answer_input(ClientId(99), "ask-1".to_owned(), typed("ok"))
        .await
        .unwrap();

    assert!(flow.fake.calls().iter().any(|call| matches!(
        call,
        Call::AnswerInput { request_id, answer, .. } if request_id == "ask-1" && *answer == typed("ok")
    )));
    let resumed = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!(resumed, TaskState::Running);
    assert!(flow.engine.flow.inputs.is_empty());
}

#[tokio::test]
async fn elicitation_decline_and_cancel_reach_the_provider_as_given() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(input_request(agent, "ask-1")).await;
    flow.claude_event(input_request(agent, "ask-2")).await;

    flow.engine
        .answer_input(ClientId(99), "ask-1".to_owned(), InputAnswer::Decline)
        .await
        .unwrap();
    flow.engine
        .answer_input(ClientId(99), "ask-2".to_owned(), InputAnswer::Cancel)
        .await
        .unwrap();

    let calls = flow.fake.calls();
    assert!(calls.iter().any(|call| matches!(
        call,
        Call::AnswerInput { request_id, answer: InputAnswer::Decline, .. } if request_id == "ask-1"
    )));
    assert!(calls.iter().any(|call| matches!(
        call,
        Call::AnswerInput { request_id, answer: InputAnswer::Cancel, .. } if request_id == "ask-2"
    )));
}

#[tokio::test]
async fn elicitation_answer_for_a_request_nobody_asked_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;

    let error = flow
        .engine
        .answer_input(ClientId(99), "nope".to_owned(), InputAnswer::Cancel)
        .await
        .unwrap_err();

    assert!(matches!(error, EngineError::UnexpectedAnswer { .. }));
}

#[tokio::test]
async fn elicitation_answer_the_provider_did_not_take_keeps_the_request() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(input_request(agent, "ask-1")).await;
    flow.fake.answer_input_with([Err(ProviderError::NotSent {
        reason: "write failed".to_owned(),
    })]);

    let first = flow
        .engine
        .answer_input(ClientId(99), "ask-1".to_owned(), typed("a"))
        .await;
    let second = flow
        .engine
        .answer_input(ClientId(99), "ask-1".to_owned(), typed("a"))
        .await;

    assert!(matches!(first, Err(EngineError::Provider(_))));
    assert!(second.is_ok());
    assert!(flow.engine.flow.inputs.is_empty());
}

#[tokio::test]
async fn elicitation_turn_end_withdraws_requests_nobody_answered() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(input_request(agent, "ask-1")).await;

    flow.claude_event(turn_completed(agent)).await;

    let resolved = client
        .until(|notification| match notification {
            Notification::InputResolved { request_id } => Some(request_id.clone()),
            _ => None,
        })
        .await;
    assert_eq!(resolved, "ask-1");
    assert!(flow.engine.flow.inputs.is_empty());
}

#[tokio::test]
async fn elicitation_request_waits_for_a_tui_that_attaches_later() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(input_request(agent, "ask-1")).await;

    let (_client, greeting) = flow.attach().await;

    assert!(greeting.iter().any(|notification| matches!(
        notification,
        Notification::InputRequested { request_id, .. } if request_id == "ask-1"
    )));
}
