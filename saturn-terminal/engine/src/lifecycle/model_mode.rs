//! 기본 모델과 모델 선택 방식(오토, 매뉴얼) 테스트.
//! 설계: docs/design/providers-and-sessions.md#기본-모델과-선택-방식

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ModelChoice, ModelInfo, ModelMode, Notification, Request};

use super::drive;
use super::support::{CLIENT, Flow, idle_reply, model_reply};
use crate::providers::test_support::{CLAUDE, CODEX, Call, FakeProvider};
use crate::rpc::ClientId;

const DEFAULT_OPUS: &str = "[model]\ndefault = \"claude/opus\"\n";
const MANUAL_OPUS: &str = "[model]\ndefault = \"claude/opus\"\nmode = \"manual\"\n";

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

fn last_request(flow: &Flow) -> String {
    flow.transport
        .calls()
        .last()
        .and_then(|call| call.2.clone())
        .expect("router should have been called")
}

fn choice(provider: Provider, model: &str) -> ModelChoice {
    ModelChoice {
        provider,
        model: model.to_owned(),
    }
}

#[tokio::test]
async fn default_model_receives_a_new_task_the_router_did_not_place() {
    let mut flow = Flow::with_config(DEFAULT_OPUS, vec![idle_reply(0.1)]).await;

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![Some("opus".to_owned())]);
}

#[tokio::test]
async fn no_default_model_keeps_the_provider_default() {
    let mut flow = Flow::new(vec![idle_reply(0.1)]).await;

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![None]);
}

#[tokio::test]
async fn auto_mode_router_choice_beats_the_default_model() {
    let options = ["claude/opus", "claude/haiku", "other"];
    let mut flow = Flow::with_config(
        DEFAULT_OPUS,
        vec![model_reply(0.1, &options, "claude/haiku")],
    )
    .await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);

    flow.submit("hello").await;

    assert!(last_request(&flow).contains("target_model"));
    assert_eq!(opened_models(&flow.fake), vec![Some("haiku".to_owned())]);
}

#[tokio::test]
async fn auto_mode_falls_back_to_the_default_model_on_other() {
    let options = ["claude/opus", "claude/haiku", "other"];
    let mut flow = Flow::with_config(DEFAULT_OPUS, vec![model_reply(0.1, &options, "other")]).await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![Some("opus".to_owned())]);
}

#[tokio::test]
async fn manual_mode_does_not_ask_target_model_and_uses_the_default_model() {
    let mut flow = Flow::with_config(MANUAL_OPUS, vec![idle_reply(0.1)]).await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);

    flow.submit("hello").await;

    assert!(!last_request(&flow).contains("target_model"));
    assert_eq!(opened_models(&flow.fake), vec![Some("opus".to_owned())]);
}

#[tokio::test]
async fn manual_mode_still_judges_the_relation_between_inputs() {
    let mut flow = Flow::with_config(MANUAL_OPUS, vec![idle_reply(0.1)]).await;

    flow.submit("hello").await;

    assert!(last_request(&flow).contains("keep_current"));
}

#[tokio::test]
async fn manual_mode_sends_to_the_pinned_model_instead_of_the_default() {
    let mut flow = Flow::with_config(MANUAL_OPUS, vec![idle_reply(0.1)]).await;

    flow.submit_with("hello", Some(choice(CLAUDE, "haiku")), false)
        .await;

    assert_eq!(opened_models(&flow.fake), vec![Some("haiku".to_owned())]);
}

#[tokio::test]
async fn default_model_decides_the_provider() {
    let mut flow = Flow::with_config(
        "[model]\ndefault = \"codex/gpt-x\"\n",
        vec![idle_reply(0.1)],
    )
    .await;
    let codex = flow.add_provider(CODEX);

    flow.submit("hello").await;

    assert_eq!(opened_models(&codex), vec![Some("gpt-x".to_owned())]);
    assert!(opened_models(&flow.fake).is_empty());
}

#[tokio::test]
async fn default_model_of_an_unregistered_provider_is_ignored() {
    let mut flow = Flow::with_config(
        "[model]\ndefault = \"nobody/model-x\"\n",
        vec![idle_reply(0.1)],
    )
    .await;

    flow.submit("hello").await;

    assert_eq!(opened_models(&flow.fake), vec![None]);
}

#[tokio::test]
async fn attaching_tells_the_tui_that_no_default_model_is_chosen_yet() {
    let mut flow = Flow::new(Vec::new()).await;
    let chat = flow.chat;

    let (_client, greeting) = flow.attach().await;

    assert!(greeting.contains(&Notification::ModelSettings {
        chat,
        default: None,
        mode: ModelMode::Auto,
    }));
}

#[tokio::test]
async fn attaching_tells_the_tui_the_configured_default_and_mode() {
    let mut flow = Flow::with_config(MANUAL_OPUS, Vec::new()).await;
    let chat = flow.chat;

    let (_client, greeting) = flow.attach().await;

    assert!(greeting.contains(&Notification::ModelSettings {
        chat,
        default: Some(choice(CLAUDE, "opus")),
        mode: ModelMode::Manual,
    }));
}

#[tokio::test]
async fn choosing_the_default_model_writes_the_user_config_and_tells_the_tui() {
    let mut flow = Flow::new(vec![idle_reply(0.1)]).await;
    let (mut client, _) = flow.attach().await;
    let chat = flow.chat;
    let model = choice(CLAUDE, "sonnet");

    let told = drive(&mut flow.engine, async {
        client
            .send(
                2,
                Request::SetDefaultModel {
                    chat,
                    model: model.clone(),
                },
            )
            .await;
        client
            .until(|notification| match notification {
                Notification::ModelSettings { default, mode, .. } => Some((default.clone(), *mode)),
                _ => None,
            })
            .await
    })
    .await;

    assert_eq!(told, (Some(model), ModelMode::Auto));
    let config = std::fs::read_to_string(flow.fixture.options.home.join("config.toml")).unwrap();
    assert!(config.contains("default = \"claude/sonnet\""));
    flow.submit("hello").await;
    assert_eq!(opened_models(&flow.fake), vec![Some("sonnet".to_owned())]);
}

#[tokio::test]
async fn changing_the_mode_writes_the_user_config_and_applies_to_the_next_input() {
    let mut flow = Flow::with_config(DEFAULT_OPUS, vec![idle_reply(0.1)]).await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);
    let (mut client, _) = flow.attach().await;
    let chat = flow.chat;

    let told = drive(&mut flow.engine, async {
        client
            .send(
                2,
                Request::SetModelMode {
                    chat,
                    mode: ModelMode::Manual,
                },
            )
            .await;
        client
            .until(|notification| match notification {
                Notification::ModelSettings { mode, .. } => Some(*mode),
                _ => None,
            })
            .await
    })
    .await;

    assert_eq!(told, ModelMode::Manual);
    let config = std::fs::read_to_string(flow.fixture.options.home.join("config.toml")).unwrap();
    assert!(config.contains("mode = \"manual\""));
    assert!(config.contains("default = \"claude/opus\""));
    flow.submit("hello").await;
    assert!(!last_request(&flow).contains("target_model"));
}

#[tokio::test]
async fn setting_the_default_from_a_client_that_is_not_attached_is_refused() {
    let mut flow = Flow::new(Vec::new()).await;
    let chat = flow.chat;

    let result = flow
        .engine
        .set_default_model(ClientId(999), chat, &choice(CLAUDE, "opus"))
        .await;

    assert!(result.is_err());
}

// #490
#[tokio::test]
async fn connection_layer_model_mode_reaches_the_tui_of_that_connection() {
    let mut flow = Flow::new(Vec::new()).await;
    let (mut client, _) = flow.attach().await;
    let chat = flow.chat;
    let attached = *flow
        .engine
        .attachments
        .keys()
        .find(|id| **id != CLIENT)
        .expect("the TUI should be attached");
    flow.engine
        .attachments
        .get_mut(&attached)
        .unwrap()
        .overrides = vec![("model.mode".into(), "manual".into())];
    let run = flow.engine.run_layer_of(attached);
    flow.engine
        .settings
        .apply_trusted(&flow.engine.store, Some(chat), &flow.fixture.workdir, &run)
        .await
        .unwrap();

    flow.engine
        .send_model_settings(attached, chat)
        .await
        .unwrap();

    let told = client
        .until(|notification| match notification {
            Notification::ModelSettings { mode, .. } => Some(*mode),
            _ => None,
        })
        .await;
    assert_eq!(told, ModelMode::Manual);
}
