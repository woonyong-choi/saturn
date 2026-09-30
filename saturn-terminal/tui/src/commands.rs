//! `/` 명령: 입력창에서 제출한 줄의 해석과 팝업 명령 목록.
//!
//! 설계: docs/design/tui.md(키, 상태 표시), docs/design/input-handling.md(대기와 취소, 보류 재개와 보류 종료).
//! 명령은 `app::App`이 `Request`로 바꾼다. 해석은 순수 함수다.

use saturn_protocol::ids::TaskLabel;
use saturn_protocol::rpc::UsageRange;

/// 명령 해석 오류. 입력창 아래 한 줄로 보이고 입력은 보내지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// 목록에 없는 명령.
    #[error("unknown command: {name}")]
    Unknown { name: String },
    /// 인자가 명령 형식과 맞지 않는다.
    #[error("invalid argument for /{command}: {argument}")]
    InvalidArgument {
        command: &'static str,
        argument: String,
    },
}

/// Saturn 명령 하나(팝업 표시용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// 전체 경로(`judge version`). 팝업 `Enter`·`Tab`이 이 값을 입력창에 기입한다.
    pub path: &'static str,
    /// 한국어 키 설명. 화면에는 `Lang::tr`로 바꿔 쓴다.
    pub description: &'static str,
    /// 값 목록. 명령을 고른 뒤 팝업이 값 목록으로 바뀐다. 없으면 빈 배열.
    pub values: &'static [&'static str],
}

/// Saturn 명령 목록. 팝업 오른쪽 출처는 모두 `Saturn`이다. provider 명령은 engine이 알려 준다.
/// TODO(#41): 메인 에이전트가 아닌 provider의 명령을 고르면 session을 새로 열지, 메인 전환을 물을지, 거절할지
pub const SATURN_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        path: "help",
        description: "도움말",
        values: &[],
    },
    CommandSpec {
        path: "send",
        description: "대기 입력 지금 보내기",
        values: &[],
    },
    CommandSpec {
        path: "cancel",
        description: "보내기 전 입력 취소",
        values: &[],
    },
    CommandSpec {
        path: "continue",
        description: "보류 이어서",
        values: &[],
    },
    CommandSpec {
        path: "feedback",
        description: "판단 피드백",
        values: &["1", "2"],
    },
    CommandSpec {
        path: "tasks",
        description: "작업 목록",
        values: &[],
    },
    CommandSpec {
        path: "usage",
        description: "사용량",
        values: &["chat", "today", "week", "all"],
    },
    CommandSpec {
        path: "train",
        description: "판단 모델 학습",
        values: &[],
    },
    CommandSpec {
        path: "judge version",
        description: "판단 모델 버전",
        values: &[],
    },
];

/// 해석한 명령.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/help` 도움말.
    Help,
    /// `/send [이름표]` 대기 입력을 지금 보낸다(`Request::SendNow`). 이름표가 없으면 가장 최근 대기 입력.
    Send { target: Option<TaskLabel> },
    /// `/cancel [이름표]` 보내기 전 입력 취소(`Request::CancelInput`), 보류면 보류 닫기 확인. 없으면 가장 최근 대기 입력.
    Cancel { target: Option<TaskLabel> },
    /// `/continue [이름표]` 보류 재개(`Request::Continue`). 없으면 채팅의 보류 전부를 접수 순서로.
    Continue { target: Option<TaskLabel> },
    /// `/feedback 1|2` 피드백 질문 답. `1` 맞아요(`true`), `2` 아니에요(`false`).
    Feedback { correct: bool },
    /// `/tasks` 작업 목록 화면.
    Tasks,
    /// `/usage [chat|today|week|all]` 사용량 화면. 기본 `chat`.
    Usage { range: UsageRange },
    /// `/train [--reset-thresholds] [--from 버전]` 학습. 채점 후보가 200건 미만이면 engine이 거절한다.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// `/judge version` judge 버전 화면.
    JudgeVersion,
    /// 목록에 없는 provider 명령. 원문 그대로 메인 에이전트 provider에 넘긴다. TODO(#41): 비메인 provider 명령 처리
    Provider { line: String },
}

/// 제출한 줄을 명령으로 해석한다. `/`로 시작하지 않으면 `Ok(None)`(일반 입력).
/// 앞뒤 공백은 무시하고, 이름표 인자는 대문자 한 글자(`A`–`Z`)만 받는다.
///
/// # Errors
/// 알 수 없는 Saturn 명령이면 `Unknown`, 인자가 틀리면 `InvalidArgument`.
pub fn parse(line: &str) -> Result<Option<SlashCommand>, CommandError> {
    todo!("#92")
}

/// 입력 토큰(`/ju`)에 맞는 명령을 앞부분 일치 우선, 그다음 포함 순서로 고른다. 팝업이 최대 8행을 보인다.
pub fn filter(token: &str) -> Vec<&'static CommandSpec> {
    todo!("#92")
}
