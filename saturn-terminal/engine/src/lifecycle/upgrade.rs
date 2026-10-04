//! engine 업데이트 교체 테스트(#178): 버전 확인, 종료 요청, 보류로 넘긴 작업의 재개.

use saturn_protocol::ids::{ChatId, TaskLabel};
use saturn_protocol::rpc::{Alert, PROTOCOL_VERSION};
use saturn_protocol::state::EffectScope;

use super::crash_recovery::{Restarted, resume_suggested};
use super::support::{Flow, idle_reply};
use super::*;

fn version_of(notification: &Notification) -> Option<(String, u32)> {
    match notification {
        Notification::EngineVersion {
            saturn_version,
            protocol_version,
        } => Some((saturn_version.clone(), *protocol_version)),
        _ => None,
    }
}

fn has_alert(seen: &[Notification], expected: &Alert) -> bool {
    seen.iter().any(
        |notification| matches!(notification, Notification::Alert { alert } if alert == expected),
    )
}

// #178: 붙기 전에도 engine의 빌드 버전과 protocol 판을 알려 준다
#[tokio::test]
async fn version_is_answered_before_attach() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;

    let answered = drive(&mut engine, async {
        client.send(1, Request::Version).await;
        let version = client.until(version_of).await;
        (version, client.response().await)
    })
    .await;

    assert_eq!(
        answered.0,
        (env!("CARGO_PKG_VERSION").to_owned(), PROTOCOL_VERSION)
    );
    assert_eq!(answered.1, Response::ok(RequestId(1)));
}

// #178: router 키를 기다리는 동안에도 버전을 알려 주어 교체가 키 입력보다 먼저 일어난다
#[tokio::test]
async fn version_is_answered_while_waiting_for_the_router_key() {
    let fixture = Fixture::new();
    let mut engine = fixture.waiting_for_key(Vec::new()).await;
    let mut client = Client::connect(&fixture.socket()).await;

    let version = drive(&mut engine, async {
        client.send(1, Request::Version).await;
        client.until(version_of).await
    })
    .await;

    assert_eq!(version.1, PROTOCOL_VERSION);
}

// #178: 붙을 때 보내는 시작 정보에도 protocol 판이 실린다
#[tokio::test]
async fn start_info_carries_the_protocol_version() {
    let mut flow = Flow::new(Vec::new()).await;

    let (_client, greeting) = flow.attach().await;

    assert!(greeting.iter().any(|notification| matches!(
        notification,
        Notification::StartInfo { protocol_version, .. } if *protocol_version == PROTOCOL_VERSION
    )));
}

/// 연결이 끊길 때까지 받은 알림.
async fn until_closed(client: &mut Client) -> Vec<Notification> {
    let mut notified = Vec::new();
    while let Some(line) = timeout(WAIT, client.lines.next_line())
        .await
        .expect("the connection should be closed, not left open")
        .unwrap()
    {
        if let ServerMessage::Notification(message) = decode_server_line(&line).unwrap() {
            notified.push(message.notification);
        }
    }
    notified
}

/// 작업 중인 채팅에 TUI를 붙이고 그 실행의 효과 범위를 `scope`로 정한다.
async fn working_with_a_tui(flow: &mut Flow, scope: EffectScope) -> Client {
    flow.submit("fix the build").await;
    let (tui, _) = flow.attach().await;
    let run = flow.engine.store.unfinished_runs().await.unwrap()[0].id;
    flow.engine
        .store
        .set_effect_scope(run, scope)
        .await
        .unwrap();
    tui
}

/// 요청 처리를 끝낸 engine이 실행을 끝내지 않은 채 남겼는지 보고 마저 종료한다.
async fn finish_shutdown(flow: Flow) -> (Fixture, ChatId) {
    let Flow {
        engine,
        fixture,
        chat,
        ..
    } = flow;
    assert!(
        !engine.store.unfinished_runs().await.unwrap().is_empty(),
        "the running work should stay unfinished for recovery"
    );
    engine.shutdown().await.unwrap();
    (fixture, chat)
}

/// 작업 중인 engine에 다른 접속이 `Shutdown`을 보내 `serve`가 끝나고 `shutdown`까지 마친 뒤, 같은 홈으로 다시 시작한 engine.
async fn upgraded_while_working(scope: EffectScope) -> (Restarted, Vec<Notification>, Response) {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut tui = working_with_a_tui(&mut flow, scope).await;
    let mut admin = Client::connect(&flow.fixture.socket()).await;
    admin.send(7, Request::Shutdown).await;

    let served = timeout(WAIT, flow.engine.serve()).await;
    assert!(matches!(served, Ok(Ok(()))));
    let (fixture, chat) = finish_shutdown(flow).await;

    let answered = admin.response().await;
    let notified = until_closed(&mut tui).await;
    let restarted = Restarted::start_with_replies(fixture, chat, Vec::new()).await;
    (restarted, notified, answered)
}

// #178: 종료 요청을 받으면 새 요청을 받지 않고 끝나며, 붙은 TUI에는 알린 뒤 연결을 끊는다
#[tokio::test]
async fn shutdown_ends_the_engine_and_tells_the_attached_tui_before_closing() {
    let (_restarted, notified, answered) = upgraded_while_working(EffectScope::Unobserved).await;

    assert_eq!(answered, Response::ok(RequestId(7)));
    assert!(has_alert(&notified, &Alert::EngineRestarting));
}

// #178: 실행 중이던 작업은 끊기지 않고 기록에 남아 새 engine이 보류하고 `/continue`를 제안한다
#[tokio::test]
async fn unproven_work_is_held_by_the_new_engine_and_suggested() {
    let (mut restarted, _, _) = upgraded_while_working(EffectScope::NetworkPossible).await;

    assert!(restarted.fake.calls().is_empty());
    let seen = restarted.attach().await;
    assert_eq!(resume_suggested(&seen), Some(vec![TaskLabel('A')]));
}

// #178: 효과 범위가 증명된 작업은 새 engine이 크래시 복구와 같은 길로 스스로 잇는다
#[tokio::test]
async fn proven_work_continues_by_itself_in_the_new_engine() {
    let (mut restarted, _, _) = upgraded_while_working(EffectScope::ProvenByObservation).await;
    restarted.settle().await;

    assert_eq!(restarted.turns().len(), 1);
    let seen = restarted.attach().await;
    assert!(resume_suggested(&seen).is_none());
}

// #178: 업데이트로 띄운 engine은 첫 TUI에만 다시 시작했다는 알림을 보낸다
#[tokio::test]
async fn engine_started_after_an_upgrade_tells_only_the_first_tui() {
    let mut fixture = Fixture::new();
    fixture.options.after_upgrade = true;
    let mut engine = fixture.ready().await;
    let mut first = Client::connect(&fixture.socket()).await;
    let mut second = Client::connect(&fixture.socket()).await;
    let workdir = fixture.workdir.clone();

    let seen = drive(&mut engine, async {
        let first_seen = first.attach(1, new_chat(&workdir)).await;
        let second_seen = second.attach(1, new_chat(&workdir)).await;
        (first_seen, second_seen)
    })
    .await;

    assert!(has_alert(&seen.0, &Alert::EngineRestarted));
    assert!(!has_alert(&seen.1, &Alert::EngineRestarted));
}

// #178: 업데이트 없이 시작한 engine은 알림을 보내지 않는다
#[tokio::test]
async fn engine_started_normally_sends_no_restart_alert() {
    let fixture = Fixture::new();
    let mut engine = fixture.ready().await;
    let mut client = Client::connect(&fixture.socket()).await;
    let workdir = fixture.workdir.clone();

    let seen = drive(&mut engine, async {
        client.attach(1, new_chat(&workdir)).await
    })
    .await;

    assert!(!has_alert(&seen, &Alert::EngineRestarted));
}
