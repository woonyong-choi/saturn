//! engine과 TUI, CLI가 주고받는 메시지 타입. provider 고유 이름은 여기에 넣지 않는다.
//!
//! 이 crate의 타입이 JSON-RPC 메시지의 정본이다. JSON Schema와 TypeScript 타입은 `codegen`이 여기서 만든다.

pub mod codegen;
pub mod envelope;
pub mod event;
pub mod ids;
pub mod rpc;
pub mod state;
