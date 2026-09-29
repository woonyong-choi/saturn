//! Saturn 엔진: saturn-core의 규칙을 실제 연결(공급자, 판단기, SQLite, 프로세스, 키)과 조립해 화면 없이 돌린다.

pub mod judges;
pub mod processes;
pub mod providers;
pub mod rpc;
pub mod secrets;
pub mod settings;
pub mod store;

/// 엔진 진입점.
pub fn run() {}
