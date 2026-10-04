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
use crate::providers::mask_values;
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
    loop {
        let line = next_line_or_settle(&mut lines, &state, &events).await;
        let Ok(Some(line)) = line else {
            break;
        };
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
