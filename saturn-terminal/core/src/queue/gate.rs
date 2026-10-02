use std::path::{Path, PathBuf};

use saturn_protocol::ids::AgentId;

/// 트리가 유휴일 때 풀어 subagent가 쓰는 중에 다음 쓰기가 시작되지 않게 한다.
#[derive(Debug, Default)]
pub struct WriteGate {
    holders: Vec<(PathBuf, AgentId)>,
}

impl WriteGate {
    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    /// 다른 에이전트가 같은 폴더를 쥐고 있으면 거짓, 자기가 이미 쥐고 있으면 참.
    pub fn try_acquire(&mut self, workdir: &Path, agent: AgentId) -> bool {
        if self.is_held_by_other(workdir, Some(agent)) {
            return false;
        }
        let is_held_by_self = self
            .holders
            .iter()
            .any(|(path, holder)| path == workdir && *holder == agent);
        if !is_held_by_self {
            self.holders.push((workdir.to_path_buf(), agent));
        }
        true
    }

    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    pub fn release(&mut self, agent: AgentId) {
        self.holders.retain(|(_, holder)| *holder != agent);
    }

    // cost: time O(h), heap O(1), stack O(1)
    // vars: h = 잠금을 쥔 에이전트 수
    // basis: estimate
    pub(super) fn is_held_by_other(&self, workdir: &Path, agent: Option<AgentId>) -> bool {
        self.holders
            .iter()
            .any(|(path, holder)| path == workdir && Some(*holder) != agent)
    }
}
