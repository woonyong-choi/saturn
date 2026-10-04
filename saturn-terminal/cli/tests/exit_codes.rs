//! `saturn` 실행 파일의 종료 코드. 가짜 engine을 `HOME` 아래 소켓에 세워 원인마다 코드를 확인한다.
//! 설계: docs/design/engine-lifecycle.md
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::JoinHandle;

use saturn_protocol::envelope::{
    ErrorKind, NotificationMessage, Response, ServerMessage, decode_client_line, encode_line,
};
use saturn_protocol::ids::{ChatId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::{
    ChatNotice, Notification, PROTOCOL_VERSION, QueryResult, Request, RouterVersionInfo,
};
use saturn_protocol::state::TaskState;

/// 요청 하나에 대한 가짜 engine의 행동.
enum Act {
    Ok(Vec<Notification>),
    /// 조회 요청의 응답 `result`.
    Query(QueryResult),
    Fail {
        code: i32,
        kind: Option<ErrorKind>,
    },
    /// 응답 없이 연결을 끊는다.
    Hangup,
}

fn fail(code: i32, kind: ErrorKind) -> Act {
    Act::Fail {
        code,
        kind: Some(kind),
    }
}

const INTERNAL: i32 = -32603;
const INVALID_PARAMS: i32 = -32602;
const ROUTER_KEY_REQUIRED: i32 = -32001;

fn serve(home: &Path, mut act: impl FnMut(&Request) -> Act + Send + 'static) -> JoinHandle<()> {
    let dir = home.join(".saturn");
    std::fs::create_dir_all(&dir).unwrap();
    let listener = UnixListener::bind(dir.join("engine.sock")).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        for line in BufReader::new(stream).lines() {
            let message = decode_client_line(&line.unwrap()).unwrap();
            let step = match message.request {
                Request::Version => Act::Ok(vec![Notification::EngineVersion {
                    saturn_version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocol_version: PROTOCOL_VERSION,
                }]),
                Request::Detach => Act::Ok(Vec::new()),
                ref request => act(request),
            };
            let (notes, response) = match step {
                Act::Ok(notes) => (notes, Response::ok(message.id)),
                Act::Query(result) => (Vec::new(), Response::result(message.id, result)),
                Act::Fail { code, kind } => (
                    Vec::new(),
                    Response::error_of_kind(Some(message.id), code, kind, "refused"),
                ),
                Act::Hangup => return,
            };
            let mut lines: Vec<String> = notes
                .into_iter()
                .map(|note| {
                    let note = ServerMessage::Notification(NotificationMessage::new(note));
                    encode_line(&note).unwrap()
                })
                .collect();
            lines.push(encode_line(&ServerMessage::from(response)).unwrap());
            for line in lines {
                // 클라이언트가 `Detach` 뒤 먼저 끝나면 쓸 곳이 없다
                if writer.write_all(line.as_bytes()).is_err() {
                    return;
                }
            }
        }
    })
}

struct Run {
    code: Option<i32>,
    stderr: String,
}

/// 터미널이 없는 입력으로 `saturn`을 실행한다. 실제 `~/.saturn`과 에이전트 표지는 쓰지 않는다.
fn saturn(home: &Path, args: &[&str], envs: &[(&str, &str)], stdin: &str) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_saturn"))
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env_remove("SATURN_AGENT")
        .env_remove("SATURN_KEY")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Run {
        code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run_with_engine(args: &[&str], act: impl FnMut(&Request) -> Act + Send + 'static) -> Run {
    let home = tempfile::tempdir().unwrap();
    let engine = serve(home.path(), act);
    let run = saturn(home.path(), args, &[], "");
    engine.join().unwrap();
    run
}

#[test]
fn help_exits_zero_and_unknown_flag_exits_two() {
    let home = tempfile::tempdir().unwrap();

    assert_eq!(saturn(home.path(), &["--help"], &[], "").code, Some(0));
    assert_eq!(saturn(home.path(), &["--bogus"], &[], "").code, Some(2));
}

#[test]
fn nested_run_inside_an_agent_exits_two() {
    let home = tempfile::tempdir().unwrap();

    let run = saturn(home.path(), &[], &[("SATURN_AGENT", "1")], "");

    assert_eq!(run.code, Some(2), "{}", run.stderr);
}

#[test]
fn picking_a_chat_without_a_terminal_exits_two() {
    let home = tempfile::tempdir().unwrap();

    let run = saturn(home.path(), &["--resume"], &[], "");

    assert_eq!(run.code, Some(2), "{}", run.stderr);
}

#[test]
fn missing_add_dir_exits_sixty_six() {
    let home = tempfile::tempdir().unwrap();

    let run = saturn(home.path(), &["--add-dir", "no-such-folder"], &[], "");

    assert_eq!(run.code, Some(66), "{}", run.stderr);
}

#[test]
fn add_dir_that_is_a_file_exits_two() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("file"), "x").unwrap();

    let run = saturn(home.path(), &["--add-dir", "file"], &[], "");

    assert_eq!(run.code, Some(2), "{}", run.stderr);
}

#[test]
fn continue_without_a_chat_in_the_folder_exits_sixty_six() {
    let run = run_with_engine(&["--continue"], |request| match request {
        Request::LatestChat { .. } => Act::Query(QueryResult::LatestChat { chat: None }),
        other => panic!("unexpected {other:?}"),
    });

    assert_eq!(run.code, Some(66), "{}", run.stderr);
}

#[test]
fn unknown_router_version_exits_sixty_six() {
    let run = run_with_engine(&["router", "use", "v9", "--yes"], |request| match request {
        Request::ListRouterVersions => Act::Query(QueryResult::RouterVersions {
            current: "v1".to_owned(),
            versions: vec![RouterVersionInfo {
                version: "v1".to_owned(),
                router: "jev".to_owned(),
                ece: None,
                questions: Vec::new(),
            }],
        }),
        other => panic!("unexpected {other:?}"),
    });

    assert_eq!(run.code, Some(66), "{}", run.stderr);
}

#[test]
fn engine_that_hangs_up_exits_sixty_nine() {
    let run = run_with_engine(&["usage"], |_| Act::Hangup);

    assert_eq!(run.code, Some(69), "{}", run.stderr);
}

#[test]
fn engine_internal_error_exits_seventy() {
    let run = run_with_engine(&["usage"], |_| Act::Fail {
        code: INTERNAL,
        kind: None,
    });

    assert_eq!(run.code, Some(70), "{}", run.stderr);
}

#[test]
fn training_without_enough_samples_exits_seventy_five() {
    let run = run_with_engine(&["router", "train", "--yes"], |_| {
        fail(INTERNAL, ErrorKind::RetryLater)
    });

    assert_eq!(run.code, Some(75), "{}", run.stderr);
}

#[test]
fn missing_router_key_exits_seventy_seven() {
    let run = run_with_engine(&["usage"], |_| {
        fail(ROUTER_KEY_REQUIRED, ErrorKind::RouterKey)
    });

    assert_eq!(run.code, Some(77), "{}", run.stderr);
    assert!(run.stderr.contains("SATURN_KEY"), "{}", run.stderr);
}

#[test]
fn router_key_check_failure_exits_seventy_seven() {
    let run = run_with_engine(&["usage"], |_| fail(INTERNAL, ErrorKind::RouterKey));

    assert_eq!(run.code, Some(77), "{}", run.stderr);
}

#[test]
fn setting_error_exits_seventy_eight() {
    let run = run_with_engine(&["prune"], |_| fail(INVALID_PARAMS, ErrorKind::Config));

    assert_eq!(run.code, Some(78), "{}", run.stderr);
}

#[test]
fn request_the_caller_must_change_exits_two() {
    let run = run_with_engine(&["export", "out.jsonl"], |_| Act::Fail {
        code: INVALID_PARAMS,
        kind: None,
    });

    assert_eq!(run.code, Some(2), "{}", run.stderr);
}

#[test]
fn expected_failure_exits_one() {
    let run = run_with_engine(&["export", "out.jsonl"], |_| {
        fail(INTERNAL, ErrorKind::Failed)
    });

    assert_eq!(run.code, Some(1), "{}", run.stderr);
}

#[test]
fn training_without_a_terminal_to_confirm_exits_two_and_cancels() {
    let run = run_with_engine(&["router", "train"], |request| match request {
        Request::Train { .. } => Act::Ok(vec![Notification::TrainPreview {
            candidates: 250,
            grader: "grader-a".to_owned(),
            estimated_tokens: 1,
            threshold_targets: Vec::new(),
            retrain_model: false,
        }]),
        Request::ConfirmTrain { proceed: false } => Act::Ok(Vec::new()),
        other => panic!("unexpected {other:?}"),
    });

    assert_eq!(run.code, Some(2), "{}", run.stderr);
}

#[test]
fn plain_run_with_a_failed_task_exits_one() {
    let home = tempfile::tempdir().unwrap();
    let engine = serve(home.path(), |request| match request {
        Request::Attach { .. } => Act::Ok(vec![Notification::HistoryChunk {
            chat: ChatId(1),
            entries: Vec::new(),
            oldest: None,
            has_more: false,
        }]),
        Request::SubmitInput { .. } => Act::Ok(vec![
            Notification::TaskChanged {
                task: TaskId(1),
                label: TaskLabel('A'),
                state: TaskState::Failed,
                provider: Some(Provider::from_static("codex")),
                elapsed_ms: 5,
                failure: Some("provider failed".to_owned()),
            },
            Notification::ChatNotice {
                chat: ChatId(1),
                task: None,
                notice: ChatNotice::RequestSummary {
                    provider_tokens: Vec::new(),
                    router_calls: 0,
                    router_tokens: 0,
                    elapsed_ms: 5,
                },
            },
        ]),
        other => panic!("unexpected {other:?}"),
    });

    let run = saturn(home.path(), &[], &[], "go\n");

    engine.join().unwrap();
    assert_eq!(run.code, Some(1), "{}", run.stderr);
}

#[test]
fn plain_run_that_finishes_cleanly_exits_zero() {
    let home = tempfile::tempdir().unwrap();
    let engine = serve(home.path(), |request| match request {
        Request::Attach { .. } => Act::Ok(vec![Notification::HistoryChunk {
            chat: ChatId(1),
            entries: Vec::new(),
            oldest: None,
            has_more: false,
        }]),
        Request::SubmitInput { .. } => Act::Ok(vec![Notification::ChatNotice {
            chat: ChatId(1),
            task: None,
            notice: ChatNotice::RequestSummary {
                provider_tokens: Vec::new(),
                router_calls: 0,
                router_tokens: 0,
                elapsed_ms: 5,
            },
        }]),
        other => panic!("unexpected {other:?}"),
    });

    let run = saturn(home.path(), &[], &[], "go\n");

    engine.join().unwrap();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}
