//! Saturn 소유 PreToolUse 훅: 키 저장소 조회 명령, 대체 파일 읽기, Saturn 비밀 파일 접근을 막는 판정과 실행별 훅 설정 생성.
//!
//! 설계: docs/design/judge-key-security.md(Saturn 소유 PreToolUse 훅). engine이 Claude를 실행할 때 실행별 설정으로 넘긴다.
//! 사용자가 둔 훅은 감싸거나 지우지 않는다. 실행별 설정에 Saturn 훅 항목 하나만 더하고 사용자 설정 파일은 고치지 않는다.
//! 훅 명령은 `saturn hook pre-tool-use`이고, 그 명령이 stdin의 도구 호출을 `ToolCall`로 읽어 `HookPolicy::check` 결과를 돌려준다.
//! 실제 차단 여부는 실험 #3, subagent 적용은 실험 #23으로 확인한다.

use std::path::{Path, PathBuf};

/// 훅이 보는 도구 호출 하나. provider 고유 형식은 `providers/claude`가 이것으로 바꿔 넘긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCall {
    /// 셸 명령 실행. 명령 문자열 원문.
    Command(String),
    /// 파일 읽기, 검색, 목록 등 경로를 받는 도구. 절대 경로로 바꾼 값.
    Path(PathBuf),
    /// 그 밖의 도구. 판정하지 않고 허용한다.
    Other,
}

/// 훅 판정.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookVerdict {
    /// 허용. 사용자의 다른 훅이 이어서 판정한다.
    Allow,
    /// 차단. 이유는 에이전트에 그대로 보이는 한 줄이다(키 경로나 키 값을 넣지 않는다).
    Deny {
        /// 차단 이유.
        reason: String,
    },
}

/// 차단 규칙. engine 시작 때 한 번 만든다.
#[derive(Debug, Clone)]
pub struct HookPolicy {
    /// 차단할 명령: macOS `security`의 조회 하위 명령(`find-generic-password`, `find-internet-password`, `dump-keychain`, `export`)과
    /// 키체인 파일을 여는 명령. 셸 연결(`;`, `&&`, `|`, `$(...)`)로 나뉜 각 부분을 따로 본다.
    blocked_commands: Vec<String>,
    /// 차단할 경로: 대체 키 파일(`~/.saturn/judge.key`), Saturn 비밀 파일, `~/Library/Keychains/` 아래. 심볼릭 링크를 푼 뒤 비교한다.
    blocked_paths: Vec<PathBuf>,
}

impl HookPolicy {
    /// `home`(`~/.saturn`)과 사용자 홈을 기준으로 차단 목록을 만든다.
    pub fn new(saturn_home: &Path, user_home: &Path) -> Self {
        todo!("#84")
    }

    /// 도구 호출을 판정한다. `Command`는 차단 명령이나 차단 경로를 담으면 `Deny`, `Path`는 차단 경로 아래면 `Deny`.
    pub fn check(&self, call: &ToolCall) -> HookVerdict {
        todo!("#84")
    }

    /// Claude 실행별 설정(`--settings`에 넘기는 JSON)의 `hooks.PreToolUse` 항목 하나. 모든 도구(`matcher: "*"`)에
    /// `saturn_bin hook pre-tool-use`를 건다. 사용자 설정과 합치는 일은 Claude가 한다(여기서 사용자 훅을 읽거나 바꾸지 않는다).
    pub fn pre_tool_use_settings(&self, saturn_bin: &Path) -> serde_json::Value {
        todo!("#84")
    }
}
