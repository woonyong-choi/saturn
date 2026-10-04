use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::AgentId;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc;

use super::convert::{convert_notification, convert_server_request, rejection};
use super::threads::HeldEvents;
use super::{Approvals, Pending, Stdin, Threads, lock};
use crate::providers::mask_values;
use crate::secrets::Masker;

/// 한 줄을 처리한 결과. `replies`는 app-server로 바로 돌려줄 응답이다.
#[derive(Debug, Default)]
pub(super) struct Routed {
    pub(super) events: Vec<ProviderEvent>,
    pub(super) replies: Vec<Value>,
}

/// 끝나면 진행 중이던 에이전트마다 `StreamLost`.
pub(super) async fn read_loop(
    stdout: ChildStdout,
    stdin: Stdin,
    pending: Pending,
    threads: Threads,
    approvals: Approvals,
    events: mpsc::Sender<ProviderEvent>,
    masker: Masker,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut held = HeldEvents::default();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!("skipping non-json line from codex app-server");
            continue;
        };
        mask_values(&mut message, &masker);
        let routed = route_message(&message, &pending, (&threads, &mut held), &approvals);
        deliver(routed, &stdin, &events).await;
    }
    lock(&pending).clear();
    lock(&approvals).clear();
    let lost: Vec<AgentId> = {
        let threads = lock(&threads);
        let mut agents: Vec<AgentId> = threads
            .values()
            .filter(|state| state.active_turn.is_some())
            .map(|state| state.agent)
            .collect();
        agents.sort_unstable();
        agents.dedup();
        agents
    };
    for agent in lost {
        let _ = events.send(ProviderEvent::StreamLost { agent }).await; // 받는 쪽이 연결을 버렸다
    }
}

/// 응답은 app-server로 돌려주고 이벤트는 연결로 넘긴다.
async fn deliver(routed: Routed, stdin: &Stdin, events: &mpsc::Sender<ProviderEvent>) {
    for reply in &routed.replies {
        write_reply(stdin, reply).await;
    }
    for event in routed.events {
        let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
    }
}

/// 읽기 작업이 직접 쓰는 응답 한 줄.
async fn write_reply(stdin: &Stdin, reply: &Value) {
    let mut line = reply.to_string();
    line.push('\n');
    let mut stdin = stdin.lock().await;
    let written = async {
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await
    }
    .await;
    if let Err(error) = written {
        tracing::warn!(kind = ?error.kind(), "failed to write a reply to codex app-server");
    }
}

pub(super) fn route_message(
    message: &Value,
    pending: &Pending,
    (threads, held): (&Threads, &mut HeldEvents),
    approvals: &Approvals,
) -> Routed {
    let events = route_events(message, pending, (threads, held), approvals);
    let (released, dropped) = held.drain_requests();
    lock(approvals).extend(released);
    let replies = dropped.iter().filter_map(rejection).collect();
    for item in &dropped {
        tracing::warn!(
            ?item,
            "a request of an unregistered codex thread was rejected"
        );
    }
    Routed { events, replies }
}

fn route_events(
    message: &Value,
    pending: &Pending,
    (threads, held): (&Threads, &mut HeldEvents),
    approvals: &Approvals,
) -> Vec<ProviderEvent> {
    match (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) {
        (Some(method), Some(id)) => convert_server_request(
            &lock(threads),
            &mut lock(approvals),
            held,
            method,
            id,
            &message["params"],
        ),
        (Some(method), None) => {
            convert_notification(&mut lock(threads), held, method, &message["params"])
        }
        (None, Some(id)) => {
            let reply = id.as_u64().and_then(|id| lock(pending).remove(&id));
            if let Some(reply) = reply {
                let result = match message.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = reply.send(result); // 기다리던 요청이 이미 포기했다
            }
            Vec::new()
        }
        (None, None) => Vec::new(),
    }
}

pub(super) async fn log_stderr(stderr: ChildStderr, masker: Masker) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(line = %masker.mask(&line).as_str(), "codex app-server stderr");
    }
}
