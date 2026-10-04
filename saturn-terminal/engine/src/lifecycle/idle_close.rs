//! 유휴 session 닫기 테스트(#463): TUI가 붙어 있어도 트리 유휴 뒤 5분 유예가 지난 session은 닫고, 다음 입력이 보관한 ID로 재개한다.
//! 시간은 `Instant`를 직접 넘겨 가상으로 흘린다.

use std::time::{Duration, Instant};

use saturn_core::sessions::{AgentRole, SendTarget};
use saturn_protocol::ids::{AgentId, LedgerSeq, SessionId};
use saturn_protocol::state::SessionState;

use super::crash_recovery::Restarted;
use super::support::{Flow, context_size, idle_reply, subagent_started, text, turn_completed};
use crate::providers::test_support::Call;
use crate::sessions::SendRequest;

/// 첫 입력의 턴을 끝내 session을 유휴로 둔다. 돌려주는 값은 (에이전트, session, 유휴가 된 시각).
async fn finished_turn(flow: &mut Flow) -> (AgentId, SessionId, Instant) {
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    flow.claude_event(text(agent, "the cache is fixed")).await;
    flow.claude_event(context_size(agent, 1_000)).await;
    flow.claude_event(turn_completed(agent)).await;
    let since = flow
        .engine
        .sessions
        .get(session)
        .and_then(|record| record.idle_since)
        .expect("the session should be idle after the turn");
    (agent, session, since)
}

fn state_of(flow: &Flow, session: SessionId) -> SessionState {
    flow.engine.sessions.get(session).unwrap().state
}

fn closes(flow: &Flow) -> usize {
    flow.fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Close { .. }))
        .count()
}

/// 기다리지 않고 보낸 닫기 요청이 가짜 provider에 닿을 때까지 기다린다.
async fn closes_after_settling(flow: &Flow) -> usize {
    for _ in 0..100 {
        if closes(flow) > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    closes(flow)
}

fn opens(flow: &Flow) -> Vec<Call> {
    flow.fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .collect()
}

#[tokio::test]
async fn attached_tui_keeps_the_session_until_the_grace_and_closes_it_after() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (_client, _) = flow.attach().await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let grace = flow.engine.idle_grace;

    let ended = flow
        .engine
        .check_idle(since + grace - Duration::from_secs(1))
        .await;
    assert!(!ended);
    assert_eq!(state_of(&flow, session), SessionState::Open);
    assert_eq!(closes(&flow), 0);

    let ended = flow.engine.check_idle(since + grace).await;
    assert!(!ended, "an attached engine must not end");
    let record = flow.engine.sessions.get(session).unwrap().clone();
    assert_eq!(record.state, SessionState::ClosedResumable);
    assert!(record.provider_session.is_some());
    assert!(!flow.engine.flow.live.contains_key(&agent));
    assert_eq!(closes_after_settling(&flow).await, 1);
    let stored = flow.engine.store.live_mains().await.unwrap();
    assert_eq!(stored[0].0.state, SessionState::ClosedResumable);
    assert_eq!(stored[0].0.provider_session, record.provider_session);
}

#[tokio::test]
async fn a_new_turn_during_the_grace_cancels_the_clock() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (_client, _) = flow.attach().await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let grace = flow.engine.idle_grace;

    flow.submit("now run the tests").await;
    assert!(
        flow.engine
            .sessions
            .get(session)
            .unwrap()
            .idle_since
            .is_none()
    );
    flow.engine.check_idle(since + grace * 2).await;
    assert_eq!(state_of(&flow, session), SessionState::Open);

    flow.claude_event(turn_completed(agent)).await;
    let restarted = flow
        .engine
        .sessions
        .get(session)
        .unwrap()
        .idle_since
        .unwrap();
    flow.engine
        .check_idle(restarted + grace - Duration::from_secs(1))
        .await;
    assert_eq!(state_of(&flow, session), SessionState::Open);
    assert_eq!(closes(&flow), 0);
}

#[tokio::test]
async fn a_running_subagent_keeps_the_session() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let far = since + flow.engine.idle_grace * 2;

    flow.claude_event(subagent_started(agent, "sub-1", None))
        .await;
    flow.engine.check_idle(far).await;
    assert_eq!(state_of(&flow, session), SessionState::Open);
}

// #463: 유휴 시계와 트리 유휴는 그대로 두고 대기 중인 허가 요청 guard만으로 닫기가 막히는지 본다. 턴이 끝난 뒤의 허가
// 요청은 engine이 올리지 않으므로 대기 목록에 직접 넣고, guard를 거두면 닫히는 것까지 확인한다.
#[tokio::test]
async fn a_pending_permission_alone_keeps_an_idle_session_open() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let chat = flow.chat;
    let task = flow.engine.runs.task_of.get(&agent).copied();
    let task = task.unwrap_or(saturn_protocol::ids::TaskId(1));
    flow.engine.flow.permissions.insert(
        "perm-1".to_owned(),
        crate::events::PendingPermission {
            chat,
            agent,
            provider: crate::providers::test_support::CLAUDE,
            task,
            provider_request: "req-1".to_owned(),
            call: None,
        },
    );
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().idle_since,
        Some(since)
    );
    assert!(flow.engine.agents.is_tree_idle(agent));
    let far = since + flow.engine.idle_grace * 2;

    flow.engine.check_idle(far).await;

    assert_eq!(state_of(&flow, session), SessionState::Open);
    assert_eq!(closes(&flow), 0);

    flow.engine.flow.permissions.clear();
    flow.engine.check_idle(far).await;

    assert_eq!(state_of(&flow, session), SessionState::ClosedResumable);
}

#[tokio::test]
async fn a_failed_archive_write_keeps_the_session_open_and_the_next_check_closes_it() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let grace = flow.engine.idle_grace;
    flow.engine.store.deny_writes().await;

    flow.engine.check_idle(since + grace).await;

    assert_eq!(state_of(&flow, session), SessionState::Open);
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().idle_since,
        Some(since)
    );
    assert!(flow.engine.flow.live.contains_key(&agent));
    assert_eq!(closes(&flow), 0);

    flow.engine.store.allow_writes().await;
    flow.engine.check_idle(since + grace).await;

    assert_eq!(state_of(&flow, session), SessionState::ClosedResumable);
    assert!(!flow.engine.flow.live.contains_key(&agent));
    assert_eq!(closes_after_settling(&flow).await, 1);
    let stored = flow.engine.store.live_mains().await.unwrap();
    assert_eq!(stored[0].0.state, SessionState::ClosedResumable);
}

#[tokio::test]
async fn the_next_input_resumes_the_closed_session_with_the_stored_id() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (agent, session, since) = finished_turn(&mut flow).await;
    let before = flow.engine.sessions.get(session).unwrap().clone();
    flow.engine.check_idle(since + flow.engine.idle_grace).await;
    assert_eq!(state_of(&flow, session), SessionState::ClosedResumable);

    flow.submit("now run the tests").await;

    let calls = opens(&flow);
    assert_eq!(calls.len(), 2);
    let Call::Open { resume, .. } = &calls[1] else {
        panic!("the second call should open a session");
    };
    assert_eq!(*resume, before.provider_session);
    assert_eq!(state_of(&flow, session), SessionState::Open);
    assert_eq!(flow.engine.flow.live[&agent].session, session);
    assert!(flow.engine.sessions.get(session).unwrap().delivered >= before.delivered);
}

#[tokio::test]
async fn closed_session_keeps_the_id_and_the_delivered_number_across_an_engine_restart() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (_agent, session, since) = finished_turn(&mut flow).await;
    let before = flow.engine.sessions.get(session).unwrap().clone();
    flow.engine.check_idle(since + flow.engine.idle_grace).await;
    let (chat, settings) = (flow.chat, flow.engine.settings.current().unwrap());

    let restarted = Restarted::after_shutdown(flow).await;

    let main = restarted.engine.sessions.live_main(chat).unwrap();
    assert_eq!(main.state, SessionState::ClosedResumable);
    assert_eq!(main.provider_session, before.provider_session);
    assert_eq!(main.delivered, before.delivered);
    assert!(main.delivered > LedgerSeq(0));
    let request = SendRequest {
        chat,
        provider: crate::providers::test_support::CLAUDE,
        role: AgentRole::Main,
        packet: 1,
        settings,
    };
    let target = restarted
        .engine
        .send_target(request, std::time::SystemTime::now())
        .await
        .unwrap();
    assert_eq!(target, SendTarget::Resume(session));
}

#[tokio::test]
async fn returning_to_a_closed_session_after_the_cache_window_still_judges_the_idle_return() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (agent, old, since) = finished_turn(&mut flow).await;
    let ended_at = std::time::SystemTime::now() - Duration::from_secs(3_600);
    flow.engine.sessions.record_last_turn(
        old,
        saturn_core::sessions::LastTurn {
            active: 50_000,
            ended_at,
        },
    );
    flow.engine.check_idle(since + flow.engine.idle_grace).await;
    assert_eq!(state_of(&flow, old), SessionState::ClosedResumable);

    flow.submit("now run the tests").await;

    let calls = opens(&flow);
    let Call::Open { resume, packet, .. } = &calls[1] else {
        panic!("the second call should open a session");
    };
    assert_eq!(*resume, None);
    assert!(
        packet
            .as_deref()
            .is_some_and(|p| p.contains("fix the build"))
    );
    assert_eq!(state_of(&flow, old), SessionState::Ended);
    assert_ne!(flow.engine.flow.live[&agent].session, old);
}
