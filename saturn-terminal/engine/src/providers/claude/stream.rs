use std::sync::{Arc, Mutex};

use saturn_core::providers::ProviderCommand;
use saturn_protocol::event::ProviderEvent;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{ChildStderr, ChildStdout};
use tokio::sync::mpsc;

use super::convert::convert_line;
use super::{SessionState, lock};
use crate::providers::codex::mask_values;
use crate::secrets::Masker;
use saturn_protocol::ids::AgentId;

/// 끝나면 턴이나 subagent가 남았을 때 `StreamLost`.
pub(super) async fn read_loop(
    stdout: ChildStdout,
    state: Arc<Mutex<SessionState>>,
    events: mpsc::Sender<ProviderEvent>,
    latest_commands: Arc<Mutex<Vec<ProviderCommand>>>,
    masker: Masker,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(mut message) = serde_json::from_str::<Value>(&line) else {
            tracing::debug!("skipping non-json line from claude");
            continue;
        };
        mask_values(&mut message, &masker);
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
    state.running.clear();
    lost.then_some(state.agent)
}

pub(super) async fn log_stderr(stderr: ChildStderr, masker: Masker) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(line = %masker.mask(&line).as_str(), "claude stderr");
    }
}
