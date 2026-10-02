//! router `target_model` 테스트: 후보 전달, 고른 모델 적용, 고정 모델이면 묻지 않음, 후보 밖 값 무시.

use saturn_protocol::ids::{Provider, SessionId};
use saturn_protocol::rpc::{ModelChoice, ModelInfo};
use saturn_protocol::state::SessionState;

use super::support::{Flow, idle_reply, model_reply, turn_completed};
use crate::providers::test_support::{Call, FakeProvider};

/// 연결에서 받아 둔 모델 목록을 정한다.
fn know_models(flow: &mut Flow, provider: Provider, models: &[&str]) {
    let infos = models
        .iter()
        .map(|model| ModelInfo {
            choice: ModelChoice {
                provider,
                model: (*model).to_owned(),
            },
            name: (*model).to_owned(),
        })
        .collect();
    flow.engine.flow.models.insert((flow.chat, provider), infos);
}

fn opened_models(fake: &FakeProvider) -> Vec<Option<String>> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { model, .. } => Some(model),
            _ => None,
        })
        .collect()
}

/// router에 보낸 마지막 요청 본문.
fn last_request(flow: &Flow) -> String {
    flow.transport
        .calls()
        .last()
        .and_then(|call| call.2.clone())
        .expect("router should have been called")
}

#[tokio::test]
async fn target_model_candidates_are_the_model_list_in_provider_order() {
    let mut flow = Flow::new(vec![idle_reply(0.1)]).await;
    flow.add_provider(Provider::Codex);
    know_models(&mut flow, Provider::Codex, &["gpt-x"]);
    know_models(&mut flow, Provider::Claude, &["opus", "haiku"]);

    let input = flow.accept_only("hello").await;
    let request = flow.engine.router_request(&flow.record(input), false);

    let (_, questions) = &request.sets[0];
    let target = questions
        .iter()
        .find(|question| question.id == "target_model")
        .expect("target_model should be asked");
    let text = format!("{:?}", target.kind);
    let positions: Vec<usize> = ["claude/opus", "claude/haiku", "codex/gpt-x", "other"]
        .iter()
        .map(|option| text.find(option).expect("option should be listed"))
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
async fn target_model_is_not_asked_before_the_model_list_arrives() {
    let mut flow = Flow::new(vec![idle_reply(0.1)]).await;

    flow.submit("hello").await;

    assert!(!last_request(&flow).contains("target_model"));
    assert_eq!(opened_models(&flow.fake), vec![None]);
}

#[tokio::test]
async fn target_model_chosen_by_the_router_is_applied() {
    let options = ["claude/opus", "claude/haiku", "other"];
    let mut flow = Flow::new(vec![model_reply(0.1, &options, "claude/haiku")]).await;
    know_models(&mut flow, Provider::Claude, &["opus", "haiku"]);

    flow.submit("hello").await;

    assert!(last_request(&flow).contains("target_model"));
    assert_eq!(opened_models(&flow.fake), vec![Some("haiku".to_owned())]);
    let session = flow.engine.sessions.get(SessionId(1)).unwrap();
    assert_eq!(session.model.as_deref(), Some("haiku"));
}

#[tokio::test]
async fn target_model_picks_the_provider_of_the_chosen_model() {
    let options = ["claude/opus", "codex/gpt-x", "other"];
    let mut flow = Flow::new(vec![model_reply(0.1, &options, "codex/gpt-x")]).await;
    let codex = flow.add_provider(Provider::Codex);
    know_models(&mut flow, Provider::Claude, &["opus"]);
    know_models(&mut flow, Provider::Codex, &["gpt-x"]);

    flow.submit("hello").await;

    assert_eq!(opened_models(&codex), vec![Some("gpt-x".to_owned())]);
    assert!(opened_models(&flow.fake).is_empty());
}

#[tokio::test]
async fn target_model_on_a_second_new_task_opens_a_session_with_that_model() {
    let options = ["claude/opus", "claude/haiku", "other"];
    let mut flow = Flow::new(vec![
        model_reply(0.1, &options, "claude/opus"),
        model_reply(0.1, &options, "claude/haiku"),
    ])
    .await;
    know_models(&mut flow, Provider::Claude, &["opus", "haiku"]);
    flow.submit("one").await;
    let agent = flow.agent();
    flow.claude_event(turn_completed(agent)).await;

    flow.submit("two").await;

    assert_eq!(
        opened_models(&flow.fake),
        vec![Some("opus".to_owned()), Some("haiku".to_owned())]
    );
    // 두 번째 새 작업은 보조 에이전트라 첫 session은 그대로 열려 있다
    let old = flow.engine.sessions.get(SessionId(1)).unwrap();
    let new = flow.engine.sessions.get(SessionId(2)).unwrap();
    assert_eq!(old.state, SessionState::Open);
    assert_eq!(new.state, SessionState::Open);
    assert_eq!(new.model.as_deref(), Some("haiku"));
}

#[tokio::test]
async fn target_model_is_not_asked_when_the_model_is_pinned() {
    let mut flow = Flow::new(vec![idle_reply(0.1)]).await;
    know_models(&mut flow, Provider::Claude, &["opus", "haiku"]);
    let pinned = ModelChoice {
        provider: Provider::Claude,
        model: "opus".to_owned(),
    };

    flow.submit_with("hello", Some(pinned), false).await;

    assert!(!last_request(&flow).contains("target_model"));
    assert_eq!(opened_models(&flow.fake), vec![Some("opus".to_owned())]);
}

#[tokio::test]
async fn target_model_other_keeps_the_default_model() {
    let options = ["claude/opus", "other"];
    let mut flow = Flow::new(vec![model_reply(0.1, &options, "other")]).await;
    know_models(&mut flow, Provider::Claude, &["opus"]);

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![None]);
}

#[tokio::test]
async fn target_model_outside_the_candidates_is_ignored() {
    let options = ["claude/opus", "claude/sonnet", "other"];
    let mut flow = Flow::new(vec![model_reply(0.1, &options, "claude/sonnet")]).await;
    know_models(&mut flow, Provider::Claude, &["opus"]);

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![None]);
}

#[tokio::test]
async fn target_model_is_ignored_when_the_input_continues_current_work() {
    let options = ["claude/opus", "claude/haiku", "other"];
    let mut flow = Flow::new(vec![model_reply(0.95, &options, "claude/haiku")]).await;
    know_models(&mut flow, Provider::Claude, &["opus", "haiku"]);

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![None]);
}
