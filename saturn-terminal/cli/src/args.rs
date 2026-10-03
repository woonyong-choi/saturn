//! 명령줄 정의. router 키 인자는 두지 않는다.
//! 설계: docs/design/router-key-security.md

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, CommandFactory, Parser, Subcommand};
use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::UsageRange;
use saturn_tui::i18n::{self, Lang};

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
    pub(crate) fn open_mode(&self, lang: Lang) -> Result<OpenMode, clap::Error> {
        let mode = match (&self.resume, self.continue_last) {
            (Some(None), _) => OpenMode::PickInFolder,
            (Some(Some(ResumeTarget::All)), _) => OpenMode::PickInAll,
            (Some(Some(ResumeTarget::Chat(chat))), _) => OpenMode::Chat(*chat),
            (None, true) => OpenMode::ContinueLast,
            (None, false) => OpenMode::New,
        };
        if self.command.is_some() && (mode != OpenMode::New || !self.add_dir.is_empty()) {
            return Err(command(lang).error(
                clap::error::ErrorKind::ArgumentConflict,
                lang.tr(i18n::CLI_ARGS_CONFLICT),
            ));
        }
        Ok(mode)
    }
}

// cost: time O(a), heap O(a), stack O(d)
// vars: a = 인자와 하위 명령 수, d = 하위 명령 깊이
// basis: estimate
/// 도움말은 doc 주석의 한국어를 키로 `lang`의 문구로 바꿔 보인다. clap 자체 문구(`Usage:` 등)는 그대로다.
pub(crate) fn command(lang: Lang) -> clap::Command {
    localize(Cli::command(), lang)
}

fn localize(mut command: clap::Command, lang: Lang) -> clap::Command {
    if let Some(about) = command.get_about().map(ToString::to_string) {
        command = command.about(lang.tr(&about).to_owned());
    }
    let helps: Vec<(String, String)> = command
        .get_arguments()
        .filter_map(|arg| Some((arg.get_id().to_string(), arg.get_help()?.to_string())))
        .collect();
    for (id, help) in helps {
        command = command.mut_arg(id, |arg| arg.help(lang.tr(&help).to_owned()));
    }
    let names: Vec<String> = command
        .get_subcommands()
        .map(|sub| sub.get_name().to_owned())
        .collect();
    for name in names {
        command = command.mut_subcommand(name, |sub| localize(sub, lang));
    }
    command
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
            .map_err(|_| {
                Lang::detect()
                    .tr(i18n::CLI_RESUME_VALUE)
                    .replace("{text}", text)
            })
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
    /// 기록 정리. `--yes`가 없으면 지울 대상만 미리 보인다.
    Prune(PruneArgs),
    /// 판단 기록을 JSONL로 내보낸다. 채점하지 않은 기록도 내보낸다.
    Export(ExportArgs),
    /// router 관리.
    Router {
        /// router 하위 명령.
        #[command(subcommand)]
        command: RouterCommand,
    },
    /// 사용량 조회.
    Usage(UsageArgs),
}

/// `router` 하위 명령.
#[derive(Debug, Subcommand)]
pub(crate) enum RouterCommand {
    /// router 학습. 채점 안 된 판단이 200건 미만이면 engine이 거절한다.
    Train(TrainArgs),
    /// 고른 router 버전을 확인 한 줄 뒤 현재 버전으로 쓴다.
    Use(RouterUseArgs),
    /// router 버전 목록을 보인다.
    List,
}

/// `train` 인자.
#[derive(Debug, Args)]
pub(crate) struct TrainArgs {
    /// 기준값을 1차 영점으로 되돌린다.
    #[arg(long)]
    pub(crate) reset_thresholds: bool,
    /// 이 router 버전에서 다시 학습한다(`Request::Train`의 `from`).
    #[arg(long, value_name = "VERSION")]
    pub(crate) from: Option<String>,
    /// 확인 없이 학습을 시작한다.
    #[arg(long)]
    pub(crate) yes: bool,
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

/// `router use` 인자.
#[derive(Debug, Args)]
pub(crate) struct RouterUseArgs {
    // TODO(#49): 버전 표기 형식
    /// 쓸 router 버전.
    #[arg(value_name = "VERSION")]
    pub(crate) version: String,
    /// 확인 없이 바꾼다.
    #[arg(long)]
    pub(crate) yes: bool,
}

/// `usage` 인자. 둘 다 없으면 지금 폴더의 최근 채팅.
#[derive(Debug, Args)]
pub(crate) struct UsageArgs {
    /// 모든 채팅의 최근 24시간 사용량을 본다.
    #[arg(long, conflicts_with = "week")]
    pub(crate) day: bool,
    /// 모든 채팅의 최근 7일 사용량을 본다.
    #[arg(long)]
    pub(crate) week: bool,
}

impl UsageArgs {
    pub(crate) fn range(&self) -> UsageRange {
        match (self.day, self.week) {
            (true, _) => UsageRange::Day,
            (_, true) => UsageRange::Week,
            _ => UsageRange::Chat,
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
        let (key, value) = text.split_once('=').ok_or_else(|| {
            Lang::detect()
                .tr(i18n::CLI_CONFIG_FORMAT)
                .replace("{text}", text)
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(Lang::detect()
                .tr(i18n::CLI_CONFIG_EMPTY_KEY)
                .replace("{text}", text));
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

    /// 이름 붙은 명령과 인자의 도움말 문구.
    fn help_texts(command: &clap::Command, texts: &mut Vec<String>) {
        texts.extend(command.get_about().map(ToString::to_string));
        texts.extend(
            command
                .get_arguments()
                .filter_map(|arg| arg.get_help().map(ToString::to_string)),
        );
        for sub in command.get_subcommands() {
            help_texts(sub, texts);
        }
    }

    #[test]
    fn help_has_english_for_every_korean_text() {
        let mut texts = Vec::new();
        help_texts(&Cli::command(), &mut texts);

        let missing: Vec<&String> = texts
            .iter()
            .filter(|text| text.chars().any(|c| ('가'..='힣').contains(&c)))
            .filter(|text| Lang::En.tr(text) == text.as_str())
            .collect();

        assert!(texts.len() > 15, "help scan should find the texts");
        assert!(missing.is_empty(), "missing english: {missing:?}");
    }

    #[test]
    fn localized_help_replaces_korean_with_english() {
        let command = command(Lang::En);

        let usage = command.find_subcommand("usage").unwrap();
        let day = usage.get_arguments().find(|arg| arg.get_id() == "day");

        assert_eq!(
            usage.get_about().map(ToString::to_string).as_deref(),
            Some("Show usage")
        );
        assert!(day.is_some_and(|arg| {
            arg.get_help()
                .is_some_and(|h| h.to_string().contains("last 24 hours"))
        }));
    }

    #[test]
    fn usage_flags_choose_the_range() {
        let range = |args: &[&str]| match parse(args).unwrap().command {
            Some(Command::Usage(usage)) => usage.range(),
            other => panic!("not usage: {other:?}"),
        };

        assert_eq!(range(&["usage"]), UsageRange::Chat);
        assert_eq!(range(&["usage", "--day"]), UsageRange::Day);
        assert_eq!(range(&["usage", "--week"]), UsageRange::Week);
        assert!(parse(&["usage", "--day", "--week"]).is_err());
        assert!(parse(&["usage", "--range", "week"]).is_err());
    }

    #[test]
    fn router_subcommands_parse() {
        let train = parse(&["router", "train", "--from", "v1"]).unwrap();
        assert!(matches!(
            train.command,
            Some(Command::Router { command: RouterCommand::Train(ref args) }) if args.from.as_deref() == Some("v1")
        ));
        let used = parse(&["router", "use", "v2"]).unwrap();
        assert!(matches!(
            used.command,
            Some(Command::Router { command: RouterCommand::Use(ref args) }) if args.version == "v2"
        ));
        let list = parse(&["router", "list"]).unwrap();
        assert!(matches!(
            list.command,
            Some(Command::Router {
                command: RouterCommand::List
            })
        ));
        let yes = parse(&["router", "use", "v2", "--yes"]).unwrap();
        assert!(matches!(
            yes.command,
            Some(Command::Router { command: RouterCommand::Use(ref args) }) if args.yes
        ));
        assert!(parse(&["train"]).is_err());
    }

    #[test]
    fn config_override_splits_at_first_equals() {
        let parsed: ConfigOverride = "router.url=https://a.example/?x=1".parse().unwrap();

        assert_eq!(parsed.key, "router.url");
        assert_eq!(parsed.value, "https://a.example/?x=1");
    }

    #[test]
    fn config_override_without_equals_or_key_is_error() {
        assert!("permission.mode".parse::<ConfigOverride>().is_err());
        assert!("=full".parse::<ConfigOverride>().is_err());
    }

    #[test]
    fn config_override_allows_empty_value() {
        let parsed: ConfigOverride = "router.key.command=".parse().unwrap();

        assert_eq!(parsed.value, "");
    }

    #[test]
    fn continue_resume_without_arguments_opens_new_chat() {
        assert_eq!(
            parse(&[]).unwrap().open_mode(Lang::En).unwrap(),
            OpenMode::New
        );
    }

    #[test]
    fn continue_resume_continue_flag_picks_last_chat() {
        let mode = parse(&["--continue"]).unwrap().open_mode(Lang::En).unwrap();

        assert_eq!(mode, OpenMode::ContinueLast);
    }

    #[test]
    fn continue_resume_without_id_picks_in_folder() {
        let mode = parse(&["--resume"]).unwrap().open_mode(Lang::En).unwrap();

        assert_eq!(mode, OpenMode::PickInFolder);
    }

    #[test]
    fn continue_resume_all_picks_in_every_folder() {
        let mode = parse(&["--resume", "all"])
            .unwrap()
            .open_mode(Lang::En)
            .unwrap();

        assert_eq!(mode, OpenMode::PickInAll);
    }

    #[test]
    fn continue_resume_numeric_id_opens_that_chat() {
        let mode = parse(&["--resume", "42"])
            .unwrap()
            .open_mode(Lang::En)
            .unwrap();

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
        assert_eq!(cli.open_mode(Lang::En).unwrap(), OpenMode::New);
    }

    #[test]
    fn continue_resume_conflict_is_error() {
        assert!(parse(&["--continue", "--resume"]).is_err());
    }

    #[test]
    fn continue_resume_with_subcommand_is_error() {
        let cli = parse(&["--continue", "usage"]).unwrap();

        assert!(cli.open_mode(Lang::En).is_err());
    }

    #[test]
    fn add_dir_repeats_and_conflicts_with_subcommands() {
        let cli = parse(&["--add-dir", "/a", "--add-dir", "b"]).unwrap();
        let with_command = parse(&["--add-dir", "/a", "usage"]).unwrap();

        assert_eq!(cli.add_dir, vec![PathBuf::from("/a"), PathBuf::from("b")]);
        assert!(cli.open_mode(Lang::En).is_ok());
        assert!(with_command.open_mode(Lang::En).is_err());
    }

    #[test]
    fn config_overrides_repeat() {
        let cli = parse(&["-c", "a=1", "-c", "b=2", "usage"]).unwrap();

        assert_eq!(cli.config.len(), 2);
    }
}
