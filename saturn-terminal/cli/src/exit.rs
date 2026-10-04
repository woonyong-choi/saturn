//! 종료 코드. 모든 종료 경로가 이 열거형을 거친다.
//! 설계: docs/design/engine-lifecycle.md

use std::fmt;

use saturn_protocol::envelope::{ErrorKind, INVALID_PARAMS};
use saturn_tui::TuiError;
use saturn_tui::client::ClientError;

/// 숫자는 BSD sysexits와 셸 관례를 따른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitCode {
    Success,
    /// 그 밖의 실패. 확인 질문에 아니라고 답한 경우도 여기다.
    Failure,
    /// 호출을 바꿔야 풀리는 경우.
    Usage,
    /// 이어 열 채팅, 폴더, router 버전이 없다.
    NotFound,
    /// engine을 시작하지 못했거나, 답이 없거나, 교체에 실패했거나, 연결이 끊겼다.
    EngineUnavailable,
    /// engine이 예상하지 못한 오류로 요청을 거절했다.
    EngineInternal,
    /// 지금은 안 되고 나중에 된다.
    TryLater,
    /// router 키가 없거나 확인에 실패했다.
    RouterKey,
    /// 설정이 없거나 틀렸다.
    Config,
    /// 사용자가 창에서 중단했다.
    Interrupted,
}

impl ExitCode {
    pub(crate) fn number(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Failure => 1,
            Self::Usage => 2,
            Self::NotFound => 66,
            Self::EngineUnavailable => 69,
            Self::EngineInternal => 70,
            Self::TryLater => 75,
            Self::RouterKey => 77,
            Self::Config => 78,
            Self::Interrupted => 130,
        }
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        Self::from(code.number())
    }
}

/// 종료 코드를 달고 올라가는 오류. 문구와 원인 사슬은 감싼 오류 그대로다.
#[derive(Debug)]
pub(crate) struct Exit {
    code: ExitCode,
    error: anyhow::Error,
}

impl Exit {
    pub(crate) fn error(code: ExitCode, message: impl fmt::Display) -> anyhow::Error {
        Self::wrap(code, anyhow::anyhow!(message.to_string()))
    }

    /// 이미 종료 코드가 있는 오류는 그대로 둔다.
    pub(crate) fn wrap(code: ExitCode, error: anyhow::Error) -> anyhow::Error {
        if error.downcast_ref::<Self>().is_some() {
            return error;
        }
        anyhow::Error::new(Self { code, error })
    }
}

impl fmt::Display for Exit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for Exit {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.error.source()
    }
}

/// 오류 사슬에서 가장 바깥의 원인으로 종료 코드를 정한다. 어디에도 해당하지 않으면 `Failure`.
pub(crate) fn of(error: &anyhow::Error) -> ExitCode {
    error
        .chain()
        .find_map(|cause| {
            if let Some(exit) = cause.downcast_ref::<Exit>() {
                return Some(exit.code);
            }
            if let Some(client) = cause.downcast_ref::<ClientError>() {
                return Some(of_client(client));
            }
            match cause.downcast_ref::<TuiError>()? {
                TuiError::RouterKeyRequired { .. } => Some(ExitCode::RouterKey),
                TuiError::Aborted => Some(ExitCode::Interrupted),
                TuiError::TaskFailed => Some(ExitCode::Failure),
                _ => None,
            }
        })
        .unwrap_or(ExitCode::Failure)
}

fn of_client(error: &ClientError) -> ExitCode {
    match error {
        ClientError::NotRunning { .. } | ClientError::Closed => ExitCode::EngineUnavailable,
        ClientError::Decode(_) => ExitCode::EngineInternal,
        ClientError::Rejected { code, kind, .. } => match kind {
            Some(ErrorKind::NotFound) => ExitCode::NotFound,
            Some(ErrorKind::RetryLater) => ExitCode::TryLater,
            Some(ErrorKind::RouterKey) => ExitCode::RouterKey,
            Some(ErrorKind::Config) => ExitCode::Config,
            Some(ErrorKind::Failed) => ExitCode::Failure,
            None if *code == INVALID_PARAMS => ExitCode::Usage,
            None => ExitCode::EngineInternal,
        },
    }
}
