//! 요청 합계 알림 테스트: 채팅의 모든 일이 끝난 순간 그 요청의 합계를 한 번 알리고, 다른 요청과 다른 채팅의 사용량을 섞지 않는다.
//! 설계: docs/design/tui.md#채팅-알림

use saturn_protocol::event::{ProviderEvent, UsageReport, UsageScope};
use saturn_protocol::ids::{AgentId, Provider};
use saturn_protocol::rpc::{ChatNotice, Notification};

use super::support::{Flow, idle_reply, router_down, turn_completed};
use super::*;
use crate::providers::test_support::CLAUDE;

type Summary = (Vec<(Provider, u64)>, u32, Option<u64>);

fn usage(agent: AgentId, input: u64, output: u64) -> ProviderEvent {
    ProviderEvent::Usage(UsageReport {
        agent,
        subagent: None,
        model: Some("model".to_owned()),
        scope: UsageScope::MainTurn,
        input: Some(input),
        cache_read: Some(5_000),
        cache_write: None,
        output: Some(output),
        reasoning: None,
    })
}

fn summaries(seen: &[Notification]) -> Vec<Summary> {
    seen.iter()
        .filter_map(|notification| match notification {
            Notification::ChatNotice {
                notice:
                    ChatNotice::RequestSummary {
                        provider_tokens,
                        router_calls,
                        router_tokens,
                        ..
                    },
                ..
            } => Some((provider_tokens.clone(), *router_calls, *router_tokens)),
            _ => None,
        })
        .collect()
}

fn summary_of(notification: &Notification) -> Option<Summary> {
    summaries(std::slice::from_ref(notification)).pop()
}

#[tokio::test]
async fn a_request_ends_with_one_summary_that_a_late_tui_sees_the_same_as_the_stored_totals() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(usage(agent, 100, 20)).await;
    let mut late_tui = flow.client().await;
    let fake = flow.fake.clone();

    let (first, rest) = drive(&mut flow.engine, async {
        fake.emit(turn_completed(agent));
        let first = late_tui.until(summary_of).await;
        (first, summaries(&late_tui.window().await))
    })
    .await;

    let (_, stored_tokens) = flow
        .engine
        .store
        .router_usage_since(flow.chat, 0)
        .await
        .unwrap();
    // 새 입력 100과 출력 20만 센다. 캐시 읽기는 실행 줄의 Token과 같게 뺀다
    assert_eq!(first, (vec![(CLAUDE, 120)], 1, stored_tokens));
    assert!(stored_tokens.is_some());
    assert!(rest.is_empty(), "a second summary came: {rest:?}");
}

#[tokio::test]
async fn the_next_request_and_another_chat_do_not_add_into_a_summary() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95), idle_reply(0.95)]).await;
    let other = flow.open_other_chat().await;
    flow.claude_event(usage(other.agent, 1_000, 1_000)).await;
    flow.submit("fix the build").await;
    // 두 채팅이 열려 있어 `flow.agent()`는 어느 쪽이든 될 수 있다
    let agent = flow
        .engine
        .flow
        .live
        .values()
        .find(|live| {
            flow.engine
                .session_chat(live.session)
                .is_ok_and(|chat| chat == flow.chat)
        })
        .map(|live| live.agent)
        .expect("the main chat should have an open session");
    let mut client = flow.client().await;
    flow.claude_event(usage(agent, 100, 20)).await;

    flow.claude_event(turn_completed(agent)).await;
    flow.submit("then run the tests").await;
    flow.claude_event(usage(agent, 50, 10)).await;
    flow.claude_event(turn_completed(agent)).await;

    let seen: Vec<Summary> = summaries(&client.window().await)
        .into_iter()
        .map(|(providers, calls, _)| (providers, calls, None))
        .collect();
    assert_eq!(
        seen,
        vec![
            (vec![(CLAUDE, 120)], 1, None),
            (vec![(CLAUDE, 60)], 1, None)
        ]
    );
}

#[tokio::test]
async fn nothing_reported_is_left_out_instead_of_counted_as_zero() {
    let mut flow = Flow::new(router_down()).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.claude_event(turn_completed(agent)).await;

    let seen = summaries(&client.window().await);
    assert_eq!(seen, vec![(Vec::new(), 1, None)]);
}
