//! `saturn` 실행 파일의 종료 코드. 가짜 engine을 `HOME` 아래 소켓에 세워 원인마다 코드를 확인한다.
//! 설계: docs/design/engine-lifecycle.md
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use saturn_protocol::envelope::{
    ErrorKind, NotificationMessage, Response, ServerMessage, decode_client_line, encode_line,
};
use saturn_protocol::ids::{ChatId, InputId, Provider, TaskId, TaskLabel};
use saturn_protocol::rpc::{
    Notification, PROTOCOL_VERSION, QueryResult, Request, RouterVersionInfo,
};
use saturn_protocol::state::{Disposition, InputState, TaskState};

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
    /// 확인 번호(`plan`)가 없는 옛 모양의 `PrunePreview` 응답.
    PlanlessPreview,
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

fn serve(home: &Path, act: impl FnMut(&Request) -> Act + Send + 'static) -> JoinHandle<()> {
    let dir = home.join(".saturn");
    std::fs::create_dir_all(&dir).unwrap();
    serve_at(&dir.join("engine.sock"), act)
}

fn serve_at(
    socket: &Path,
    mut act: impl FnMut(&Request) -> Act + Send + 'static,
) -> JoinHandle<()> {
    let listener = UnixListener::bind(socket).unwrap();
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
                Act::PlanlessPreview => {
                    let preview = QueryResult::PrunePreview {
                        chats: Vec::new(),
                        skipped: Vec::new(),
                        rows: 0,
                        plan: "legacy".to_owned(),
                    };
                    let message = ServerMessage::from(Response::result(message.id, preview));
                    let mut value = serde_json::to_value(&message).unwrap();
                    without_key(&mut value, "plan");
                    let line = format!("{value}\n");
                    if writer.write_all(line.as_bytes()).is_err() {
                        return;
                    }
                    continue;
                }
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

/// 옛 engine의 응답 모양을 흉내 내려고 키 하나를 어디에 있든 지운다.
fn without_key(value: &mut serde_json::Value, key: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(key);
            map.values_mut().for_each(|inner| without_key(inner, key));
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().for_each(|inner| without_key(inner, key))
        }
        _ => {}
    }
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
        .env_remove("SATURN_HOME")
        .env_remove("SATURN_PASS")
        .env_remove("SATURN_ENGINE_SOCKET")
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

/// 에이전트 작업 안에서 도는 `saturn`의 환경. 출입증과 소켓은 engine이 넣어 주는 값이다.
fn child_env(socket: &str) -> [(&str, &str); 3] {
    [
        ("SATURN_AGENT", "1"),
        ("SATURN_PASS", "saturn-pass-test"),
        ("SATURN_ENGINE_SOCKET", socket),
    ]
}

#[test]
fn a_child_attaches_with_its_pass_and_mode_on_the_socket_it_was_given() {
    let home = tempfile::tempdir().unwrap();
    let socket = home.path().join("child.sock");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let engine = serve_at(&socket, move |request| {
        seen.lock().unwrap().push(request.clone());
        Act::Ok(vec![Notification::HistoryChunk {
            chat: ChatId(2),
            entries: Vec::new(),
            oldest: None,
            has_more: false,
        }])
    });
    let socket_text = socket.display().to_string();

    let run = saturn(
        home.path(),
        &["--mode", "read-only"],
        &child_env(&socket_text),
        "",
    );
    engine.join().unwrap();

    assert_eq!(run.code, Some(0), "{}", run.stderr);
    // 접속 요청 하나뿐이다. 새 채팅을 여는 `Attach`나 입력은 없다
    assert_eq!(
        *requests.lock().unwrap(),
        vec![Request::AttachChild {
            pass: "saturn-pass-test".to_owned(),
            mode: Some("read-only".to_owned()),
        }]
    );
}

#[test]
fn a_child_refuses_arguments_the_parent_already_decides() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("none.sock");
    let socket_text = missing.display().to_string();
    let cases: [(&str, &[&str]); 5] = [
        ("subcommand", &["usage"]),
        ("config", &["-c", "a=1"]),
        ("add-dir", &["--add-dir", "."]),
        ("continue", &["--continue"]),
        ("resume", &["--resume"]),
    ];

    for (name, args) in cases {
        let run = saturn(home.path(), args, &child_env(&socket_text), "");

        // 엔진 없이 사용법 오류(2)로 끝나므로 접속을 시도하기 전에 거절했다
        assert_eq!(run.code, Some(2), "{name}: {}", run.stderr);
    }
}

#[test]
fn a_child_does_not_start_an_engine_when_none_is_running() {
    let home = tempfile::tempdir().unwrap();
    let socket = home.path().join("none.sock");
    let socket_text = socket.display().to_string();

    let run = saturn(home.path(), &[], &child_env(&socket_text), "");

    assert_eq!(run.code, Some(69), "{}", run.stderr);
    assert!(!socket.exists());
    assert!(!home.path().join(".saturn").exists());
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
    let cancelled = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&cancelled);
    let run = run_with_engine(&["router", "train"], move |request| match request {
        Request::Train { .. } => Act::Ok(vec![Notification::TrainPreview {
            candidates: 250,
            grader: "grader-a".to_owned(),
            estimated_tokens: 1,
            threshold_targets: Vec::new(),
            retrain_model: false,
        }]),
        Request::ConfirmTrain { proceed: false } => {
            seen.store(true, Ordering::SeqCst);
            Act::Ok(Vec::new())
        }
        other => panic!("unexpected {other:?}"),
    });

    assert_eq!(run.code, Some(2), "{}", run.stderr);
    assert!(
        cancelled.load(Ordering::SeqCst),
        "no cancel request arrived"
    );
}

/// engine가 입력 하나를 접수해 작업 A로 실행하고 `end`로 끝내는 순서. `RequestSummary`는 보내지 않는다.
fn accepted_and_ended(end: TaskState, failure: Option<&str>) -> Vec<Notification> {
    let task = |state, failure: Option<&str>| Notification::TaskChanged {
        task: TaskId(1),
        label: TaskLabel('A'),
        state,
        provider: Some(Provider::from_static("codex")),
        elapsed_ms: 5,
        failure: failure.map(str::to_owned),
    };
    vec![
        Notification::InputAccepted {
            client_ref: 1,
            input: InputId(1),
        },
        Notification::InputChanged {
            input: InputId(1),
            text: "go".to_owned(),
            label: Some(TaskLabel('A')),
            state: InputState::Applied,
            disposition: Some(Disposition::NewTask),
            reason: None,
        },
        task(TaskState::Running, None),
        task(end, failure),
    ]
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
        Request::SubmitInput { .. } => Act::Ok(accepted_and_ended(
            TaskState::Failed,
            Some("provider failed"),
        )),
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
        Request::SubmitInput { .. } => Act::Ok(accepted_and_ended(TaskState::Done, None)),
        other => panic!("unexpected {other:?}"),
    });

    let run = saturn(home.path(), &[], &[], "go\n");

    engine.join().unwrap();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

// #457: 확인 번호를 모르는 옛 engine의 미리보기 응답은 읽지 못해도 기다리지 않고 실패로 끝낸다.
#[test]
fn prune_against_an_engine_that_answers_without_a_plan_fails_instead_of_waiting() {
    let run = run_with_engine(&["prune"], |_| Act::PlanlessPreview);

    assert!(
        matches!(run.code, Some(code) if code != 0),
        "{:?} {}",
        run.code,
        run.stderr
    );
}
