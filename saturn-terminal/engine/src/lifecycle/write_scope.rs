//! 쓰기 범위 겹침 테스트: 작업 폴더와 더한 폴더가 겹치는 채팅끼리는 쓰기 작업이 한 번에 하나만 실행된다.

use std::path::PathBuf;

use saturn_protocol::ids::{AgentId, ChatId, InputId};
use saturn_protocol::state::{InputState, QueueReason};

use super::support::{Flow, idle_reply};
use crate::providers::test_support::Call;

async fn flow_with_one_running_writer() -> Flow {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    flow
}

fn made_in_workdir(flow: &Flow, name: &str) -> PathBuf {
    let dir = flow.fixture.workdir.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn made_beside_workdir(flow: &Flow, name: &str) -> PathBuf {
    let dir = flow.fixture.workdir.with_file_name(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn is_waiting_for_write_turn(flow: &Flow, input: InputId) -> bool {
    flow.state(input) == InputState::Queued
        && flow.record(input).reason == Some(QueueReason::WriteTurn)
}

// #327
#[tokio::test]
async fn write_in_a_subfolder_waits_for_a_write_in_the_parent_folder() {
    let mut flow = flow_with_one_running_writer().await;
    let sub = made_in_workdir(&flow, "sub");

    let (_, _, input) = flow.submit_in_chat_at(sub, &[]).await;

    assert!(is_waiting_for_write_turn(&flow, input));
}

// #327
#[tokio::test]
async fn write_in_the_parent_folder_waits_for_a_write_in_a_subfolder() {
    let mut flow = flow_with_one_running_writer().await;
    let parent = flow
        .fixture
        .workdir
        .parent()
        .expect("the workdir should have a parent")
        .to_path_buf();

    let (_, _, input) = flow.submit_in_chat_at(parent, &[]).await;

    assert!(is_waiting_for_write_turn(&flow, input));
}

// #327
#[tokio::test]
async fn writes_with_a_shared_added_folder_wait_for_each_other() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let shared = made_beside_workdir(&flow, "shared");
    let first = flow.chat;
    flow.engine
        .register_dir(first, shared.canonicalize().unwrap())
        .await
        .unwrap();
    flow.submit("fix the build").await;
    let other_work = made_beside_workdir(&flow, "other-work");

    let (_, _, input) = flow
        .submit_in_chat_at(other_work, &[shared.canonicalize().unwrap()])
        .await;

    assert!(is_waiting_for_write_turn(&flow, input));
}

#[tokio::test]
async fn write_in_a_folder_that_only_shares_a_name_prefix_runs_at_once() {
    let mut flow = flow_with_one_running_writer().await;
    let beside = made_beside_workdir(&flow, "work-extra");

    let (_, _, input) = flow.submit_in_chat_at(beside, &[]).await;

    assert_ne!(flow.state(input), InputState::Queued);
}

// #327
#[cfg(unix)]
#[tokio::test]
async fn write_in_a_symlink_to_a_subfolder_waits_for_a_write_in_the_parent_folder() {
    let mut flow = flow_with_one_running_writer().await;
    let sub = made_in_workdir(&flow, "sub");
    let link = flow.fixture.workdir.with_file_name("sub-link");
    std::os::unix::fs::symlink(&sub, &link).unwrap();

    let (_, _, input) = flow.submit_in_chat_at(link, &[]).await;

    assert!(is_waiting_for_write_turn(&flow, input));
}

// #455
#[tokio::test]
async fn added_folder_after_acceptance_still_serializes_writes() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let shared = made_beside_workdir(&flow, "late-shared");
    let (_, _, other_input) = flow.submit_in_chat_at(shared.clone(), &[]).await;
    assert_eq!(flow.state(other_input), InputState::Applied);
    let input = flow.accept_only("write to the newly added folder").await;
    flow.engine
        .add_dir(super::support::CLIENT, flow.chat, shared.to_str().unwrap())
        .await
        .unwrap();
    flow.engine.advance(flow.chat).await;
    flow.settle().await;
    assert!(
        is_waiting_for_write_turn(&flow, input),
        "input must wait for the other writer in the newly added folder; got {:?}; provider calls {:?}",
        flow.state(input),
        flow.fake.calls()
    );
}

// #488
async fn held_input_with_other_chat_writing_in_shared() -> (Flow, ChatId, AgentId, InputId, PathBuf)
{
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let shared = made_beside_workdir(&flow, "held-shared");
    let (other_chat, _, other_input) = flow.submit_in_chat_at(shared.clone(), &[]).await;
    assert_eq!(flow.state(other_input), InputState::Applied);
    let other_agent = flow
        .engine
        .flow
        .live
        .values()
        .find(|live| {
            flow.engine
                .session_chat(live.session)
                .is_ok_and(|owner| owner == other_chat)
        })
        .map(|live| live.agent)
        .expect("the other chat should have a session");
    let input = flow.accept_only("write after resume").await;
    flow.engine.queue.stop(flow.chat);
    assert_eq!(flow.state(input), InputState::Held);
    (flow, other_chat, other_agent, input, shared)
}

fn sends_to_provider(flow: &Flow, text: &str) -> usize {
    flow.fake
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::SendTurn { text: sent, .. } if sent == text))
        .count()
}

// #488
#[tokio::test]
async fn held_input_resumed_after_adding_a_folder_waits_for_the_write_turn() {
    let (mut flow, _, _, input, shared) = held_input_with_other_chat_writing_in_shared().await;
    flow.engine
        .add_dir(super::support::CLIENT, flow.chat, shared.to_str().unwrap())
        .await
        .unwrap();
    flow.engine.queue.resume(flow.chat, None);
    flow.engine.advance(flow.chat).await;
    flow.settle().await;

    assert!(
        is_waiting_for_write_turn(&flow, input),
        "resumed writer must wait; state={:?}, calls={:?}",
        flow.state(input),
        flow.fake.calls()
    );
    assert_eq!(sends_to_provider(&flow, "write after resume"), 0);
}

// #488
#[tokio::test]
async fn held_input_resumed_after_attaching_with_an_added_folder_waits_for_the_write_turn() {
    let (mut flow, _, _, input, shared) = held_input_with_other_chat_writing_in_shared().await;
    let (_client, _) = flow
        .attach_with_dirs(vec![shared.canonicalize().unwrap().display().to_string()])
        .await;
    flow.engine.queue.resume(flow.chat, None);
    flow.engine.advance(flow.chat).await;
    flow.settle().await;

    assert!(
        is_waiting_for_write_turn(&flow, input),
        "resumed writer must wait; state={:?}, calls={:?}",
        flow.state(input),
        flow.fake.calls()
    );
    assert_eq!(sends_to_provider(&flow, "write after resume"), 0);
}

// #488
#[tokio::test]
async fn held_input_sends_once_after_the_shared_write_turn_is_released() {
    let (mut flow, other_chat, other_agent, input, shared) =
        held_input_with_other_chat_writing_in_shared().await;
    flow.engine
        .add_dir(super::support::CLIENT, flow.chat, shared.to_str().unwrap())
        .await
        .unwrap();
    flow.engine.queue.resume(flow.chat, None);
    flow.engine.advance(flow.chat).await;
    flow.settle().await;
    assert!(is_waiting_for_write_turn(&flow, input));

    flow.engine
        .finish_task(other_chat, other_agent)
        .await
        .unwrap();
    flow.settle().await;

    assert_eq!(flow.state(input), InputState::Applied);
    assert_eq!(sends_to_provider(&flow, "write after resume"), 1);
}

// #488
#[tokio::test]
async fn held_input_without_an_added_folder_still_runs_after_resume() {
    let (mut flow, _, _, input, _) = held_input_with_other_chat_writing_in_shared().await;
    flow.engine.queue.resume(flow.chat, None);
    flow.engine.advance(flow.chat).await;
    flow.settle().await;

    assert_eq!(flow.state(input), InputState::Applied);
    assert_eq!(sends_to_provider(&flow, "write after resume"), 1);
}

// #488
#[cfg(unix)]
#[tokio::test]
async fn folder_added_while_connecting_does_not_widen_the_session_beyond_the_write_scope() {
    use std::os::unix::fs::PermissionsExt;

    use saturn_protocol::ids::Provider;

    use crate::chat_env::ChatEnv;
    use crate::providers::test_support::{FakeAdapter, FakeProvider, Stall, fake_descriptor};

    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let shared = made_beside_workdir(&flow, "connecting-shared");
    let (_, _, other_input) = flow.submit_in_chat_at(shared.clone(), &[]).await;
    assert_eq!(flow.state(other_input), InputState::Applied);

    let agent = Provider::from_static("fake-agent");
    let slow = FakeProvider::new(agent);
    flow.engine
        .registry
        .register(std::sync::Arc::new(FakeAdapter {
            descriptor: fake_descriptor(agent),
            provider: slow.clone(),
        }))
        .unwrap();
    let bin = flow.fixture.root.path().join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let program = bin.join("fake-agent");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let chat = flow.chat;
    flow.engine
        .providers
        .remove(&(chat, crate::providers::test_support::CLAUDE));
    flow.engine.chats.insert(
        chat,
        ChatEnv::new(
            flow.fixture.workdir.clone(),
            vec![("PATH".to_owned(), bin.display().to_string())],
        ),
    );
    slow.stall(Stall::Connect);

    flow.engine
        .submit_input(
            super::support::CLIENT,
            chat,
            1,
            "write while connecting".to_owned(),
            false,
        )
        .await
        .unwrap();
    // 연결이 막혀 있으므로 판단 결과가 적용돼 전송 중이 될 때까지만 돌린다
    loop {
        let engine = &mut flow.engine.flow;
        tokio::select! {
            Some(done) = engine.router_rx.recv() => flow.engine.on_routed(done).await,
            Some(message) = engine.provider_rx.recv() => flow.engine.on_provider_msg(message).await,
            () = tokio::time::sleep(std::time::Duration::from_millis(500)) => break,
        }
    }
    let input = flow
        .engine
        .store
        .history_page(chat, None, 500)
        .await
        .unwrap()
        .entries
        .into_iter()
        .filter_map(|entry| match entry {
            crate::store::HistoryEntry::Input { input, .. } => Some(input),
            _ => None,
        })
        .max()
        .unwrap();
    assert_eq!(flow.state(input), InputState::Delivering);

    flow.engine
        .add_dir(super::support::CLIENT, chat, shared.to_str().unwrap())
        .await
        .unwrap();
    slow.release(Stall::Connect);
    flow.settle().await;

    let opened: Vec<Vec<PathBuf>> = slow
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Open { add_dirs, .. } => Some(add_dirs),
            _ => None,
        })
        .collect();
    assert_eq!(opened, vec![Vec::<PathBuf>::new()]);
    assert!(
        !flow
            .record(input)
            .write_scope
            .iter()
            .any(|dir| dir.starts_with(&shared)),
        "the lock scope was fixed before the folder was added"
    );
}

/// 실행 중인 쓰기 작업 하나와, 그 작업과 무관하다고 판단돼 새 작업으로 기다리는 쓰기 입력 하나를 둔 채 멈춘다.
async fn stopped_with_a_new_task_waiting_for_the_write_lock() -> (Flow, AgentId, InputId) {
    use super::support::{idle_reply, running_reply, turn_completed};

    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "independent", "spawn"),
    ])
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("also write elsewhere").await;
    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    assert_eq!(flow.state(waiting), InputState::Held);
    (flow, agent, waiting)
}

fn state_check_turns(flow: &Flow) -> usize {
    flow.fake
        .calls()
        .iter()
        .filter(|call| {
            matches!(call, Call::SendTurn { text, .. } if text.starts_with("Previous turn result"))
        })
        .count()
}

// #519
#[tokio::test]
async fn continue_sends_the_state_check_even_when_a_new_task_waits_for_the_stopped_write_lock() {
    let (mut flow, agent, waiting) = stopped_with_a_new_task_waiting_for_the_write_lock().await;

    flow.engine.continue_held(flow.chat, None).await.unwrap();
    flow.settle().await;

    assert_eq!(state_check_turns(&flow), 1, "calls={:?}", flow.fake.calls());
    assert!(is_waiting_for_write_turn(&flow, waiting));
    flow.claude_event(super::support::turn_completed(agent))
        .await;
    flow.settle().await;
    assert_ne!(
        flow.state(waiting),
        InputState::Queued,
        "the new task should start once the resumed task ends"
    );
}

// #519
#[tokio::test]
async fn continue_after_adding_a_folder_sends_the_state_check_while_a_new_task_waits() {
    let (mut flow, _, waiting) = stopped_with_a_new_task_waiting_for_the_write_lock().await;
    let extra = made_beside_workdir(&flow, "continue-extra");

    flow.engine
        .add_dir(
            super::support::CLIENT,
            flow.chat,
            extra.canonicalize().unwrap().to_str().unwrap(),
        )
        .await
        .unwrap();
    flow.engine.continue_held(flow.chat, None).await.unwrap();
    flow.settle().await;

    assert_eq!(state_check_turns(&flow), 1, "calls={:?}", flow.fake.calls());
    assert!(is_waiting_for_write_turn(&flow, waiting));
}
