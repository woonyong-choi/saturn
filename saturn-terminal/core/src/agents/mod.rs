//! provider가 띄운 subagent 트리 추적, 트리 유휴 판정, 사용량 턴 값 계산.
//!
//! 설계: docs/design/providers-and-sessions.md(에이전트 트리 추적, 사용량 턴 값 계산).
//! subagent 사용은 막거나 바꾸지 않고 추적만 한다.

use std::collections::HashMap;

use saturn_protocol::event::{ProviderEvent, UsageReport};
use saturn_protocol::ids::{AgentId, SessionId, SubagentId};

/// Saturn 에이전트 하나와 그 아래 subagent 트리.
#[derive(Debug, Default)]
struct AgentTree {
    /// 부모 턴이 끝났는지. 부모 턴 완료만 작업 끝 후보다.
    main_done: bool,
    /// 살아 있는 subagent와 부모. 부모가 `None`이면 메인 바로 아래.
    running: HashMap<SubagentId, Option<SubagentId>>,
}

/// 에이전트별 subagent 트리와 사용량 보고 행을 모은다.
#[derive(Debug, Default)]
pub struct AgentTracker {
    trees: HashMap<AgentId, AgentTree>,
    usage: Vec<(SessionId, UsageReport)>,
}

/// 이벤트 하나를 반영한 뒤 에이전트 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeStatus {
    /// 메인이나 subagent가 실행 중.
    Running,
    /// 답은 나왔고 subagent가 남았다(`answered-tree-running`).
    AnsweredTreeRunning,
    /// 에이전트와 모든 subagent가 끝났다(트리 유휴). 작업 끝, 쓰기 잠금 해제, compaction 판정의 조건.
    TreeIdle,
}

/// 턴 하나의 사용량. 누적 보고는 차감해서 구하고, 중간 보고가 빠지면 여러 턴에 걸친다고 표시한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnUsage {
    /// 새 입력, 캐시 읽기, 캐시 쓰기, 출력, 추론. 보고되지 않았으면 `None`.
    pub tokens: [Option<u64>; 5],
    /// 중간 보고가 빠져 차이가 여러 턴에 걸쳤는지. 0으로 채우지 않는다.
    pub spans_turns: bool,
}

impl AgentTracker {
    /// 빈 추적기.
    pub fn new() -> Self {
        Self::default()
    }

    /// 이벤트를 반영한다. subagent 시작과 끝을 부모 아래에 등록하고, 사용량 보고는 원값과 범위를 행으로 기록한다.
    pub fn on_event(&mut self, session: SessionId, event: &ProviderEvent) -> TreeStatus {
        todo!("#79")
    }

    /// 에이전트가 트리 유휴인지.
    pub fn is_tree_idle(&self, agent: AgentId) -> bool {
        todo!("#79")
    }

    /// 멈춤 신호를 보낼 순서. 추적된 subagent부터(깊은 것 먼저), 마지막이 메인.
    pub fn interrupt_order(&self, agent: AgentId) -> Vec<Option<SubagentId>> {
        todo!("#79")
    }

    /// 턴 값 계산. 저장하지 않고 필요할 때 기록 순서로 계산한다.
    /// `ThreadCumulative`는 같은 session의 이번 누적에서 직전 누적을 빼고, session이 바뀌면 새로 시작한다. `MainTurn`은 그대로.
    pub fn turn_usage(&self, session: SessionId, index: usize) -> TurnUsage {
        todo!("#79")
    }
}
