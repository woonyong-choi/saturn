//! 입력 접수 테스트: 기록 저장소에 접수된 뒤에만 provider로 보낸다.

use saturn_core::queue::QueueError;
use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::ids::{ChatId, InputId, ProviderSessionId};
use saturn_protocol::state::InputState;

use super::support::{
    CLIENT, Flow, idle_held_reply, idle_reply, running_held_reply, running_reply, turn_completed,
};
use super::*;
use crate::chat_env::ChatEnv;
use crate::providers::test_support::{Call, FakeProvider};
use crate::rpc::ClientId;
use crate::secrets::ROUTER_KEY_ENV;
use crate::store::HistoryEntry;

#[tokio::test]
async fn record_write_failure_sends_nothing_anywhere() {
    let mut flow = Flow::new(Vec::new()).await;
    let workdir = flow.fixture.workdir.clone();
    flow.engine
        .settings
        .apply_trusted(&flow.engine.store, Some(flow.chat), &workdir, &[])
        .await
        .unwrap();
    flow.engine.store.deny_writes().await;

    let error = flow
        .engine
        .submit_input(CLIENT, flow.chat, 1, "fix the build".to_owned(), false)
        .await
        .unwrap_err();

    assert!(matches!(error, EngineError::Store(_)));
    assert!(flow.fake.calls().is_empty());
    assert_eq!(flow.router_calls(), 0);
    assert_eq!(flow.engine.queue.next_to_route(flow.chat), None);
    flow.engine.store.allow_writes().await;
    let entries = flow
        .engine
        .store
        .history_page(flow.chat, None, 10)
        .await
        .unwrap()
        .entries;
    assert!(entries.is_empty());
}

#[tokio::test]
async fn accepted_input_reaches_first_provider_after_it_is_recorded() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;

    let input = flow.submit("fix the build").await;

    let session = ProviderSessionId("fake-session-1".to_owned());
    assert_eq!(
        flow.fake.calls(),
        vec![
            Call::Open {
                agent: flow.agent(),
                model: None,
                resume: None,
                packet: None,
                add_dirs: Vec::new(),
                interrupted_children: Vec::new(),
                settings: flow.engine.settings.current().unwrap(),
            },
            Call::SendTurn {
                session,
                text: "fix the build".to_owned()
            },
        ]
    );
    assert_eq!(flow.state(input), InputState::Applied);
    let entries = flow
        .engine
        .store
        .history_page(flow.chat, None, 10)
        .await
        .unwrap()
        .entries;
    assert!(matches!(
        entries.as_slice(),
        [
            HistoryEntry::Input { state: InputState::Applied, text, .. },
            HistoryEntry::Run { end: None, .. },
        ] if text == "fix the build"
    ));
}

#[tokio::test]
async fn unattached_client_cannot_submit() {
    let mut flow = Flow::new(Vec::new()).await;

    let wrong_client = flow
        .engine
        .submit_input(ClientId(2), flow.chat, 1, "hi".to_owned(), false)
        .await
        .unwrap_err();
    let wrong_chat = flow
        .engine
        .submit_input(CLIENT, ChatId(99), 1, "hi".to_owned(), false)
        .await
        .unwrap_err();

    assert!(matches!(wrong_client, EngineError::ChatNotAttached { .. }));
    assert!(matches!(wrong_chat, EngineError::ChatNotAttached { .. }));
    assert_eq!(wrong_chat.code(), INVALID_PARAMS);
    assert!(flow.fake.calls().is_empty());
}

#[tokio::test]
async fn inputs_are_routed_one_at_a_time_in_accept_order() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;

    flow.submit("first request").await;
    let second = flow.submit("second request").await;

    let bodies: Vec<String> = flow
        .transport
        .calls()
        .into_iter()
        .filter_map(|call| call.2)
        .collect();
    assert!(bodies[1].contains("first request"));
    assert!(bodies[2].contains("second request"));
    assert!(bodies[2].contains("chat: running"));
    assert_eq!(flow.state(second), InputState::Queued);
    assert_eq!(flow.router_calls(), 2);
}

#[tokio::test]
async fn first_input_prefers_claude_then_codex() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let chat = flow.chat;
    flow.engine
        .providers
        .remove(&(chat, crate::providers::test_support::CLAUDE));
    let codex = FakeProvider::new(crate::providers::test_support::CODEX);
    flow.engine.add_connection(chat, codex.connection());

    let input = flow.submit("hello").await;

    assert_eq!(flow.state(input), InputState::Applied);
    assert_eq!(codex.calls().len(), 2);
    let main = flow.engine.sessions.live_main(chat).unwrap();
    assert_eq!(main.provider, crate::providers::test_support::CODEX);
}

#[tokio::test]
async fn first_input_without_any_provider_is_rejected_and_sent_nowhere() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let chat = flow.chat;
    flow.engine
        .providers
        .remove(&(chat, crate::providers::test_support::CLAUDE));

    let input = flow.submit("hello").await;

    assert_eq!(flow.state(input), InputState::Rejected);
    assert!(flow.fake.calls().is_empty());
    let entries = flow
        .engine
        .store
        .history_page(chat, None, 10)
        .await
        .unwrap()
        .entries;
    assert!(matches!(
        entries.as_slice(),
        [HistoryEntry::Input {
            state: InputState::Rejected,
            ..
        }]
    ));
}

#[tokio::test]
async fn pinned_model_input_is_judged_and_goes_to_the_session_with_that_model() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let model = saturn_protocol::rpc::ModelChoice {
        provider: crate::providers::test_support::CLAUDE,
        model: "the-pinned-model".to_owned(),
    };

    let input = flow.submit_with("hello", Some(model), false).await;

    assert_eq!(flow.router_calls(), 1);
    assert_eq!(flow.state(input), InputState::Applied);
    assert!(matches!(
        &flow.fake.calls()[0],
        Call::Open { model: Some(model), .. } if model == "the-pinned-model"
    ));
}

#[tokio::test]
async fn launch_spec_takes_workdir_and_env_from_the_chat() {
    let flow = Flow::new(Vec::new()).await;
    let mut engine = flow.engine;
    let chat = flow.chat;
    let other = flow.fixture.root.path().join("other");
    engine.chats.insert(
        chat,
        ChatEnv::new(
            other.clone(),
            vec![
                ("PATH".to_owned(), "/opt/tui/bin".to_owned()),
                (ROUTER_KEY_ENV.to_owned(), "sk-secret".to_owned()),
            ],
        ),
    );
    let revision = engine.settings.current().unwrap();

    let launch = engine
        .launch_spec(crate::providers::test_support::CLAUDE, chat, revision)
        .await
        .unwrap();

    assert_eq!(launch.workdir, other);
    assert_eq!(launch.program, PathBuf::from("claude"));
    assert_eq!(launch.settings, revision);
    assert!(
        launch
            .env
            .iter()
            .all(|(name, _)| name != ROUTER_KEY_ENV && name != "SATURN_KEY")
    );
    assert!(launch.env.iter().any(|(name, _)| name == "PATH"));
    assert!(launch.hook_settings.is_some());
    let denied: Vec<String> = launch
        .key_deny_read
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    assert!(denied.contains(&"/Library/Keychains".to_owned()));
    assert!(denied.iter().any(|path| path.ends_with("router.key")));
}

#[tokio::test]
async fn unknown_input_is_invalid_params() {
    let mut flow = Flow::new(Vec::new()).await;

    let error = flow
        .engine
        .cancel_input(CLIENT, InputId(77))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        EngineError::Queue(QueueError::NotFound(InputId(77)))
    ));
    assert_eq!(error.code(), INVALID_PARAMS);
}

fn router_bodies(flow: &Flow) -> Vec<String> {
    flow.transport
        .calls()
        .into_iter()
        .filter_map(|call| call.2)
        .collect()
}

/// 작업 하나와 대기 입력 하나를 멈춰 보류로 만든다. 보류된 대기 입력을 돌려준다.
async fn hold_work(flow: &mut Flow) -> InputId {
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("also run the tests").await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    assert_eq!(flow.state(waiting), InputState::Held);
    waiting
}

#[tokio::test]
async fn resume_held_is_asked_only_when_the_chat_has_held_work() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        idle_held_reply(0.95, 0.1),
    ])
    .await;
    hold_work(&mut flow).await;

    flow.submit("what changed so far").await;

    let bodies = router_bodies(&flow);
    assert!(!bodies[1].contains("resume_held"));
    assert!(!bodies[2].contains("resume_held"));
    assert!(bodies[3].contains("resume_held"));
}

#[tokio::test]
async fn resume_intent_at_threshold_resumes_every_held_task() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        idle_held_reply(0.95, 0.85),
    ])
    .await;
    let waiting = hold_work(&mut flow).await;

    flow.submit("yes, go on with the build").await;

    assert_ne!(flow.state(waiting), InputState::Held);
    assert!(!flow.engine.queue.has_held_task(flow.chat));
}

#[tokio::test]
async fn resume_intent_below_threshold_keeps_the_work_held_and_counts_the_input() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        idle_held_reply(0.95, 0.84),
    ])
    .await;
    let waiting = hold_work(&mut flow).await;

    flow.submit("what changed so far").await;

    assert!(router_bodies(&flow)[3].contains("resume_held"));
    assert_eq!(flow.state(waiting), InputState::Held);
    assert!(flow.engine.queue.has_held_task(flow.chat));
}

#[tokio::test]
async fn third_input_without_resume_intent_closes_the_held_work() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        idle_held_reply(0.95, 0.1),
        running_held_reply(0.95, "continues", "queue", 0.1),
        running_held_reply(0.95, "continues", "queue", 0.1),
    ])
    .await;
    let waiting = hold_work(&mut flow).await;
    let session = flow.engine.flow.live.values().next().unwrap().session;

    flow.submit("what changed so far").await;
    flow.submit("and the second thing").await;
    assert_eq!(flow.state(waiting), InputState::Held);
    flow.submit("and the third thing").await;

    assert_eq!(flow.state(waiting), InputState::Cancelled);
    assert!(!flow.engine.queue.has_held_task(flow.chat));
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().state,
        saturn_protocol::state::SessionState::Ended
    );
}

#[tokio::test]
async fn provider_mode_leaves_out_the_auto_compact_safety_net() {
    let flow = Flow::with_config("[context]\nmode = \"provider\"\n", Vec::new()).await;
    let saturn = Flow::new(Vec::new()).await;

    let revision = flow.engine.settings.current().unwrap();
    let provider = flow
        .engine
        .launch_spec(crate::providers::test_support::CLAUDE, flow.chat, revision)
        .await
        .unwrap();
    let revision = saturn.engine.settings.current().unwrap();
    let default = saturn
        .engine
        .launch_spec(
            crate::providers::test_support::CLAUDE,
            saturn.chat,
            revision,
        )
        .await
        .unwrap();

    assert_eq!(provider.defaults.auto_compact_tokens, None);
    assert!(default.defaults.auto_compact_tokens.is_some());
}

#[tokio::test]
async fn inputs_without_a_resume_judgment_never_close_the_held_work() {
    // 두 번째 이후 답에 `resume_held`가 없으면 판단이 없는 것이다
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
        running_reply(0.95, "continues", "queue"),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    let waiting = hold_work(&mut flow).await;

    for text in ["one", "two", "three", "four"] {
        flow.submit(text).await;
    }

    assert!(router_bodies(&flow)[3].contains("resume_held"));
    assert_eq!(flow.state(waiting), InputState::Held);
    assert!(flow.engine.queue.has_held_task(flow.chat));
}

#[tokio::test]
async fn attachment_read_only_override_is_applied_to_input() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.engine.attachments.get_mut(&CLIENT).unwrap().overrides =
        vec![("permission.mode".into(), "read-only".into())];
    let input = flow.submit("inspect only").await;
    assert_eq!(
        flow.record(input).permission,
        saturn_core::queue::Permission::ReadOnly,
        "attachment -c permission.mode=read-only must constrain the input"
    );
}

#[tokio::test]
async fn chat_settings_do_not_leak_between_chats() {
    let mut flow = Flow::new(Vec::new()).await;
    let first_dir = flow.fixture.workdir.clone();
    let (first, _) = flow
        .engine
        .settings
        .apply_trusted(&flow.engine.store, Some(flow.chat), &first_dir, &[])
        .await
        .unwrap();
    let other_dir = first_dir.with_file_name("other-chat");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other = flow
        .engine
        .store
        .create_chat(other_dir.clone())
        .await
        .unwrap();
    flow.engine
        .store
        .set_chat_layer(other, "[permission]\nmode = \"read-only\"\n")
        .await
        .unwrap();
    let (second, _) = flow
        .engine
        .settings
        .apply_trusted(&flow.engine.store, Some(other), &other_dir, &[])
        .await
        .unwrap();
    assert_ne!(first.revision, second.revision);
    assert!(
        !flow
            .engine
            .settings
            .changed(Some(flow.chat), &first_dir, &[])
            .await
            .unwrap()
    );
    flow.engine
        .submit_input(CLIENT, flow.chat, 73, "first chat again".into(), true)
        .await
        .unwrap();
    let queue = &flow.engine.queue;
    let id = queue
        .inputs_in_state(flow.chat, InputState::Delivering)
        .first()
        .copied()
        .or_else(|| {
            queue
                .inputs_in_state(flow.chat, InputState::Queued)
                .first()
                .copied()
        })
        .unwrap();
    assert_eq!(
        queue.input(id).unwrap().settings,
        first.revision,
        "first chat must retain its own settings revision"
    );
}

#[tokio::test]
async fn read_only_and_full_chats_alternate_without_mixing_permissions() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95), idle_reply(0.95)]).await;
    let strict_dir = flow.fixture.workdir.with_file_name("strict-work");
    std::fs::create_dir_all(&strict_dir).unwrap();
    let strict = flow
        .engine
        .store
        .create_chat(strict_dir.clone())
        .await
        .unwrap();
    flow.engine
        .store
        .set_chat_layer(strict, "[permission]\nmode = \"read-only\"\n")
        .await
        .unwrap();
    flow.engine.chats.insert(
        strict,
        ChatEnv::new(
            strict_dir,
            vec![("PATH".to_owned(), "/nonexistent".to_owned())],
        ),
    );
    flow.engine.attachments.insert(
        crate::lifecycle::support::OTHER_CLIENT,
        crate::Attachment {
            chat: strict,
            overrides: Vec::new(),
            folder_trust: None,
        },
    );
    let mut seen = Vec::new();
    for (client, chat) in [
        (CLIENT, flow.chat),
        (crate::lifecycle::support::OTHER_CLIENT, strict),
        (CLIENT, flow.chat),
    ] {
        flow.engine
            .submit_input(client, chat, 1, "work".to_owned(), true)
            .await
            .unwrap();
        let page = flow
            .engine
            .store
            .history_page(chat, None, 500)
            .await
            .unwrap();
        let id = page
            .entries
            .into_iter()
            .filter_map(|entry| match entry {
                crate::store::HistoryEntry::Input { input, .. } => Some(input),
                _ => None,
            })
            .max()
            .unwrap();
        let queue = &flow.engine.queue;
        seen.push(queue.input(id).unwrap().permission);
        flow.settle().await;
    }
    use saturn_core::queue::Permission::{ReadOnly, Write};
    assert_eq!(seen, vec![Write, ReadOnly, Write]);
}

// #456
#[tokio::test]
async fn applied_steer_is_preserved_in_handoff() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "refines", "steer"),
    ])
    .await;
    flow.fake.verify_steer();
    flow.submit("implement authentication").await;
    let steer = flow
        .submit("use steered_unique_refinement_branch for this work")
        .await;
    assert_eq!(flow.state(steer), InputState::Applied);
    let agent = flow.agent();
    flow.claude_event(super::support::text(agent, "done")).await;
    flow.claude_event(turn_completed(agent)).await;
    let rows = flow
        .engine
        .store
        .ledger_since(flow.chat, saturn_protocol::ids::LedgerSeq(0))
        .await
        .unwrap();
    let steers = flow.engine.store.steered_inputs(flow.chat).await.unwrap();
    let budget = flow
        .engine
        .context_budget(flow.chat, agent, crate::providers::test_support::CLAUDE)
        .await
        .unwrap();
    let packet = crate::handoff::build_handoff(
        &rows,
        &steers,
        &[],
        &flow.engine.pending_work(flow.chat, None),
        &[],
        &budget,
    );
    let crate::handoff::HandoffOutcome::Ready(packet) = packet else {
        panic!("packet should exist")
    };
    assert!(
        packet.text.contains("steered_unique_refinement_branch"),
        "applied steer disappeared from handoff: {}",
        packet.text
    );
}

// #458
fn current_id(
    flow: &Flow,
    provider: saturn_protocol::ids::Provider,
) -> crate::providers::ConnectionId {
    flow.engine.providers[&(flow.chat, provider)].id()
}

// #458
#[tokio::test]
async fn late_close_from_replaced_connection_does_not_remove_new_connection() {
    let mut flow = Flow::new(Vec::new()).await;
    let provider = crate::providers::test_support::CLAUDE;
    let old = current_id(&flow, provider);
    let replacement = FakeProvider::new(provider);
    flow.engine
        .add_connection(flow.chat, replacement.connection());

    flow.engine
        .on_provider_msg(crate::providers::ProviderMsg::Closed {
            chat: flow.chat,
            provider,
            connection: old,
        })
        .await;

    assert!(
        flow.engine.providers.contains_key(&(flow.chat, provider)),
        "old close must not remove replacement connection"
    );
    assert_ne!(current_id(&flow, provider), old);
}

// #458
#[tokio::test]
async fn close_of_the_current_connection_still_removes_it() {
    let mut flow = Flow::new(Vec::new()).await;
    let provider = crate::providers::test_support::CLAUDE;
    let current = current_id(&flow, provider);

    flow.engine
        .on_provider_msg(crate::providers::ProviderMsg::Closed {
            chat: flow.chat,
            provider,
            connection: current,
        })
        .await;

    assert!(!flow.engine.providers.contains_key(&(flow.chat, provider)));
}

// #458
#[tokio::test]
async fn late_event_and_loss_from_a_replaced_connection_do_not_touch_the_new_run() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let provider = crate::providers::test_support::CLAUDE;
    let input = flow.submit("fix the build").await;
    let agent = flow.agent();
    let old = current_id(&flow, provider);
    let replacement = FakeProvider::new(provider);
    flow.engine
        .add_connection(flow.chat, replacement.connection());

    flow.engine
        .on_provider_msg(crate::providers::ProviderMsg::Event {
            chat: flow.chat,
            provider,
            connection: old,
            event: turn_completed(agent),
        })
        .await;
    flow.engine
        .on_provider_msg(crate::providers::ProviderMsg::Lost {
            chat: flow.chat,
            provider,
            connection: old,
        })
        .await;

    assert!(flow.engine.runs.active.contains_key(&agent));
    assert_eq!(flow.state(input), InputState::Applied);
    assert!(flow.engine.providers.contains_key(&(flow.chat, provider)));
    assert!(flow.engine.flow.live.contains_key(&agent));
}

// #458
#[tokio::test]
async fn reply_from_another_connection_does_not_advance_a_waiting_delivery() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let provider = crate::providers::test_support::CLAUDE;
    flow.fake.stall(crate::providers::test_support::Stall::Open);
    flow.engine
        .submit_input(CLIENT, flow.chat, 1, "work".to_owned(), false)
        .await
        .unwrap();
    while !flow.is_delivering() {
        let engine_flow = &mut flow.engine.flow;
        tokio::select! {
            Some(done) = engine_flow.router_rx.recv() => flow.engine.on_routed(done).await,
            Some(message) = engine_flow.provider_rx.recv() => flow.engine.on_provider_msg(message).await,
            () = tokio::time::sleep(std::time::Duration::from_secs(10)) => panic!("delivery should be waiting"),
        }
    }
    let replacement = FakeProvider::new(provider);
    flow.engine
        .add_connection(flow.chat, replacement.connection());
    let stranger = current_id(&flow, provider);

    flow.engine
        .on_provider_msg(crate::providers::ProviderMsg::Reply {
            chat: flow.chat,
            provider,
            connection: Some(stranger),
            reply: crate::providers::Reply::Opened(Err(
                saturn_core::providers::ProviderError::ConnectionLost,
            )),
        })
        .await;

    assert!(
        flow.is_delivering(),
        "a stranger's reply must not settle it"
    );
    flow.fake
        .release(crate::providers::test_support::Stall::Open);
    flow.settle().await;
    assert!(!flow.is_delivering());
}
