//! 크래시 뒤 하위 에이전트 처리 테스트(#66): 실행 중으로 남은 하위 에이전트는 끊김으로 기록하고, provider가 다시 하지 못하게
//! 정리를 넘기며, 끊긴 하위 에이전트가 다시 오면 막고 알린다. 다시 할지는 사용자에게 제안만 한다.

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{LedgerSeq, SubagentId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::EffectScope;

use super::crash_recovery::Restarted;
use super::support::{Flow, idle_reply, subagent_ended, subagent_started};
use super::*;
use crate::providers::test_support::Call;

fn sub(id: &str) -> SubagentId {
    SubagentId(id.to_owned())
}

/// `sub-1`은 끝나지 않았고 `sub-2`는 끝난 채로 크래시한 채팅을 다시 시작한 engine.
async fn crashed_with_a_running_subagent() -> Restarted {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(subagent_started(agent, "sub-1", None))
        .await;
    flow.claude_event(subagent_started(agent, "sub-2", None))
        .await;
    flow.claude_event(subagent_ended(agent, "sub-2")).await;
    Restarted::after_crash(flow, EffectScope::NetworkPossible).await
}

async fn ledger(restarted: &Restarted) -> Vec<ProviderEvent> {
    restarted
        .engine
        .store
        .events_since(restarted.chat, LedgerSeq(0))
        .await
        .unwrap()
        .into_iter()
        .map(|(_, event)| event)
        .collect()
}

fn interrupted_ids(events: &[ProviderEvent]) -> Vec<SubagentId> {
    events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::SubagentInterrupted { subagent, .. } => Some(subagent.clone()),
            _ => None,
        })
        .collect()
}

fn returned_notices(seen: &[Notification]) -> usize {
    seen.iter()
        .filter(|notification| {
            matches!(
                notification,
                Notification::ChatNotice {
                    notice: ChatNotice::InterruptedSubagentReturned { .. },
                    ..
                }
            )
        })
        .count()
}

// #66: 끝 신호가 없는 하위 에이전트만 끊김으로 기록하고, 자동으로 다시 하지 않는다
#[tokio::test]
async fn running_subagent_left_by_a_crash_is_recorded_as_interrupted() {
    let mut restarted = crashed_with_a_running_subagent().await;

    let events = ledger(&restarted).await;
    let seen = restarted.attach().await;

    assert_eq!(interrupted_ids(&events), vec![sub("sub-1")]);
    assert!(restarted.fake.calls().is_empty());
    let history = seen.iter().find_map(|notification| match notification {
        Notification::HistoryChunk { entries, .. } => Some(entries),
        _ => None,
    });
    assert!(history.is_some_and(|entries| {
        entries.iter().any(|entry| matches!(
        entry,
        Notification::TaskEvent { event: ProviderEvent::SubagentInterrupted { subagent, .. }, .. }
            if subagent == &sub("sub-1")
    ))
    }));
}

// #66: provider가 부모 session을 다시 열 때 끊긴 자식을 정리하게 한 번만 넘긴다
#[tokio::test]
async fn reopening_after_a_crash_hands_interrupted_children_to_the_provider_once() {
    let mut restarted = crashed_with_a_running_subagent().await;
    let chat = restarted.chat;

    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;

    let opened: Vec<Vec<SubagentId>> = restarted
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open {
                interrupted_children,
                ..
            } => Some(interrupted_children),
            _ => None,
        })
        .collect();
    assert_eq!(opened, vec![vec![sub("sub-1")]]);
    assert!(restarted.engine.flow.interrupted_to_clean.is_empty());
}

// #66: 다시 연 뒤 끊긴 하위 에이전트의 이벤트가 오면 기록하지 않고 작업을 멈추며, 한 번만 알린다
#[tokio::test]
async fn interrupted_subagent_coming_back_stops_the_task_and_is_reported_once() {
    let mut restarted = crashed_with_a_running_subagent().await;
    let chat = restarted.chat;
    let (mut client, _) = restarted.attach_client().await;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    let agent = *restarted.engine.flow.live.keys().next().unwrap();
    let before = ledger(&restarted).await.len();

    for _ in 0..2 {
        restarted
            .engine
            .on_provider_event(
                crate::providers::CLAUDE,
                subagent_started(agent, "sub-1", None),
            )
            .await
            .unwrap();
    }

    let seen = client.window().await;
    assert_eq!(returned_notices(&seen), 1);
    assert!(
        restarted
            .fake
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Interrupt { .. }))
    );
    assert_eq!(ledger(&restarted).await.len(), before);
}

// #66: 끊긴 목록에 없는 하위 에이전트는 평소처럼 처리한다
#[tokio::test]
async fn other_subagents_after_a_crash_are_handled_as_usual() {
    let mut restarted = crashed_with_a_running_subagent().await;
    let chat = restarted.chat;
    let (mut client, _) = restarted.attach_client().await;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    let agent = *restarted.engine.flow.live.keys().next().unwrap();

    restarted
        .engine
        .on_provider_event(
            crate::providers::CLAUDE,
            subagent_started(agent, "sub-3", None),
        )
        .await
        .unwrap();

    let seen = client.window().await;
    assert_eq!(returned_notices(&seen), 0);
    assert!(ledger(&restarted).await.iter().any(|event| matches!(
        event,
        ProviderEvent::SubagentStarted { subagent, .. } if *subagent == sub("sub-3")
    )));
}

// #368: engine이 다시 떠도 끊긴 하위 에이전트 정리 목록이 남아 보류 session을 다시 열 때 provider에 한 번 넘어간다
#[tokio::test]
async fn interrupted_children_are_handed_over_after_the_engine_restarts() {
    let mut restarted = crashed_with_a_running_subagent().await.restart().await;
    let chat = restarted.chat;

    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;

    let opened: Vec<Vec<SubagentId>> = restarted
        .fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open {
                interrupted_children,
                ..
            } => Some(interrupted_children),
            _ => None,
        })
        .collect();
    assert_eq!(opened, vec![vec![sub("sub-1")]]);
}

// #368: 한 번 정리해 넘긴 뒤 engine이 다시 떠도 같은 목록을 다시 넘기지 않지만 감시는 이어진다
#[tokio::test]
async fn cleaned_children_are_not_handed_over_again_but_still_blocked_after_a_restart() {
    let mut restarted = crashed_with_a_running_subagent().await;
    let chat = restarted.chat;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    let mut restarted = restarted.restart().await;
    let (mut client, _) = restarted.attach_client().await;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    let agent = *restarted.engine.flow.live.keys().next().unwrap();

    restarted
        .engine
        .on_provider_event(
            crate::providers::CLAUDE,
            subagent_started(agent, "sub-1", None),
        )
        .await
        .unwrap();

    assert!(restarted.fake.calls().iter().all(|call| !matches!(
        call,
        Call::Open { interrupted_children, .. } if !interrupted_children.is_empty()
    )));
    assert_eq!(returned_notices(&client.window().await), 1);
}

// #368: 끊긴 하위 에이전트가 다시 뜬 engine에서 다시 오면 기록하지 않고 막고 알린다
#[tokio::test]
async fn interrupted_subagent_is_still_blocked_after_the_engine_restarts() {
    let mut restarted = crashed_with_a_running_subagent().await.restart().await;
    let chat = restarted.chat;
    let (mut client, _) = restarted.attach_client().await;
    restarted.engine.continue_held(chat, None).await.unwrap();
    restarted.settle().await;
    let agent = *restarted.engine.flow.live.keys().next().unwrap();
    let before = ledger(&restarted).await.len();

    restarted
        .engine
        .on_provider_event(
            crate::providers::CLAUDE,
            subagent_started(agent, "sub-1", None),
        )
        .await
        .unwrap();

    assert_eq!(returned_notices(&client.window().await), 1);
    assert_eq!(ledger(&restarted).await.len(), before);
}

// #368: 닫은 보류 작업의 끊긴 하위 에이전트 정보는 기록에서 지운다
#[tokio::test]
async fn closing_a_held_task_forgets_its_interrupted_subagents() {
    let mut restarted = crashed_with_a_running_subagent().await;
    let chat = restarted.chat;
    restarted
        .engine
        .close_held(chat, saturn_protocol::ids::TaskId(1))
        .await
        .unwrap();

    let restarted = restarted.restart().await;

    assert!(restarted.engine.flow.interrupted_watch.is_empty());
    assert!(restarted.engine.flow.interrupted_to_clean.is_empty());
}
