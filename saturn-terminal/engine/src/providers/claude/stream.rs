use std::sync::{Arc, Mutex};
use std::time::Instant;

use saturn_core::providers::ProviderCommand;
use saturn_protocol::event::ProviderEvent;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc;

use super::background;
use super::convert::convert_line;
use super::{SessionState, lock};
use crate::providers::{Frame, ProviderTrace, RawTap};
use crate::secrets::Masker;
use saturn_protocol::ids::AgentId;

/// 끝나면 턴이나 subagent가 남았을 때 `StreamLost`.
pub(super) async fn read_loop(
    stdout: ChildStdout,
    state: Arc<Mutex<SessionState>>,
    events: mpsc::Sender<ProviderEvent>,
    latest_commands: Arc<Mutex<Vec<ProviderCommand>>>,
    (masker, trace, raw): (Masker, ProviderTrace, RawTap),
) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = next_line_or_settle(&mut lines, &state, &events).await;
        let Ok(Some(line)) = line else {
            break;
        };
        let agent = Some(lock(&state).agent);
        let Some(message) = RawTap::parse_masked(&line, &masker) else {
            raw.send(&line, None, (agent, None));
            tracing::debug!("skipping non-json line from claude");
            continue;
        };
        let session = message["session_id"].as_str().map(str::to_owned);
        raw.send(&line, Some(&message), (agent, session));
        trace_line(&trace, &message);
        let converted = convert_message(&state, &latest_commands, &message);
        for event in converted {
            let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
        }
    }
    let lost = close_stream(&state);
    if let Some(agent) = lost {
        let _ = events.send(ProviderEvent::StreamLost { agent }).await; // 받는 쪽이 연결을 버렸다
    }
}

// cost: time O(1), heap O(1), stack O(1), io 1
// basis: estimate
/// 다음 줄을 읽는다. 끝난 subagent는 새 작업이 이어 시작하지 않는지 기다린 뒤에 끝으로 내므로, 확정 시각이 오면
/// 줄을 기다리다 말고 그 끝을 보낸다.
async fn next_line_or_settle(
    lines: &mut Lines<BufReader<ChildStdout>>,
    state: &Mutex<SessionState>,
    events: &mpsc::Sender<ProviderEvent>,
) -> std::io::Result<Option<String>> {
    loop {
        let settle_at = lock(state).next_settle();
        let Some(at) = settle_at else {
            return lines.next_line().await;
        };
        tokio::select! {
            line = lines.next_line() => return line,
            () = tokio::time::sleep_until(at.into()) => {
                let settled = background::settle_due(&mut lock(state), Instant::now());
                for event in settled {
                    let _ = events.send(event).await; // 받는 쪽이 연결을 버렸다
                }
            }
        }
    }
}

/// 받은 줄의 모양을 관측 기록에 남긴다. 방법 이름은 `type`이고, 하위 종류(`subtype`, 제어 요청은 `request.subtype`)가 있으면 `/`로 잇는다.
fn trace_line(trace: &ProviderTrace, message: &Value) {
    if !trace.is_enabled() {
        return;
    }
    let mut kind = message["type"].as_str().unwrap_or("unknown").to_owned();
    let subtype = message["subtype"]
        .as_str()
        .or_else(|| message["request"]["subtype"].as_str());
    if let Some(subtype) = subtype {
        kind.push('/');
        kind.push_str(subtype);
    }
    trace.record(Frame::Line, &kind, message);
}

fn convert_message(
    state: &Mutex<SessionState>,
    latest_commands: &Mutex<Vec<ProviderCommand>>,
    message: &Value,
) -> Vec<ProviderEvent> {
    let mut state = lock(state);
    if message["type"] == "control_response" {
        let id = message["response"]["request_id"]
            .as_str()
            .unwrap_or_default();
        if let Some(waiter) = state.control_waiters.remove(id) {
            let _ = waiter.send(message["response"].clone()); // 기다리던 쪽이 시간을 넘겨 포기했다
        }
        return Vec::new();
    }
    let converted = convert_line(&mut state, message);
    if message["type"] == "system" && message["subtype"] == "init" {
        *lock(latest_commands) = state.commands.clone();
    }
    converted
}

/// 턴이나 subagent가 남은 채 끝났으면 그 에이전트.
fn close_stream(state: &Mutex<SessionState>) -> Option<AgentId> {
    let mut state = lock(state);
    state.control_waiters.clear();
    state.permissions.clear();
    state.inputs.clear();
    let lost = state.turn_active || !state.running.is_empty();
    state.turn_active = false;
    state.clear_subagents();
    lost.then_some(state.agent)
}

pub(super) async fn log_stderr(stderr: ChildStderr, masker: Masker) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(line = %masker.mask(&line).as_str(), "claude stderr");
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::TraceHub;
    use crate::providers::test_support::CLAUDE;
    use saturn_protocol::ids::ChatId;

    #[test]
    fn kind_joins_type_with_its_subtype_or_the_control_request_subtype() {
        let home = tempfile::tempdir().unwrap();
        let hub = TraceHub::new(home.path());
        hub.set_enabled(ChatId(1), true);
        let trace = hub.link(ChatId(1), CLAUDE, &Masker::default());

        for message in [
            json!({ "type": "system", "subtype": "init", "session_id": "s-1" }),
            json!({ "type": "control_request", "request_id": "r-9", "request": { "subtype": "can_use_tool", "tool_name": "Bash" } }),
            json!({ "type": "assistant", "session_id": "s-1", "message": { "content": [{ "type": "text", "text": "private words" }] } }),
        ] {
            trace_line(&trace, &message);
        }

        let logs = home.path().join("logs");
        let text = std::fs::read_to_string(
            std::fs::read_dir(logs)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let kinds: Vec<&str> = lines.iter().map(|l| l["kind"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            ["system/init", "control_request/can_use_tool", "assistant"]
        );
        assert_eq!(lines[1]["ids"]["request_id"], json!(["r-9"]));
        assert_eq!(lines[0]["frame"], "line");
        for value in ["private words", "Bash"] {
            assert!(!text.contains(value), "{value} leaked into the trace");
        }
    }
}
