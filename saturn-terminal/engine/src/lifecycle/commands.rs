//! provider 명령 목록 전달: 어댑터가 알린 목록이 `Commands` 알림으로 TUI에 간다.
//! 설계: docs/design/extensions.md#기능-목록

use saturn_core::providers::ProviderCommand;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::rpc::{CommandInfo, Notification};

use super::support::{Flow, idle_reply};

fn command(name: &str, is_skill: bool) -> ProviderCommand {
    ProviderCommand {
        name: name.to_owned(),
        description: format!("{name} description"),
        is_skill,
    }
}

fn commands_in(notifications: &[Notification]) -> Vec<(String, Vec<CommandInfo>)> {
    notifications
        .iter()
        .filter_map(|notification| match notification {
            Notification::Commands { provider, commands } => {
                Some((provider.to_string(), commands.clone()))
            }
            _ => None,
        })
        .collect()
}

fn info(name: &str, is_skill: bool) -> CommandInfo {
    CommandInfo {
        name: name.to_owned(),
        description: format!("{name} description"),
        is_skill,
    }
}

#[tokio::test]
async fn a_list_the_connection_reports_later_reaches_the_attached_tui() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.fake
        .set_commands(vec![command("review", false), command("deploy", true)]);

    flow.submit("hello").await;

    let seen = commands_in(&client.window().await);
    assert_eq!(
        seen,
        vec![(
            "claude".to_owned(),
            vec![info("review", false), info("deploy", true)]
        )]
    );
}

#[tokio::test]
async fn an_unchanged_list_is_not_sent_again() {
    let mut flow = Flow::new(vec![idle_reply(0.95), idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.fake.set_commands(vec![command("review", false)]);
    flow.submit("one").await;
    let first = commands_in(&client.window().await).len();

    flow.submit("two").await;

    assert_eq!(first, 1);
    assert!(commands_in(&client.window().await).is_empty());
}

#[tokio::test]
async fn a_changed_list_replaces_the_earlier_one() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    let mut client = flow.client().await;
    flow.fake.set_commands(vec![command("review", false)]);
    flow.submit("one").await;
    let _ = client.window().await;
    flow.fake.set_commands(vec![command("deploy", false)]);

    // 목록은 연결이 이벤트나 요청을 처리한 뒤 다시 읽는다
    flow.fake.emit(ProviderEvent::Text {
        agent: flow.agent(),
        subagent: None,
        text: "working".to_owned(),
    });
    for _ in 0..2 {
        let message = flow.engine.flow.provider_rx.recv().await.unwrap();
        flow.engine.on_provider_msg(message).await;
    }

    assert_eq!(
        commands_in(&client.window().await),
        vec![("claude".to_owned(), vec![info("deploy", false)])]
    );
}

#[tokio::test]
async fn a_tui_that_attaches_later_gets_the_latest_list() {
    let mut flow = Flow::new(vec![idle_reply(0.95)]).await;
    flow.fake.set_commands(vec![command("review", false)]);
    flow.submit("hello").await;

    let (_client, greeting) = flow.attach().await;

    assert_eq!(
        commands_in(&greeting),
        vec![("claude".to_owned(), vec![info("review", false)])]
    );
}
