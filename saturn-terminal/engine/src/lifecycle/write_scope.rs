//! 쓰기 범위 겹침 테스트: 작업 폴더와 더한 폴더가 겹치는 채팅끼리는 쓰기 작업이 한 번에 하나만 실행된다.

use std::path::PathBuf;

use saturn_protocol::ids::InputId;
use saturn_protocol::state::{InputState, QueueReason};

use super::support::{Flow, idle_reply};

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
