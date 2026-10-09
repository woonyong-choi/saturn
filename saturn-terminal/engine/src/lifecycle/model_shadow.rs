//! 모델 판단 그림자 테스트(#527): 켜고 꺼도 실제 모델과 입력이 같고, 그림자 답의 이상과 어긋난 판단이 선택을 바꾸지 않는다.
//! 설계: docs/design/router.md#모델-판단-그림자

use serde_json::{Value, json};

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::{ModelChoice, ModelInfo};
use saturn_protocol::state::InputState;

use super::support::{Flow, router_down};
use crate::providers::test_support::{CLAUDE, Call, FakeProvider};
use crate::routers::test_support::{HttpReply, KEY, TransportError, ok};

type FakeReply = Result<HttpReply, TransportError>;

const AUTO: &str = "[model]\nmode = \"auto\"\n";
const ON: &str = "[model]\nmode = \"auto\"\n[router.shadow]\nmodel_selection = true\n";
const OPTIONS: [&str; 3] = ["claude/opus", "claude/haiku", "other"];

pub(super) fn know_models(flow: &mut Flow, provider: Provider, models: &[&str]) {
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

pub(super) fn opened_models(fake: &FakeProvider) -> Vec<Option<String>> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { model, .. } => Some(model),
            _ => None,
        })
        .collect()
}

/// `target_model`에 `picked`를 답하고 그림자 답은 `shadow`를 준다. `shadow`의 값이 `None`이면 그 답을 빼고, 범위 밖 값이면 틀린 답이다.
fn reply(picked: &str, shadow: &[(&str, Option<f64>)], with_constraint: bool) -> FakeReply {
    let probabilities: serde_json::Map<String, Value> = OPTIONS
        .iter()
        .map(|option| {
            (
                (*option).to_owned(),
                json!(f64::from(u8::from(*option == picked))),
            )
        })
        .collect();
    let mut answers = json!({
        "keep_current": { "type": "noul", "noul": 0.1 },
        "is_actionable": { "type": "noul", "noul": 0.9 },
        "target_model": { "type": "choice", "choice": picked, "probabilities": probabilities, "confidence": 1.0 },
    });
    if with_constraint {
        answers["is_constraint"] = json!({ "type": "noul", "noul": 0.1 });
    }
    for (candidate, yes) in shadow {
        if let Some(yes) = yes {
            answers[format!("sufficient:{candidate}")] = json!({ "type": "noul", "noul": yes });
        }
    }
    ok(&json!({
        "model": "jev-1.13.0",
        "answers": answers,
        "usage": { "input_tokens": 10, "output_tokens": 2 },
    })
    .to_string())
}

/// router에 보낸 마지막 요청의 질문 id와 본문.
fn last_request(flow: &Flow) -> (Vec<String>, String) {
    let body = flow
        .transport
        .calls()
        .last()
        .and_then(|call| call.2.clone())
        .expect("router should have been called");
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let ids = parsed["questions"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    (ids, body)
}

/// 판단 기록의 내보낸 줄.
pub(super) async fn exported(flow: &Flow) -> Vec<Value> {
    let path = flow.fixture.root.path().join("judgments.jsonl");
    flow.engine.store.export_judgments(&path).await.unwrap();
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn without_shadow(ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter(|id| !id.starts_with("sufficient:"))
        .cloned()
        .collect()
}

async fn run(config: &str, replies: Vec<FakeReply>) -> (Flow, Option<String>) {
    let mut flow = Flow::with_config(config, replies).await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);
    flow.submit("fix the build").await;
    let opened = opened_models(&flow.fake).pop().flatten();
    (flow, opened)
}

// 켜고 꺼도 실제 모델과 실제 질문이 같고 원문은 한 번만 가며, 그림자가 다르게 답해도 기록만 하고, 켜면 후보·정책·확률을 내보낸다
#[tokio::test]
async fn shadow_on_and_off_apply_the_same_model_and_the_same_real_questions() {
    // 그림자는 opus가 낫다고 하지만 실제 선택은 haiku다
    let shadow = [("claude/opus", Some(0.9)), ("claude/haiku", Some(0.2))];
    let (off, off_model) = run(AUTO, vec![reply("claude/haiku", &[], true)]).await;
    let (on, on_model) = run(ON, vec![reply("claude/haiku", &shadow, true)]).await;

    assert_eq!(off_model.as_deref(), Some("haiku"));
    assert_eq!(on_model, off_model);
    let (off_ids, off_body) = last_request(&off);
    let (on_ids, on_body) = last_request(&on);
    assert!(!off_body.contains("sufficient:"));
    assert_eq!(without_shadow(&on_ids), off_ids);
    assert_eq!(on_ids.len(), off_ids.len() + 2);
    assert_eq!(
        on_body.matches("fix the build").count(),
        off_body.matches("fix the build").count()
    );

    assert!(exported(&off).await[0]["model_shadow"].is_null());
    let lines = exported(&on).await;
    let record = &lines[0]["model_shadow"];
    assert_eq!(record["status"], "answered");
    assert_eq!(record["applied_model"], "claude/haiku");
    assert_eq!(record["question_set"], "model-shadow@1.0");
    assert_eq!(record["catalog"], on.engine.catalog.version.as_str());
    assert_eq!(
        record["policy"].as_str().unwrap(),
        on.engine
            .policy_digest_at(on.engine.settings.current().unwrap())
            .await
            .unwrap()
    );
    assert_eq!(record["candidates_hash"].as_str().unwrap().len(), 64);
    assert_eq!(record["candidates"][0]["model"], "claude/opus");
    assert_eq!(record["candidates"][0]["probability"], 0.9);
    assert_eq!(record["candidates"][1]["probability"], 0.2);
    assert!(
        record["candidates"][0]["quality"]
            .as_str()
            .unwrap()
            .starts_with("unverified:")
    );
    assert!(record["request_bytes"].as_u64().unwrap() > 0);
    assert!(
        lines[0]["question_sets"]
            .to_string()
            .contains("model-shadow@1.0")
    );
    assert!(!serde_json::to_string(&lines).unwrap().contains(KEY));
}

// 그림자 답이 빠지거나 틀리거나 router가 실패해도 실제 선택은 그림자를 끈 것과 같다
#[tokio::test]
async fn missing_wrong_or_failed_shadow_answers_leave_the_real_choice_alone() {
    let missing = [("claude/opus", None), ("claude/haiku", Some(0.2))];
    let wrong = [("claude/opus", Some(7.0)), ("claude/haiku", Some(0.2))];
    for answers in [missing, wrong] {
        let (flow, model) = run(ON, vec![reply("claude/haiku", &answers, true)]).await;
        assert_eq!(model.as_deref(), Some("haiku"));
        let lines = exported(&flow).await;
        assert_eq!(lines[0]["outcome"], "Ok");
        assert_eq!(lines[0]["model_shadow"]["status"], "invalid");
        assert!(lines[0]["model_shadow"]["candidates"][0]["probability"].is_null());
    }

    let (off, off_model) = run(AUTO, router_down()).await;
    let (on, on_model) = run(ON, router_down()).await;
    assert_eq!(on_model, off_model);
    assert_eq!(opened_models(&on.fake), opened_models(&off.fake));
    assert_eq!(
        exported(&on).await[0]["model_shadow"]["status"],
        "no-answer"
    );
}

// 판단 뒤 채팅 revision이 바뀌어 어긋난 판단은 그림자도 미적용으로 남기고, 다시 판단한 입력이 실제 선택을 정한다
#[tokio::test]
async fn a_superseded_judgment_leaves_its_shadow_unapplied() {
    let shadow = [("claude/opus", Some(0.9)), ("claude/haiku", Some(0.2))];
    let mut flow = Flow::with_config(
        ON,
        vec![
            reply("claude/haiku", &shadow, true),
            reply("claude/opus", &shadow, false),
        ],
    )
    .await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);
    let first = flow.accept_only("first").await;
    let second = flow.accept_only("second").await;
    let stale = flow.router_now(first, false).await;
    let revision = flow.engine.queue.revision(flow.chat);
    let mut bump = stale.clone();
    bump.revision = revision;
    flow.engine
        .apply_decision(second, bump, false)
        .await
        .unwrap();

    flow.engine
        .apply_decision(first, stale, false)
        .await
        .unwrap();
    flow.settle().await;

    assert_eq!(flow.state(first), InputState::Applied);
    let statuses: Vec<(String, Value)> = exported(&flow)
        .await
        .iter()
        .map(|line| {
            (
                line["outcome"].as_str().unwrap().to_owned(),
                line["model_shadow"]["applied_model"].clone(),
            )
        })
        .collect();
    assert_eq!(statuses[0].0, "Superseded");
    assert!(statuses[0].1.is_null());
    assert_eq!(statuses[1], ("Ok".to_owned(), json!("claude/opus")));
}
