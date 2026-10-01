//! provider 이벤트를 Saturn 용어로 바꾼 것. engine `providers`가 만들고 `core`와 TUI가 읽는다.
//!
//! 설계: docs/design/providers-and-sessions.md(이벤트 수신, 사용량 보고).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{AgentId, SubagentId};

/// provider 이벤트 한 조각. 공급자 고유 필드는 engine에서 모두 걸러 낸다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum ProviderEvent {
    /// 모델 글 조각.
    Text {
        agent: AgentId,
        subagent: Option<SubagentId>,
        text: String,
    },
    /// 도구 호출 시작. `activity`는 상태판 실행 줄의 하는 일이다.
    ToolCall {
        agent: AgentId,
        subagent: Option<SubagentId>,
        call_id: String,
        activity: Activity,
    },
    /// 도구 결과.
    ToolResult {
        agent: AgentId,
        subagent: Option<SubagentId>,
        call_id: String,
        output: String,
    },
    /// provider가 subagent를 띄웠다. 부모가 `None`이면 메인 에이전트 바로 아래다.
    SubagentStarted {
        agent: AgentId,
        subagent: SubagentId,
        parent: Option<SubagentId>,
    },
    /// subagent가 끝났다.
    SubagentEnded {
        agent: AgentId,
        subagent: SubagentId,
    },
    /// 허가 요청. 사용자 답이 올 때까지 경과 시간을 멈춘다.
    PermissionRequested {
        agent: AgentId,
        request_id: String,
        summary: String,
        reason: String,
    },
    /// 부모 턴 완료. Codex는 부모 작업의 `turn/completed`만 여기로 온다.
    TurnCompleted { agent: AgentId, origin: TurnOrigin },
    /// 사용량 보고 한 건. 원값과 범위를 그대로 담고 0으로 채우지 않는다.
    Usage(UsageReport),
    /// provider가 알린 활성 맥락 크기(토큰). 턴 끝마다 온다.
    ContextSize { agent: AgentId, tokens: Option<u64> },
    /// provider 흐름이 완료 신호 없이 끝났다. 효과 범위를 `Unobserved`로 바꾼다.
    StreamLost { agent: AgentId },
    /// provider 명령으로 바뀐 적용 설정 값. 막지 않고 기록만 한다.
    SettingsApplied {
        agent: AgentId,
        values: Vec<(String, String)>,
    },
}

/// 실행 줄의 하는 일.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Activity {
    /// 생각 중.
    Thinking,
    /// 파일 조회.
    ReadingFile,
    /// 파일 수정.
    EditingFile,
    /// 명령 실행. 화면은 명령 앞 40칸만 보인다.
    RunningCommand { command: String },
    /// 맥락 정리 중.
    Compacting,
    /// provider 전환 중.
    SwitchingProvider,
}

/// 턴을 누가 시작했나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum TurnOrigin {
    /// 사용자 입력으로 시작.
    User,
    /// 입력 없이 provider가 시작(`provider-wake`).
    ProviderWake,
}

/// 사용량 보고 원값. 보고하지 않은 값은 `None`으로 두고 0으로 채우지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct UsageReport {
    /// 보고 대상 에이전트.
    pub agent: AgentId,
    /// 보고 대상 subagent. 메인이면 `None`.
    pub subagent: Option<SubagentId>,
    /// 보고한 모델 이름. 보고하지 않으면 `None`(`모델 미보고`).
    pub model: Option<String>,
    /// 값의 범위. 턴 값 계산 방식을 가른다.
    pub scope: UsageScope,
    /// 새 입력 토큰.
    pub input: Option<u64>,
    /// 캐시 읽기 토큰.
    pub cache_read: Option<u64>,
    /// 캐시 쓰기 토큰.
    pub cache_write: Option<u64>,
    /// 출력 토큰.
    pub output: Option<u64>,
    /// 추론 토큰.
    pub reasoning: Option<u64>,
}

/// 사용량 보고 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub enum UsageScope {
    /// 메인 턴 하나의 값. 보고값을 그대로 턴 값으로 쓴다.
    MainTurn,
    /// 트리 전체 합계.
    TreeTotal,
    /// session 누적값(Codex `tokenUsage`). 같은 session의 직전 누적을 빼서 턴 값을 구한다.
    ThreadCumulative,
}
