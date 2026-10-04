use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::AgentId;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc;

use super::convert::{convert_notification, convert_server_request};
use super::threads::HeldEvents;
use super::{Approvals, Pending, Threads, lock};
use crate::providers::mask_values;
use crate::secrets::Masker;

/// 끝나면 진행 중이던 에이전트마다 `StreamLost`.
pub(super) async fn read_loop(
    stdout: ChildStdout,
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
        for event in route_message(&message, &pending, (&threads, &mut held), &approvals) {
            let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
        }
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

pub(super) fn route_message(
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
