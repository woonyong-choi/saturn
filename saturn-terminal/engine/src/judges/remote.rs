//! 외부 판단기 API.

use saturn_core::judges::JudgeClient;

pub struct RemoteJudge;

impl JudgeClient for RemoteJudge {}
