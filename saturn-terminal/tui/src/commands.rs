//! `/` 명령 해석과 팝업 명령 목록. 해석은 순수 함수다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::{Provider, TaskLabel};

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
    /// 값이 engine이 알린 provider id 중 하나다. `values` 대신 붙을 때 받은 목록을 쓴다.
    pub takes_provider: bool,
}

/// TODO(#41): 메인이 아닌 provider의 명령을 골랐을 때 처리
pub(crate) const SATURN_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        path: "help",
        description: "도움말",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "send",
        description: "대기 입력 지금 보내기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "cancel",
        description: "보내기 전 입력 취소",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "continue",
        description: "보류 이어서",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "feedback",
        description: "판단 피드백",
        values: &["1", "2"],
        takes_provider: false,
    },
    CommandSpec {
        path: "tasks",
        description: "작업 목록",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "usage",
        description: "사용량",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "prune",
        description: "기록 정리",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "train",
        description: "판단 모델 학습",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "router use",
        description: "판단 모델 버전",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "record",
        description: "판단 기록 켜기와 끄기",
        values: &["on", "off"],
        takes_provider: false,
    },
    CommandSpec {
        path: "permissions",
        description: "권한 모드 바꾸기",
        values: &PERMISSION_MODES,
        takes_provider: false,
    },
    CommandSpec {
        path: "add-dir",
        description: "폴더 더하기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "extensions",
        description: "확장 목록, 설치, 제거",
        values: &["install", "remove"],
        takes_provider: false,
    },
    CommandSpec {
        path: "model",
        description: "다음 입력부터 쓸 모델 고르기",
        values: &[],
        takes_provider: true,
    },
    CommandSpec {
        path: "mode",
        description: "권한 모드 돌리기와 정하기",
        values: &PERMISSION_MODES,
        takes_provider: false,
    },
    CommandSpec {
        path: "agents",
        description: "상태판 버튼 고르기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "stop",
        description: "작업 모두 멈추기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "rewind",
        description: "되돌리기(구현 전)",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "keymap",
        description: "키 묶음 고르기",
        values: &saturn_protocol::keymap::PRESET_NAMES,
        takes_provider: false,
    },
    CommandSpec {
        path: "transcript",
        description: "전체 기록",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "redraw",
        description: "화면 다시 그리기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "plain",
        description: "단순 방식 켜고 끄기",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "suspend",
        description: "화면 일시 중지",
        values: &[],
        takes_provider: false,
    },
    CommandSpec {
        path: "quit",
        description: "종료",
        values: &[],
        takes_provider: false,
    },
];

/// engine이 받는 권한 모드 이름. 초안.
pub(crate) const PERMISSION_MODES: [&str; 4] = ["ask", "edit", "read-only", "full"];

/// `Shift+Tab`과 `/mode`가 돌아가는 순서. `full`은 `/mode full`이나 `/permissions full`로만 켠다.
pub(crate) const PERMISSION_CYCLE: [&str; 3] = ["ask", "edit", "read-only"];

/// 현재 모드를 모를 때 시작하는 기본 모드(`permission.mode`의 기본값).
pub(crate) const DEFAULT_PERMISSION_MODE: &str = "edit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExtensionsAction {
    List,
    /// 로컬 폴더 경로나 git 저장소 주소. 공백을 포함할 수 있어 `install` 뒤 나머지 전체다.
    Install {
        source: String,
    },
    Remove {
        name: String,
    },
    /// provider에 직접 설치된 항목을 Saturn 확장 저장소로 옮긴다.
    Move {
        provider: Provider,
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SlashCommand {
    /// `/help`
    Help,
    /// `/record on|off`
    Record { on: bool },
    /// `/permissions ask|edit|read-only|full`
    /// 값 없이 실행하면 현재 모드를 보이는 동작은 아직 없다. 조회 요청을 더해 응답 `result`로 받는다
    Permissions { mode: &'static str },
    /// `/model [provider]`. provider를 주면 그 provider 모델만 목록에 보인다. 고르는 것은 목록 창에서 한다.
    Model { provider: Option<Provider> },
    /// `/add-dir <폴더>`. 경로는 공백을 포함할 수 있어 명령 이름 뒤 나머지 전체다.
    AddDir { path: String },
    /// `/extensions`, `/extensions install <원천>`, `/extensions remove <이름>`, `/extensions move <provider> <이름>`
    Extensions(ExtensionsAction),
    /// 이름표가 없으면 가장 최근 대기 입력.
    Send { target: Option<TaskLabel> },
    /// 이름표가 없으면 가장 최근 대기 입력.
    Cancel { target: Option<TaskLabel> },
    /// 이름표가 없으면 채팅의 보류 전부를 접수 순서로.
    Continue { target: Option<TaskLabel> },
    /// `/feedback 1|2`, `1`이면 `true`.
    Feedback { correct: bool },
    /// 인자 없는 `/feedback`, `Esc`로 닫은 바로잡기 제안을 다시 연다.
    ReopenCorrection,
    /// `/tasks`
    Tasks,
    /// `/mode [값]`. 값이 없으면 권한 모드를 돌리고, 있으면 `/permissions`처럼 그 모드로 정한다.
    Mode { target: Option<&'static str> },
    /// `/agents`. 상태판 버튼 고르기를 시작한다.
    Agents,
    /// `/stop`. 실행 중인 작업을 모두 멈춘다.
    Stop,
    /// `/rewind`. 구현 전.
    Rewind,
    /// `/keymap [이름]`. 이름이 없으면 지금 키 묶음과 목록을 보인다.
    Keymap { name: Option<&'static str> },
    /// `/transcript`
    Transcript,
    /// `/redraw`
    Redraw,
    /// `/plain`. 단순 방식을 켜고 끈다.
    Plain,
    /// `/suspend`
    Suspend,
    /// `/quit`
    Quit,
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
        "feedback" if args.is_empty() => SlashCommand::ReopenCorrection,
        "feedback" => SlashCommand::Feedback {
            correct: parse_feedback(&args)?,
        },
        "permissions" => parse_permissions(&args)?,
        "add-dir" => parse_add_dir(body)?,
        "extensions" => parse_extensions(body)?,
        "model" => parse_model(&args)?,
        "tasks" => no_args("tasks", &args, SlashCommand::Tasks)?,
        "mode" => parse_mode(&args)?,
        "agents" => no_args("agents", &args, SlashCommand::Agents)?,
        "stop" => no_args("stop", &args, SlashCommand::Stop)?,
        "rewind" => no_args("rewind", &args, SlashCommand::Rewind)?,
        "keymap" => parse_keymap(&args)?,
        "transcript" => no_args("transcript", &args, SlashCommand::Transcript)?,
        "redraw" => no_args("redraw", &args, SlashCommand::Redraw)?,
        "plain" => no_args("plain", &args, SlashCommand::Plain)?,
        "suspend" => no_args("suspend", &args, SlashCommand::Suspend)?,
        "quit" => no_args("quit", &args, SlashCommand::Quit)?,
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

// cost: time O(b), heap O(b), stack O(1)
// vars: b = 명령 글자 수
// basis: estimate
/// `body`는 `/`를 뗀 줄. 원천은 이름 뒤 나머지를 앞뒤 공백만 떼어 쓴다.
fn parse_extensions(body: &str) -> Result<SlashCommand, CommandError> {
    let rest = body.strip_prefix("extensions").unwrap_or_default().trim();
    if rest.is_empty() {
        return Ok(SlashCommand::Extensions(ExtensionsAction::List));
    }
    let (verb, argument) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let argument = argument.trim();
    match verb {
        "install" if !argument.is_empty() => {
            Ok(SlashCommand::Extensions(ExtensionsAction::Install {
                source: argument.to_owned(),
            }))
        }
        "remove" if !argument.is_empty() => {
            Ok(SlashCommand::Extensions(ExtensionsAction::Remove {
                name: argument.to_owned(),
            }))
        }
        "move" => match argument.split_once(char::is_whitespace) {
            Some((provider, name)) if !name.trim().is_empty() => {
                let provider =
                    Provider::parse(provider).map_err(|_| invalid("extensions", rest))?;
                Ok(SlashCommand::Extensions(ExtensionsAction::Move {
                    provider,
                    name: name.trim().to_owned(),
                }))
            }
            _ => Err(invalid("extensions", rest)),
        },
        _ => Err(invalid("extensions", rest)),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_model(args: &[&str]) -> Result<SlashCommand, CommandError> {
    let provider = match args {
        [] => None,
        [name] => Some(Provider::parse(name).map_err(|_| invalid("model", name))?),
        _ => return Err(invalid("model", &args.join(" "))),
    };
    Ok(SlashCommand::Model { provider })
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_mode(args: &[&str]) -> Result<SlashCommand, CommandError> {
    match args {
        [] => Ok(SlashCommand::Mode { target: None }),
        [argument] => PERMISSION_MODES
            .iter()
            .find(|mode| *mode == argument)
            .map(|mode| SlashCommand::Mode { target: Some(mode) })
            .ok_or_else(|| invalid("mode", argument)),
        _ => Err(invalid("mode", &args.join(" "))),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 인자 글자 수(오류 문구를 만들 때만)
// basis: estimate
fn parse_keymap(args: &[&str]) -> Result<SlashCommand, CommandError> {
    match args {
        [] => Ok(SlashCommand::Keymap { name: None }),
        [argument] => saturn_protocol::keymap::PRESET_NAMES
            .iter()
            .find(|name| *name == argument)
            .map(|name| SlashCommand::Keymap { name: Some(name) })
            .ok_or_else(|| invalid("keymap", argument)),
        _ => Err(invalid("keymap", &args.join(" "))),
    }
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
                provider: Some(Provider::from_static("codex"))
            })
        );
        assert_eq!(
            parse("/model gemini").unwrap(),
            Some(SlashCommand::Model {
                provider: Some(Provider::from_static("gemini"))
            })
        );
        assert!(matches!(
            parse("/model Gem/ini"),
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
    fn parse_extensions_reads_list_install_and_remove() {
        assert_eq!(
            parse("/extensions").unwrap(),
            Some(SlashCommand::Extensions(ExtensionsAction::List))
        );
        assert_eq!(
            parse("/extensions install  ~/my kits/review-kit ").unwrap(),
            Some(SlashCommand::Extensions(ExtensionsAction::Install {
                source: "~/my kits/review-kit".to_owned()
            }))
        );
        assert_eq!(
            parse("/extensions remove review-kit").unwrap(),
            Some(SlashCommand::Extensions(ExtensionsAction::Remove {
                name: "review-kit".to_owned()
            }))
        );
        assert_eq!(
            parse("/extensions move claude commit-helper").unwrap(),
            Some(SlashCommand::Extensions(ExtensionsAction::Move {
                provider: Provider::from_static("claude"),
                name: "commit-helper".to_owned()
            }))
        );
        for bad in [
            "/extensions install",
            "/extensions remove",
            "/extensions move claude",
            "/extensions add x",
        ] {
            assert!(matches!(
                parse(bad),
                Err(CommandError::InvalidArgument {
                    command: "extensions",
                    ..
                })
            ));
        }
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
}
