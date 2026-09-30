//! 기록 저장소와 메시지에서 쓰는 식별자. 모두 engine이 발급하고 TUI는 그대로 돌려준다.

use serde::{Deserialize, Serialize};

/// 사용자가 보는 대화 하나. 여러 provider session이 한 채팅에 이어진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChatId(pub u64);

/// 사용자 입력 하나. 접수(ACK) 때 발급한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct InputId(pub u64);

/// 채팅 안 작업 하나. 화면 이름표(`A`, `B`, ...)는 `TaskLabel`로 따로 붙인다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TaskId(pub u64);

/// 화면 이름표 한 글자. 끝난 작업의 글자는 비어 있는 가장 앞 글자로 재사용한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskLabel(pub char);

/// Saturn이 띄운 에이전트 하나(메인 또는 보조). provider subagent는 `SubagentId`로 구분한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentId(pub u64);

/// provider가 스스로 띄운 subagent. provider가 준 id 문자열을 그대로 담는다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubagentId(pub String);

/// Saturn 쪽 session 기록 id. provider session id는 `ProviderSessionId`에 따로 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SessionId(pub u64);

/// provider가 준 session id(Codex `thread_id`, Claude session id). 재개할 때 쓴다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderSessionId(pub String);

/// provider 턴 실행 하나. 크래시 뒤 `EffectScope`로 재개 여부를 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RunId(pub u64);

/// 채팅 트리 전체 기록에 매긴 하나의 연속 번호. session마다 전달받은 번호를 기억한다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LedgerSeq(pub u64);

/// 설정 스냅샷 번호. 입력은 접수 때 고정한 번호로 끝까지 처리한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SettingsRevision(pub u64);

/// 채팅 상태 revision. judge 결과를 적용하기 직전에 비교한다(CAS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChatRevision(pub u64);

/// 판단 호출 기록 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct JudgmentId(pub u64);

/// provider 종류. 화면에는 이 값만 보이고 provider 고유 이름은 engine 안에 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Provider {
    /// Codex CLI app-server.
    Codex,
    /// Claude Code stream-json.
    Claude,
}
