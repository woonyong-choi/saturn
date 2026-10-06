//! 턴 끝 테스트: 마지막 턴 값 기록, 트리 유휴 확인, 턴 경계에서만 하는 맥락 정리, 대기 입력 전송.

use std::time::{Duration, SystemTime};

use saturn_core::sessions::LastTurn;
use saturn_protocol::ids::{LedgerSeq, SessionId};
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::{InputState, SessionState, TaskState};

use super::support::{
    CLIENT, Flow, context_size, idle_reply, running_reply, subagent_ended, subagent_started, text,
    tool_read, turn_completed,
};
use super::*;
use crate::providers::test_support::Call;

const HUGE: u64 = 10_000_000;

/// 사용자 설정으로 줄인 발동 기준. `T`는 400토큰이라 `P_max`는 40토큰, `P_send`는 80토큰이다.
const SMALL_CONTEXT: &str = "[context.claude]\nt_abs = 400\nwindow = 400\n";

fn opens(flow: &Flow) -> Vec<Call> {
    flow.fake
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Open { .. }))
        .collect()
}

fn turns(flow: &Flow) -> Vec<String> {
    flow.fake
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::SendTurn { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn turn_end_records_the_last_turn_value_and_finishes_the_task() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    let mut client = flow.client().await;

    flow.claude_event(context_size(agent, 1_234)).await;
    flow.claude_event(turn_completed(agent)).await;

    let last = flow.engine.sessions.last_turn(session).unwrap();
    assert_eq!(last.active, 1_234);
    let stored = flow.engine.store.live_mains().await.unwrap();
    assert_eq!(stored[0].1.map(|last| last.active), Some(1_234));
    assert!(
        flow.engine
            .sessions
            .get(session)
            .unwrap()
            .idle_since
            .is_some()
    );
    assert!(!flow.engine.chat_is_running(flow.chat));
    assert!(
        flow.engine
            .store
            .unfinished_runs()
            .await
            .unwrap()
            .is_empty()
    );
    let done = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } if *state == TaskState::Done => Some(()),
            _ => None,
        })
        .await;
    assert_eq!(done, ());
}

#[tokio::test]
async fn answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(subagent_started(agent, "sub-1", None))
        .await;

    flow.claude_event(turn_completed(agent)).await;

    assert!(flow.engine.chat_is_running(flow.chat));
    let answered = client
        .until(|notification| match notification {
            Notification::TaskChanged { state, .. } => Some(*state),
            _ => None,
        })
        .await;
    assert_eq!(answered, TaskState::AnsweredTreeRunning);
    flow.claude_event(subagent_ended(agent, "sub-1")).await;
    assert!(!flow.engine.chat_is_running(flow.chat));
}

#[tokio::test]
async fn waiting_input_is_sent_after_the_turn_ends() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("then run the tests").await;
    assert_eq!(flow.state(waiting), InputState::Queued);

    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(turn_completed(agent)).await;

    assert_eq!(turns(&flow), vec!["fix the build", "then run the tests"]);
    assert_eq!(flow.state(waiting), InputState::Applied);
}

#[tokio::test]
async fn context_over_the_threshold_replaces_the_session_only_at_the_turn_boundary() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let old = flow.engine.flow.live[&agent].session;
    let mut client = flow.client().await;
    flow.claude_event(text(agent, "the cache is fixed")).await;

    flow.claude_event(context_size(agent, HUGE)).await;

    assert_eq!(opens(&flow).len(), 1);
    let live = flow.engine.flow.live[&agent].clone();
    let mid_turn = flow
        .engine
        .restart_session(
            flow.chat,
            &live,
            "p".to_owned(),
            None,
            crate::handoff::PacketEvidence::empty(LedgerSeq(1)),
        )
        .await;
    assert!(matches!(
        mid_turn,
        Err(EngineError::Session(
            saturn_core::sessions::SessionError::NotAtTurnBoundary
        ))
    ));

    flow.claude_event(turn_completed(agent)).await;

    let calls = opens(&flow);
    assert_eq!(calls.len(), 2);
    let Call::Open { packet, resume, .. } = &calls[1] else {
        panic!("second call should open a session");
    };
    assert_eq!(*resume, None);
    assert!(packet.as_deref().unwrap().contains("fix the build"));
    assert!(packet.as_deref().unwrap().contains("the cache is fixed"));
    let fresh = flow.engine.flow.live[&agent].clone();
    assert_ne!(fresh.session, old);
    assert_eq!(
        flow.engine.sessions.get(old).unwrap().state,
        SessionState::Ended
    );
    let record = flow.engine.sessions.get(fresh.session).unwrap();
    assert_eq!(record.state, SessionState::Open);
    assert_eq!(record.agent, agent);
    assert_eq!(record.delivered, LedgerSeq(3));
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. }
                if !matches!(notice, ChatNotice::CompletionEvidence { .. }) =>
            {
                Some(notice.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(notice, ChatNotice::Compacted);

    // 새 session이 패킷 턴에 도구 호출로 시작해 답하고 끝나면 그 턴이 연 실행도 끝나야 한다
    flow.claude_event(tool_read(agent, "packet-think", "src/cache.rs"))
        .await;
    flow.claude_event(text(agent, "Ready")).await;
    flow.claude_event(turn_completed(agent)).await;
    let open_runs = flow.engine.store.unfinished_runs().await.unwrap();
    assert!(open_runs.is_empty(), "{open_runs:?}");
}

#[tokio::test]
async fn restart_does_not_send_a_packet_without_its_recorded_dialogue() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(turn_completed(agent)).await;
    let live = flow.engine.flow.live[&agent].clone();

    let result = flow
        .engine
        .restart_session(
            flow.chat,
            &live,
            "changed packet".to_owned(),
            None,
            crate::handoff::PacketEvidence::empty(LedgerSeq(1)),
        )
        .await;

    assert!(matches!(
        result,
        Err(EngineError::Provider(
            saturn_core::providers::ProviderError::NotSent { .. }
        ))
    ));
    assert_eq!(opens(&flow).len(), 1);
}

#[tokio::test]
async fn compaction_is_postponed_while_an_input_is_waiting_to_merge() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.submit("then run the tests").await;
    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(context_size(agent, HUGE)).await;

    flow.claude_event(turn_completed(agent)).await;

    assert_eq!(opens(&flow).len(), 1);
    assert_eq!(turns(&flow), vec!["fix the build", "then run the tests"]);
}

#[tokio::test]
async fn unknown_context_size_leaves_compaction_to_the_provider() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    flow.claude_event(text(agent, "done")).await;

    flow.claude_event(turn_completed(agent)).await;

    assert_eq!(opens(&flow).len(), 1);
    assert_eq!(flow.engine.sessions.last_turn(session), None);
}

#[tokio::test]
async fn oversized_fixed_zone_defers_compaction_and_tells_the_user() {
    let mut flow = Flow::with_config(SMALL_CONTEXT, vec![idle_reply(0.95)]).await;
    flow.submit(&"x".repeat(400)).await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    flow.claude_event(text(agent, "done")).await;
    flow.claude_event(context_size(agent, HUGE)).await;

    flow.claude_event(turn_completed(agent)).await;

    assert_eq!(opens(&flow).len(), 1);
    let notice = client
        .until(|notification| match notification {
            Notification::ChatNotice { notice, .. }
                if !matches!(notice, ChatNotice::CompletionEvidence { .. }) =>
            {
                Some(notice.clone())
            }
            _ => None,
        })
        .await;
    assert!(matches!(notice, ChatNotice::ContextDeferred { .. }));
}

#[tokio::test]
async fn packet_turn_completion_is_not_the_end_of_the_task() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    flow.engine.flow.packet_turns.insert(agent, 1);

    flow.claude_event(turn_completed(agent)).await;
    assert!(flow.engine.chat_is_running(flow.chat));
    flow.claude_event(turn_completed(agent)).await;

    assert!(!flow.engine.chat_is_running(flow.chat));
}

/// 캐시 유지 시간(5분)을 넘긴 한 시간 전에 마지막 턴이 끝난 것으로 고친다.
fn backdate_last_turn(flow: &mut Flow, session: SessionId, active: u64) {
    let ended_at = SystemTime::now() - Duration::from_secs(3_600);
    flow.engine
        .sessions
        .record_last_turn(session, LastTurn { active, ended_at });
}

/// 첫 입력의 턴을 맥락 크기 `active`로 끝낸다. 돌려주는 값은 (에이전트, session).
async fn finished_first_turn(
    flow: &mut Flow,
    active: u64,
) -> (saturn_protocol::ids::AgentId, SessionId) {
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let session = flow.engine.flow.live[&agent].session;
    flow.claude_event(text(agent, "the cache is fixed")).await;
    flow.claude_event(context_size(agent, active)).await;
    flow.claude_event(turn_completed(agent)).await;
    (agent, session)
}

#[tokio::test]
async fn returning_after_the_cache_window_opens_a_new_session_when_the_packet_is_smaller() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (agent, old) = finished_first_turn(&mut flow, 50_000).await;
    backdate_last_turn(&mut flow, old, 50_000);

    let second = flow.submit("now run the tests").await;

    let calls = opens(&flow);
    assert_eq!(calls.len(), 2);
    let Call::Open { packet, resume, .. } = &calls[1] else {
        panic!("second call should open a session");
    };
    assert_eq!(*resume, None);
    let packet = packet.as_deref().expect("a packet should be handed over");
    assert!(packet.contains("fix the build"));
    assert!(packet.contains("the cache is fixed"));
    let fresh = flow.engine.flow.live[&agent].clone();
    assert_ne!(fresh.session, old);
    assert_eq!(
        flow.engine.sessions.get(old).unwrap().state,
        SessionState::Ended
    );
    assert_eq!(
        flow.engine.sessions.get(fresh.session).unwrap().agent,
        agent
    );
    assert_eq!(flow.state(second), InputState::Applied);
}

#[tokio::test]
async fn returning_inside_the_cache_window_keeps_the_session() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (agent, old) = finished_first_turn(&mut flow, 50_000).await;

    flow.submit("now run the tests").await;

    assert_eq!(opens(&flow).len(), 1);
    assert_eq!(flow.engine.flow.live[&agent].session, old);
    assert_eq!(turns(&flow), vec!["fix the build", "now run the tests"]);
}

#[tokio::test]
async fn returning_after_the_cache_window_keeps_the_session_when_the_packet_is_not_smaller() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let (agent, old) = finished_first_turn(&mut flow, 5).await;
    backdate_last_turn(&mut flow, old, 5);

    flow.submit("now run the tests").await;

    assert_eq!(opens(&flow).len(), 1);
    assert_eq!(flow.engine.flow.live[&agent].session, old);
}

#[tokio::test]
async fn provider_mode_neither_restarts_on_return_nor_at_the_threshold() {
    let mut flow = Flow::with_config(
        "[context]\nmode = \"provider\"\n",
        vec![idle_reply(0.95), idle_reply(0.95)],
    )
    .await;
    let (agent, old) = finished_first_turn(&mut flow, HUGE).await;
    assert_eq!(opens(&flow).len(), 1);
    backdate_last_turn(&mut flow, old, HUGE);

    flow.submit("now run the tests").await;

    assert_eq!(opens(&flow).len(), 1);
    assert_eq!(flow.engine.flow.live[&agent].session, old);
}

/// 접속 실행 설정으로 줄인 발동 기준. engine 시작 설정은 기본값이다.
fn small_context_override() -> Vec<(String, String)> {
    vec![
        ("provider.claude.context.t_abs".into(), "100000".into()),
        ("provider.claude.context.window".into(), "100000".into()),
    ]
}

// #490
#[tokio::test]
async fn connection_layer_context_settings_decide_compaction_at_the_turn_end() {
    #[derive(Clone, Copy)]
    enum Layer {
        ModeProvider,
        SmallBudget,
    }
    let cases = [
        ("mode provider skips compaction", Layer::ModeProvider),
        (
            "budget decides compaction and survives it",
            Layer::SmallBudget,
        ),
    ];

    for (name, layer) in cases {
        let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
        flow.engine.attachments.get_mut(&CLIENT).unwrap().overrides = match layer {
            Layer::ModeProvider => vec![("context.mode".into(), "provider".into())],
            Layer::SmallBudget => small_context_override(),
        };
        let input = flow.submit("fix the build").await;
        let (agent, chat) = (flow.agent(), flow.chat);
        let start = flow.engine.settings.current().unwrap();
        let pinned = flow.record(input).settings;
        match layer {
            Layer::ModeProvider => {}
            Layer::SmallBudget => {
                assert_ne!(pinned, start, "{name}");
                let budget = flow
                    .engine
                    .context_budget(chat, agent, crate::providers::test_support::CLAUDE)
                    .await
                    .unwrap();
                assert_eq!(budget.t_abs, 100_000, "{name}");
            }
        }

        match layer {
            Layer::ModeProvider => flow.claude_event(context_size(agent, HUGE)).await,
            Layer::SmallBudget => {
                flow.claude_event(text(agent, "the cache is fixed")).await;
                flow.claude_event(context_size(agent, 150_000)).await;
            }
        }
        flow.claude_event(turn_completed(agent)).await;

        let calls = opens(&flow);
        match layer {
            Layer::ModeProvider => {
                assert_eq!(calls.len(), 1, "{name}");
                assert_eq!(
                    flow.engine.context_mode(chat, agent).await.unwrap(),
                    crate::settings::ContextMode::Provider,
                    "{name}"
                );
            }
            Layer::SmallBudget => {
                assert_eq!(
                    calls.len(),
                    2,
                    "{name}: the chat's own threshold should compact"
                );
                let Call::Open { settings, .. } = &calls[1] else {
                    panic!("{name}: second call should open a session");
                };
                assert_eq!(*settings, pinned, "{name}");
                assert_eq!(
                    flow.engine.revision_of_agent(chat, agent).unwrap(),
                    pinned,
                    "{name}"
                );
            }
        }
    }
}

// #490
#[tokio::test]
async fn context_settings_of_two_chats_do_not_affect_each_other() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.engine.attachments.get_mut(&CLIENT).unwrap().overrides = small_context_override();
    flow.submit("fix the build").await;
    let (agent, chat) = (flow.agent(), flow.chat);
    let other = flow.open_other_chat().await;
    let claude = crate::providers::test_support::CLAUDE;

    let own = flow
        .engine
        .context_budget(chat, agent, claude)
        .await
        .unwrap();
    let theirs = flow
        .engine
        .context_budget(other.chat, other.agent, claude)
        .await
        .unwrap();

    assert_eq!(own.t_abs, 100_000);
    assert!(theirs.t_abs > 100_000);
}
