//! 전체 화면 TUI. engine과는 `protocol` 메시지로만 주고받고 provider를 모른다.
//!
//! 설계: docs/design/tui.md.

// TODO(#74): 뼈대 단계라 본문이 `todo!`인 함수의 인자가 쓰이지 않는다. 구현 이슈가 모두 닫히면 이 허용을 지운다
#![allow(unused_variables, dead_code)]

pub mod client;
