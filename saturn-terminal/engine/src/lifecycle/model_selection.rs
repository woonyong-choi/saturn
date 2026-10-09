//! 새 작업의 모델 정하기 테스트(#531): 명시 고정, 매뉴얼, 후보 없음, 무효 답, 낮은 확신도, 장애, 지원하지 않는 선택에서 고정된
//! 대체가 동작하고, 정한 규칙과 이유가 판단 기록과 실제 실행에 이어진다.
//! 설계: docs/design/providers-and-sessions.md#기본-모델과-선택-방식

use serde_json::Value;

use saturn_protocol::rpc::ModelChoice;

use super::model_shadow::{exported, know_models, opened_models};
use super::support::{Flow, idle_reply, model_reply, router_down};
use crate::flow::{JobKind, RouterJob};
use crate::providers::test_support::CLAUDE;
use crate::routers::test_support::{HttpReply, TransportError};

type FakeReply = Result<HttpReply, TransportError>;

const OPTIONS: [&str; 3] = ["claude/opus", "claude/haiku", "other"];
const AUTO: &str = "[model]\ndefault = \"claude/opus\"\nmode = \"auto\"\n";
const MANUAL: &str = "[model]\ndefault = \"claude/opus\"\nmode = \"manual\"\n";

fn pin(model: &str) -> Option<ModelChoice> {
    Some(ModelChoice {
        provider: CLAUDE,
        model: model.to_owned(),
    })
}

async fn run(
    config: &str,
    replies: Vec<FakeReply>,
    models: bool,
    pinned: Option<ModelChoice>,
) -> (Flow, Option<String>, Value) {
    let mut flow = Flow::with_config(config, replies).await;
    if models {
        know_models(&mut flow, CLAUDE, &["opus", "haiku"]);
    }
    flow.submit_with("hello", pinned, false).await;
    let opened = opened_models(&flow.fake).pop().flatten();
    let line = exported(&flow).await.remove(0);
    (flow, opened, line)
}

// 판단 기록에 남은 정한 규칙과 이유
fn rule(line: &Value) -> (String, Option<String>) {
    let selection = &line["model_selection"];
    (
        selection["source"].as_str().unwrap().to_owned(),
        selection["reason"].as_str().map(str::to_owned),
    )
}

// 명시 고정, 매뉴얼, 후보 없음, 무효 답, router가 고르지 않음, 장애에서 고정된 대체가 동작하고 규칙과 이유가 남으며,
// 기본 설정에서는 router가 모델을 고르지 않는다
#[tokio::test]
async fn model_selection_falls_back_in_the_fixed_order_and_records_the_rule() {
    let pick = |model: &str| model_reply(0.1, &OPTIONS, model);

    let (flow, opened, line) = run(AUTO, vec![pick("claude/haiku")], true, None).await;
    assert_eq!(opened.as_deref(), Some("haiku"));
    assert_eq!(rule(&line), ("router".to_owned(), None));
    // 실제 실행과 입력, 원문 해시가 판단 기록에 이어진다
    let applied = &line["applied"];
    assert_eq!(applied["input_model"], "claude/haiku");
    assert_eq!(applied["runs"][0]["session_model"], "haiku");
    assert_eq!(applied["input_sha256"].as_str().unwrap().len(), 64);
    drop(flow);

    let (_, opened, line) = run(AUTO, vec![idle_reply(0.1)], true, pin("opus")).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(rule(&line).0, "pinned");

    let (_, opened, line) = run(MANUAL, vec![idle_reply(0.1)], true, None).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("manual".to_owned()))
    );

    let (_, opened, line) = run(AUTO, vec![idle_reply(0.1)], false, None).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("no-candidates".to_owned()))
    );

    // 후보가 있는데 target_model 답이 없으면 무효 답이다
    let (_, opened, line) = run(AUTO, vec![idle_reply(0.1)], true, None).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("invalid".to_owned()))
    );

    let (_, opened, line) = run(AUTO, vec![pick("other")], true, None).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("fallback".to_owned()))
    );

    let (_, opened, line) = run(AUTO, router_down(), true, None).await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("router-failed".to_owned()))
    );

    // 기본 설정은 매뉴얼이라 router에 모델을 묻지 않는다
    let (flow, opened, line) = run(
        "[model]\ndefault = \"claude/opus\"\n",
        vec![idle_reply(0.1)],
        true,
        None,
    )
    .await;
    assert_eq!(opened.as_deref(), Some("opus"));
    assert_eq!(rule(&line).1.as_deref(), Some("manual"));
    let body = flow
        .transport
        .calls()
        .last()
        .and_then(|call| call.2.clone())
        .unwrap();
    assert!(!body.contains("target_model"));
}

// router가 고른 뒤 모델 목록에서 사라진 모델은 지원하지 않는 선택으로 기록하고 기본 모델로 간다
#[tokio::test]
async fn a_choice_the_provider_no_longer_lists_is_recorded_as_unsupported() {
    let mut flow = Flow::with_config(AUTO, vec![model_reply(0.1, &OPTIONS, "claude/haiku")]).await;
    know_models(&mut flow, CLAUDE, &["opus", "haiku"]);
    let input = flow.accept_only("hello").await;
    let record = flow.record(input);
    let revision = flow.engine.queue.revision(flow.chat);
    let plan = flow.engine.model_plan(record.settings).await.unwrap();
    let request = flow.engine.router_request(&record, false, &plan, true);
    let exchange = flow.engine.routers.shared().exchange(request.clone()).await;
    know_models(&mut flow, CLAUDE, &["opus"]);
    let job = RouterJob {
        chat: flow.chat,
        input,
        revision,
        retried: false,
        kind: JobKind::Route,
    };

    let verdict = flow
        .engine
        .finish_router(&job, &request, exchange)
        .await
        .unwrap();
    flow.engine.settle_record(input, false).await;

    assert_eq!(verdict.decision.model.as_deref(), Some("claude/opus"));
    let line = exported(&flow).await.remove(0);
    assert_eq!(
        rule(&line),
        ("default".to_owned(), Some("unsupported".to_owned()))
    );
}

// 선호는 품질을 확정한 후보 안에서만 우선한다. 확정하지 못한 선호는 건너뛴 이유를 남기고 기본 모델로 간다
#[tokio::test]
async fn a_preference_without_confirmed_quality_never_overrides_the_default() {
    let config = format!("{AUTO}prefer = [\"claude/haiku\", \"codex/gpt-x\"]\n");
    let (_, opened, line) = run(
        &config,
        vec![model_reply(0.1, &OPTIONS, "other")],
        true,
        None,
    )
    .await;

    assert_eq!(opened.as_deref(), Some("opus"));
    let selection = &line["model_selection"];
    assert_eq!(selection["source"], "default");
    let skipped: Vec<(String, String)> = selection["skipped_preferences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["model"].as_str().unwrap().to_owned(),
                item["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        skipped,
        [
            ("claude/haiku".to_owned(), "unverified".to_owned()),
            ("codex/gpt-x".to_owned(), "not-available".to_owned())
        ]
    );
}
