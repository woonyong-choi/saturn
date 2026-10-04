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
use saturn_protocol::envelope::INVALID_REQUEST;
use saturn_protocol::rpc::{PASS_ENV, PROTOCOL_VERSION, Request, SOCKET_ENV};
use saturn_tui::client::{ClientError, EngineClient, EngineVersion};
use saturn_tui::i18n::{self, Lang};

use crate::exit::{Exit, ExitCode};

const ENGINE_BINARY: &str = "saturn-engine";

/// engine이 에이전트 작업의 환경에 넣는 변수 이름과 같다.
const NESTED_MARKER_ENV: &str = "SATURN_AGENT";

/// engine이 날짜별 로그를 쌓는 폴더. 소켓이 있는 홈 폴더 아래.
const ENGINE_LOG_DIR: &str = saturn_protocol::home::LOG_DIR;

/// engine 로그 파일 이름의 앞과 뒤. 사이에 `YYYY-MM-DD`가 온다.
const ENGINE_LOG_PREFIX: &str = "engine-";
const ENGINE_LOG_SUFFIX: &str = ".log";

/// engine이 router 확인까지 마치고 소켓을 여는 데 기다리는 시간. 초안 값.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// 띄운 engine이 먼저 끝났을 때 다른 engine이 소켓을 열기를 더 기다리는 시간. 초안 값.
const EXIT_GRACE: Duration = Duration::from_secs(2);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 옛 engine에 종료를 요청하거나 신호를 보낸 뒤 프로세스가 끝나기를 기다리는 시간. 초안 값.
const UPGRADE_WAIT: Duration = Duration::from_secs(15);

/// 강제 종료 신호를 보낸 뒤 프로세스가 끝나기를 기다리는 시간. 초안 값.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// `Version`과 `Shutdown` 요청의 응답을 기다리는 시간. 초안 값.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// 업데이트로 옛 engine을 끝내고 띄우는 engine에 주는 인자. engine이 첫 TUI에 알림을 보낸다.
const AFTER_UPGRADE_ARG: &str = "--after-upgrade";

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

/// 에이전트 작업 안에서 실행됐는지와 출입증이 있는지. engine이 띄운 provider 프로세스 환경에만 이 변수가 있다.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Origin {
    /// 사용자 터미널이나 Saturn 밖의 도구. 일반 접속이다.
    Outside,
    /// Saturn이 띄운 에이전트 작업 안. 출입증으로 부모 채팅의 하위 작업이 된다.
    Child { pass: String, socket: PathBuf },
}

impl std::fmt::Debug for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Outside => f.write_str("Outside"),
            Self::Child { socket, .. } => f
                .debug_struct("Child")
                .field("socket", socket)
                .finish_non_exhaustive(),
        }
    }
}

/// 출입증은 engine이 provider 프로세스에만 주므로 표지는 있는데 출입증이 없으면 회수됐거나 만들지 못한 것이다.
///
/// # Errors
/// 에이전트 작업 안인데 출입증이 없으면 오류.
pub(crate) fn origin(lang: Lang) -> anyhow::Result<Origin> {
    origin_of(
        lang,
        std::env::var_os(NESTED_MARKER_ENV).as_deref(),
        std::env::var(PASS_ENV).ok(),
        std::env::var_os(SOCKET_ENV).map(PathBuf::from),
    )
}

fn origin_of(
    lang: Lang,
    marker: Option<&OsStr>,
    pass: Option<String>,
    socket: Option<PathBuf>,
) -> anyhow::Result<Origin> {
    match (marker, pass.filter(|pass| !pass.is_empty())) {
        (_, Some(pass)) => Ok(Origin::Child {
            pass,
            socket: socket.unwrap_or_else(EngineClient::default_socket),
        }),
        (None, None) => Ok(Origin::Outside),
        (Some(_), None) => Err(Exit::error(
            ExitCode::Usage,
            lang.tr(i18n::CLI_PASS_MISSING).replace("{pass}", PASS_ENV),
        )),
    }
}

/// # Errors
/// engine 실행 파일이 없거나, engine이 시작에 실패했거나 제때 소켓을 열지 않으면 오류.
/// 옛 engine을 끝내지 못해도 오류.
pub(crate) async fn connect_or_start(lang: Lang) -> anyhow::Result<EngineClient> {
    connect_or_start_at(
        lang,
        &EngineClient::default_socket(),
        || engine_binary(lang),
        &OsProcesses,
        UpgradeLimits::default(),
    )
    .await
}

/// 옛 engine을 끝낼 때 기다리는 시간.
#[derive(Debug, Clone, Copy)]
struct UpgradeLimits {
    /// 종료 요청이나 `SIGTERM` 뒤.
    wait: Duration,
    /// `SIGKILL` 뒤.
    kill_wait: Duration,
}

impl Default for UpgradeLimits {
    fn default() -> Self {
        Self {
            wait: UPGRADE_WAIT,
            kill_wait: KILL_WAIT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Term,
    Kill,
}

/// engine 프로세스를 다루는 자리. 시험은 가짜를 넣어 시험 프로세스에 신호를 보내지 않는다.
trait Processes {
    fn is_alive(&self, pid: i32) -> bool;
    fn signal(&self, pid: i32, signal: Signal);
}

struct OsProcesses;

impl Processes for OsProcesses {
    fn is_alive(&self, pid: i32) -> bool {
        // SAFETY: 신호 0은 프로세스 존재만 확인하고 아무것도 보내지 않는다.
        #[expect(unsafe_code, reason = "libc kill 호출")]
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// 자기 자신과 1 이하의 번호에는 보내지 않는다.
    fn signal(&self, pid: i32, signal: Signal) {
        let own = i32::try_from(std::process::id()).unwrap_or(0);
        if pid <= 1 || pid == own {
            return;
        }
        let number = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        // SAFETY: 확인한 번호의 프로세스에 종료 신호 하나를 보낸다.
        #[expect(unsafe_code, reason = "libc kill 호출")]
        let result = unsafe { libc::kill(pid, number) };
        if result != 0 {
            tracing::debug!(pid, ?signal, error = %std::io::Error::last_os_error(), "signal was not delivered");
        }
    }
}

/// 옛 engine을 끝내는 방법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retire {
    /// `Shutdown`을 아는 engine. 정상 종료를 요청한다.
    Request,
    /// `Version`도 모르는 더 옛 engine. 소켓 반대편 프로세스에 `SIGTERM`을 보낸다.
    Signal,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Current,
    Retire(Retire),
}

fn own_version() -> EngineVersion {
    EngineVersion {
        saturn_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: PROTOCOL_VERSION,
    }
}

/// `1.2.3-rc1`에서 `[1, 2, 3]`. 숫자가 아닌 칸이 있으면 `None`.
fn numeric_version(version: &str) -> Option<Vec<u64>> {
    version
        .split(['-', '+'])
        .next()?
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}

/// engine이 이 클라이언트보다 옛 판일 때만 교체한다. 더 새 engine에는 그대로 붙어, 판이 다른 두 `saturn`이
/// 서로의 engine을 번갈아 끝내지 않게 한다.
fn judge(own: &EngineVersion, engine: &EngineVersion) -> Verdict {
    if engine.protocol_version != own.protocol_version {
        return if engine.protocol_version < own.protocol_version {
            Verdict::Retire(Retire::Request)
        } else {
            Verdict::Current
        };
    }
    if engine.saturn_version == own.saturn_version {
        return Verdict::Current;
    }
    match (
        numeric_version(&engine.saturn_version),
        numeric_version(&own.saturn_version),
    ) {
        (Some(theirs), Some(ours)) if theirs >= ours => Verdict::Current,
        _ => Verdict::Retire(Retire::Request),
    }
}

// cost: time O(1), heap O(1), stack O(1), io 2
// basis: estimate
/// `Version`을 모르는 옛 engine은 `Signal`로 교체한다.
///
/// # Errors
/// 답이 없거나 연결이 끊겼으면 오류. 옛 engine으로 보고 끝내지 않는다.
async fn engine_verdict(client: &mut EngineClient) -> anyhow::Result<Verdict> {
    let asked = tokio::time::timeout(REQUEST_TIMEOUT, client.version())
        .await
        .context("engine did not answer the version request")?;
    match asked {
        Ok(version) => Ok(judge(&own_version(), &version)),
        Err(ClientError::Rejected { code, .. }) if code == INVALID_REQUEST => {
            Ok(Verdict::Retire(Retire::Signal))
        }
        Err(error) => Err(error.into()),
    }
}

/// 옛 engine을 끝낸다. 정상 종료를 기다리다 상한을 넘으면 `SIGKILL`로 끝내고, 그래도 남으면 오류다.
/// 끝나지 않은 실행은 기록에 그대로 남아 새 engine의 크래시 복구가 이어 받는다.
///
/// # Errors
/// 프로세스 번호를 알 수 없는 옛 engine이거나, 강제 종료 뒤에도 끝나지 않으면 오류.
async fn retire_engine(
    lang: Lang,
    socket: &Path,
    client: EngineClient,
    how: Retire,
    processes: &dyn Processes,
    limits: UpgradeLimits,
) -> anyhow::Result<()> {
    let pid = request_end(lang, socket, client, how, processes).await?;
    if wait_until_gone(socket, pid, processes, limits.wait).await {
        return Ok(());
    }
    tracing::warn!(?pid, "old engine did not end in time, killing it");
    if let Some(pid) = pid {
        processes.signal(pid, Signal::Kill);
    }
    if wait_until_gone(socket, pid, processes, limits.kill_wait).await {
        return Ok(());
    }
    anyhow::bail!(
        lang.tr(i18n::CLI_ENGINE_UPGRADE_FAILED)
            .replace("{socket}", &socket.display().to_string())
    )
}

/// 종료 요청을 보내거나 신호를 보내고 옛 engine의 프로세스 번호를 돌려준다. 접속은 여기서 닫는다.
async fn request_end(
    lang: Lang,
    socket: &Path,
    mut client: EngineClient,
    how: Retire,
    processes: &dyn Processes,
) -> anyhow::Result<Option<i32>> {
    let pid = client.peer_pid();
    match how {
        Retire::Request => {
            // 응답이 오면 곧 연결이 끊긴다. 요청이 닿았는지는 프로세스가 끝나는지로 본다
            let sent =
                tokio::time::timeout(REQUEST_TIMEOUT, client.call(Request::Shutdown, drop)).await;
            tracing::debug!(?sent, "engine shutdown requested");
        }
        Retire::Signal => {
            let pid = pid.with_context(|| {
                lang.tr(i18n::CLI_ENGINE_UPGRADE_NO_PID)
                    .replace("{socket}", &socket.display().to_string())
            })?;
            processes.signal(pid, Signal::Term);
        }
    }
    Ok(pid)
}

/// 프로세스 번호를 알면 그 프로세스가 없어질 때까지, 모르면 소켓이 닫힐 때까지 기다린다.
async fn wait_until_gone(
    socket: &Path,
    pid: Option<i32>,
    processes: &dyn Processes,
    limit: Duration,
) -> bool {
    let started = Instant::now();
    loop {
        let gone = match pid {
            Some(pid) => !processes.is_alive(pid),
            None => matches!(
                EngineClient::connect(socket).await,
                Err(ClientError::NotRunning { .. })
            ),
        };
        if gone {
            return true;
        }
        if started.elapsed() >= limit {
            return false;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

// cost: time O(t), heap O(1), stack O(1), io t
// vars: t = 소켓이 열릴 때까지의 접속 시도 수
// basis: estimate
/// 이미 도는 engine이 같거나 더 새 판이면 붙기만 한다. 옛 판이면 끝내고 새로 띄운다.
/// 없으면 `locate`가 준 실행 파일을 띄우고 소켓이 열리기를 기다린다.
/// 어느 단계에서 실패해도 engine을 쓸 수 없는 경우라 종료 코드는 `EngineUnavailable`이다.
async fn connect_or_start_at(
    lang: Lang,
    socket: &Path,
    locate: impl FnOnce() -> anyhow::Result<PathBuf>,
    processes: &dyn Processes,
    limits: UpgradeLimits,
) -> anyhow::Result<EngineClient> {
    attach_or_start(lang, socket, locate, processes, limits)
        .await
        .map_err(|error| Exit::wrap(ExitCode::EngineUnavailable, error))
}

// cost: time O(t), heap O(1), stack O(1), io t
// vars: t = 소켓이 열릴 때까지의 접속 시도 수
// basis: estimate
async fn attach_or_start(
    lang: Lang,
    socket: &Path,
    locate: impl FnOnce() -> anyhow::Result<PathBuf>,
    processes: &dyn Processes,
    limits: UpgradeLimits,
) -> anyhow::Result<EngineClient> {
    let mut after_upgrade = false;
    match EngineClient::connect(socket).await {
        Ok(mut client) => match engine_verdict(&mut client).await? {
            Verdict::Current => return Ok(client),
            Verdict::Retire(how) => {
                retire_engine(lang, socket, client, how, processes, limits).await?;
                after_upgrade = true;
            }
        },
        Err(ClientError::NotRunning { .. }) => {}
        Err(error) => return Err(error.into()),
    }
    let binary = locate()?;
    let mut engine = spawn_engine(lang, &binary, socket, after_upgrade)?;
    tracing::debug!(binary = %binary.display(), after_upgrade, "engine started");
    wait_until_ready(lang, socket, START_TIMEOUT, &mut engine).await
}

// cost: time O(p), heap O(p), stack O(1), io p
// vars: p = PATH 항목 수
// basis: estimate
/// `saturn`과 같은 폴더를 먼저 보고 없으면 `PATH`에서 찾는다.
fn engine_binary(lang: Lang) -> anyhow::Result<PathBuf> {
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
        .with_context(|| {
            lang.tr(i18n::CLI_ENGINE_NOT_FOUND)
                .replace("{binary}", ENGINE_BINARY)
        })
}

// cost: time O(1), heap O(1), stack O(1), io 3
// basis: estimate
/// `saturn`이 끝나도 남도록 새 프로세스 그룹으로 띄운다. engine은 터미널을 갖지 않으므로 router 키는 붙은 TUI가 보낸다.
fn spawn_engine(
    lang: Lang,
    binary: &Path,
    socket: &Path,
    after_upgrade: bool,
) -> anyhow::Result<StartedEngine> {
    let home = socket
        .parent()
        .context("engine socket path should have a parent folder")?;
    let log_dir = home.join(ENGINE_LOG_DIR);
    let log_before = latest_log(&log_dir).map(|path| {
        let len = std::fs::metadata(&path).map_or(0, |meta| meta.len());
        (path, len)
    });
    // engine이 로그 파일을 직접 쓴다. 날짜가 바뀌면 새 파일로 옮겨야 해서 stderr는 파일에 잇지 않는다
    let mut command = Command::new(binary);
    command.arg("--home").arg(home);
    if after_upgrade {
        command.arg(AFTER_UPGRADE_ARG);
    }
    let process = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .with_context(|| {
            lang.tr(i18n::CLI_ENGINE_START_FAILED)
                .replace("{binary}", &binary.display().to_string())
        })?;
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
    lang: Lang,
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
            && let Some(status) = engine
                .process
                .try_wait()
                .context(lang.tr(i18n::CLI_ENGINE_POLL_FAILED))?
        {
            tracing::debug!(%status, "started engine exited");
            exited_at = Some(Instant::now());
        }
        if let Some(exited) = exited_at
            && exited.elapsed() >= EXIT_GRACE
        {
            anyhow::bail!(
                lang.tr(i18n::CLI_ENGINE_EXITED)
                    .replace("{socket}", &socket.display().to_string())
                    .replace(
                        "{tail}",
                        &log_tail(lang, &engine.log_dir, engine.log_before.as_ref())
                    )
            );
        }
        if started.elapsed() >= timeout {
            anyhow::bail!(
                lang.tr(i18n::CLI_ENGINE_TIMEOUT)
                    .replace("{socket}", &socket.display().to_string())
                    .replace("{secs}", &timeout.as_secs().to_string())
                    .replace(
                        "{tail}",
                        &log_tail(lang, &engine.log_dir, engine.log_before.as_ref())
                    )
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
fn log_tail(lang: Lang, log_dir: &Path, before: Option<&(PathBuf, u64)>) -> String {
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
    let log_path = lang
        .tr(i18n::CLI_LOG_PATH)
        .replace("{path}", &log.display().to_string());
    format!("\n{}\n{log_path}", tail.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use saturn_protocol::envelope::{
        ClientMessage, Response, ServerMessage, decode_client_line, encode_line,
    };
    use saturn_protocol::rpc::Notification;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    use super::*;

    const SOCKET_FILE: &str = "engine.sock";

    const LIMITS: UpgradeLimits = UpgradeLimits {
        wait: Duration::from_millis(300),
        kill_wait: Duration::from_millis(300),
    };

    /// 신호를 보내지 않고 프로세스가 없다고 답하는 가짜.
    struct NoProcesses;

    impl Processes for NoProcesses {
        fn is_alive(&self, _pid: i32) -> bool {
            false
        }

        fn signal(&self, _pid: i32, _signal: Signal) {}
    }

    /// 소켓에서 `Version`과 `Shutdown`에 답하는 가짜 engine과, 그 프로세스를 흉내 내는 상태.
    struct FakeEngine {
        socket: PathBuf,
        alive: AtomicBool,
        shutdowns: AtomicUsize,
        signals: Mutex<Vec<Signal>>,
        /// 이 신호를 받으면 프로세스가 끝난다. `None`이면 어떤 신호에도 끝나지 않는다.
        dies_on: Option<Signal>,
        /// 참이면 `Shutdown`을 받고 끝난다.
        obeys_shutdown: bool,
    }

    impl FakeEngine {
        fn die(&self) {
            self.alive.store(false, Ordering::SeqCst);
            let _ = std::fs::remove_file(&self.socket); // 이미 없어도 된다
        }
    }

    impl Processes for Arc<FakeEngine> {
        fn is_alive(&self, _pid: i32) -> bool {
            self.alive.load(Ordering::SeqCst)
        }

        fn signal(&self, _pid: i32, signal: Signal) {
            self.signals.lock().unwrap().push(signal);
            if self.dies_on == Some(signal) {
                self.die();
            }
        }
    }

    /// `version`이 `None`이면 `Version`을 모르는 옛 engine이다.
    fn start_fake_engine(
        socket: &Path,
        version: Option<EngineVersion>,
        dies_on: Option<Signal>,
        obeys_shutdown: bool,
    ) -> Arc<FakeEngine> {
        let engine = Arc::new(FakeEngine {
            socket: socket.to_owned(),
            alive: AtomicBool::new(true),
            shutdowns: AtomicUsize::new(0),
            signals: Mutex::new(Vec::new()),
            dies_on,
            obeys_shutdown,
        });
        let listener = UnixListener::bind(socket).unwrap();
        let state = Arc::clone(&engine);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(answer(stream, Arc::clone(&state), version.clone()));
            }
        });
        engine
    }

    async fn answer(
        stream: tokio::net::UnixStream,
        engine: Arc<FakeEngine>,
        version: Option<EngineVersion>,
    ) {
        let (read_half, mut writer) = stream.into_split();
        let mut lines = BufReader::new(read_half).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let message: ClientMessage = decode_client_line(&line).unwrap();
            let mut replies: Vec<ServerMessage> = Vec::new();
            match message.request {
                Request::Version => match &version {
                    Some(version) => {
                        replies.push(
                            Notification::EngineVersion {
                                saturn_version: version.saturn_version.clone(),
                                protocol_version: version.protocol_version,
                            }
                            .into(),
                        );
                        replies.push(Response::ok(message.id).into());
                    }
                    None => replies.push(
                        Response::error(Some(message.id), INVALID_REQUEST, "invalid request")
                            .into(),
                    ),
                },
                Request::Shutdown => {
                    engine.shutdowns.fetch_add(1, Ordering::SeqCst);
                    replies.push(Response::ok(message.id).into());
                }
                _ => replies.push(Response::ok(message.id).into()),
            }
            for reply in replies {
                writer
                    .write_all(encode_line(&reply).unwrap().as_bytes())
                    .await
                    .unwrap();
            }
            if engine.obeys_shutdown && engine.shutdowns.load(Ordering::SeqCst) > 0 {
                engine.die();
                return;
            }
        }
    }

    fn version(text: &str, protocol: u32) -> EngineVersion {
        EngineVersion {
            saturn_version: text.to_owned(),
            protocol_version: protocol,
        }
    }

    /// 실행하면 받은 인자를 `args`에 남기고 끝나는 가짜 새 engine 실행 파일.
    fn fake_binary(home: &Path) -> PathBuf {
        let script = home.join("fake-engine");
        std::fs::write(&script, "#!/bin/sh\necho \"$*\" > \"$2/args\"\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        script
    }

    /// 새 engine이 뜬 것처럼 잠시 뒤 소켓을 연다.
    fn open_new_engine_later(socket: &Path) -> tokio::task::JoinHandle<UnixListener> {
        let socket = socket.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(600)).await;
            let _ = std::fs::remove_file(&socket); // 옛 소켓 파일이 남았을 수 있다
            UnixListener::bind(&socket).unwrap()
        })
    }

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
    fn origin_with_marker_and_no_pass_is_error() {
        let error = origin_of(Lang::En, Some(OsStr::new("1")), None, None).unwrap_err();

        assert!(error.to_string().contains(PASS_ENV));
    }

    #[test]
    fn origin_with_an_empty_pass_is_error() {
        let result = origin_of(Lang::En, Some(OsStr::new("1")), Some(String::new()), None);

        assert!(result.is_err());
    }

    #[test]
    fn origin_without_marker_or_pass_is_outside() {
        assert_eq!(
            origin_of(Lang::En, None, None, None).unwrap(),
            Origin::Outside
        );
    }

    #[test]
    fn origin_with_a_pass_is_a_child_on_the_given_socket() {
        let origin = origin_of(
            Lang::En,
            Some(OsStr::new("1")),
            Some("pass-value".to_owned()),
            Some(PathBuf::from("/run/engine.sock")),
        )
        .unwrap();

        assert_eq!(
            origin,
            Origin::Child {
                pass: "pass-value".to_owned(),
                socket: PathBuf::from("/run/engine.sock"),
            }
        );
    }

    #[tokio::test]
    async fn connect_or_start_with_running_engine_only_attaches() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let current = start_fake_engine(&socket, Some(own_version()), None, true);

        let client = connect_or_start_at(
            Lang::En,
            &socket,
            || anyhow::bail!("must not locate"),
            &current,
            LIMITS,
        )
        .await;

        assert!(client.is_ok());
        assert_eq!(current.shutdowns.load(Ordering::SeqCst), 0);
        assert!(current.signals.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn connect_or_start_keeps_an_engine_of_a_newer_version() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let newer = start_fake_engine(&socket, Some(version("999.0.0", 1)), None, true);

        let client = connect_or_start_at(
            Lang::En,
            &socket,
            || anyhow::bail!("must not locate"),
            &newer,
            LIMITS,
        )
        .await;

        assert!(client.is_ok());
        assert_eq!(newer.shutdowns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn connect_or_start_replaces_an_older_engine_by_asking_it_to_shut_down() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let old = start_fake_engine(&socket, Some(version("0.0.1", 1)), None, true);
        let binary = fake_binary(home.path());
        let opener = open_new_engine_later(&socket);

        let client = connect_or_start_at(Lang::En, &socket, || Ok(binary), &old, LIMITS).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
        assert_eq!(old.shutdowns.load(Ordering::SeqCst), 1);
        assert!(old.signals.lock().unwrap().is_empty());
        let args = home.path().join("args");
        for _ in 0..100 {
            if args.exists() {
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        assert_eq!(
            std::fs::read_to_string(args).unwrap(),
            format!("--home {} --after-upgrade\n", home.path().display())
        );
    }

    #[tokio::test]
    async fn connect_or_start_kills_an_engine_that_ignores_the_shutdown_request() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let stuck = start_fake_engine(
            &socket,
            Some(version("0.0.1", 1)),
            Some(Signal::Kill),
            false,
        );
        let binary = fake_binary(home.path());
        let opener = open_new_engine_later(&socket);

        let client = connect_or_start_at(Lang::En, &socket, || Ok(binary), &stuck, LIMITS).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
        assert_eq!(stuck.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(*stuck.signals.lock().unwrap(), vec![Signal::Kill]);
    }

    #[tokio::test]
    async fn connect_or_start_fails_without_starting_when_the_old_engine_survives_the_kill() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let stuck = start_fake_engine(&socket, Some(version("0.0.1", 1)), None, false);

        let error = connect_or_start_at(
            Lang::En,
            &socket,
            || anyhow::bail!("must not locate"),
            &stuck,
            LIMITS,
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("Could not end the old engine"));
        assert_eq!(*stuck.signals.lock().unwrap(), vec![Signal::Kill]);
    }

    #[tokio::test]
    async fn connect_or_start_ends_an_engine_without_version_support_with_a_signal() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let legacy = start_fake_engine(&socket, None, Some(Signal::Term), false);
        let binary = fake_binary(home.path());
        let opener = open_new_engine_later(&socket);

        let client = connect_or_start_at(Lang::En, &socket, || Ok(binary), &legacy, LIMITS).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
        assert_eq!(legacy.shutdowns.load(Ordering::SeqCst), 0);
        assert_eq!(*legacy.signals.lock().unwrap(), vec![Signal::Term]);
    }

    #[test]
    fn judge_replaces_only_an_engine_older_than_this_client() {
        let own = version("0.3.0", 2);

        assert_eq!(judge(&own, &own), Verdict::Current);
        assert_eq!(
            judge(&own, &version("0.2.9", 2)),
            Verdict::Retire(Retire::Request)
        );
        assert_eq!(judge(&own, &version("0.10.0", 2)), Verdict::Current);
        assert_eq!(
            judge(&own, &version("0.3.0", 1)),
            Verdict::Retire(Retire::Request)
        );
        assert_eq!(judge(&own, &version("0.1.0", 3)), Verdict::Current);
        assert_eq!(
            judge(&own, &version("nightly", 2)),
            Verdict::Retire(Retire::Request)
        );
    }

    #[test]
    fn numeric_version_ignores_the_pre_release_suffix() {
        assert_eq!(numeric_version("1.2.3-rc1"), Some(vec![1, 2, 3]));
        assert_eq!(numeric_version("1.x"), None);
    }

    #[tokio::test]
    async fn connect_or_start_without_binary_reports_missing_engine() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);

        let error = connect_or_start_at(
            Lang::En,
            &socket,
            || anyhow::bail!("saturn-engine not found"),
            &NoProcesses,
            LIMITS,
        )
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

        let client = wait_until_ready(Lang::En, &socket, Duration::from_secs(5), &mut engine).await;

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

        let client = wait_until_ready(Lang::En, &socket, Duration::from_secs(5), &mut engine).await;

        assert!(client.is_ok());
        let _listener = opener.await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_ready_after_engine_exit_reports_log_tail() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine =
            exiting_engine(home.path(), "echo 'router host is not allowed' >&2; exit 1");

        let error = wait_until_ready(Lang::En, &socket, Duration::from_secs(10), &mut engine)
            .await
            .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("Engine exited"));
        assert!(message.contains("router host is not allowed"));
    }

    #[tokio::test]
    async fn wait_until_ready_times_out_while_engine_keeps_running() {
        let home = tempfile::tempdir().unwrap();
        let socket = home.path().join(SOCKET_FILE);
        let mut engine = exiting_engine(home.path(), "sleep 5");

        let error = wait_until_ready(Lang::En, &socket, Duration::from_millis(200), &mut engine)
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

        let mut engine = spawn_engine(Lang::En, &script, &socket, false).unwrap();
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

        let tail = log_tail(Lang::En, home.path(), None);

        assert!(tail.contains("today"));
        assert!(!tail.contains("yesterday"));
        assert!(!tail.contains("other"));
    }

    #[test]
    fn engine_log_tail_without_a_log_file_is_empty() {
        let home = tempfile::tempdir().unwrap();

        assert_eq!(log_tail(Lang::En, home.path(), None), "");
    }

    #[test]
    fn engine_log_tail_keeps_only_lines_after_the_start_offset_of_the_same_file() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join(LOG_FILE);
        std::fs::write(&log, "old line\nnew line\n").unwrap();
        let before = (log, "old line\n".len() as u64);

        let tail = log_tail(Lang::En, home.path(), Some(&before));

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

        let tail = log_tail(Lang::En, home.path(), Some(&before));

        assert!(tail.contains("new line"));
    }
}
