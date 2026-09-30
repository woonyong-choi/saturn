//! engine과 TUI, CLI가 주고받는 메시지 타입. provider 고유 이름은 여기에 넣지 않는다.
//!
//! 이 crate의 타입이 JSON-RPC 메시지의 정본이다. JSON Schema와 TypeScript 타입은 여기서 만든다.
//! TODO(#75): `schemars`로 JSON Schema, `ts-rs`로 TypeScript 타입을 만드는 생성 명령 추가

pub mod event;
pub mod ids;
pub mod rpc;
pub mod state;
