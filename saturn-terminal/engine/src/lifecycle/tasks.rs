use saturn_protocol::ids::TaskLabel;
use saturn_protocol::rpc::PermissionAnswer;
use saturn_protocol::rpc::TaskListItem;
use saturn_protocol::state::TaskState;

use super::support::{
    Flow, idle_reply, input_request, permission, subagent_started, turn_completed,
};
use super::*;

async fn task_list(flow: &mut Flow, client: &mut Client, id: u64) -> Vec<TaskListItem> {
    drive(&mut flow.engine, async {
        client.send(id, Request::ListTasks).await;
        let mut items = None;
        loop {
            match client.recv().await {
                ServerMessage::Notification(message) => {
                    if let Notification::TaskList { items: found } = message.notification {
                        items = Some(found);
                    }
                }
                ServerMessage::Response(response) => {
                    assert_eq!(response, Response::ok(RequestId(id)));
                    return items.expect("TaskList should arrive before the response");
                }
            }
        }
    })
    .await
}

#[tokio::test]
async fn task_list_reflects_the_chat_name_and_group_after_they_change() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let mut client = flow.client().await;
    let chat = flow.chat;

    let before = task_list(&mut flow, &mut client, 10).await;
    flow.engine.rename_chat(chat, " login fix ").await.unwrap();
    flow.engine
        .set_chat_group(chat, Some("auth"))
        .await
        .unwrap();
    let renamed = task_list(&mut flow, &mut client, 11).await;
    flow.engine.rename_chat(chat, "").await.unwrap();
    flow.engine.set_chat_group(chat, None).await.unwrap();
    let cleared = task_list(&mut flow, &mut client, 12).await;

    assert_eq!(before.len(), 1);
    assert_eq!(before[0].chat_name, format!("#{}", chat.0));
    assert_eq!(before[0].group, None);
    assert_eq!(before[0].chat, chat);
    assert_eq!(before[0].label, TaskLabel('A'));
    assert_eq!(before[0].state, TaskState::Running);
    assert_eq!(
        before[0].folder.as_deref(),
        Some(flow.fixture.workdir.display().to_string().as_str())
    );
    assert_eq!(renamed[0].chat_name, "login fix");
    assert_eq!(renamed[0].group.as_deref(), Some("auth"));
    assert_eq!(cleared[0].chat_name, before[0].chat_name);
    assert_eq!(cleared[0].group, None);
}

#[tokio::test]
async fn task_list_shows_waiting_states_and_running_subagents() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.claude_event(subagent_started(agent, "sub-1", None))
        .await;
    let running = task_list(&mut flow, &mut client, 10).await;
    flow.claude_event(permission(agent, "r1")).await;
    let asking = task_list(&mut flow, &mut client, 11).await;
    flow.answer_permission("r1", PermissionAnswer::AllowOnce)
        .await
        .unwrap();
    flow.claude_event(input_request(agent, "q1")).await;
    let waiting_input = task_list(&mut flow, &mut client, 12).await;

    assert_eq!(
        (running[0].state, running[0].children),
        (TaskState::Running, 1)
    );
    assert!(!running[0].needs_permission);
    assert_eq!(asking[0].state, TaskState::AwaitingPermission);
    assert!(asking[0].needs_permission);
    assert_eq!(waiting_input[0].state, TaskState::AwaitingInput);
    assert!(!waiting_input[0].needs_permission);
}

#[tokio::test]
async fn task_list_shows_held_tasks_and_drops_closed_ones() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.engine.stop_chat(flow.chat).await.unwrap();
    flow.claude_event(turn_completed(agent)).await;
    let held = task_list(&mut flow, &mut client, 10).await;
    flow.engine
        .close_held(flow.chat, held[0].task)
        .await
        .unwrap();
    let closed = task_list(&mut flow, &mut client, 11).await;

    assert_eq!(
        held.iter().map(|item| item.state).collect::<Vec<_>>(),
        [TaskState::Held]
    );
    assert!(closed.is_empty());
}

#[tokio::test]
async fn task_list_lists_tasks_of_every_chat_in_chat_order() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let other = flow.open_other_chat().await;
    let mut client = flow.client().await;

    let items = task_list(&mut flow, &mut client, 10).await;

    assert_eq!(
        items.iter().map(|item| item.chat).collect::<Vec<_>>(),
        [flow.chat, other.chat]
    );
}
