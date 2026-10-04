use saturn_protocol::ids::{Provider, TaskLabel};
use saturn_protocol::rpc::ModelChoice;
use saturn_protocol::rpc::PermissionAnswer;
use saturn_protocol::rpc::TaskListItem;
use saturn_protocol::state::TaskState;

use super::support::{
    Flow, idle_reply, input_request, permission, running_reply, subagent_started, turn_completed,
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
    assert_eq!(before[0].label, Some(TaskLabel('A')));
    assert_eq!(before[0].state, Some(TaskState::Running));
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
        (Some(TaskState::Running), 1)
    );
    assert!(!running[0].needs_permission);
    assert_eq!(asking[0].state, Some(TaskState::AwaitingPermission));
    assert!(asking[0].needs_permission);
    assert_eq!(waiting_input[0].state, Some(TaskState::AwaitingInput));
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
        .close_held(flow.chat, held[0].task.unwrap())
        .await
        .unwrap();
    let closed = task_list(&mut flow, &mut client, 11).await;

    assert_eq!(
        held.iter().map(|item| item.state).collect::<Vec<_>>(),
        [Some(TaskState::Held)]
    );
    // 닫은 보류는 작업 행이 아니고, 채팅은 작업 없는 채팅 행으로만 남는다
    assert!(closed.iter().all(|item| item.task.is_none()));
    assert_eq!(closed.len(), 1);
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

#[tokio::test]
async fn task_list_shows_a_finished_task_as_done_with_its_end_time_and_no_label() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let input = flow.submit("fix the build").await;
    let task = flow.record(input).task.unwrap();
    let agent = flow.agent();
    let mut client = flow.client().await;

    flow.engine.finish_task(flow.chat, agent).await.unwrap();
    let items = task_list(&mut flow, &mut client, 10).await;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].task, Some(task));
    assert_eq!(items[0].label, None);
    assert_eq!(items[0].state, Some(TaskState::Done));
    assert!(items[0].ended_at_ms.is_some_and(|at| at > 0));
}

#[tokio::test]
async fn task_list_shows_a_failed_last_run_as_failed() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let mut client = flow.client().await;
    let run = flow.engine.store.unfinished_runs().await.unwrap()[0].id;

    flow.engine.queue.finish_task(agent);
    flow.engine
        .store
        .finish_run(run, crate::store::RunEnd::Failed)
        .await
        .unwrap();
    let items = task_list(&mut flow, &mut client, 10).await;

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].state, Some(TaskState::Failed));
}

#[tokio::test]
async fn task_list_shows_a_chat_without_tasks_as_a_chat_row() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let empty = flow
        .engine
        .store
        .create_chat(flow.fixture.workdir.clone())
        .await
        .unwrap();
    flow.engine.rename_chat(empty, "scratch").await.unwrap();
    let mut client = flow.client().await;

    let items = task_list(&mut flow, &mut client, 10).await;
    let row = items.iter().find(|item| item.chat == empty).unwrap();

    assert_eq!(row.chat_name, "scratch");
    assert_eq!((row.task, row.label, row.state), (None, None, None));
    assert!(row.queued.is_empty());
}

#[tokio::test]
async fn task_list_attaches_a_waiting_input_to_the_task_it_waits_for() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "continues", "queue"),
    ])
    .await;
    flow.submit("fix the build").await;
    let waiting = flow.submit("then run the tests").await;
    let mut client = flow.client().await;

    let items = task_list(&mut flow, &mut client, 10).await;

    let rows: Vec<_> = items
        .iter()
        .map(|item| (item.task.is_some(), item.queued.clone()))
        .collect();
    assert_eq!(rows, [(true, vec![waiting])]);
}

#[tokio::test]
async fn task_list_reports_the_model_of_the_task_session() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let pinned = ModelChoice {
        provider: Provider::Claude,
        model: "opus".to_owned(),
    };
    flow.submit_with("fix the build", Some(pinned), false).await;
    let mut client = flow.client().await;

    let items = task_list(&mut flow, &mut client, 10).await;

    assert_eq!(items[0].model.as_deref(), Some("opus"));
}

#[tokio::test]
async fn task_list_attaches_a_waiting_new_task_to_the_chat_row() {
    let mut flow = Flow::new(vec![
        idle_reply(0.95),
        running_reply(0.95, "independent", "spawn"),
    ])
    .await;
    flow.submit("fix the build").await;
    let waiting = flow.submit("write the docs").await;
    let mut client = flow.client().await;

    let items = task_list(&mut flow, &mut client, 10).await;

    let rows: Vec<_> = items
        .iter()
        .map(|item| (item.task.is_some(), item.queued.clone()))
        .collect();
    assert_eq!(rows, [(false, vec![waiting]), (true, vec![])]);
}
