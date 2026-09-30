//! 하위 명령별 실행. 명령마다 engine 요청을 보내고 결과를 stdout에 쓴다. 진단은 stderr(`tracing`).

pub(crate) mod chat;
pub(crate) mod export;
pub(crate) mod judge;
pub(crate) mod prune;
pub(crate) mod train;
pub(crate) mod usage;
