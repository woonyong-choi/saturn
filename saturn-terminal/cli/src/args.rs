//! 명령줄 정의. judge 키 인자는 두지 않는다.
//! 설계: docs/design/judge-key-security.md

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::UsageRange;

/// `saturn` 명령줄.
#[derive(Debug, Parser)]
#[command(
    name = "saturn",
    version,
    about = "Codex와 Claude Code를 하나의 대화로 이어 쓰는 터미널 도구"
)]
pub(crate) struct Cli {
    /// 이번 실행의 설정 값(`-c key=value`, 여러 번). 설정의 실행 층이 된다.
    #[arg(short = 'c', value_name = "KEY=VALUE")]
    pub(crate) config: Vec<ConfigOverride>,
    /// 현재 폴더에서 가장 최근에 쓴 채팅을 잇는다.
    #[arg(long = "continue", conflicts_with = "resume")]
    pub(crate) continue_last: bool,
    /// 채팅 id가 있으면 그 채팅을, 없으면 현재 폴더의 채팅 목록에서 골라 잇는다. `all`은 모든 폴더의 목록.
    #[arg(long, value_name = "CHAT_ID|all", num_args = 0..=1)]
    pub(crate) resume: Option<Option<ResumeTarget>>,
    /// 채팅에 폴더를 더한다(여러 번). 더한 폴더는 채팅 기록에 저장되고 모든 provider session이 그 폴더에 접근한다.
    #[arg(long = "add-dir", value_name = "DIR")]
    pub(crate) add_dir: Vec<PathBuf>,
    /// 하위 명령. 없으면 대화 화면(터미널이면 전체 화면, 파이프나 CI면 plain)을 연다.
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

impl Cli {
    /// 이어 열기 인자와 `--add-dir`가 하위 명령과 함께 오면 오류. 둘 다 대화 화면에만 있다.
    pub(crate) fn open_mode(&self) -> Result<OpenMode, clap::Error> {
        let mode = match (&self.resume, self.continue_last) {
            (Some(None), _) => OpenMode::PickInFolder,
            (Some(Some(ResumeTarget::All)), _) => OpenMode::PickInAll,
            (Some(Some(ResumeTarget::Chat(chat))), _) => OpenMode::Chat(*chat),
            (None, true) => OpenMode::ContinueLast,
            (None, false) => OpenMode::New,
        };
        if self.command.is_some() && (mode != OpenMode::New || !self.add_dir.is_empty()) {
            return Err(Cli::command().error(
                clap::error::ErrorKind::ArgumentConflict,
                "--continue, --resume and --add-dir open the chat screen and cannot be used with a subcommand",
            ));
        }
        Ok(mode)
    }
}

/// `--resume`의 값. 채팅 id는 숫자라 `all`과 겹치지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeTarget {
    Chat(ChatId),
    All,
}

impl FromStr for ResumeTarget {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == "all" {
            return Ok(Self::All);
        }
        text.parse::<u64>()
            .map(|id| Self::Chat(ChatId(id)))
            .map_err(|_| format!("expected a chat id (number) or `all`, got `{text}`"))
    }
}

/// 대화 화면을 여는 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    New,
    ContinueLast,
    PickInFolder,
    PickInAll,
    Chat(ChatId),
}

/// 하위 명령.
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
    /// 이 judge 버전에서 다시 학습한다(`Request::Train`의 `from`).
    #[arg(long, value_name = "VERSION")]
    pub(crate) from: Option<String>,
}

/// `prune` 인자.
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
    pub(crate) key: String,
    pub(crate) value: String,
}

impl FromStr for ConfigOverride {
    type Err = String;

    // cost: time O(n), heap O(n), stack O(1), alloc 2
    // vars: n = text.len()
    // basis: estimate
    /// `=`가 없거나 키가 비면 오류.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (key, value) = text
            .split_once('=')
            .ok_or_else(|| format!("expected KEY=VALUE, got `{text}`"))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("key is empty in `{text}`"));
        }
        Ok(Self {
            key: key.to_owned(),
            value: value.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("saturn").chain(args.iter().copied()))
    }

    #[test]
    fn config_override_splits_at_first_equals() {
        let parsed: ConfigOverride = "judge.url=https://a.example/?x=1".parse().unwrap();

        assert_eq!(parsed.key, "judge.url");
        assert_eq!(parsed.value, "https://a.example/?x=1");
    }

    #[test]
    fn config_override_without_equals_or_key_is_error() {
        assert!("permission.mode".parse::<ConfigOverride>().is_err());
        assert!("=full".parse::<ConfigOverride>().is_err());
    }

    #[test]
    fn config_override_allows_empty_value() {
        let parsed: ConfigOverride = "judge.key.command=".parse().unwrap();

        assert_eq!(parsed.value, "");
    }

    #[test]
    fn continue_resume_without_arguments_opens_new_chat() {
        assert_eq!(parse(&[]).unwrap().open_mode().unwrap(), OpenMode::New);
    }

    #[test]
    fn continue_resume_continue_flag_picks_last_chat() {
        let mode = parse(&["--continue"]).unwrap().open_mode().unwrap();

        assert_eq!(mode, OpenMode::ContinueLast);
    }

    #[test]
    fn continue_resume_without_id_picks_in_folder() {
        let mode = parse(&["--resume"]).unwrap().open_mode().unwrap();

        assert_eq!(mode, OpenMode::PickInFolder);
    }

    #[test]
    fn continue_resume_all_picks_in_every_folder() {
        let mode = parse(&["--resume", "all"]).unwrap().open_mode().unwrap();

        assert_eq!(mode, OpenMode::PickInAll);
    }

    #[test]
    fn continue_resume_numeric_id_opens_that_chat() {
        let mode = parse(&["--resume", "42"]).unwrap().open_mode().unwrap();

        assert_eq!(mode, OpenMode::Chat(ChatId(42)));
    }

    #[test]
    fn continue_resume_rejects_non_numeric_id() {
        assert!(parse(&["--resume", "latest"]).is_err());
    }

    #[test]
    fn continue_resume_has_no_short_names() {
        assert!(parse(&["-r"]).is_err());
        assert!(parse(&["-r", "all"]).is_err());
    }

    #[test]
    fn continue_resume_short_c_stays_config_layer() {
        let cli = parse(&["-c", "permission.mode=\"full\""]).unwrap();

        assert_eq!(cli.config.len(), 1);
        assert_eq!(cli.open_mode().unwrap(), OpenMode::New);
    }

    #[test]
    fn continue_resume_conflict_is_error() {
        assert!(parse(&["--continue", "--resume"]).is_err());
    }

    #[test]
    fn continue_resume_with_subcommand_is_error() {
        let cli = parse(&["--continue", "usage"]).unwrap();

        assert!(cli.open_mode().is_err());
    }

    #[test]
    fn add_dir_repeats_and_conflicts_with_subcommands() {
        let cli = parse(&["--add-dir", "/a", "--add-dir", "b"]).unwrap();
        let with_command = parse(&["--add-dir", "/a", "usage"]).unwrap();

        assert_eq!(cli.add_dir, vec![PathBuf::from("/a"), PathBuf::from("b")]);
        assert!(cli.open_mode().is_ok());
        assert!(with_command.open_mode().is_err());
    }

    #[test]
    fn config_overrides_repeat() {
        let cli = parse(&["-c", "a=1", "-c", "b=2", "usage"]).unwrap();

        assert_eq!(cli.config.len(), 2);
    }
}
