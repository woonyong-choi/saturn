//! 순수 규칙: 대기열, 세션, 에이전트 트리 판정, 판단. 외부 연결(DB, 네트워크, 프로세스)을 모른다.
//! 외부와 닿는 부분은 trait으로만 정의하고, 구현은 saturn-engine이 맡는다.

pub mod agents;
pub mod judges;
pub mod providers;
pub mod queue;
pub mod sessions;
