//! engine 찾기와 시작, 붙기. 동시 시작은 engine 잠금이 하나로 줄인다.
//! 설계: docs/design/engine-lifecycle.md

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use saturn_tui::client::{ClientError, EngineClient};

const ENGINE_BINARY: &str = "saturn-engine";

/// engine이 에이전트 작업의 환경에 넣는 변수 이름과 같다.
const NESTED_MARKER_ENV: &str = "SATURN_AGENT";

/// engine이 날짜별 로그를 쌓는 폴더. 소켓이 있는 폴더 아래. TODO(#235): 경로 설정 키
const ENGINE_LOG_DIR: &str = "logs";

/// engine 로그 파일 이름의 앞과 뒤. 사이에 `YYYY-MM-DD`가 온다.
const ENGINE_LOG_PREFIX: &str = "engine-";
const ENGINE_LOG_SUFFIX: &str = ".log";

/// engine이 router 확인까지 마치고 소켓을 여는 데 기다리는 시간. 초안 값.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// 띄운 engine이 먼저 끝났을 때 다른 engine이 소켓을 열기를 더 기다리는 시간. 초안 값.
const EXIT_GRACE: Duration = Duration::from_secs(2);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 실패 안내에 보이는 engine 로그 줄 수. 초안 값.
const LOG_TAIL_LINES: usize = 5;

/// 띄운 engine 프로세스와 그 로그 위치.
#[derive(Debug)]
struct StartedEngine {
    process: Child,
    log_dir: PathBuf,
    /// 이번 시작 전 가장 최근 로그 파일과 그 길이. 같은 파일이면 이 뒤의 내용만 실패 안내에 쓴다.
    log_before: Option<(PathBuf, u64)>,
}

/// TODO(#33): 자식 Saturn을 부모와 잇는 방식과 판별 신호가 정해지면 거절 대신 연결한다
///
/// # Errors
/// 에이전트 작업 안에서 실행됐으면 오류.
pub(crate) fn ensure_not_nested() -> anyhow::Result<()> {
    check_nested(std::env::var_os(NESTED_MARKER_ENV).as_deref())
}

fn check_nested(marker: Option<&OsStr>) -> anyhow::Result<()> {
    anyhow::ensure!(
        marker.is_none(),
        "saturn cannot run inside an agent task (the {NESTED_MARKER_ENV} variable is set)"
    );
    Ok(())
}

/// # Errors
/// engine 실행 파일이 없거나, engine이 시작에 실패했거나 제때 소켓을 열지 않으면 오류.
pub(crate) async fn connect_or_start() -> anyhow::Result<EngineClient> {
    connect_or_start_at(&EngineClient::default_socket(), engine_binary).await
}

// cost: time O(t) , heap O(1), stack O(1), io t
// vars: t = 소켓이 열릴 때까지의 접속 시도 수
// basis: estimate
/// 이미 도는 engine이 있으면 붙기만 한다. 없으면 `locate`가 준 실행 파일을 띄우고 소켓이 열리기를 기다린다.
async fn connect_or_start_at(
    socket: &Path,
    locate: impl FnOnce() -> anyhow::Result<PathBuf>,
) -> anyhow::Result<EngineClient> {
    match EngineClient::connect(socket).await {
        Ok(client) => return Ok(client),
        Err(ClientError::NotRunning { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let binary = locate()?;
    let mut engine = spawn_engine(&binary, socket)?;
    tracing::debug!(binary = %binary.display(), "engine started");
    wait_until_ready(socket, START_TIMEOUT, &mut engine).await
}

// cost: time O(p), heap O(p), stack O(1), io p
// vars: p = PATH 항목 수
// basis: estimate
/// `saturn`과 같은 폴더를 먼저 보고 없으면 `PATH`에서 찾는다.
fn engine_binary() -> anyhow::Result<PathBuf> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(ENGINE_BINARY)));
    if let Some(path) = sibling.filter(|path| path.is_file()) {
        return Ok(path);
    }
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(ENGINE_BINARY))
        .find(|candidate| candidate.is_file())
        .with_context(|| format!("{ENGINE_BINARY} not found next to saturn or in PATH"))
}

// cost: time O(1), heap O(1), stack O(1), io 3
// basis: estimate
/// `saturn`이 끝나도 남도록 새 프로세스 그룹으로 띄운다. engine은 터미널을 갖지 않으므로 router 키는 붙은 TUI가 보낸다.
fn spawn_engine(binary: &Path, socket: &Path) -> anyhow::Result<StartedEngine> {
    let home = socket
        .parent()
        .context("engine socket path should have a parent folder")?;
    let log_dir = home.join(ENGINE_LOG_DIR);
    let log_before = latest_log(&log_dir).map(|path| {
        let len = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        (path, len)
    });
    // engine이 로그 파일을 직접 쓴다. 날짜가 바뀌면 새 파일로 옮겨야 해서 stderr는 파일에 잇지 않는다
    let process = Command::new(binary)
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| format!("failed to start {}", binary.display()))?;
    Ok(StartedEngine {
        process,
        log_dir,
        log_before,
    })
}

// cost: time O(n), heap O(1), stack O(1), io n
// vars: n = 로그 폴더의 파일 수
// basis: estimate
/// 가장 최근 날짜의 `engine-YYYY-MM-DD.log`. 날짜 형식이라 이름순이 날짜순이다.
fn latest_log(log_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(log_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| {
                    name.starts_with(ENGINE_LOG_PREFIX) && name.ends_with(ENGINE_LOG_SUFFIX)
                })
        })
        .max()
}

// cost: time O(t), heap O(1), stack O(1), io t
// vars: t = timeout / POLL_INTERVAL
// basis: estimate
/// 띄운 engine이 잠금을 얻지 못해 끝나도, 동시에 시작한 다른 engine이 소켓을 열면 거기에 붙는다.
///
/// # Errors
/// `timeout` 안에 붙지 못하거나 engine이 먼저 끝나면 오류.
async fn wait_until_ready(
    socket: &Path,
    timeout: Duration,
    engine: &mut StartedEngine,
) -> anyhow::Result<EngineClient> {
    let started = Instant::now();
    let mut exited_at: Option<Instant> = None;
    loop {
        match EngineClient::connect(socket).await {
            Ok(client) => return Ok(client),
            Err(ClientError::NotRunning { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        if exited_at.is_none()
            && let Some(status) = engine.process.try_wait().context("failed to poll engine")?
        {
            tracing::debug!(%status, "started engine exited");
            exited_at = Some(Instant::now());
        }
        if let Some(exited) = exited_at
            && exited.elapsed() >= EXIT_GRACE
        {
            anyhow::bail!(
                "engine exited before opening {}{}",
                socket.display(),
                log_tail(&engine.log_dir, engine.log_before.as_ref())
            );
        }
        if started.elapsed() >= timeout {
            anyhow::bail!(
                "engine did not open {} within {}s{}",
                socket.display(),
                timeout.as_secs(),
                log_tail(&engine.log_dir, engine.log_before.as_ref())
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// cost: time O(b), heap O(b), stack O(1), io 1
// vars: b = 이번 시작 뒤 로그 바이트 수
// basis: estimate
/// 안내 문구 뒤에 붙일 가장 최근 로그 파일의 끝 줄. 이번 시작 전과 같은 파일이면 시작 뒤 내용만 쓴다.
/// 읽을 수 없거나 비었으면 빈 문자열.
fn log_tail(log_dir: &Path, before: Option<&(PathBuf, u64)>) -> String {
    let Some(log) = latest_log(log_dir) else {
        return String::new();
    };
    let start = before
        .filter(|(path, _)| *path == log)
        .map_or(0, |(_, len)| *len);
    let Ok(mut file) = File::open(&log) else {
        return String::new();
    };
    let mut text = String::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_string(&mut text).is_err() {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(LOG_TAIL_LINES)..];
    if tail.is_empty() {
        return String::new();
    }
    format!("\n{}\n(log: {})", tail.join("\n"), log.display())
}

#[cfg(test)]
mod tests {
    use tokio::net::UnixListener;

    use super::*;

    const SOCKET_FILE: &str = "engine.sock";

    const LOG_FILE: &str = "engine-2026-10-02.log";

    /// `script`의 stderr가 로그 폴더의 날짜별 파일에 쌓이는 가짜 engine.
    fn exiting_engine(home: &Path, script: &str) -> StartedEngine {
        let log_dir = home.join(ENGINE_LOG_DIR);
        std::fs::create_dir_all(&log_dir).unwrap();
        let process = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "exec 2>>'{}'; {script}",
                log_dir.join(LOG_FILE).display()
            ))
            .spawn()
            .unwrap();
        StartedEngine {
            process,
            log_dir,
            log_before: None,
        }
    }

    #[test]
    fn check_nested_with_marker_is_error() {
        assert!(check_nested(Some(OsStr::new("1"))).is_err());
    }

    #[test]
    fn check_nested_without_marker_is_ok() {
        assert!(check_nested(None).is_ok());
    }

    #[tokio::test]
    async fn connect_or_start_with_running_engine_only_attaches() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let _listener = UnixListener::bind(&socket).unwrap();

        let client = connect_or_start_at(&socket, || anyhow::bail!("must not locate")).await;

        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn connect_or_start_without_binary_reports_missing_engine() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);

        let error = connect_or_start_at(&socket, || anyhow::bail!("saturn-engine not found"))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn wait_until_ready_attaches_when_socket_opens_later() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine = exiting_engine(home.path(), "sleep 5");
        let late_socket = socket.clone();
        let opener = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            UnixListener::bind(&late_socket).unwrap()
        });

        let client = wait_until_ready(&socket, Duration::from_secs(5), &mut engine).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
        engine.process.kill().unwrap();
        engine.process.wait().unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_attaches_to_other_engine_after_lock_loss() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine = exiting_engine(home.path(), "echo 'engine already running' >&2; exit 1");
        let late_socket = socket.clone();
        let opener = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            UnixListener::bind(&late_socket).unwrap()
        });

        let client = wait_until_ready(&socket, Duration::from_secs(5), &mut engine).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_after_engine_exit_reports_log_tail() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine =
            exiting_engine(home.path(), "echo 'router host is not allowed' >&2; exit 1");

        let error = wait_until_ready(&socket, Duration::from_secs(10), &mut engine)
            .await
            .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("engine exited"));
        assert!(message.contains("router host is not allowed"));
    }

    #[tokio::test]
    async fn wait_until_ready_times_out_while_engine_keeps_running() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine = exiting_engine(home.path(), "sleep 5");

        let error = wait_until_ready(&socket, Duration::from_millis(200), &mut engine)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("did not open"));
        engine.process.kill().unwrap();
        engine.process.wait().unwrap();
    }

    #[test]
    fn spawn_engine_passes_home_and_discards_stderr() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let script = home.path().join("fake-engine");
        std::fs::write(
            &script,
            "#!/bin/sh\necho \"args: $*\" > \"$2/args\"\necho noise >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let mut engine = spawn_engine(&script, &socket).unwrap();
        engine.process.wait().unwrap();

        assert_eq!(
            std::fs::read_to_string(home.path().join("args")).unwrap(),
            format!("args: --home {}\n", home.path().display())
        );
        assert_eq!(engine.log_dir, home.path().join("logs"));
        assert!(!home.path().join("logs").exists());
    }

    #[test]
    fn engine_log_tail_reads_the_latest_dated_file() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("engine-2026-10-01.log"), "yesterday\n").unwrap();
        std::fs::write(home.path().join(LOG_FILE), "today\n").unwrap();
        std::fs::write(home.path().join("engine-2026-10-03.txt"), "other\n").unwrap();

        let tail = log_tail(home.path(), None);

        assert!(tail.contains("today"));
        assert!(!tail.contains("yesterday"));
        assert!(!tail.contains("other"));
    }

    #[test]
    fn engine_log_tail_without_a_log_file_is_empty() {
        let home = tempfile::tempdir().unwrap();

        assert_eq!(log_tail(home.path(), None), "");
    }

    #[test]
    fn engine_log_tail_keeps_only_lines_after_the_start_offset_of_the_same_file() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join(LOG_FILE);
        std::fs::write(&log, "old line\nnew line\n").unwrap();
        let before = (log, "old line\n".len() as u64);

        let tail = log_tail(home.path(), Some(&before));

        assert!(tail.contains("new line"));
        assert!(!tail.contains("old line"));
    }

    #[test]
    fn engine_log_tail_reads_a_new_day_file_from_the_start() {
        let home = tempfile::tempdir().unwrap();
        let yesterday = home.path().join("engine-2026-10-01.log");
        std::fs::write(&yesterday, "old line\n").unwrap();
        std::fs::write(home.path().join(LOG_FILE), "new line\n").unwrap();
        let before = (yesterday, 100);

        let tail = log_tail(home.path(), Some(&before));

        assert!(tail.contains("new line"));
    }
}
