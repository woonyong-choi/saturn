//! provider 이벤트를 Saturn 용어로 옮긴 것. 설계: docs/design/providers-and-sessions.md

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{AgentId, SubagentId};

/// provider 고유 필드는 engine에서 걸러 낸다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum ProviderEvent {
    Text {
        agent: AgentId,
        subagent: Option<SubagentId>,
        text: String,
    },
    ToolCall {
        agent: AgentId,
        subagent: Option<SubagentId>,
        call_id: String,
        activity: Activity,
    },
    ToolResult {
        agent: AgentId,
        subagent: Option<SubagentId>,
        call_id: String,
        output: String,
    },
    /// `parent`가 `None`이면 메인 에이전트 바로 아래.
    SubagentStarted {
        agent: AgentId,
        subagent: SubagentId,
        parent: Option<SubagentId>,
    },
    SubagentEnded {
        agent: AgentId,
        subagent: SubagentId,
    },
    PermissionRequested {
        agent: AgentId,
        request_id: String,
        summary: String,
        reason: String,
    },
    /// Codex는 부모 작업의 `turn/completed`만 온다.
    TurnCompleted {
        agent: AgentId,
        origin: TurnOrigin,
    },
    Usage(UsageReport),
    /// 토큰 수.
    ContextSize {
        agent: AgentId,
        tokens: Option<u64>,
    },
    /// 효과 범위를 `Unobserved`로 바꾼다.
    StreamLost {
        agent: AgentId,
    },
    /// 막지 않고 기록만 한다.
    SettingsApplied {
        agent: AgentId,
        values: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Activity {
    Thinking,
    ReadingFile,
    EditingFile,
    RunningCommand { command: String },
    Compacting,
    SwitchingProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TurnOrigin {
    User,
    /// `provider-wake`: 입력 없이 provider가 시작.
    ProviderWake,
}

/// 보고하지 않은 값은 0이 아니라 `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct UsageReport {
    pub agent: AgentId,
    pub subagent: Option<SubagentId>,
    pub model: Option<String>,
    pub scope: UsageScope,
    pub input: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub output: Option<u64>,
    pub reasoning: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum UsageScope {
    /// 보고값이 곧 턴 값.
    MainTurn,
    TreeTotal,
    /// Codex `tokenUsage`. 턴 값은 직전 누적과의 차이.
    ThreadCumulative,
}
