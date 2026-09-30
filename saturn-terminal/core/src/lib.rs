//! 순수 규칙: 대기열, session, 에이전트 트리, 판단 규칙. 파일, 네트워크, 프로세스를 직접 다루지 않는다.
//!
//! 외부와 닿는 부분은 trait(`ProviderClient`, `JudgeClient`)으로만 정하고 구현은 `saturn-engine`이 맡는다.
//! engine은 기록 저장소에 먼저 쓰고 나서 이 crate의 규칙 함수를 불러 다음 행동을 받는다.
//! 설계: docs/architecture.md(코드 지도, 불변 조건).

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

pub mod agents;
pub mod judges;
pub mod providers;
pub mod queue;
pub mod sessions;
