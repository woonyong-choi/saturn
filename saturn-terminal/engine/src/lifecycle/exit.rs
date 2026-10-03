//! TUI 종료 뒤 처리 테스트(#70): `on_exit`가 `stop`이면 닫은 뒤 모든 작업을 보류하고, `ask`와 `background`는 계속한다.

use saturn_protocol::envelope::INVALID_PARAMS;
use saturn_protocol::rpc::ExitPlan;
use saturn_protocol::state::{InputState, SessionState};

use super::support::{CLIENT, Flow, OTHER_CLIENT, idle_reply, running_reply};
use super::*;
use crate::providers::test_support::{Call, FakeProvider};
use crate::rpc::RpcEvent;

fn was_interrupted(fake: &FakeProvider) -> bool {
    fake.calls()
        .iter()
        .any(|call| matches!(call, Call::Interrupt { .. }))
}

fn on_exit(value: &str) -> String {
    format!("on_exit = \"{value}\"\n")
}

/// 소켓으로 붙은 TUI가 `PrepareExit`을 보내 받은 계획. `others`가 거짓이면 이 TUI만 붙은 상태다.
async fn plan_of(flow: &mut Flow, others: bool) -> ExitPlan {
    let (mut client, _) = flow.attach().await;
    if !others {
        flow.engine.attachments.remove(&CLIENT);
    }
    let chat = flow.chat;
    drive(&mut flow.engine, async {
        client.send(2, Request::PrepareExit { chat }).await;
        client
            .until(|notification| match notification {
                Notification::ExitPlan { plan } => Some(*plan),
                _ => None,
            })
            .await
    })
    .await
}

// #70: stop은 마지막 TUI가 떨어질 때 모든 채팅의 작업을 멈춤과 같게 보류한다
#[tokio::test]
async fn stop_holds_the_work_of_every_chat_only_when_the_last_tui_detaches() {
    let mut flow =
        Flow::with_config(&on_exit("stop"), vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let session = flow.engine.flow.live[&flow.agent()].session;
    let other = flow.open_other_chat().await;

    flow.engine
        .handle_event(RpcEvent::Disconnected(CLIENT))
        .await
        .unwrap();

    assert!(!was_interrupted(&flow.fake));
    assert!(!was_interrupted(&other.fake));

    flow.engine
        .handle_event(RpcEvent::Disconnected(OTHER_CLIENT))
        .await
        .unwrap();

    assert!(was_interrupted(&flow.fake));
    assert!(was_interrupted(&other.fake));
    assert_eq!(
        flow.engine.sessions.get(session).unwrap().state,
        SessionState::Held
    );
}

// #70: 멈춘 작업은 자동으로 이어 가지 않는다
#[tokio::test]
async fn stop_on_exit_keeps_waiting_input_held_after_the_turn_ends() {
    let mut flow = Flow::with_config(
        &on_exit("stop"),
        vec![idle_reply(0.95), running_reply(0.95, "continues", "queue")],
    )
    .await;
    flow.submit("fix the build").await;
    let agent = flow.agent();
    let waiting = flow.submit("also run the tests").await;

    flow.engine
        .handle_event(RpcEvent::Disconnected(CLIENT))
        .await
        .unwrap();
    flow.claude_event(super::support::turn_completed(agent))
        .await;
    flow.engine.dispatch_next(flow.chat).await.unwrap();

    assert_eq!(flow.state(waiting), InputState::Held);
}

// #70: ask와 background는 TUI가 떨어져도 작업을 멈추지 않는다
#[tokio::test]
async fn ask_and_background_keep_running_after_the_last_tui_detaches() {
    for value in ["ask", "background"] {
        let mut flow = Flow::with_config(&on_exit(value), vec![idle_reply(0.95)]).await;
        flow.submit("fix the build").await;

        flow.engine
            .handle_event(RpcEvent::Disconnected(CLIENT))
            .await
            .unwrap();

        assert!(!was_interrupted(&flow.fake), "{value}");
    }
}

// #70: 닫을 때 할 일은 on_exit, 계속될 작업 수, 다른 TUI가 붙어 있는지로 정한다
#[tokio::test]
async fn exit_plan_follows_on_exit_work_and_other_tuis() {
    struct Case {
        on_exit: &'static str,
        inputs: usize,
        others: bool,
        plan: ExitPlan,
    }
    let cases = [
        Case {
            on_exit: "stop",
            inputs: 1,
            others: false,
            plan: ExitPlan::Close,
        },
        Case {
            on_exit: "ask",
            inputs: 1,
            others: false,
            plan: ExitPlan::Ask { running: 1 },
        },
        Case {
            on_exit: "ask",
            inputs: 2,
            others: false,
            plan: ExitPlan::Ask { running: 2 },
        },
        Case {
            on_exit: "background",
            inputs: 1,
            others: false,
            plan: ExitPlan::Notice { running: 1 },
        },
        Case {
            on_exit: "ask",
            inputs: 0,
            others: false,
            plan: ExitPlan::Close,
        },
        Case {
            on_exit: "background",
            inputs: 0,
            others: false,
            plan: ExitPlan::Close,
        },
        Case {
            on_exit: "ask",
            inputs: 1,
            others: true,
            plan: ExitPlan::Close,
        },
    ];
    for case in cases {
        let mut flow = Flow::with_config(
            &on_exit(case.on_exit),
            vec![idle_reply(0.95), running_reply(0.95, "continues", "queue")],
        )
        .await;
        for text in ["fix the build", "also run the tests"]
            .iter()
            .take(case.inputs)
        {
            flow.submit(text).await;
        }

        let plan = plan_of(&mut flow, case.others).await;

        assert_eq!(
            plan, case.plan,
            "on_exit {} inputs {} others {}",
            case.on_exit, case.inputs, case.others
        );
    }
}

// #70: ask에서 멈춤을 고르면 TUI가 StopAll을 보내 모든 채팅의 작업을 멈춘다
#[tokio::test]
async fn stop_all_holds_the_work_of_every_chat() {
    let mut flow =
        Flow::with_config(&on_exit("ask"), vec![idle_reply(0.95), idle_reply(0.95)]).await;
    flow.submit("fix the build").await;
    let other = flow.open_other_chat().await;
    let (mut client, _) = flow.attach().await;

    drive(&mut flow.engine, async {
        client.send(2, Request::StopAll).await;
        client.response().await;
    })
    .await;

    assert!(was_interrupted(&flow.fake));
    assert!(was_interrupted(&other.fake));
}

#[tokio::test]
async fn prepare_exit_for_a_chat_the_client_is_not_attached_to_is_refused() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let (mut client, _) = flow.attach().await;
    let other_chat = saturn_protocol::ids::ChatId(flow.chat.0 + 1);

    let response = drive(&mut flow.engine, async {
        client
            .send(2, Request::PrepareExit { chat: other_chat })
            .await;
        client.response().await
    })
    .await;

    assert_eq!(error_code(&response), INVALID_PARAMS);
}
