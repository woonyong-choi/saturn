//! 순수 규칙. 파일, 네트워크, 프로세스는 직접 다루지 않고 trait으로만 정한다.
//! 설계: docs/architecture.md

// TODO(#74): `todo!` 뼈대의 미사용 인자 허용. 구현 이슈가 모두 닫히면 지운다
#![allow(unused_variables, dead_code)]

pub mod agents;
pub mod permission;
pub mod providers;
pub mod queue;
pub mod routers;
pub mod sessions;
