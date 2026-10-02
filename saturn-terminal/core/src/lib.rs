//! 순수 규칙. 파일, 네트워크, 프로세스는 직접 다루지 않고 trait으로만 정한다.
//! 설계: docs/architecture.md

pub mod agents;
pub mod permission;
pub mod providers;
pub mod queue;
pub mod routers;
pub mod sessions;
