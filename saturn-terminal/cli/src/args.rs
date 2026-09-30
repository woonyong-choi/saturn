//! 명령줄 정의(clap derive). 하위 명령이 없으면 대화 화면을 연다.
//!
//! 설계: docs/design/records.md(정리, 내보내기), docs/design/judge-training.md(`train`, `judge version`),
//! docs/design/settings.md(`-c` 실행 층), docs/design/tui.md(`usage`).
//! judge 키는 명령 인자로 받지 않는다(docs/design/judge-key-security.md). 키 인자를 더하지 않는다.

use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use saturn_protocol::rpc::UsageRange;

/// `saturn` 명령줄.
#[derive(Debug, Parser)]
#[command(
    name = "saturn",
    version,
    about = "Codex와 Claude Code를 하나의 대화로 이어 쓰는 터미널 도구"
)]
pub(crate) struct Cli {
    /// 이번 실행의 설정 값(`-c key=value`, 여러 번). 설정의 실행 층이 된다. TODO(#49): 설정 키 목록
    #[arg(short = 'c', value_name = "KEY=VALUE")]
    pub(crate) config: Vec<ConfigOverride>,
    /// 하위 명령. 없으면 대화 화면(터미널이면 전체 화면, 파이프나 CI면 plain)을 연다.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// 하위 명령. TODO(#42): 명령 이름 확정
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// 판단 모델 학습. 채점 안 된 판단이 200건 미만이면 engine이 거절한다.
    Train(TrainArgs),
    /// 기록 정리. `--yes`가 없으면 지울 대상만 미리 보인다.
    Prune(PruneArgs),
    /// 판단 기록을 JSONL로 내보낸다. 채점하지 않은 기록도 내보낸다.
    Export(ExportArgs),
    /// judge 관리.
    Judge {
        /// judge 하위 명령.
        #[command(subcommand)]
        command: JudgeCommand,
    },
    /// 사용량 조회.
    Usage(UsageArgs),
}

/// `judge` 하위 명령.
#[derive(Debug, Subcommand)]
pub(crate) enum JudgeCommand {
    /// 고른 judge 버전을 확인 한 줄 뒤 현재 버전으로 쓴다.
    Version(JudgeVersionArgs),
}

/// `train` 인자.
#[derive(Debug, Args)]
pub(crate) struct TrainArgs {
    /// 기준값을 1차 영점으로 되돌린다.
    #[arg(long)]
    pub(crate) reset_thresholds: bool,
    /// 이 judge 버전에서 다시 학습한다. TODO(#46): protocol `Train` 요청에 시작 버전 필드가 없다
    #[arg(long, value_name = "VERSION")]
    pub(crate) from: Option<String>,
}

/// `prune` 인자.
///
/// TODO(#42): 판단 기록 전용 정리(docs/design/records.md)의 명령 이름과 자리
#[derive(Debug, Args)]
pub(crate) struct PruneArgs {
    /// 미리보기 없이 지운다.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `export` 인자.
#[derive(Debug, Args)]
pub(crate) struct ExportArgs {
    /// 쓸 JSONL 파일 경로. engine이 이 경로에 쓴다.
    #[arg(value_name = "PATH")]
    pub(crate) path: std::path::PathBuf,
}

/// `judge version` 인자.
#[derive(Debug, Args)]
pub(crate) struct JudgeVersionArgs {
    /// 쓸 judge 버전. TODO(#49): 버전 표기 형식
    #[arg(value_name = "VERSION")]
    pub(crate) version: String,
}

/// `usage` 인자.
#[derive(Debug, Args)]
pub(crate) struct UsageArgs {
    /// 조회 범위.
    #[arg(long, value_enum, default_value_t = UsageRangeArg::Chat)]
    pub(crate) range: UsageRangeArg,
}

/// 명령줄의 사용량 범위. protocol `UsageRange`에 clap 의존을 넣지 않으려고 따로 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum UsageRangeArg {
    /// 현재 채팅.
    Chat,
    /// 오늘.
    Today,
    /// 이번 주.
    Week,
    /// 전체.
    All,
}

impl From<UsageRangeArg> for UsageRange {
    fn from(range: UsageRangeArg) -> Self {
        match range {
            UsageRangeArg::Chat => Self::Chat,
            UsageRangeArg::Today => Self::Today,
            UsageRangeArg::Week => Self::Week,
            UsageRangeArg::All => Self::All,
        }
    }
}

/// `-c key=value` 한 개. 값 검사는 engine의 설정 병합이 한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigOverride {
    /// 설정 키.
    pub(crate) key: String,
    /// 설정 값 원문.
    pub(crate) value: String,
}

impl FromStr for ConfigOverride {
    type Err = String;

    /// `key=value`를 나눈다. `=`가 없거나 키가 비면 오류.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        todo!("#93")
    }
}
