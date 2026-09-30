//! Saturn engine: `saturn-core` 규칙을 실제 연결(provider, judge, SQLite, 프로세스, 키체인)과 조립해 화면 없이 돌린다.
//!
//! 설계: docs/architecture.md, docs/design/engine-lifecycle.md. 사용자당 하나이고 잠금으로 지킨다.
//! TODO(#90): `Engine` 조립과 입력 흐름은 이 파일에 둔다(뼈대 2단계에서 작성)

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

pub mod judges;
pub mod processes;
pub mod providers;
pub mod rpc;
pub mod secrets;
pub mod settings;
pub mod store;
