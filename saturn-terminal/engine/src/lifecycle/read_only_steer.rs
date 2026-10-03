//! 읽기 전용 실행에 끼워 넣기 테스트: 읽기 전용으로 접수해 쓰기 잠금 없이 도는 실행에는 쓰기 입력을 끼워 넣지 않는다.
//! 설계: docs/design/input-handling.md#쓰기-규칙

use saturn_core::providers::ProviderError;
use saturn_protocol::event::PermissionTool;
use saturn_protocol::rpc::PermissionAnswer;
use saturn_protocol::state::{InputState, QueueReason};

use super::support::{Flow, idle_reply, permission_for, running_reply, turn_completed};
use crate::providers::test_support::Call;

fn sent(flow: &Flow) -> (Vec<String>, Vec<String>) {
    let (mut turns, mut steers) = (Vec::new(), Vec::new());
    for call in flow.fake.calls() {
        match call {
            Call::SendTurn { text, .. } => turns.push(text),
            Call::Steer { text, .. } => steers.push(text),
            _ => {}
        }
    }
    (turns, steers)
}

async fn read_only_run_then_write_mode() -> Flow {
    let mut flow = Flow::with_config(
        "[permission]\nmode = \"read-only\"\n",
        vec![idle_reply(0.95), running_reply(0.95, "refines", "steer")],
    )
    .await;
    flow.fake.verify_steer();
    flow.submit("look around").await;
    flow.engine
        .set_permission_mode(flow.chat, "edit")
        .await
        .unwrap();
    flow
}

// #354
#[tokio::test]
async fn write_input_is_not_steered_into_a_read_only_run_and_waits_for_a_write_turn() {
    let mut flow = read_only_run_then_write_mode().await;

    let write = flow.submit("also write the cache module").await;

    let (turns, steers) = sent(&flow);
    assert_eq!(steers, Vec::<String>::new());
    assert_eq!(turns, vec!["look around"]);
    assert_eq!(flow.state(write), InputState::Queued);
    assert_eq!(flow.record(write).reason, Some(QueueReason::WriteTurn));
}

// #354
#[tokio::test]
async fn waiting_write_input_takes_a_new_turn_after_the_read_only_run_ends() {
    let mut flow = read_only_run_then_write_mode().await;
    let write = flow.submit("also write the cache module").await;

    flow.claude_event(turn_completed(flow.agent())).await;
    flow.settle().await;

    let (turns, steers) = sent(&flow);
    assert_eq!(steers, Vec::<String>::new());
    assert_eq!(turns, vec!["look around", "also write the cache module"]);
    assert_eq!(flow.state(write), InputState::Applied);
}

// #354
#[tokio::test]
async fn write_input_that_would_be_sent_as_a_new_turn_cannot_write_during_a_read_only_run() {
    let mut flow = read_only_run_then_write_mode().await;
    let reader = flow.agent();
    flow.fixture.write_user_config(
        "[permission]\nmode = \"read-only\"\n[permission.edit]\n\"src/*\" = \"allow\"\n",
    );
    flow.fake.answer_steer([Err(ProviderError::NoActiveTurn)]);
    flow.submit("also write the cache module").await;
    let inside = flow.fixture.workdir.join("src/a.rs").display().to_string();

    flow.claude_event(permission_for(
        reader,
        "reader",
        PermissionTool::Edit,
        "",
        &[&inside],
    ))
    .await;

    let allowed = flow.fake.calls().into_iter().any(|call| {
        matches!(
            call,
            Call::AnswerPermission {
                answer: PermissionAnswer::AllowOnce,
                ..
            }
        )
    });
    assert!(!allowed);
}
