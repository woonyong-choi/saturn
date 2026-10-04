//! provider 전환 테스트: Codex → Claude → Codex 왕복에서 패킷, session별 전달 번호, 중복과 누락을 확인한다.

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{LedgerSeq, Provider, ProviderSessionId, SessionId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState};

use super::support::{Flow, idle_reply, text, tool_read, tool_result, turn_completed};
use super::*;
use crate::providers::test_support::{Call, FakeProvider};

/// Codex가 연 첫 session과 Claude가 연 session. 번호는 열린 순서다.
const CODEX_FIRST: SessionId = SessionId(1);
const CLAUDE_FIRST: SessionId = SessionId(2);

/// 설치된 provider는 Claude뿐이라 첫 입력이 Codex로 가려면 먼저 정해 둔다. 판단은 모두 유휴 판단이다.
async fn round_trip_flow() -> (Flow, FakeProvider) {
    let replies = (0..4).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::new(replies).await;
    let codex = flow.add_provider(crate::providers::CODEX);
    (flow, codex)
}

fn opened(fake: &FakeProvider) -> Vec<(Option<ProviderSessionId>, Option<String>)> {
    fake.calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { resume, packet, .. } => Some((resume, packet)),
            _ => None,
        })
        .collect()
}

fn delivered(flow: &Flow, session: SessionId) -> LedgerSeq {
    flow.engine.sessions.get(session).unwrap().delivered
}

fn state(flow: &Flow, session: SessionId) -> SessionState {
    flow.engine.sessions.get(session).unwrap().state
}

/// 이 provider의 턴 하나: 글, 파일 읽기, 완료. `packet`이 참이면 앞서 보낸 패킷 턴의 완료도 먼저 보낸다.
async fn run_turn(flow: &mut Flow, provider: Provider, answer: &str, call: &str, packet: bool) {
    let agent = flow.agent();
    if packet {
        flow.event(provider, turn_completed(agent)).await;
    }
    flow.event(provider, text(agent, answer)).await;
    flow.event(provider, tool_read(agent, call, "src/cache.rs"))
        .await;
    flow.event(provider, tool_result(agent, call, &format!("{call} body")))
        .await;
    flow.event(provider, turn_completed(agent)).await;
}

#[tokio::test]
async fn codex_to_claude_to_codex_hands_over_without_duplicates_or_gaps() {
    let (mut flow, codex) = round_trip_flow().await;
    let claude = flow.fake.clone();

    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("write the cache module").await;
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex wrote cache",
        "c1",
        false,
    )
    .await;
    let codex_end = delivered(&flow, CODEX_FIRST);
    assert_eq!(codex_end, LedgerSeq(4));
    assert_eq!(opened(&codex), vec![(None, None)]);

    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("review the cache module").await;

    let to_claude = opened(&claude);
    assert_eq!(to_claude.len(), 1);
    let packet = to_claude[0]
        .1
        .as_deref()
        .expect("claude should get a packet");
    assert!(packet.contains("write the cache module"));
    assert!(packet.contains("codex wrote cache"));
    assert!(packet.contains("c1 body"));
    assert_eq!(state(&flow, CODEX_FIRST), SessionState::ClosedResumable);
    assert_eq!(state(&flow, CLAUDE_FIRST), SessionState::Open);
    assert_eq!(delivered(&flow, CLAUDE_FIRST), codex_end);
    assert!(
        codex
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Close { .. }))
    );
    run_turn(
        &mut flow,
        crate::providers::CLAUDE,
        "claude reviewed cache",
        "c2",
        true,
    )
    .await;
    let claude_end = delivered(&flow, CLAUDE_FIRST);
    assert_eq!(claude_end, LedgerSeq(9));
    assert_eq!(delivered(&flow, CODEX_FIRST), codex_end);

    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("apply the review notes").await;

    let back = opened(&codex);
    assert_eq!(back.len(), 2);
    assert_eq!(
        back[1].0,
        Some(ProviderSessionId("fake-session-1".to_owned()))
    );
    let catch_up = back[1].1.as_deref().expect("codex should get the changes");
    assert!(catch_up.contains("claude reviewed cache"));
    assert!(catch_up.contains("c2 body"));
    assert!(!catch_up.contains("codex wrote cache"));
    assert!(!catch_up.contains("c1 body"));
    assert_eq!(state(&flow, CODEX_FIRST), SessionState::Open);
    assert_eq!(state(&flow, CLAUDE_FIRST), SessionState::ClosedResumable);
    assert_eq!(delivered(&flow, CODEX_FIRST), claude_end);
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex applied notes",
        "c3",
        true,
    )
    .await;
    let codex_again = delivered(&flow, CODEX_FIRST);
    assert_eq!(codex_again, LedgerSeq(14));
    assert_eq!(delivered(&flow, CLAUDE_FIRST), claude_end);

    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("final check").await;

    let again = opened(&claude);
    assert_eq!(again.len(), 2);
    assert_eq!(
        again[1].0,
        Some(ProviderSessionId("fake-session-1".to_owned()))
    );
    let second_catch_up = again[1]
        .1
        .as_deref()
        .expect("claude should get the changes");
    assert!(second_catch_up.contains("codex applied notes"));
    assert!(!second_catch_up.contains("claude reviewed cache"));
    assert!(!second_catch_up.contains("codex wrote cache"));
    assert_eq!(delivered(&flow, CLAUDE_FIRST), codex_again);
}

#[tokio::test]
async fn delivered_numbers_never_go_down_across_switches() {
    let (mut flow, _codex) = round_trip_flow().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("write the cache module").await;
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex wrote cache",
        "c1",
        false,
    )
    .await;
    let mut seen = vec![delivered(&flow, CODEX_FIRST)];

    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("review the cache module").await;
    seen.push(delivered(&flow, CLAUDE_FIRST));
    run_turn(
        &mut flow,
        crate::providers::CLAUDE,
        "claude reviewed cache",
        "c2",
        true,
    )
    .await;
    seen.push(delivered(&flow, CLAUDE_FIRST));

    assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "{seen:?}");
    let stored = flow.engine.store.sessions(flow.chat).await.unwrap();
    let saved: Vec<(SessionId, LedgerSeq)> = stored
        .iter()
        .map(|record| (record.id, record.delivered))
        .collect();
    assert_eq!(
        saved,
        vec![(CODEX_FIRST, LedgerSeq(4)), (CLAUDE_FIRST, LedgerSeq(9))]
    );
}

#[tokio::test]
async fn packet_turn_completion_does_not_end_the_task_on_the_new_provider() {
    let (mut flow, _codex) = round_trip_flow().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("write the cache module").await;
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex wrote cache",
        "c1",
        false,
    )
    .await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("review the cache module").await;
    let agent = flow.agent();

    flow.event(crate::providers::CLAUDE, turn_completed(agent))
        .await;

    assert!(flow.engine.chat_is_running(flow.chat));
    flow.event(crate::providers::CLAUDE, turn_completed(agent))
        .await;
    assert!(!flow.engine.chat_is_running(flow.chat));
}

/// 전환 뒤 패킷 턴에 provider가 "알겠습니다"로 답해도 사용자 입력의 답으로 보이지 않는다(#332).
#[tokio::test]
async fn packet_turn_reply_is_not_shown_as_the_input_reply() {
    let (mut flow, _codex) = round_trip_flow().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("write the cache module").await;
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex wrote cache",
        "c1",
        false,
    )
    .await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("review the cache module").await;
    let mut client = flow.client().await;
    let agent = flow.agent();

    flow.event(crate::providers::CLAUDE, text(agent, "알겠습니다"))
        .await;
    flow.event(crate::providers::CLAUDE, turn_completed(agent))
        .await;
    flow.event(crate::providers::CLAUDE, text(agent, "리뷰했습니다"))
        .await;
    flow.event(crate::providers::CLAUDE, turn_completed(agent))
        .await;

    let live = client
        .until(|notification| match notification {
            Notification::TaskEvent {
                event: ProviderEvent::Text { text, .. },
                ..
            } => Some(text.clone()),
            _ => None,
        })
        .await;
    assert_eq!(live, "리뷰했습니다");
    let (_, greeting) = flow.attach().await;
    let Some(Notification::HistoryChunk { entries, .. }) = greeting.get(1) else {
        panic!("expected HistoryChunk, got {greeting:?}");
    };
    let shown: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            Notification::TaskEvent {
                event: ProviderEvent::Text { text, .. },
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(shown, vec!["codex wrote cache", "리뷰했습니다"]);
    let recorded = flow
        .engine
        .store
        .ledger_since(flow.chat, LedgerSeq(0))
        .await
        .unwrap();
    assert!(recorded.iter().any(|row| matches!(
        &row.event,
        ProviderEvent::PacketReply { text, .. } if text == "알겠습니다"
    )));
}

#[tokio::test]
async fn switch_tells_the_user_which_provider_took_over() {
    let (mut flow, _codex) = round_trip_flow().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);
    flow.submit("write the cache module").await;
    run_turn(
        &mut flow,
        crate::providers::CODEX,
        "codex wrote cache",
        "c1",
        false,
    )
    .await;
    let mut client = flow.client().await;

    flow.engine
        .switch_provider(flow.chat, crate::providers::CLAUDE);
    flow.submit("review the cache module").await;

    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .await;
    assert_eq!(
        notice,
        ChatNotice::ProviderSwitched {
            from: crate::providers::CODEX,
            to: crate::providers::CLAUDE
        }
    );
}

/// 설정은 사용자 파일이 시작 전에 읽는다. Codex의 `T`를 400토큰으로 줄여 패킷의 `P_hard`를 80토큰으로 만든다.
const SMALL_CODEX: &str = "[context.codex]\nt_abs = 400\nwindow = 400\n";

#[tokio::test]
async fn packet_over_the_hard_limit_is_not_sent_and_the_input_is_held() {
    let replies = (0..2).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(SMALL_CODEX, replies).await;
    let codex = flow.add_provider(crate::providers::CODEX);
    let first = flow.submit(&"x".repeat(600)).await;
    let agent = flow.agent();
    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(turn_completed(agent)).await;
    let mut client = flow.client().await;
    flow.engine
        .switch_provider(flow.chat, crate::providers::CODEX);

    let second = flow.submit("now with codex").await;

    assert_eq!(flow.state(first), InputState::Applied);
    assert_eq!(flow.state(second), InputState::Held);
    assert!(codex.calls().is_empty());
    assert_eq!(state(&flow, SessionId(1)), SessionState::Open);
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. } => Some(notice.clone()),
            _ => None,
        })
        .await;
    assert!(matches!(notice, ChatNotice::ContextDeferred { .. }));
}
