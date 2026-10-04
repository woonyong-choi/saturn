//! provider 입력 요청 테스트: 요청이 TUI와 provider 사이를 오가고, 답이 없는 동안 작업이 기다린다.

use saturn_core::providers::ProviderError;
use saturn_protocol::input::{InputAnswer, InputValue};
use saturn_protocol::rpc::Notification;
use saturn_protocol::state::TaskState;

use super::support::{Flow, OTHER_CLIENT, idle_reply, input_request, turn_completed};
use crate::EngineError;
use crate::providers::test_support::Call;

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
    assert_eq!(Some(request_id), flow.input_id("ask-1"));
    assert_eq!(
        (provider, title.as_str()),
        (crate::providers::test_support::CLAUDE, "Which?")
    );
    assert_eq!(waiting, TaskState::AwaitingInput);

    flow.answer_input("ask-1", typed("ok")).await.unwrap();

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

    flow.answer_input("ask-1", InputAnswer::Decline)
        .await
        .unwrap();
    flow.answer_input("ask-2", InputAnswer::Cancel)
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
        .answer_input("nope", InputAnswer::Cancel)
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

    let first = flow.answer_input("ask-1", typed("a")).await;
    let second = flow.answer_input("ask-1", typed("a")).await;

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
    let engine_request = flow.input_id("ask-1");

    flow.claude_event(turn_completed(agent)).await;

    let resolved = client
        .until(|notification| match notification {
            Notification::InputResolved { request_id } => Some(request_id.clone()),
            _ => None,
        })
        .await;
    assert_eq!(Some(resolved), engine_request);
    assert!(flow.engine.flow.inputs.is_empty());
}

#[tokio::test]
async fn elicitation_request_waits_for_a_tui_that_attaches_later() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(input_request(agent, "ask-1")).await;

    let (_client, greeting) = flow.attach().await;

    let engine_request = flow.input_id("ask-1").unwrap();
    assert!(greeting.iter().any(|notification| matches!(
        notification,
        Notification::InputRequested { request_id, .. } if *request_id == engine_request
    )));
}

// #295: 허가 요청과 같이 입력 요청도 채팅마다 따로 보관하고 답을 그 채팅 연결로만 보낸다
#[tokio::test]
async fn same_provider_input_id_in_two_chats_keeps_both_and_answers_stay_apart() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let first_agent = flow.agent();
    let other = flow.open_other_chat().await;
    flow.claude_event(input_request(first_agent, "1")).await;
    flow.claude_event(input_request(other.agent, "1")).await;
    let mut ids: Vec<(String, saturn_protocol::ids::ChatId)> = flow
        .engine
        .flow
        .inputs
        .iter()
        .map(|(id, pending)| (id.clone(), pending.chat))
        .collect();
    ids.sort_by_key(|(_, chat)| *chat == other.chat);
    let (first_id, other_id) = (ids[0].0.clone(), ids[1].0.clone());
    assert_eq!(ids.len(), 2);
    assert_ne!(first_id, other_id);

    flow.answer_input_as(OTHER_CLIENT, other_id, typed("other"))
        .await
        .unwrap();

    assert!(
        flow.fake
            .calls()
            .iter()
            .all(|call| !matches!(call, Call::AnswerInput { .. }))
    );
    assert!(other.fake.calls().iter().any(|call| matches!(
        call,
        Call::AnswerInput { request_id, answer, .. } if request_id == "1" && *answer == typed("other")
    )));
    assert!(flow.engine.flow.inputs.contains_key(&first_id));
}

#[tokio::test]
async fn input_answer_from_a_tui_attached_to_another_chat_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let first_agent = flow.agent();
    let other = flow.open_other_chat().await;
    flow.claude_event(input_request(first_agent, "1")).await;
    let id = flow.input_id("1").unwrap();

    let error = flow
        .answer_input_as(OTHER_CLIENT, id.clone(), typed("x"))
        .await
        .unwrap_err();

    assert!(matches!(error, EngineError::ChatNotAttached { .. }));
    assert!(
        flow.fake
            .calls()
            .iter()
            .all(|call| !matches!(call, Call::AnswerInput { .. }))
    );
    assert!(
        other
            .fake
            .calls()
            .iter()
            .all(|call| !matches!(call, Call::AnswerInput { .. }))
    );
    assert!(flow.engine.flow.inputs.contains_key(&id));
}
