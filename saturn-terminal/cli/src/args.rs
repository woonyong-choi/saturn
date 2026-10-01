//! 명령줄 정의. judge 키 인자는 두지 않는다.
//! 설계: docs/design/judge-key-security.md

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use saturn_core::sessions::ranking::DEFAULT_RRF_K;
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
    /// 개발용. 실험 수집기가 `SATURN_PACKET_CMD`로 불러 패킷을 만든다. engine에 붙지 않는다.
    #[command(hide = true)]
    Packet(PacketArgs),
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
    /// 이 judge 버전에서 다시 학습한다(`Request::Train`의 `from`).
    #[arg(long, value_name = "VERSION")]
    pub(crate) from: Option<String>,
}

/// `prune` 인자. TODO(#42): 판단 기록 전용 정리의 명령 이름과 자리
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

/// `packet` 인자. 입력은 표준 입력의 기록 JSONL이거나 `--scenarios` 파일이다.
#[derive(Debug, Args)]
pub(crate) struct PacketArgs {
    /// `saturn`은 Saturn 기록 원문으로, `provider`는 `--summary-file`의 요약을 경쟁 구역 첫 항목으로 채운다.
    #[arg(long, value_enum, default_value_t = PacketMode::Saturn)]
    pub(crate) mode: PacketMode,
    /// 패킷 상한 `P_max`(토큰). 발동 기준 `T`는 이 값의 10배로 둔다.
    #[arg(long, visible_alias = "budget", value_name = "TOKENS")]
    pub(crate) budget_tokens: u64,
    /// 경쟁 구역의 순서 규칙. 없으면 `--judgments`가 있을 때 `judge-only`, 없을 때 `rrf-only`다.
    #[arg(long, value_enum)]
    pub(crate) condition: Option<PacketCondition>,
    /// `rrf-judge`에서 judge 판단을 쓰는 RRF 상위 후보 수.
    #[arg(long, value_name = "N", default_value_t = 10)]
    pub(crate) top_n: usize,
    /// RRF 합치기 상수.
    #[arg(long, value_name = "K", default_value_t = DEFAULT_RRF_K)]
    pub(crate) k: u32,
    /// judge가 남기는 확률의 하한.
    #[arg(long, value_name = "P", default_value_t = 0.5)]
    pub(crate) keep_threshold: f64,
    /// judge 판단 JSON(`compact`, `constraints`). 없으면 RRF 순서로 채운다.
    #[arg(long, value_name = "FILE")]
    pub(crate) judgments: Option<PathBuf>,
    /// 시나리오 JSONL. 있으면 표준 입력 대신 이 파일의 `--scenario-id` 시나리오를 읽는다.
    #[arg(long, value_name = "FILE", requires = "scenario_id")]
    pub(crate) scenarios: Option<PathBuf>,
    /// `--scenarios`에서 고를 시나리오.
    #[arg(long, value_name = "ID", requires = "scenarios")]
    pub(crate) scenario_id: Option<String>,
    /// provider 압축 요약 글 파일.
    #[arg(long, value_name = "FILE", requires = "summary_record")]
    pub(crate) summary_file: Option<PathBuf>,
    /// 요약 시점의 마지막 기록 번호. 이 번호 뒤의 기록에서만 경쟁 구역을 고른다.
    #[arg(long, value_name = "RECORD_NO", requires = "summary_file")]
    pub(crate) summary_record: Option<u64>,
    /// 출력 형식. 없으면 `--scenarios`가 있을 때 `json`, 없을 때 `text`다.
    #[arg(long, value_enum)]
    pub(crate) format: Option<PacketFormat>,
}

/// 패킷 정리 모드.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketMode {
    /// Saturn 기록 원문에서 고른다.
    Saturn,
    /// provider 압축 요약을 첫 항목으로 쓴다.
    Provider,
}

/// 경쟁 구역 순서 조건.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketCondition {
    /// judge 판단을 쓰지 않고 RRF 순서로 채운다.
    RrfOnly,
    /// RRF 상위 `--top-n`개의 판단만 쓴다.
    RrfJudge,
    /// 후보 전체의 판단을 쓴다.
    JudgeOnly,
}

/// 패킷 출력 형식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum PacketFormat {
    /// 패킷 글만.
    Text,
    /// 패킷 글과 넣은 항목, RRF 순서를 담은 JSON.
    Json,
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
    pub(crate) key: String,
    pub(crate) value: String,
}

impl FromStr for ConfigOverride {
    type Err = String;

    /// `=`가 없거나 키가 비면 오류.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        todo!("#93")
    }
}
