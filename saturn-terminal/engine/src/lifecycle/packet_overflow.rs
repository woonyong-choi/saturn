//! 패킷 맥락 초과 테스트: provider가 맥락 한도로 거절하면 낮은 순 항목을 빼 한 번만 다시 보내고, 안 되면 멈추고 알린다.

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::SessionId;
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState};

use super::support::{
    Flow, context_size, idle_reply, text, tool_read, tool_result, turn_completed,
};
use super::*;
use crate::providers::test_support::{Call, FakeProvider};
use crate::store::{PacketKind, PacketState, StoredPacket, sha256_hex};

const FIRST_INPUT: &str = "write the cache module";
const ANSWER: &str = "cache module written";
const CALLS: [&str; 4] = ["c1", "c2", "c3", "c4"];

fn exceeded() -> Result<(), ProviderError> {
    Err(ProviderError::ContextExceeded { limit_tokens: None })
}

/// Claude가 도구 호출이 많은 턴 하나를 끝낸 채팅과, 거절 응답을 줄 Codex.
async fn flow_with_codex(first_input: &str, tools: &[&str]) -> (Flow, FakeProvider) {
    let replies = (0..3).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::new(replies).await;
    let codex = flow.add_provider(crate::providers::test_support::CODEX);
    flow.submit(first_input).await;
    let agent = flow.agent();
    flow.claude_event(text(agent, ANSWER)).await;
    for call in tools {
        flow.claude_event(tool_read(agent, call, "src/cache.rs"))
            .await;
        let output = format!("{call} body {}", "y".repeat(400));
        flow.claude_event(tool_result(agent, call, &output)).await;
    }
    flow.claude_event(turn_completed(agent)).await;
    (flow, codex)
}

fn packets(fake: &FakeProvider) -> Vec<String> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { packet, .. } => packet,
            _ => None,
        })
        .collect()
}

/// 채팅이 기록한 패킷 시도와 시도마다의 경쟁 구역에 원문으로 든 항목 수와 빠진 항목 수.
async fn recorded(flow: &Flow) -> Vec<(StoredPacket, usize, usize)> {
    let mut attempts = Vec::new();
    for packet in flow.engine.store.packets_of_chat(flow.chat).await.unwrap() {
        let items = flow.engine.store.packet_items(packet.id).await.unwrap();
        let competing = items.iter().filter(|item| item.0 == "Competing");
        let kept = competing
            .clone()
            .filter(|item| item.2.as_deref() == Some("Full"))
            .count();
        let dropped = competing.filter(|item| item.3.as_deref() == Some("budget"));
        attempts.push((packet, kept, dropped.count()));
    }
    attempts
}

fn kept_tools(packet: &str) -> usize {
    CALLS
        .iter()
        .filter(|call| packet.contains(&format!("{call} body")))
        .count()
}

#[tokio::test]
async fn packet_overflow_rejection_resends_once_without_the_lowest_items() {
    let (mut flow, codex) = flow_with_codex(FIRST_INPUT, &CALLS).await;
    codex.answer_open([exceeded(), Ok(())]);
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);

    let second = flow.submit("now with codex").await;

    let sent = packets(&codex);
    assert_eq!(sent.len(), 2);
    assert_eq!(kept_tools(&sent[0]), CALLS.len());
    assert!(sent[1].len() < sent[0].len());
    assert!(kept_tools(&sent[1]) < CALLS.len());
    assert_eq!(flow.state(second), InputState::Applied);
    let attempts = recorded(&flow).await;
    let [
        (first, first_kept, first_dropped),
        (again, again_kept, again_dropped),
    ] = &attempts[..]
    else {
        panic!("both attempts should be recorded: {attempts:?}");
    };
    for (stored, body) in [(first, &sent[0]), (again, &sent[1])] {
        assert_eq!(stored.body_hash, sha256_hex(body.as_bytes()));
        assert_eq!(stored.body_bytes, body.len() as u64);
        assert_eq!(stored.input, Some(second));
        assert_eq!(stored.provider, crate::providers::test_support::CODEX);
    }
    assert_eq!((first.attempt, first.state), (1, PacketState::NotSent));
    assert_eq!((again.attempt, again.state), (2, PacketState::Sent));
    assert_eq!(again.reduced_from, Some(first.id));
    assert!(again_kept < first_kept, "{attempts:?}");
    assert_eq!(
        (*first_kept, *first_dropped, *again_dropped),
        (CALLS.len(), 0, 0)
    );
    assert_ne!(first.session, again.session);
    assert!(again.provider_session.is_some() && first.provider_session.is_none());
    assert!(again.run.is_some());
}

#[tokio::test]
async fn packet_overflow_reduction_keeps_the_fixed_zone() {
    let (mut flow, codex) = flow_with_codex(FIRST_INPUT, &CALLS).await;
    codex.answer_open([exceeded(), Ok(())]);
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);

    flow.submit("now with codex").await;

    let sent = packets(&codex);
    assert_eq!(sent.len(), 2);
    assert!(sent[1].len() < sent[0].len());
    for packet in &sent {
        assert!(packet.contains(FIRST_INPUT));
        assert!(packet.contains(ANSWER));
    }
}

#[tokio::test]
async fn packet_overflow_after_the_reduced_resend_stops_and_tells_the_user() {
    let (mut flow, codex) = flow_with_codex(FIRST_INPUT, &CALLS).await;
    codex.answer_open([exceeded(), exceeded(), Ok(())]);
    let mut client = flow.client().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);

    let second = flow.submit("now with codex").await;

    assert_eq!(packets(&codex).len(), 2);
    assert_eq!(flow.state(second), InputState::Held);
    let states: Vec<PacketState> = recorded(&flow)
        .await
        .iter()
        .map(|(packet, ..)| packet.state)
        .collect();
    assert_eq!(states, [PacketState::NotSent, PacketState::NotSent]);
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .await;
    assert_eq!(notice, ChatNotice::PacketOverflow);
}

#[tokio::test]
async fn packet_overflow_with_only_the_fixed_zone_over_the_target_is_not_resent() {
    let (mut flow, codex) = flow_with_codex(&"x".repeat(600), &[]).await;
    codex.answer_open([exceeded(), Ok(())]);
    let mut client = flow.client().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::test_support::CODEX);

    let second = flow.submit("now with codex").await;

    assert_eq!(packets(&codex).len(), 1);
    assert_eq!(flow.state(second), InputState::Held);
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .await;
    assert_eq!(notice, ChatNotice::PacketOverflow);
}

/// 맥락이 기준을 넘은 채 도구 호출이 많은 턴을 끝내는 Claude 채팅. 턴이 끝나면 맥락 정리가 새 session을 열고, 그 열기에 `answers`를 준다.
/// 돌려주는 session은 맥락 정리 전의 것이다.
async fn restart_with(
    answers: impl IntoIterator<Item = Result<(), ProviderError>>,
) -> (Flow, Client, SessionId) {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit(FIRST_INPUT).await;
    let agent = flow.agent();
    let old = flow.engine.flow.live[&agent].session;
    let client = flow.client().await;
    flow.claude_event(text(agent, ANSWER)).await;
    for call in CALLS {
        flow.claude_event(tool_read(agent, call, "src/cache.rs"))
            .await;
        let output = format!("{call} body {}", "y".repeat(400));
        flow.claude_event(tool_result(agent, call, &output)).await;
    }
    flow.claude_event(context_size(agent, 10_000_000)).await;
    flow.fake.answer_open(answers);
    flow.claude_event(turn_completed(agent)).await;
    (flow, client, old)
}

async fn first_notice(client: &mut Client) -> ChatNotice {
    client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .await
}

#[tokio::test]
async fn compaction_overflow_rejection_resends_once_without_the_lowest_items() {
    let (flow, mut client, old) = restart_with([exceeded(), Ok(())]).await;

    let sent = packets(&flow.fake);
    assert_eq!(sent.len(), 2);
    assert_eq!(kept_tools(&sent[0]), CALLS.len());
    assert!(sent[1].len() < sent[0].len());
    assert!(kept_tools(&sent[1]) < CALLS.len());
    for packet in &sent {
        assert!(packet.contains(FIRST_INPUT));
        assert!(packet.contains(ANSWER));
    }
    let fresh = flow.engine.flow.live[&flow.agent()].session;
    assert_ne!(fresh, old);
    assert_eq!(
        flow.engine.sessions.get(old).unwrap().state,
        SessionState::Ended
    );
    assert_eq!(first_notice(&mut client).await, ChatNotice::Compacted);
    let attempts = recorded(&flow).await;
    let [(first, ..), (again, ..)] = &attempts[..] else {
        panic!("both attempts should be recorded: {attempts:?}");
    };
    assert_eq!(first.kind, PacketKind::Restart);
    assert_eq!(first.input, None);
    assert_eq!(first.body_hash, sha256_hex(sent[0].as_bytes()));
    assert_eq!(again.body_hash, sha256_hex(sent[1].as_bytes()));
    assert_eq!(
        (first.state, again.state, again.reduced_from),
        (PacketState::NotSent, PacketState::Sent, Some(first.id))
    );
    assert_eq!(again.session, fresh);
}

#[tokio::test]
async fn compaction_overflow_after_the_reduced_resend_keeps_the_session_and_tells_the_user() {
    let (flow, mut client, old) = restart_with([exceeded(), exceeded(), Ok(())]).await;

    assert_eq!(packets(&flow.fake).len(), 2);
    assert_eq!(flow.engine.flow.live[&flow.agent()].session, old);
    assert_eq!(
        flow.engine.sessions.get(old).unwrap().state,
        SessionState::Open
    );
    assert_eq!(first_notice(&mut client).await, ChatNotice::PacketOverflow);
}
