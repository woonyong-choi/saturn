//! 하위 접속 테스트(#33): 에이전트 작업 안의 `saturn`이 출입증으로 부모 채팅의 하위 작업으로 붙고, 부모 권한과 상한을
//! 넘지 못하며, 부모가 멈추면 함께 끝나고, 동시 요청이 서로 막지 않는다.

use std::time::Duration;

use saturn_core::permission::Mode;
use saturn_protocol::envelope::ServerMessage;
use saturn_protocol::event::{PermissionTool, ProviderEvent};
use saturn_protocol::ids::{AgentId, ChatId, InputId};
use saturn_protocol::rpc::{
    CHILD_REJECTED, Notification, PASS_ENV, PermissionAnswer, Request, SOCKET_ENV,
};
use saturn_protocol::state::InputState;
use tokio::time::timeout;

use super::support::{Flow, idle_reply, permission_for, turn_completed};
use super::*;
use crate::chat_env::ChatEnv;
use crate::passes::Entry;
use crate::providers::LaunchSpec;
use crate::providers::test_support::{CLAUDE, Call, FakeProvider};
use crate::secrets::ROUTER_KEY_ENV;

/// 부모 작업이 돌고 있는 채팅과 부모의 출입증.
pub(super) struct Family {
    pub(super) flow: Flow,
    pub(super) parent_agent: AgentId,
    pub(super) pass: String,
}

/// 부모 입력 하나가 실행 중이다. `child_inputs`는 하위 채팅이 낼 입력 수(router 답)다.
pub(super) async fn family(config: &str, child_inputs: usize) -> Family {
    let replies = (0..=child_inputs).map(|_| idle_reply(0.95)).collect();
    let mut flow = Flow::with_config(config, replies).await;
    flow.submit("parent task").await;
    let parent_agent = flow.agent();
    let pass = pass_of(&flow, flow.chat).await;
    Family {
        flow,
        parent_agent,
        pass,
    }
}

async fn launch_spec_of(flow: &Flow, chat: ChatId) -> LaunchSpec {
    let revision = flow.engine.settings.current().unwrap();
    flow.engine
        .launch_spec(CLAUDE, chat, revision)
        .await
        .unwrap()
}

fn env_value(spec: &LaunchSpec, name: &str) -> Option<String> {
    spec.env
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.to_string_lossy().into_owned())
}

/// provider를 띄울 때 환경에 실리는 출입증.
async fn pass_of(flow: &Flow, chat: ChatId) -> String {
    env_value(&launch_spec_of(flow, chat).await, PASS_ENV).expect("launch env should carry a pass")
}

pub(super) fn attach_child(pass: &str, mode: Option<&str>) -> Request {
    Request::AttachChild {
        pass: pass.to_owned(),
        mode: mode.map(str::to_owned),
    }
}

pub(super) fn chat_of(greeting: &[Notification]) -> ChatId {
    greeting
        .iter()
        .find_map(|notification| match notification {
            Notification::HistoryChunk { chat, .. } => Some(*chat),
            _ => None,
        })
        .expect("the greeting should carry the child chat")
}

/// 하위 접속을 열고 하위 채팅을 돌려준다. 소켓 연결 작업이 출입증을 확인하고 요청 처리 루프가 채팅을 만든다.
pub(super) async fn open_child(
    flow: &mut Flow,
    pass: &str,
    mode: Option<&str>,
) -> (Client, ChatId) {
    let mut client = Client::connect(&flow.fixture.socket()).await;
    let request = attach_child(pass, mode);
    let greeting = drive(&mut flow.engine, client.attach(1, request)).await;
    let chat = chat_of(&greeting);
    (client, chat)
}

/// 하위 채팅에 가짜 provider를 붙인다. 실제 provider 프로세스는 띄우지 않는다.
pub(super) fn give_provider(flow: &mut Flow, chat: ChatId) -> FakeProvider {
    let fake = FakeProvider::new(CLAUDE);
    flow.engine
        .flow
        .questions_of_connection
        .insert((chat, CLAUDE), false);
    flow.engine.add_connection(chat, fake.connection());
    fake
}

fn client_of(flow: &Flow, chat: ChatId) -> crate::rpc::ClientId {
    flow.engine
        .attachments
        .iter()
        .find(|(_, attachment)| attachment.chat == chat)
        .map(|(client, _)| *client)
        .expect("the child chat should have an attached client")
}

/// 하위 채팅이 입력 하나를 내고 에이전트가 열린다.
pub(super) async fn child_task(
    flow: &mut Flow,
    caller: &mut Client,
    chat: ChatId,
) -> (InputId, AgentId) {
    let client = client_of(flow, chat);
    flow.engine
        .submit_input(client, chat, 1, "child task".to_owned(), false)
        .await
        .unwrap();
    flow.settle().await;
    let input = caller
        .until(|notification| match notification {
            Notification::InputAccepted { input, .. } => Some(*input),
            _ => None,
        })
        .await;
    let agent = flow
        .engine
        .flow
        .live
        .values()
        .find(|live| {
            flow.engine
                .session_chat(live.session)
                .is_ok_and(|c| c == chat)
        })
        .map(|live| live.agent)
        .expect("the child should have an open session");
    (input, agent)
}

fn rejected(response: &saturn_protocol::envelope::Response) -> (i32, String) {
    match &response.outcome {
        saturn_protocol::envelope::Outcome::Err(error) => (error.code, error.message.clone()),
        saturn_protocol::envelope::Outcome::Ok(_) => panic!("expected an error response"),
    }
}

async fn reply_to_child_request(flow: &mut Flow, pass: &str, mode: Option<&str>) -> (i32, String) {
    let mut client = Client::connect(&flow.fixture.socket()).await;
    let request = attach_child(pass, mode);
    let response = drive(&mut flow.engine, async {
        client.send(1, request).await;
        client.response().await
    })
    .await;
    rejected(&response)
}

fn running_children(flow: &Flow, agent: AgentId) -> usize {
    flow.engine.agents.running_subagents(agent)
}

// #33: 하위 접속은 부모 작업 하나의 하위 에이전트로 트리에 오른다
#[tokio::test]
async fn child_attaches_as_a_subagent_of_the_running_parent_task() {
    let mut family = family("", 0).await;
    let mut parent_tui = family.flow.client().await;
    let pass = family.pass.clone();

    let (_child, chat) = open_child(&mut family.flow, &pass, None).await;

    let flow = &family.flow;
    assert_eq!(flow.engine.passes.parent_of(chat), Some(flow.chat));
    assert_eq!(flow.engine.passes.depth_of(chat), Some(1));
    assert_eq!(running_children(flow, family.parent_agent), 1);
    let started = parent_tui.window().await.into_iter().any(|notification| {
        matches!(
            notification,
            Notification::TaskEvent {
                event: ProviderEvent::SubagentStarted { .. },
                ..
            }
        )
    });
    assert!(started);
}

// #33: 하위 채팅은 부모의 작업 폴더와 더한 폴더를 물려받고 요청으로 바꿀 수 없다
#[tokio::test]
async fn child_chat_inherits_the_parent_folder_and_added_folders() {
    let mut family = family("", 0).await;
    let extra = family.flow.fixture.workdir.with_file_name("extra");
    std::fs::create_dir_all(&extra).unwrap();
    let parent = family.flow.chat;
    family
        .flow
        .engine
        .register_dir(parent, extra.clone())
        .await
        .unwrap();
    let pass = family.pass.clone();

    let (_child, chat) = open_child(&mut family.flow, &pass, None).await;

    let engine = &family.flow.engine;
    assert_eq!(
        engine.store.chat_workdir(chat).await.unwrap(),
        family.flow.fixture.workdir
    );
    assert_eq!(engine.chat_dirs_of(chat), vec![extra]);
}

// #33: 부모 모드를 넘는 모드는 거절하고 채팅도 자리도 만들지 않는다
#[tokio::test]
async fn request_above_the_parent_mode_is_rejected_without_creating_anything() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();

    let (code, message) = reply_to_child_request(&mut family.flow, &pass, Some("full")).await;

    assert_eq!(code, CHILD_REJECTED);
    assert!(message.contains("above the parent mode edit"), "{message}");
    assert_eq!(family.flow.engine.passes.running_total(), 0);
    assert!(family.flow.engine.children.is_empty());
}

// #33: 모드를 내려 달라는 요청은 받아 하위 채팅의 모드로 쓴다
#[tokio::test]
async fn request_below_the_parent_mode_sets_the_child_mode() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();

    let (_child, chat) = open_child(&mut family.flow, &pass, Some("read-only")).await;

    let engine = &family.flow.engine;
    let revision = engine.settings.current().unwrap();
    assert_eq!(
        engine.chat_mode(chat, revision).await.unwrap(),
        Mode::ReadOnly
    );
}

// #33: 부모가 모드를 낮추면 하위 채팅도 다음 판정부터 따라간다
#[tokio::test]
async fn lowering_the_parent_mode_lowers_the_child_too() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();
    let (_child, chat) = open_child(&mut family.flow, &pass, None).await;
    let parent = family.flow.chat;

    family
        .flow
        .engine
        .set_permission_mode(parent, "ask")
        .await
        .unwrap();

    let engine = &family.flow.engine;
    let revision = engine.settings.current().unwrap();
    assert_eq!(engine.chat_mode(chat, revision).await.unwrap(), Mode::Ask);
}

// #33: 하위 채팅은 자기 모드를 부모 모드 위로 올리지 못한다
#[tokio::test]
async fn child_cannot_raise_its_mode_above_the_parent_mode() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();
    let (_child, chat) = open_child(&mut family.flow, &pass, Some("ask")).await;

    let raised = family.flow.engine.set_permission_mode(chat, "full").await;
    let kept = family.flow.engine.set_permission_mode(chat, "edit").await;

    assert!(matches!(raised, Err(EngineError::ChildRejected { .. })));
    assert!(kept.is_ok());
}

// #33: 출입증이 없거나 틀린 요청은 요청 처리 루프를 거치지 않고 연결 작업이 바로 거절한다
#[tokio::test]
async fn unknown_pass_is_rejected_by_the_connection_without_the_engine_loop() {
    let mut family = family("", 0).await;
    let mut client = Client::connect(&family.flow.fixture.socket()).await;
    // 연결을 받는 일은 요청 처리 루프가 한다. 응답이 오면 연결 작업이 떠 있으니 그 뒤로는 루프를 돌리지 않는다
    drive(&mut family.flow.engine, async {
        client.send(1, Request::Version).await;
        while !matches!(client.recv().await, ServerMessage::Response(_)) {}
    })
    .await;
    let guess = format!("{}{}", "saturn-pass-", "0".repeat(64));

    client.send(1, attach_child(&guess, None)).await;
    let response = client.response().await;

    let (code, message) = rejected(&response);
    assert_eq!(code, CHILD_REJECTED);
    assert!(!message.contains(&guess));
    assert_eq!(family.flow.engine.passes.running_total(), 0);
}

// #33: 부모에 실행 중인 작업이 없으면 하위 접속을 받지 않고 자리를 돌려준다
#[tokio::test]
async fn child_is_rejected_when_the_parent_has_no_running_task() {
    let mut family = family("", 0).await;
    let ended = turn_completed(family.parent_agent);
    family.flow.claude_event(ended).await;
    let pass = family.pass.clone();

    let (code, message) = reply_to_child_request(&mut family.flow, &pass, None).await;

    assert_eq!(code, CHILD_REJECTED);
    assert!(message.contains("no running task"), "{message}");
    assert_eq!(family.flow.engine.passes.running_total(), 0);
}

// #33: 깊이 상한을 넘는 하위 접속은 거절한다
#[tokio::test]
async fn depth_above_the_limit_is_rejected() {
    let mut family = family("child.max_depth = 1\n", 0).await;
    let pass = family.pass.clone();
    let (_child, chat) = open_child(&mut family.flow, &pass, None).await;
    let child_pass = pass_of(&family.flow, chat).await;

    let (code, message) = reply_to_child_request(&mut family.flow, &child_pass, None).await;

    assert_eq!(code, CHILD_REJECTED);
    assert!(message.contains("depth limit 1"), "{message}");
}

// #33: 하위 채팅의 provider 환경에는 부모 출입증이 아닌 하위 채팅 자신의 출입증이 실린다
#[tokio::test]
async fn child_launch_env_carries_its_own_pass_not_the_parents() {
    let mut family = family("", 0).await;
    let parent = family.flow.chat;
    family.flow.engine.chats.insert(
        parent,
        ChatEnv::new(
            family.flow.fixture.workdir.clone(),
            vec![
                ("PATH".to_owned(), "/nonexistent".to_owned()),
                (PASS_ENV.to_owned(), "stolen".to_owned()),
            ],
        ),
    );
    let parent_pass = pass_of(&family.flow, parent).await;
    let (_child, chat) = open_child(&mut family.flow, &parent_pass, None).await;

    let spec = launch_spec_of(&family.flow, chat).await;

    let child_pass = env_value(&spec, PASS_ENV).unwrap();
    assert_ne!(child_pass, parent_pass);
    assert_ne!(child_pass, "stolen");
    assert!(env_value(&spec, SOCKET_ENV).is_some());
}

// #33: router 키는 하위와 바깥 어디에도 가지 않는다. 바이패스 모드(full)여도 키 보호 설정은 그대로다
#[tokio::test]
async fn router_key_never_reaches_a_child_even_in_full_mode() {
    let mut family = family("permission.mode = \"full\"\n", 0).await;
    let parent = family.flow.chat;
    family.flow.engine.chats.insert(
        parent,
        ChatEnv::new(
            family.flow.fixture.workdir.clone(),
            vec![
                ("PATH".to_owned(), "/nonexistent".to_owned()),
                (ROUTER_KEY_ENV.to_owned(), "sk-secret".to_owned()),
            ],
        ),
    );
    let parent_spec = launch_spec_of(&family.flow, parent).await;
    let pass = env_value(&parent_spec, PASS_ENV).unwrap();
    let (_child, chat) = open_child(&mut family.flow, &pass, Some("full")).await;

    let spec = launch_spec_of(&family.flow, chat).await;

    assert!(env_value(&spec, ROUTER_KEY_ENV).is_none());
    assert!(
        spec.env
            .iter()
            .all(|(_, value)| value.to_string_lossy() != "sk-secret")
    );
    assert!(spec.hook_settings.is_some());
    assert_eq!(spec.key_deny_read, parent_spec.key_deny_read);
    assert_eq!(spec.hook_settings, parent_spec.hook_settings);
}

// #33: 하위 채팅에는 답할 사용자가 없어 묻기 판정은 거부하고, 부모 TUI에도 올리지 않는다
#[tokio::test]
async fn child_permission_ask_is_denied_without_asking_the_parent_tui() {
    let mut family = family("", 1).await;
    let mut parent_tui = family.flow.client().await;
    let pass = family.pass.clone();
    let (mut child, chat) = open_child(&mut family.flow, &pass, None).await;
    let fake = give_provider(&mut family.flow, chat);
    let (_, agent) = child_task(&mut family.flow, &mut child, chat).await;

    let ask = permission_for(agent, "c1", PermissionTool::Shell, "cargo test", &[]);
    family.flow.claude_event(ask).await;

    let denied = fake.calls().into_iter().any(|call| {
        matches!(
            call,
            Call::AnswerPermission { request_id, answer: PermissionAnswer::Deny { .. }, .. }
                if request_id == "c1"
        )
    });
    assert!(denied);
    assert!(family.flow.engine.flow.permissions.is_empty());
    let asked = parent_tui
        .window()
        .await
        .into_iter()
        .any(|notification| matches!(notification, Notification::PermissionRequested { .. }));
    assert!(!asked);
}

// #33: 하위 채팅의 쓰기 입력은 부모가 쥔 쓰기 잠금을 기다리지 않는다. 기다리면 부모와 하위가 서로를 기다려 멈춘다
#[tokio::test]
async fn child_write_input_does_not_wait_for_the_parent_write_lock() {
    let mut family = family("", 1).await;
    let pass = family.pass.clone();
    let (mut child, chat) = open_child(&mut family.flow, &pass, None).await;
    give_provider(&mut family.flow, chat);

    let (input, _) = child_task(&mut family.flow, &mut child, chat).await;

    assert_ne!(family.flow.state(input), InputState::Queued);
    assert!(family.flow.record(input).write_scope.is_empty());
}

// #33: 부모를 멈추면 하위 접속이 함께 끝난다: 트리에서 내리고, 하위 작업을 멈추고, 출입증을 회수하고, 연결을 닫는다
#[tokio::test]
async fn stopping_the_parent_ends_the_child_with_it() {
    let mut family = family("", 1).await;
    let pass = family.pass.clone();
    let (mut child, chat) = open_child(&mut family.flow, &pass, None).await;
    let fake = give_provider(&mut family.flow, chat);
    child_task(&mut family.flow, &mut child, chat).await;
    let child_pass = pass_of(&family.flow, chat).await;
    let parent = family.flow.chat;

    family.flow.engine.stop_chat(parent).await.unwrap();
    family.flow.settle().await;

    let flow = &family.flow;
    assert!(flow.engine.children.is_empty());
    assert_eq!(running_children(flow, family.parent_agent), 0);
    assert_eq!(flow.engine.passes.running_total(), 0);
    assert!(
        fake.calls()
            .iter()
            .any(|call| matches!(call, Call::Interrupt { .. }))
    );
    assert!(!flow.engine.providers.contains_key(&(chat, CLAUDE)));
    assert!(matches!(
        flow.engine.passes.request(&child_pass, None),
        Entry::Rejected(_)
    ));
}

// #33: 부모의 연결이 끊기면 부모 출입증과 하위 접속이 만료된다
#[tokio::test]
async fn parent_connection_loss_expires_the_pass_and_ends_the_children() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();
    let (_child, chat) = open_child(&mut family.flow, &pass, None).await;
    let parent = family.flow.chat;

    family
        .flow
        .engine
        .on_connection_closed(parent, CLAUDE)
        .await;

    let flow = &family.flow;
    assert!(!flow.engine.children.contains_key(&chat));
    assert!(matches!(
        flow.engine.passes.request(&pass, None),
        Entry::Rejected(_)
    ));
}

// #33: 하위 접속을 건 쪽이 끊기면 그 하위 작업도 끝나고 자리를 돌려준다
#[tokio::test]
async fn child_disconnect_ends_the_child_and_frees_its_place() {
    let mut family = family("", 0).await;
    let pass = family.pass.clone();
    let (child, _) = open_child(&mut family.flow, &pass, None).await;
    assert_eq!(family.flow.engine.passes.running_total(), 1);

    drop(child);
    let parent_agent = family.parent_agent;
    drive_until(&mut family.flow.engine, WAIT, |engine| {
        engine.children.is_empty()
            && engine.passes.running_total() == 0
            && engine.agents.running_subagents(parent_agent) == 0
    })
    .await;

    let flow = &family.flow;
    assert!(flow.engine.children.is_empty());
    assert_eq!(flow.engine.passes.running_total(), 0);
    assert_eq!(running_children(flow, family.parent_agent), 0);
}

// #33: 동시 상한을 넘는 요청은 대기열에 서고, 자리가 나면 이어서 붙는다
#[tokio::test]
async fn request_over_the_concurrent_limit_waits_and_attaches_when_a_place_frees() {
    let mut family = family("child.max_concurrent = 1\n", 0).await;
    let pass = family.pass.clone();
    let (first, _) = open_child(&mut family.flow, &pass, None).await;
    let mut second = Client::connect(&family.flow.fixture.socket()).await;
    let request = attach_child(&pass, None);

    let greeting = drive(&mut family.flow.engine, async {
        second.send(1, request).await;
        let queued = second.notification().await;
        assert_eq!(queued, Notification::ChildQueued { position: 1 });
        assert!(
            timeout(Duration::from_millis(300), second.recv())
                .await
                .is_err(),
            "the queued request should not be answered yet"
        );
        drop(first);
        let mut notifications = Vec::new();
        loop {
            match second.recv().await {
                ServerMessage::Notification(message) => notifications.push(message.notification),
                ServerMessage::Response(_) => return notifications,
            }
        }
    })
    .await;

    let chat = chat_of(&greeting);
    assert_eq!(
        family.flow.engine.passes.parent_of(chat),
        Some(family.flow.chat)
    );
    assert_eq!(family.flow.engine.passes.running_total(), 1);
}

// #33: 대기열에 선 요청의 연결이 끊기면 자리를 받지 않는다
#[tokio::test]
async fn queued_request_that_disconnects_does_not_take_a_place() {
    let mut family = family("child.max_concurrent = 1\n", 0).await;
    let pass = family.pass.clone();
    let (first, _) = open_child(&mut family.flow, &pass, None).await;
    let mut second = Client::connect(&family.flow.fixture.socket()).await;
    let request = attach_child(&pass, None);

    drive(&mut family.flow.engine, async {
        second.send(1, request).await;
        second.notification().await;
    })
    .await;
    assert_eq!(family.flow.engine.passes.queued_total(), 1);

    // 대기 요청의 연결이 끊겨 줄에서 빠진 것을 확인한 뒤에 실행 중인 자리를 돌려준다
    drop(second);
    drive_until(&mut family.flow.engine, WAIT, |engine| {
        engine.passes.queued_total() == 0
    })
    .await;
    assert_eq!(family.flow.engine.passes.running_total(), 1);
    drop(first);
    drive_until(&mut family.flow.engine, WAIT, |engine| {
        engine.passes.running_total() == 0 && engine.children.is_empty()
    })
    .await;

    assert_eq!(family.flow.engine.passes.running_total(), 0);
    assert_eq!(family.flow.engine.passes.queued_total(), 0);
    assert!(family.flow.engine.children.is_empty());
}

/// 새 접속으로 `Version`을 보내 응답이 올 때까지 기다린다. 시간 수치는 재지 않는다(부하 시간은 `child_load`가 잰다).
async fn version_answered(socket: std::path::PathBuf) {
    let mut client = Client::connect(&socket).await;
    client.send(1, Request::Version).await;
    while !matches!(client.recv().await, ServerMessage::Response(_)) {}
}

// #33: 대기열에 선 요청이 많아도 다른 요청은 기다리지 않고 바로 답을 받는다
#[tokio::test]
async fn queued_children_do_not_hold_back_other_requests() {
    const QUEUED: usize = 30;
    let mut family = family("child.max_concurrent = 1\n", 0).await;
    let pass = family.pass.clone();
    let (_first, _) = open_child(&mut family.flow, &pass, None).await;
    let socket = family.flow.fixture.socket();

    drive(&mut family.flow.engine, async {
        let mut waiting = Vec::new();
        for _ in 0..QUEUED {
            let mut client = Client::connect(&socket).await;
            client.send(1, attach_child(&pass, None)).await;
            waiting.push(client);
        }
        let probes = (0..QUEUED).map(|_| {
            let socket = socket.clone();
            tokio::spawn(version_answered(socket))
        });
        for probe in probes.collect::<Vec<_>>() {
            probe.await.unwrap();
        }
        drop(waiting);
    })
    .await;

    // 대기열이 그대로 찬 채로 다른 요청이 모두 응답을 받았다
    assert_eq!(family.flow.engine.passes.queued_total(), QUEUED as u32);
}

// #33: 대기열에 선 요청이 있어도 같은 연결의 다음 요청을 계속 읽어 답한다
#[tokio::test]
async fn queued_request_does_not_block_the_next_request_on_the_same_connection() {
    let mut family = family("child.max_concurrent = 1\n", 0).await;
    let pass = family.pass.clone();
    let (_first, _) = open_child(&mut family.flow, &pass, None).await;
    let mut client = Client::connect(&family.flow.fixture.socket()).await;
    let request = attach_child(&pass, None);

    let answered = drive(&mut family.flow.engine, async {
        client.send(1, request).await;
        client.notification().await;
        client.send(2, Request::Version).await;
        loop {
            if let ServerMessage::Response(response) = client.recv().await {
                return response.id;
            }
        }
    })
    .await;

    assert_eq!(answered, Some(saturn_protocol::envelope::RequestId(2)));
}

// #33: 이미 허용받은 자리가 있는 접속이 끊겨도 그 자리는 다음 요청에 간다
#[tokio::test]
async fn grant_for_a_client_that_left_goes_to_the_next_request() {
    let family = family("child.max_concurrent = 1\n", 0).await;
    let pass = family.pass.clone();
    let Entry::Admitted(grant) = family.flow.engine.passes.request(&pass, None) else {
        panic!("the first request should be admitted");
    };
    let Entry::Queued { mut wait, .. } = family.flow.engine.passes.request(&pass, None) else {
        panic!("the second request should wait");
    };

    family.flow.engine.passes.abandon(&grant);

    assert!(matches!(
        wait.try_recv(),
        Ok(crate::passes::Waited::Granted(_))
    ));
}
