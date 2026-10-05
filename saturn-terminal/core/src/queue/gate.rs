use std::path::{Path, PathBuf};

use saturn_protocol::ids::AgentId;

/// 트리가 유휴일 때 풀어 subagent가 쓰는 중에 다음 쓰기가 시작되지 않게 한다. 잠금은 쓰기 범위(작업 폴더와 더한 폴더)
/// 단위이고, 두 범위의 경로 하나씩이 같거나 한쪽이 다른 쪽의 조상이면 겹친다.
#[derive(Debug, Default)]
pub struct WriteGate {
    holders: Vec<(Vec<PathBuf>, AgentId)>,
}

impl WriteGate {
    // cost: time O(h * s^2), heap O(s), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수, s = 쓰기 범위의 경로 수
    // basis: estimate
    /// 다른 에이전트가 겹치는 범위를 쥐고 있으면 거짓, 자기가 이미 같은 범위를 쥐고 있으면 참.
    /// 경로는 호출하는 쪽이 정규화해서 넘긴다.
    pub fn try_acquire(&mut self, scope: &[PathBuf], agent: AgentId) -> bool {
        if self.is_held_by_other(scope, Some(agent)) {
            return false;
        }
        let is_held_by_self = self
            .holders
            .iter()
            .any(|(held, holder)| held == scope && *holder == agent);
        if !is_held_by_self {
            self.holders.push((scope.to_vec(), agent));
        }
        true
    }

    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    pub fn release(&mut self, agent: AgentId) {
        self.holders.retain(|(_, holder)| *holder != agent);
    }

    // cost: time O(h * s^2), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수, s = 쓰기 범위의 경로 수
    // basis: estimate
    pub(super) fn is_held_by_other(&self, scope: &[PathBuf], agent: Option<AgentId>) -> bool {
        self.holders
            .iter()
            .any(|(held, holder)| Some(*holder) != agent && overlaps(held, scope))
    }

    // cost: time O(h * s^2), heap O(h), stack O(1), alloc 1
    // vars: h = 잠금을 쥔 에이전트 수, s = 쓰기 범위의 경로 수
    // basis: estimate
    /// `scope`와 겹치는 범위를 쥔 에이전트. 쓰기 차례를 기다리는 입력이 누구를 기다리는지 가린다.
    pub(super) fn holders_overlapping(&self, scope: &[PathBuf]) -> Vec<AgentId> {
        self.holders
            .iter()
            .filter(|(held, _)| overlaps(held, scope))
            .map(|(_, holder)| *holder)
            .collect()
    }
}

// cost: time O(a * b * d), heap O(1), stack O(1)
// vars: a, b = 두 범위의 경로 수, d = 경로 깊이
// basis: estimate
/// 두 범위에서 경로 하나씩 골랐을 때 같거나 한쪽이 다른 쪽의 조상인 쌍이 있으면 참. 이름의 앞부분만 같은
/// 폴더(`/work`와 `/work-extra`)는 겹치지 않는다.
pub(super) fn overlaps(first: &[PathBuf], second: &[PathBuf]) -> bool {
    first.iter().any(|a| {
        second
            .iter()
            .any(|b| Path::starts_with(a, b) || Path::starts_with(b, a))
    })
}
