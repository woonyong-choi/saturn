//! engine 찾기와 시작, 붙기. 동시 시작은 engine 잠금이 하나로 줄인다.
//! 설계: docs/design/engine-lifecycle.md

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use saturn_tui::client::{ClientError, EngineClient};

const ENGINE_BINARY: &str = "saturn-engine";

/// engine이 에이전트 작업의 환경에 넣는 변수 이름과 같다.
const NESTED_MARKER_ENV: &str = "SATURN_AGENT";

/// engine의 stderr를 받는 파일이 있는 폴더. 소켓이 있는 폴더 아래. TODO(#235): 경로 설정 키
const ENGINE_LOG_DIR: &str = "logs";

/// engine의 stderr를 받는 파일 이름. `ENGINE_LOG_DIR` 안에 둔다.
const ENGINE_LOG_FILE: &str = "engine.log";

/// engine을 띄울 때 로그가 이 크기(바이트)를 넘었으면 돌린다. 초안 값.
const ENGINE_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// 돌린 뒤 보관하는 이전 로그 파일 수(`engine.log.1`이 가장 최근). 초안 값.
const ENGINE_LOG_KEEP: u32 = 5;

/// engine이 router 확인까지 마치고 소켓을 여는 데 기다리는 시간. 초안 값.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// 띄운 engine이 먼저 끝났을 때 다른 engine이 소켓을 열기를 더 기다리는 시간. 초안 값.
const EXIT_GRACE: Duration = Duration::from_secs(2);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 실패 안내에 보이는 engine 로그 줄 수. 초안 값.
const LOG_TAIL_LINES: usize = 5;

/// 띄운 engine 프로세스와 그 stderr 로그 위치.
#[derive(Debug)]
struct StartedEngine {
    process: Child,
    log: PathBuf,
    /// 이번 시작 전 로그 길이. 이 뒤의 내용만 실패 안내에 쓴다.
    log_start: u64,
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
    std::fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;
    std::fs::set_permissions(&log_dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", log_dir.display()))?;
    let log = log_dir.join(ENGINE_LOG_FILE);
    rotate_log(&log, ENGINE_LOG_MAX_BYTES, ENGINE_LOG_KEEP)?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .with_context(|| format!("failed to open {}", log.display()))?;
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", log.display()))?;
    let log_start = log_file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let process = Command::new(binary)
        .arg("--home")
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        .process_group(0)
        .spawn()
        .with_context(|| format!("failed to start {}", binary.display()))?;
    Ok(StartedEngine {
        process,
        log,
        log_start,
    })
}

// cost: time O(k), heap O(1), stack O(1), io k
// vars: k = 보관하는 이전 로그 파일 수
// basis: estimate
/// `log`가 `max_bytes`를 넘었으면 `log.1`로 옮기고, 앞서 있던 `log.n`은 `log.(n+1)`로 한 칸씩 밀어
/// `keep`개까지만 남긴다. 상한 안이면 아무것도 하지 않는다. 돌리는 때는 engine을 띄울 때뿐이라 실행 중인
/// engine의 로그는 다음 시작 때 돈다.
///
/// # Errors
/// 파일 이름을 바꾸거나 지울 수 없으면 오류.
fn rotate_log(log: &Path, max_bytes: u64, keep: u32) -> anyhow::Result<()> {
    let over = std::fs::metadata(log).is_ok_and(|meta| meta.len() > max_bytes);
    if !over {
        return Ok(());
    }
    let numbered = |n: u32| {
        let mut name = log.as_os_str().to_owned();
        name.push(format!(".{n}"));
        PathBuf::from(name)
    };
    if keep == 0 {
        return std::fs::remove_file(log)
            .with_context(|| format!("failed to remove {}", log.display()));
    }
    let oldest = numbered(keep);
    if oldest.exists() {
        std::fs::remove_file(&oldest)
            .with_context(|| format!("failed to remove {}", oldest.display()))?;
    }
    for n in (1..keep).rev() {
        let from = numbered(n);
        if from.exists() {
            std::fs::rename(&from, numbered(n + 1))
                .with_context(|| format!("failed to rotate {}", from.display()))?;
        }
    }
    std::fs::rename(log, numbered(1)).with_context(|| format!("failed to rotate {}", log.display()))
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
                log_tail(&engine.log, engine.log_start)
            );
        }
        if started.elapsed() >= timeout {
            anyhow::bail!(
                "engine did not open {} within {}s{}",
                socket.display(),
                timeout.as_secs(),
                log_tail(&engine.log, engine.log_start)
            );
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// cost: time O(b), heap O(b), stack O(1), io 1
// vars: b = 이번 시작 뒤 로그 바이트 수
// basis: estimate
/// 안내 문구 뒤에 붙일 로그 끝 줄. 읽을 수 없거나 비었으면 빈 문자열.
fn log_tail(log: &Path, start: u64) -> String {
    let Ok(mut file) = File::open(log) else {
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

    fn exiting_engine(home: &Path, script: &str) -> StartedEngine {
        let log = home.join(ENGINE_LOG_FILE);
        let log_file = File::create(&log).unwrap();
        let process = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stderr(log_file)
            .spawn()
            .unwrap();
        StartedEngine {
            process,
            log,
            log_start: 0,
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
    fn spawn_engine_passes_home_and_writes_stderr_to_engine_log_under_logs() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let script = home.path().join("fake-engine");
        std::fs::write(&script, "#!/bin/sh\necho \"args: $*\" >&2\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let mut engine = spawn_engine(&script, &socket).unwrap();
        engine.process.wait().unwrap();

        let log = std::fs::read_to_string(&engine.log).unwrap();
        assert_eq!(engine.log, home.path().join("logs/engine.log"));
        assert_eq!(log, format!("args: --home {}\n", home.path().display()));
        assert_eq!(
            std::fs::metadata(&engine.log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(home.path().join("logs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn spawn_engine_rotates_an_engine_log_over_the_cap_before_writing() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let logs = home.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let old = File::create(logs.join("engine.log")).unwrap();
        old.set_len(ENGINE_LOG_MAX_BYTES + 1).unwrap();
        let script = home.path().join("fake-engine");
        std::fs::write(&script, "#!/bin/sh\necho started >&2\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();

        let mut engine = spawn_engine(&script, &socket).unwrap();
        engine.process.wait().unwrap();

        assert_eq!(std::fs::read_to_string(&engine.log).unwrap(), "started\n");
        assert_eq!(
            std::fs::metadata(logs.join("engine.log.1")).unwrap().len(),
            ENGINE_LOG_MAX_BYTES + 1
        );
        assert_eq!(engine.log_start, 0);
    }

    #[test]
    fn engine_log_within_the_cap_is_not_rotated() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("engine.log");
        std::fs::write(&log, "1234").unwrap();

        rotate_log(&log, 4, 3).unwrap();

        assert_eq!(std::fs::read_to_string(&log).unwrap(), "1234");
        assert!(!home.path().join("engine.log.1").exists());
    }

    #[test]
    fn engine_log_over_the_cap_shifts_numbered_files_and_keeps_only_the_newest() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("engine.log");
        let numbered = |n: u32| home.path().join(format!("engine.log.{n}"));
        std::fs::write(&log, "current!").unwrap();
        std::fs::write(numbered(1), "one").unwrap();
        std::fs::write(numbered(2), "two").unwrap();

        rotate_log(&log, 4, 2).unwrap();

        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(numbered(1)).unwrap(), "current!");
        assert_eq!(std::fs::read_to_string(numbered(2)).unwrap(), "one");
        assert!(!numbered(3).exists());
    }

    #[test]
    fn engine_log_without_a_file_has_nothing_to_rotate() {
        let home = tempfile::tempdir().unwrap();

        rotate_log(&home.path().join("engine.log"), 4, 2).unwrap();

        assert!(!home.path().join("engine.log.1").exists());
    }

    #[test]
    fn log_tail_keeps_only_lines_after_start_offset() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join(ENGINE_LOG_FILE);
        std::fs::write(&log, "old line\nnew line\n").unwrap();

        let tail = log_tail(&log, "old line\n".len() as u64);

        assert!(tail.contains("new line"));
        assert!(!tail.contains("old line"));
    }
}
