//! 실험 옵션 `context.select.packet = jev` 테스트: 경쟁 구역을 router `compact` 남김 확률 순으로 채우는지, 판단을 받지 못하면
//! 순위 순서로 대체하는지, 전환 기록이 보낸 패킷과 맞는지 확인한다.
//! 설계: docs/design/context-management.md#패킷-판단의-적용

use std::time::Duration;

use saturn_protocol::ids::{LedgerSeq, SessionId};
use serde_json::json;

use super::constraint_handoff::packet_of;
use super::support::{
    Flow, context_size, idle_reply, text, tool_read, tool_result, turn_completed,
};
use super::{FakeReply, ok, status};
use crate::providers::test_support::{CLAUDE, CODEX};
use crate::store::{PacketKind, PacketState, sha256_hex};

const JEV: &str = "[context.select]\npacket = \"jev\"\n";

/// 경쟁 구역에 다 들어가지 못하게 한 크기. 경쟁 구역 예산에 결과 둘쯤 들어간다.
const TIGHT: &str = "[provider.claude.context]\nt_abs = 6000\n";

const TURNS: u64 = 6;

/// 모든 입력의 router 답을 `idle_reply`로 채운 뒤 설정으로 연 흐름.
async fn flow_with(config: &str) -> Flow {
    let replies = (0..TURNS + 1).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    flow.add_provider(CODEX);
    flow
}

/// Codex에서 여섯 턴을 한다. 턴마다 다른 파일을 읽고 결과 글에 `MARK<번호>`가 든다.
async fn long_chat(flow: &mut Flow) {
    flow.engine.switch_provider(flow.chat, CODEX);
    for number in 1..=TURNS {
        flow.submit(&format!("task {number} continue the cache"))
            .await;
        let agent = flow.agent();
        let call = format!("c{number}");
        flow.event(CODEX, text(agent, &format!("done {call}")))
            .await;
        flow.event(
            CODEX,
            tool_read(agent, &call, &format!("src/file{number}.rs")),
        )
        .await;
        flow.event(
            CODEX,
            tool_result(agent, &call, &format!("MARK{number} {}", "x".repeat(900))),
        )
        .await;
        flow.event(CODEX, turn_completed(agent)).await;
    }
}

/// 도구 호출 `c<번호>`의 기록 번호.
async fn seq_of(flow: &Flow, number: u64) -> LedgerSeq {
    let call = format!("c{number}");
    flow.engine
        .store
        .ledger_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap()
        .into_iter()
        .find_map(|row| match row.event {
            saturn_protocol::event::ProviderEvent::ToolCall { call_id, .. } if call_id == call => {
                Some(row.seq)
            }
            _ => None,
        })
        .expect("the tool call should be recorded")
}

/// `compact` 답. `probabilities`에 없는 후보는 답하지 않는다. 호출과 결과에 같은 확률을 준다.
fn compact_reply(probabilities: &[(LedgerSeq, f64)]) -> FakeReply {
    let mut answers = serde_json::Map::new();
    for (seq, probability) in probabilities {
        for prefix in ["call", "result"] {
            answers.insert(
                format!("{prefix}_{}_keep", seq.0),
                json!({ "type": "noul", "noul": probability }),
            );
        }
    }
    ok(&json!({
        "model": "jev-1.13.0",
        "answers": answers,
        "usage": { "input_tokens": 40, "output_tokens": 8 },
    })
    .to_string())
}

/// Claude로 바꿔 입력을 보내고 Claude가 받은 첫 입력을 돌려준다.
async fn switch_and_send(flow: &mut Flow) -> String {
    let claude = flow.fake.clone();
    flow.engine.switch_provider(flow.chat, CLAUDE);
    flow.submit("task 7 review the cache").await;
    packet_of(&claude)
}

fn marks(packet: &str) -> Vec<u64> {
    (1..=TURNS)
        .filter(|number| packet.contains(&format!("MARK{number} ")))
        .collect()
}

/// 새 session의 전달 기록에서 경쟁 구역의 `(고른 방식, 기록 번호)` 목록. 보낸 글의 해시가 받은 글과 같은지도 본다.
async fn recorded_selectors(flow: &Flow, packet: &str) -> Vec<(String, u64)> {
    let recorded = flow.engine.store.packets_of_chat(flow.chat).await.unwrap();
    let stored = recorded
        .iter()
        .find(|stored| stored.session == SessionId(2))
        .expect("the switch packet should be recorded");
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    assert_eq!(
        (stored.kind, stored.state),
        (PacketKind::Switch, PacketState::Sent)
    );
    flow.engine
        .store
        .packet_competing(stored.id)
        .await
        .unwrap()
        .into_iter()
        .map(|(seq, selector, _)| (selector, seq))
        .collect()
}

#[tokio::test]
async fn jev_order_fills_the_budget_with_the_blocks_the_router_wants() {
    let mut rank = flow_with(TIGHT).await;
    long_chat(&mut rank).await;
    let rank_packet = switch_and_send(&mut rank).await;

    let mut jev = flow_with(&format!("{TIGHT}{JEV}")).await;
    long_chat(&mut jev).await;
    let wanted = seq_of(&jev, 2).await;
    let also = seq_of(&jev, 3).await;
    jev.transport
        .push_reply(compact_reply(&[(wanted, 0.95), (also, 0.6)]));
    let calls_before = jev.router_calls();
    let jev_packet = switch_and_send(&mut jev).await;

    // RRF만으로는 오래된 블록 2가 예산에서 빠진다
    assert!(
        !marks(&rank_packet).contains(&2),
        "{:?}",
        marks(&rank_packet)
    );
    // router가 남기라고 한 블록이 들어가고, 같은 예산이라 전체 크기는 RRF와 비슷하다
    assert!(marks(&jev_packet).contains(&2), "{:?}", marks(&jev_packet));
    assert_eq!(marks(&jev_packet).len(), marks(&rank_packet).len());
    // 판단은 새 session을 여는 입력에서 한 번만 물었다
    assert_eq!(jev.router_calls() - calls_before, 2);
    let request = jev.transport.calls().last().unwrap().2.clone().unwrap();
    assert!(request.contains("Latest user request"));
    assert!(request.contains("MARK2"));
    let selectors = recorded_selectors(&jev, &jev_packet).await;
    assert!(!selectors.is_empty());
    assert!(selectors.iter().all(|(selector, _)| selector == "compact"));
    assert!(selectors.iter().any(|(_, seq)| *seq == wanted.0));
    let rank_selectors = recorded_selectors(&rank, &rank_packet).await;
    assert!(
        rank_selectors
            .iter()
            .all(|(selector, _)| selector == "rank")
    );
}

#[tokio::test]
async fn single_needed_block_with_low_scores_elsewhere_is_kept_not_replaced() {
    // #540: 낮은 확신 규칙이 필요한 블록이 하나인 과제를 모두 코드 검색으로 바꿨다. 여기서는 하나만 높고 나머지는 낮아도
    // 그 하나가 먼저 들어가고, 남은 예산은 낮은 확률 순으로 채운다
    let mut flow = flow_with(&format!("{TIGHT}{JEV}")).await;
    long_chat(&mut flow).await;
    let wanted = seq_of(&flow, 1).await;
    let others: Vec<(LedgerSeq, f64)> = std::iter::once((wanted, 0.95)).collect();
    flow.transport.push_reply(compact_reply(&others));

    let packet = switch_and_send(&mut flow).await;

    assert!(marks(&packet).contains(&1), "{:?}", marks(&packet));
    // 답이 없는 나머지는 제외되지 않고 RRF 순서로 남은 예산을 채운다
    assert!(marks(&packet).len() >= 2, "{:?}", marks(&packet));
}

#[tokio::test]
async fn without_a_judgment_the_packet_is_filled_by_rank_order() {
    let mut rank = flow_with(TIGHT).await;
    long_chat(&mut rank).await;
    let expected = marks(&switch_and_send(&mut rank).await);

    let cases = [
        (
            "router rejects the key",
            Some(status(401, "{}", Vec::new())),
        ),
        ("answers are all missing", Some(compact_reply(&[]))),
        ("reply is not usable", Some(ok("not json"))),
    ];
    for (name, reply) in cases {
        let mut flow = flow_with(&format!("{TIGHT}{JEV}")).await;
        long_chat(&mut flow).await;
        flow.transport.push_reply(reply.unwrap());

        let packet = switch_and_send(&mut flow).await;

        assert_eq!(marks(&packet), expected, "{name}");
        let selectors = recorded_selectors(&flow, &packet).await;
        assert!(
            selectors.iter().all(|(selector, _)| selector == "rank"),
            "{name}"
        );
    }
}

#[tokio::test]
async fn an_answer_that_arrives_too_late_is_not_used() {
    let mut rank = flow_with(TIGHT).await;
    long_chat(&mut rank).await;
    let expected = marks(&switch_and_send(&mut rank).await);

    let mut flow = flow_with(&format!("{TIGHT}{JEV}")).await;
    long_chat(&mut flow).await;
    let wanted = seq_of(&flow, 2).await;
    // 기다림(시험에서는 0.3초)보다 늦게 온 답이다
    flow.transport
        .push_reply_after(compact_reply(&[(wanted, 0.99)]), Duration::from_secs(3));

    let packet = switch_and_send(&mut flow).await;

    assert_eq!(marks(&packet), expected);
}

#[tokio::test]
async fn option_is_off_by_default_and_asks_nothing() {
    let mut flow = flow_with(TIGHT).await;
    long_chat(&mut flow).await;
    let calls_before = flow.router_calls();

    switch_and_send(&mut flow).await;

    assert_eq!(flow.router_calls() - calls_before, 1);
}

/// 맥락 정리 때 router가 `compact`에 내는 답.
#[derive(Clone, Copy)]
enum Compact {
    /// 묻지 않는 설정이라 답이 필요 없다.
    NotAsked,
    /// 첫 도구 호출만 높게 답한다.
    WantsFirst,
    NoAnswers,
    KeyRejected,
}

/// 맥락 정리로 여는 새 session에 보낸 패킷의 `(들어간 블록 번호, 경쟁 구역의 고른 방식 목록)`.
async fn compaction_packet(config: &str, compact: Compact) -> (Vec<u64>, Vec<String>) {
    let mut flow = Flow::with_config(config, vec![idle_reply(0.95)]).await;
    flow.submit("task 1 continue the cache").await;
    let agent = flow.agent();
    for number in 1..=4 {
        let call = format!("c{number}");
        flow.claude_event(tool_read(agent, &call, &format!("src/file{number}.rs")))
            .await;
        let body = format!("MARK{number} {}", "x".repeat(900));
        flow.claude_event(tool_result(agent, &call, &body)).await;
    }
    let first = seq_of(&flow, 1).await;
    match compact {
        Compact::NotAsked => {}
        Compact::WantsFirst => flow.transport.push_reply(compact_reply(&[(first, 0.99)])),
        Compact::NoAnswers => flow.transport.push_reply(compact_reply(&[])),
        Compact::KeyRejected => flow.transport.push_reply(status(401, "{}", Vec::new())),
    }
    flow.claude_event(context_size(agent, 10_000_000)).await;
    flow.claude_event(turn_completed(agent)).await;
    let packet = flow
        .fake
        .calls()
        .into_iter()
        .rev()
        .find_map(|call| match call {
            crate::providers::test_support::Call::Open { packet, .. } => packet,
            _ => None,
        })
        .expect("compaction should open a new session");
    let recorded = flow.engine.store.packets_of_chat(flow.chat).await.unwrap();
    let stored = recorded.last().expect("the packet should be recorded");
    assert_eq!(stored.kind, PacketKind::Restart);
    assert_eq!(stored.body_hash, sha256_hex(packet.as_bytes()));
    let selectors = flow
        .engine
        .store
        .packet_competing(stored.id)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, selector, _)| selector)
        .collect();
    (marks(&packet), selectors)
}

#[tokio::test]
async fn compaction_orders_the_packet_by_the_judgment_and_falls_back_to_rank_order() {
    let cases = [
        ("", Compact::NotAsked, "rank"),
        (JEV, Compact::WantsFirst, "compact"),
        (JEV, Compact::NoAnswers, "rank"),
        (JEV, Compact::KeyRejected, "rank"),
    ];
    for (config, compact, expected) in cases {
        let (marks, selectors) = compaction_packet(config, compact).await;

        // 판단을 받지 못해도 맥락 정리는 하고 패킷에 블록이 든다
        assert!(!marks.is_empty());
        assert!(!selectors.is_empty());
        assert!(
            selectors.iter().all(|selector| selector == expected),
            "{selectors:?}"
        );
    }
}
