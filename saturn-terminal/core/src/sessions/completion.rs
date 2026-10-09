//! 끝난 실행의 완료 검사 근거를 가르는 순수 규칙. 마지막 수정 뒤 설정한 검사 명령이 실제로 종료 코드 0으로 끝났는지만 본다.
//! 설계: docs/design/providers-and-sessions.md#완료-검사-근거
//!
//! 파일과 프로세스는 다루지 않는다. 수정 목록(`changes`)과 기록된 이벤트를 받아 판정만 하고, 검사를 실행하거나
//! 입력을 다시 보내지 않는다.

use std::collections::HashSet;

use saturn_protocol::event::{Activity, ProviderEvent, ToolCategory};
use saturn_protocol::ids::{AgentId, SubagentId};
use saturn_protocol::state::{CompletionEvidence, EvidenceState, UnverifiedReason};

use super::changes::ChangeSet;
use crate::permission::{is_plain_shell, is_read_only_plain_shell};

/// 이 명령은 종료 코드가 검사 결과와 무관하므로 검사 명령으로 설정해도 인정하지 않는다.
const TRIVIAL_COMMANDS: &[&str] = &["echo", "printf", "true", "false", ":", "exit", "test", "["];

/// 판정에 쓰는 값. `events`는 기록 번호(채팅 안의 `events.seq`)와 이벤트를 번호순으로 담는다.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceInput<'a> {
    /// 실행 시작 때 폴더 상태가 없어 목록을 만들지 못했으면 `None`.
    pub changes: Option<&'a ChangeSet>,
    pub events: &'a [(u64, ProviderEvent)],
    /// 설정한 검사 명령. 명령 앞부분이 낱말 경계에서 같으면 그 검사로 본다.
    pub checks: &'a [String],
}

/// 도구 호출 하나와 그 결과.
struct Call {
    start: u64,
    /// 결과 이벤트의 번호. 결과가 없으면 `None`.
    end: Option<u64>,
    /// 셸 명령이면 그 글.
    command: Option<String>,
    is_edit: bool,
    exit_code: Option<i32>,
    /// 인정한 검사 명령이면 설정 목록의 자리.
    check: Option<usize>,
}

impl Call {
    /// 파일을 바꿀 수 있는 셸 명령. 인정한 검사와 읽기 전용 명령은 뺀다.
    fn is_shell_barrier(&self) -> bool {
        self.check.is_none()
            && self
                .command
                .as_deref()
                .is_some_and(|command| !is_read_only_plain_shell(command))
    }

    /// 수정일 수 있는 호출: 파일 수정 도구와 `is_shell_barrier`.
    fn is_barrier(&self) -> bool {
        (self.check.is_none() && self.is_edit) || self.is_shell_barrier()
    }

    /// 호출이 끝난 기록 번호. 결과가 없으면 파일 수정 도구는 시작과 같고 셸 명령은 끝나지 않은 것으로 본다.
    fn end_bound(&self) -> u64 {
        self.end
            .unwrap_or(if self.is_edit { self.start } else { u64::MAX })
    }
}

// cost: time O(e + c·e), heap O(e), stack O(1)
// vars: e = 실행의 이벤트 수, c = 설정한 검사 수
// basis: estimate
/// 순서는 위에서부터 먼저 걸리는 것이 답이다. 수정이 없으면 `NotApplicable`, 목록을 못 만들었거나 부분이거나 하위 에이전트가
/// 남았거나 검사가 없거나 수정과 검사의 순서를 모르면 `Unverified`, 모든 설정 검사가 마지막 수정 뒤 종료 코드 0으로
/// 끝났을 때만 `Verified`다.
#[must_use]
pub fn judge(input: EvidenceInput<'_>) -> CompletionEvidence {
    let unverified = |reason| CompletionEvidence {
        state: EvidenceState::Unverified,
        reason: Some(reason),
        events: Vec::new(),
    };
    let Some(changes) = input.changes else {
        return unverified(UnverifiedReason::Unmeasured);
    };
    if changes.files.is_empty() && !changes.is_partial {
        return CompletionEvidence {
            state: EvidenceState::NotApplicable,
            reason: None,
            events: Vec::new(),
        };
    }
    if changes.is_partial {
        return unverified(UnverifiedReason::PartialSnapshot);
    }
    if tree_has_residue(input.events) {
        return unverified(UnverifiedReason::TreeNotIdle);
    }
    let checks: Vec<String> = input
        .checks
        .iter()
        .filter(|check| is_valid_check(check))
        .map(|check| normalize(check))
        .collect();
    if checks.is_empty() {
        return unverified(UnverifiedReason::NotChecked);
    }
    let calls = calls_of(input.events, &checks);
    let has_shell_barrier = calls.iter().any(Call::is_shell_barrier);
    if !has_shell_barrier && changes.files.iter().any(|file| file.actors.is_empty()) {
        return unverified(UnverifiedReason::OrderUnknown);
    }
    let mut evidence = Vec::new();
    let mut worst: Option<UnverifiedReason> = None;
    for index in 0..checks.len() {
        match judge_check(&calls, index) {
            Ok(seq) => evidence.push(seq),
            Err(reason) => worst = Some(worse(worst, reason)),
        }
    }
    if let Some(reason) = worst {
        return unverified(reason);
    }
    evidence.sort_unstable();
    evidence.dedup();
    CompletionEvidence {
        state: EvidenceState::Verified,
        reason: None,
        events: evidence,
    }
}

/// 설정한 검사 하나의 마지막 실행이 마지막 수정 뒤에 0으로 끝났으면 그 결과의 기록 번호.
fn judge_check(calls: &[Call], index: usize) -> Result<u64, UnverifiedReason> {
    let Some(run) = calls.iter().rfind(|call| call.check == Some(index)) else {
        return Err(UnverifiedReason::NotChecked);
    };
    let (Some(end), Some(exit_code)) = (run.end, run.exit_code) else {
        return Err(UnverifiedReason::NotChecked);
    };
    let barriers = || calls.iter().filter(|call| call.is_barrier());
    if barriers().any(|call| call.start < end && call.end_bound() > run.start) {
        return Err(UnverifiedReason::EditedDuringCheck);
    }
    if barriers().any(|call| call.start > end) {
        return Err(UnverifiedReason::NotChecked);
    }
    if exit_code != 0 {
        return Err(UnverifiedReason::CheckFailed);
    }
    Ok(end)
}

/// 여러 검사가 서로 다른 까닭으로 실패하면 더 먼저 고쳐야 하는 까닭을 남긴다.
fn worse(current: Option<UnverifiedReason>, next: UnverifiedReason) -> UnverifiedReason {
    let rank = |reason| match reason {
        UnverifiedReason::CheckFailed => 3,
        UnverifiedReason::EditedDuringCheck => 2,
        _ => 1,
    };
    match current {
        Some(current) if rank(current) >= rank(next) => current,
        _ => next,
    }
}

fn calls_of(events: &[(u64, ProviderEvent)], checks: &[String]) -> Vec<Call> {
    let mut calls: Vec<(Key, Call)> = Vec::new();
    for (seq, event) in events {
        match event {
            ProviderEvent::ToolCall {
                agent,
                subagent,
                call_id,
                activity,
                detail,
            } => {
                let command = match activity {
                    Activity::RunningCommand { command } => Some(command.clone()),
                    _ => None,
                };
                let check = command
                    .as_deref()
                    .and_then(|command| matched(checks, command));
                calls.push((
                    (*agent, subagent.clone(), call_id.clone()),
                    Call {
                        start: *seq,
                        end: None,
                        command,
                        is_edit: detail.category == ToolCategory::FileEdit,
                        exit_code: None,
                        check,
                    },
                ));
            }
            ProviderEvent::ToolResult {
                agent,
                subagent,
                call_id,
                exit_code,
                ..
            } => {
                let key = (*agent, subagent.clone(), call_id.clone());
                if let Some((_, call)) = calls
                    .iter_mut()
                    .rev()
                    .find(|(known, call)| *known == key && call.end.is_none())
                {
                    call.end = Some(*seq);
                    call.exit_code = *exit_code;
                }
            }
            _ => {}
        }
    }
    calls.into_iter().map(|(_, call)| call).collect()
}

type Key = (AgentId, Option<SubagentId>, String);

/// 단순한 명령 하나이고 앞부분이 설정한 검사와 낱말 경계에서 같으면 그 검사의 자리. 인자는 판정하지 않는다.
fn matched(checks: &[String], command: &str) -> Option<usize> {
    if !is_plain_shell(command) {
        return None;
    }
    let command = normalize(command);
    checks.iter().position(|check| {
        command == *check
            || command
                .strip_prefix(check.as_str())
                .is_some_and(|rest| rest.starts_with(' '))
    })
}

/// 단순한 명령 하나이고 첫 낱말이 종료 코드와 무관한 명령이 아닌 설정만 검사 명령으로 인정한다.
fn is_valid_check(check: &str) -> bool {
    is_plain_shell(check)
        && check
            .split_whitespace()
            .next()
            .is_some_and(|program| !TRIVIAL_COMMANDS.contains(&program))
}

fn normalize(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 끝나지 않았거나 끊긴 하위 에이전트가 있는지.
fn tree_has_residue(events: &[(u64, ProviderEvent)]) -> bool {
    let mut running: HashSet<&SubagentId> = HashSet::new();
    for (_, event) in events {
        match event {
            ProviderEvent::SubagentStarted { subagent, .. } => {
                running.insert(subagent);
            }
            ProviderEvent::SubagentEnded { subagent, .. } => {
                running.remove(subagent);
            }
            ProviderEvent::SubagentInterrupted { .. } => return true,
            _ => {}
        }
    }
    !running.is_empty()
}

#[cfg(test)]
mod tests;
