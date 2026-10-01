//! `/` 명령 해석과 팝업 명령 목록. 해석은 순수 함수다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::TaskLabel;
use saturn_protocol::rpc::UsageRange;

use crate::labels::LABEL_RANGE;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {name}")]
    Unknown { name: String },
    #[error("invalid argument for /{command}: {argument}")]
    InvalidArgument {
        command: &'static str,
        argument: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub path: &'static str,
    /// `Lang::tr`의 한국어 키.
    pub description: &'static str,
    pub values: &'static [&'static str],
}

/// TODO(#41): 메인이 아닌 provider의 명령을 골랐을 때 처리
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
    CommandSpec {
        path: "record",
        description: "판단 기록 켜기와 끄기",
        values: &["on", "off"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// `/help`
    Help,
    /// `/record on|off`
    Record { on: bool },
    /// 이름표가 없으면 가장 최근 대기 입력.
    Send { target: Option<TaskLabel> },
    /// 이름표가 없으면 가장 최근 대기 입력.
    Cancel { target: Option<TaskLabel> },
    /// 이름표가 없으면 채팅의 보류 전부를 접수 순서로.
    Continue { target: Option<TaskLabel> },
    /// `/feedback 1|2`, `1`이면 `true`.
    Feedback { correct: bool },
    /// `/tasks`
    Tasks,
    /// `/usage [chat|today|week|all]`, 기본 `chat`.
    Usage { range: UsageRange },
    /// 채점 후보가 200건 미만이면 engine이 거절한다.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// `/judge version`
    JudgeVersion,
    /// 목록에 없는 provider 명령. 원문 그대로 메인 에이전트 provider에 넘긴다. TODO(#41): 비메인 provider 명령 처리
    Provider { line: String },
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = line.len()
// basis: estimate
/// `/`로 시작하지 않으면 `Ok(None)`, Saturn 명령이 아닌 이름은 `SlashCommand::Provider`.
///
/// # Errors
/// 알 수 없는 Saturn 명령이면 `Unknown`, 인자가 틀리면 `InvalidArgument`.
pub fn parse(line: &str) -> Result<Option<SlashCommand>, CommandError> {
    let line = line.trim();
    let Some(body) = line.strip_prefix('/') else {
        return Ok(None);
    };
    let mut words = body.split_whitespace();
    let name = words.next().unwrap_or_default();
    let args: Vec<&str> = words.collect();
    let command = match name {
        "help" => no_args("help", &args, SlashCommand::Help)?,
        "record" => SlashCommand::Record {
            on: parse_switch(&args)?,
        },
        "send" => SlashCommand::Send {
            target: parse_target("send", &args)?,
        },
        "cancel" => SlashCommand::Cancel {
            target: parse_target("cancel", &args)?,
        },
        "continue" => SlashCommand::Continue {
            target: parse_target("continue", &args)?,
        },
        "feedback" => SlashCommand::Feedback {
            correct: parse_feedback(&args)?,
        },
        "tasks" => no_args("tasks", &args, SlashCommand::Tasks)?,
        "usage" => SlashCommand::Usage {
            range: parse_range(&args)?,
        },
        "train" => parse_train(&args)?,
        "judge" => parse_judge(&args)?,
        "" => {
            return Err(CommandError::Unknown {
                name: String::new(),
            });
        }
        _ => SlashCommand::Provider {
            line: line.to_string(),
        },
    };
    Ok(Some(command))
}

// cost: time O(k·t), heap O(k), stack O(1)
// vars: k = SATURN_COMMANDS.len(), t = token.len()
// basis: estimate
/// 앞부분 일치 우선, 그다음 포함 순서.
pub fn filter(token: &str) -> Vec<&'static CommandSpec> {
    let token = token.trim_start_matches('/');
    let prefixed = SATURN_COMMANDS
        .iter()
        .filter(|spec| spec.path.starts_with(token));
    let contained = SATURN_COMMANDS
        .iter()
        .filter(|spec| !spec.path.starts_with(token) && spec.path.contains(token));
    prefixed.chain(contained).collect()
}

fn no_args(
    command: &'static str,
    args: &[&str],
    parsed: SlashCommand,
) -> Result<SlashCommand, CommandError> {
    match args.first() {
        None => Ok(parsed),
        Some(argument) => Err(invalid(command, argument)),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_switch(args: &[&str]) -> Result<bool, CommandError> {
    match args {
        ["on"] => Ok(true),
        ["off"] => Ok(false),
        _ => Err(invalid("record", &args.join(" "))),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
/// 없어도 되고, 소문자는 대문자로 읽는다.
fn parse_target(command: &'static str, args: &[&str]) -> Result<Option<TaskLabel>, CommandError> {
    let argument = match args {
        [] => return Ok(None),
        [argument] => *argument,
        _ => return Err(invalid(command, &args.join(" "))),
    };
    let mut chars = argument.chars();
    match (chars.next().map(|c| c.to_ascii_uppercase()), chars.next()) {
        (Some(c), None) if LABEL_RANGE.contains(&c) => Ok(Some(TaskLabel(c))),
        _ => Err(invalid(command, argument)),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_feedback(args: &[&str]) -> Result<bool, CommandError> {
    match args {
        ["1"] => Ok(true),
        ["2"] => Ok(false),
        _ => Err(invalid("feedback", &args.join(" "))),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
/// 기본 `chat`.
fn parse_range(args: &[&str]) -> Result<UsageRange, CommandError> {
    match args {
        [] | ["chat"] => Ok(UsageRange::Chat),
        ["today"] => Ok(UsageRange::Today),
        ["week"] => Ok(UsageRange::Week),
        ["all"] => Ok(UsageRange::All),
        _ => Err(invalid("usage", &args.join(" "))),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수
// basis: estimate
fn parse_train(args: &[&str]) -> Result<SlashCommand, CommandError> {
    let mut reset_thresholds = false;
    let mut from = None;
    let mut rest = args.iter();
    while let Some(argument) = rest.next() {
        match *argument {
            "--reset-thresholds" => reset_thresholds = true,
            "--from" => match rest.next() {
                Some(version) => from = Some((*version).to_string()),
                None => return Err(invalid("train", argument)),
            },
            _ => return Err(invalid("train", argument)),
        }
    }
    Ok(SlashCommand::Train {
        reset_thresholds,
        from,
    })
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_judge(args: &[&str]) -> Result<SlashCommand, CommandError> {
    match args {
        ["version"] => Ok(SlashCommand::JudgeVersion),
        _ => Err(invalid("judge", &args.join(" "))),
    }
}

fn invalid(command: &'static str, argument: &str) -> CommandError {
    CommandError::InvalidArgument {
        command,
        argument: argument.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_plain_text_returns_none() {
        assert_eq!(parse("고쳐 줘").unwrap(), None);
    }

    #[test]
    fn parse_send_with_label_returns_target() {
        let parsed = parse(" /send c ").unwrap();

        assert_eq!(
            parsed,
            Some(SlashCommand::Send {
                target: Some(TaskLabel('C'))
            })
        );
    }

    #[test]
    fn parse_continue_without_label_returns_all() {
        assert_eq!(
            parse("/continue").unwrap(),
            Some(SlashCommand::Continue { target: None })
        );
    }

    #[test]
    fn parse_label_outside_range_returns_invalid_argument() {
        let result = parse("/cancel 9");

        assert!(matches!(
            result,
            Err(CommandError::InvalidArgument {
                command: "cancel",
                ..
            })
        ));
    }

    #[test]
    fn parse_usage_default_is_chat() {
        assert_eq!(
            parse("/usage").unwrap(),
            Some(SlashCommand::Usage {
                range: UsageRange::Chat
            })
        );
        assert_eq!(
            parse("/usage week").unwrap(),
            Some(SlashCommand::Usage {
                range: UsageRange::Week
            })
        );
    }

    #[test]
    fn parse_train_flags_are_read() {
        let parsed = parse("/train --reset-thresholds --from v2").unwrap();

        assert_eq!(
            parsed,
            Some(SlashCommand::Train {
                reset_thresholds: true,
                from: Some("v2".to_string())
            })
        );
    }

    #[test]
    fn parse_record_switch_is_read() {
        assert_eq!(
            parse("/record off").unwrap(),
            Some(SlashCommand::Record { on: false })
        );
        assert!(parse("/record maybe").is_err());
    }

    #[test]
    fn parse_judge_version_and_feedback() {
        assert_eq!(
            parse("/judge version").unwrap(),
            Some(SlashCommand::JudgeVersion)
        );
        assert_eq!(
            parse("/feedback 2").unwrap(),
            Some(SlashCommand::Feedback { correct: false })
        );
    }

    #[test]
    fn parse_unknown_command_goes_to_provider() {
        assert_eq!(
            parse("/review src").unwrap(),
            Some(SlashCommand::Provider {
                line: "/review src".to_string()
            })
        );
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn filter_prefers_prefix_then_contains() {
        let paths: Vec<&str> = filter("/ver").iter().map(|spec| spec.path).collect();

        assert_eq!(paths, vec!["judge version"]);
        assert_eq!(filter("/c")[0].path, "cancel");
        assert_eq!(filter("/c")[1].path, "continue");
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn saturn_commands_include_record() {
        assert!(SATURN_COMMANDS.iter().any(|spec| spec.path == "record"));
    }
}
