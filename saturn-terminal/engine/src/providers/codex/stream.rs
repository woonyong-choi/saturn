use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ProviderSessionId};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc;

use super::convert::{convert_notification, convert_server_request, rejection};
use super::threads::HeldEvents;
use super::{Approvals, Pending, Stdin, Threads, lock};
use crate::providers::{Frame, ProviderTrace, RawTap};
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
    (masker, trace, raw): (Masker, ProviderTrace, RawTap),
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut held = HeldEvents::default();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(message) = RawTap::parse_masked(&line, &masker) else {
            raw.send(&line, None, (None, None));
            tracing::debug!("skipping non-json line from codex app-server");
            continue;
        };
        raw.send(&line, Some(&message), owner_of(&message, &threads));
        trace_message(&trace, &message);
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

/// 메시지가 속한 thread의 식별자와 그 thread의 에이전트. 알림과 요청은 `params.threadId`, thread를 여는 알림은
/// `params.thread.id`, 우리 요청의 응답은 `result.thread.id`에 적힌다. 등록되지 않은 thread나 thread가 없는 메시지의 에이전트는 비운다.
fn owner_of(message: &Value, threads: &Threads) -> (Option<AgentId>, Option<String>) {
    let thread = [
        &message["params"]["threadId"],
        &message["params"]["thread"]["id"],
        &message["result"]["thread"]["id"],
    ]
    .into_iter()
    .find_map(Value::as_str);
    let Some(thread) = thread else {
        return (None, None);
    };
    let agent = lock(threads)
        .get(&ProviderSessionId(thread.to_owned()))
        .map(|state| state.agent);
    (agent, Some(thread.to_owned()))
}

/// 받은 메시지의 모양을 관측 기록에 남긴다. 방법 이름이 있으면 요청이나 알림이고, 없으면 우리 요청의 응답이다.
fn trace_message(trace: &ProviderTrace, message: &Value) {
    if !trace.is_enabled() {
        return;
    }
    let (frame, kind) = match (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) {
        (Some(method), Some(_)) => (Frame::Request, method),
        (Some(method), None) => (Frame::Notification, method),
        (None, _) if message.get("error").is_some() => (Frame::Response, "error"),
        (None, _) => (Frame::Response, "result"),
    };
    trace.record(frame, kind, message);
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::TraceHub;
    use crate::providers::test_support::CODEX;
    use saturn_protocol::ids::ChatId;

    fn lines(home: &std::path::Path) -> Vec<Value> {
        let logs = home.join("logs");
        std::fs::read_dir(logs)
            .unwrap()
            .flat_map(|entry| {
                std::fs::read_to_string(entry.unwrap().path())
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn requests_notifications_and_responses_are_told_apart_by_method_and_id() {
        let home = tempfile::tempdir().unwrap();
        let hub = TraceHub::new(home.path());
        hub.set_enabled(ChatId(1), true);
        let trace = hub.link(ChatId(1), CODEX, &Masker::default());

        for message in [
            json!({ "method": "thread/closed", "params": { "threadId": "t-2" } }),
            json!({ "id": 5, "method": "item/commandExecution/requestApproval", "params": { "threadId": "t-2", "command": "ls" } }),
            json!({ "id": 3, "result": { "thread": { "id": "t-2" } } }),
            json!({ "id": 4, "error": { "code": -32600, "message": "no" } }),
        ] {
            trace_message(&trace, &message);
        }

        let seen: Vec<(String, String)> = lines(home.path())
            .iter()
            .map(|line| {
                (
                    line["frame"].as_str().unwrap().to_owned(),
                    line["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("notification".to_owned(), "thread/closed".to_owned()),
                (
                    "request".to_owned(),
                    "item/commandExecution/requestApproval".to_owned()
                ),
                ("response".to_owned(), "result".to_owned()),
                ("response".to_owned(), "error".to_owned()),
            ]
        );
        let text = std::fs::read_to_string(
            std::fs::read_dir(home.path().join("logs"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(!text.contains("\"ls\""));
        assert!(!text.contains("no\""));
    }
}
