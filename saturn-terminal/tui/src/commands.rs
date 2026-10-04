//! `/` 명령 해석과 팝업 명령 목록. 해석은 순수 함수다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::{Provider, TaskLabel};

use crate::i18n::provider_name;
use crate::labels::LABEL_RANGE;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandError {
    #[error("unknown command: {name}")]
    Unknown { name: String },
    #[error("invalid argument for /{command}: {argument}")]
    InvalidArgument {
        command: &'static str,
        argument: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandSpec {
    pub path: &'static str,
    /// `Lang::tr`의 한국어 키.
    pub description: &'static str,
    pub values: &'static [&'static str],
}

/// TODO(#41): 메인이 아닌 provider의 명령을 골랐을 때 처리
pub(crate) const SATURN_COMMANDS: &[CommandSpec] = &[
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
        values: &[],
    },
    CommandSpec {
        path: "prune",
        description: "기록 정리",
        values: &[],
    },
    CommandSpec {
        path: "train",
        description: "판단 모델 학습",
        values: &[],
    },
    CommandSpec {
        path: "router use",
        description: "판단 모델 버전",
        values: &[],
    },
    CommandSpec {
        path: "record",
        description: "판단 기록 켜기와 끄기",
        values: &["on", "off"],
    },
    CommandSpec {
        path: "permissions",
        description: "권한 모드 바꾸기",
        values: &PERMISSION_MODES,
    },
    CommandSpec {
        path: "add-dir",
        description: "폴더 더하기",
        values: &[],
    },
    CommandSpec {
        path: "model",
        description: "다음 입력부터 쓸 모델 고르기",
        values: &MODEL_PROVIDERS,
    },
];

/// `/model <provider>`에서 고를 수 있는 이름. `i18n::provider_name`과 같다.
const MODEL_PROVIDERS: [&str; 2] = ["codex", "claude"];

/// engine이 받는 권한 모드 이름. 초안.
const PERMISSION_MODES: [&str; 4] = ["ask", "edit", "read-only", "full"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlashCommand {
    /// `/help`
    Help,
    /// `/record on|off`
    Record { on: bool },
    /// `/permissions ask|edit|read-only|full`
    /// TODO(#177): 값 없이 실행하면 현재 모드를 보이는 동작은 조회 결과를 돌려주는 방식이 정해진 뒤에 넣는다
    Permissions { mode: &'static str },
    /// `/model [provider]`. provider를 주면 그 provider 모델만 목록에 보인다. 고르는 것은 목록 창에서 한다.
    Model { provider: Option<Provider> },
    /// `/add-dir <폴더>`. 경로는 공백을 포함할 수 있어 명령 이름 뒤 나머지 전체다.
    AddDir { path: String },
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
    /// `/usage`. 범위는 화면에서 `d`, `w`로 바꾼다.
    Usage,
    /// `/prune`. 지울 채팅을 미리 보이는 창을 연다.
    Prune,
    /// 채점 후보가 200건 미만이면 engine이 거절한다.
    Train {
        reset_thresholds: bool,
        from: Option<String>,
    },
    /// `/router use`
    RouterVersion,
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
pub(crate) fn parse(line: &str) -> Result<Option<SlashCommand>, CommandError> {
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
        "permissions" => parse_permissions(&args)?,
        "add-dir" => parse_add_dir(body)?,
        "model" => parse_model(&args)?,
        "tasks" => no_args("tasks", &args, SlashCommand::Tasks)?,
        "usage" => no_args("usage", &args, SlashCommand::Usage)?,
        "prune" => no_args("prune", &args, SlashCommand::Prune)?,
        "train" => parse_train(&args)?,
        "router" => parse_router(&args)?,
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

// cost: time O(b), heap O(b), stack O(1)
// vars: b = 명령 글자 수
// basis: estimate
/// `body`는 `/`를 뗀 줄. 이름 뒤 나머지를 앞뒤 공백만 떼어 경로로 쓴다.
fn parse_add_dir(body: &str) -> Result<SlashCommand, CommandError> {
    let path = body.strip_prefix("add-dir").unwrap_or_default().trim();
    if path.is_empty() {
        return Err(invalid("add-dir", ""));
    }
    Ok(SlashCommand::AddDir {
        path: path.to_owned(),
    })
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_model(args: &[&str]) -> Result<SlashCommand, CommandError> {
    let provider = match args {
        [] => None,
        [name] => Some(
            [Provider::Codex, Provider::Claude]
                .into_iter()
                .find(|provider| provider_name(*provider) == *name)
                .ok_or_else(|| invalid("model", name))?,
        ),
        _ => return Err(invalid("model", &args.join(" "))),
    };
    Ok(SlashCommand::Model { provider })
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_permissions(args: &[&str]) -> Result<SlashCommand, CommandError> {
    let [argument] = args else {
        return Err(invalid("permissions", &args.join(" ")));
    };
    PERMISSION_MODES
        .iter()
        .find(|mode| *mode == argument)
        .map(|mode| SlashCommand::Permissions { mode })
        .ok_or_else(|| invalid("permissions", argument))
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
fn parse_router(args: &[&str]) -> Result<SlashCommand, CommandError> {
    match args {
        ["use"] => Ok(SlashCommand::RouterVersion),
        _ => Err(invalid("router", &args.join(" "))),
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
    fn parse_model_reads_an_optional_provider_and_keeps_the_provider_command_out() {
        assert_eq!(
            parse("/model").unwrap(),
            Some(SlashCommand::Model { provider: None })
        );
        assert_eq!(
            parse("/model codex").unwrap(),
            Some(SlashCommand::Model {
                provider: Some(Provider::Codex)
            })
        );
        assert!(matches!(
            parse("/model gemini"),
            Err(CommandError::InvalidArgument {
                command: "model",
                ..
            })
        ));
        assert!(matches!(
            parse("/model codex claude"),
            Err(CommandError::InvalidArgument {
                command: "model",
                ..
            })
        ));
    }

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
    fn parse_usage_takes_no_arguments() {
        assert_eq!(parse("/usage").unwrap(), Some(SlashCommand::Usage));
        assert_eq!(parse("/prune").unwrap(), Some(SlashCommand::Prune));
        assert!(parse("/prune all").is_err());
        assert!(matches!(
            parse("/usage week"),
            Err(CommandError::InvalidArgument {
                command: "usage",
                ..
            })
        ));
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
    fn parse_add_dir_keeps_the_rest_of_the_line_as_the_path() {
        assert_eq!(
            parse("/add-dir ~/my docs/ ").unwrap(),
            Some(SlashCommand::AddDir {
                path: "~/my docs/".to_owned()
            })
        );
        assert!(parse("/add-dir").is_err());
        assert!(parse("/add-dir   ").is_err());
        assert!(SATURN_COMMANDS.iter().any(|spec| spec.path == "add-dir"));
    }

    #[test]
    fn parse_permissions_reads_one_known_mode() {
        for mode in ["ask", "edit", "read-only", "full"] {
            assert_eq!(
                parse(&format!("/permissions {mode}")).unwrap(),
                Some(SlashCommand::Permissions { mode })
            );
        }
        assert!(parse("/permissions").is_err());
        assert!(parse("/permissions plan").is_err());
        assert!(parse("/permissions edit full").is_err());
        assert!(
            SATURN_COMMANDS
                .iter()
                .any(|spec| spec.path == "permissions" && spec.values.contains(&"read-only"))
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
    fn parse_router_use_and_feedback() {
        assert_eq!(
            parse("/router use").unwrap(),
            Some(SlashCommand::RouterVersion)
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
    fn saturn_commands_include_record() {
        assert!(SATURN_COMMANDS.iter().any(|spec| spec.path == "record"));
    }
}
