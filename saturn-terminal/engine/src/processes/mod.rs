//! provider 프로세스 실행, 감시, 단계별 중지. provider 규약은 모르고 프로세스 묶음(process group)만 다룬다.
//!
//! 설계: docs/design/providers-and-sessions.md(트리 전체 중지), docs/design/engine-lifecycle.md(프로세스 배치와 수명, 중첩 saturn 거절),
//! docs/design/judge-key-security.md(자식 환경의 변수 제거).
//!
//! 호출 흐름:
//! 1. `providers`가 `ProcessSpec`을 만들어 `Supervisor::spawn`을 부른다. `env`는 engine이 `secrets::scrub`으로 한 번 거른 환경이다.
//! 2. `spawn`은 `env_clear` 뒤 `secrets::scrub`을 다시 거친 환경만 넣고(두 번째 제거), 중첩 표지를 더해 새 묶음의 리더로 실행한다.
//! 3. 멈춤: 호출자가 provider 멈춤 신호를 모두 보낸 뒤 `stop_tree`를 부른다.
//!    `STOP_GRACE`(10초) 안에 끝나지 않으면 묶음에 중지 신호(SIGTERM), `KILL_GRACE` 뒤에도 남으면 강제 종료(SIGKILL).
//! 4. `stop_tree`는 묶음과 묶음 밖으로 빠져나간 자손이 모두 끝난 것을 확인할 때만 `StopOutcome::Stopped`를 돌려준다.
//!
//! 작업 공간에 `libc`, `nix`가 없다. 묶음 신호(`kill(-pgid, sig)`)와 프로세스 표 조회는 구현 때 방법을 정한다
//! (`/bin/kill -TERM -<pgid>`, `ps -A -o pid=,ppid=,pgid=` 실행, 또는 crate 추가).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::{ChildStderr, ChildStdin, ChildStdout};

/// 멈춤 신호 뒤 묶음에 중지 신호를 보내기 전까지 기다리는 시간.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

/// 중지 신호(SIGTERM) 뒤 강제 종료(SIGKILL) 전까지 기다리는 시간.
/// TODO(#85): 설계 문서에 값이 없다. 정하면 providers-and-sessions.md 트리 전체 중지 절에 적는다
pub const KILL_GRACE: Duration = Duration::from_secs(5);

/// 자식 환경에 넣는 중첩 표지. 에이전트가 작업 중 실행한 `saturn`은 이 변수가 있으면 거절한다(판정은 `cli`).
/// TODO(#33): 표지 이름과 방식(환경 변수만, 소켓 경로 포함 등) 미정. 자식 Saturn을 부모에 붙이는 방식이 정해지면 바꾼다
pub const NESTED_MARKER_ENV: &str = "SATURN_AGENT";

/// 멈춤 결과를 확인하지 못했을 때 화면 문구. `{n}`은 남은 프로세스 수.
pub const UNCONFIRMED_NOTICE: &str = "멈춤 확인 안 됨 · {n}개 남음";

/// 프로세스 실행과 중지 오류.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// 실행 실패. 프로세스가 뜨지 않았으므로 provider 입장에서는 보내기 전 실패다.
    #[error("failed to spawn process: {program}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 등록되지 않았거나 이미 정리한 묶음.
    #[error("process group not found: {0:?}")]
    UnknownGroup(ProcessGroupId),
    /// 신호 전송 실패(권한 없음 등). 이미 끝난 프로세스(ESRCH)는 실패로 보지 않는다.
    #[error("failed to signal process group: {0:?}")]
    Signal(ProcessGroupId, #[source] std::io::Error),
    /// 프로세스 표 조회 실패. 남은 수를 모르므로 호출자는 완료라고 하지 않는다.
    #[error("failed to inspect process table")]
    Inspect(#[source] std::io::Error),
}

/// 프로세스 묶음 id. 묶음 리더(provider 본 프로세스)의 pid와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessGroupId(pub u32);

/// 실행할 프로세스. `providers`가 만든다.
#[derive(Debug, Clone)]
pub struct ProcessSpec {
    /// 실행 파일(`codex`, `claude` 또는 절대 경로).
    pub program: PathBuf,
    /// 인자. Saturn 기본값 인자는 `providers`가 이미 넣어 둔다.
    pub args: Vec<String>,
    /// 작업 폴더.
    pub workdir: PathBuf,
    /// 자식 환경 전체. engine이 부모 환경(`std::env::vars_os`)을 `secrets::scrub`으로 거른 값에 provider별 변수를 더한 것.
    pub env: Vec<(OsString, OsString)>,
}

/// 실행한 프로세스의 표준 입출력. `providers`가 규약대로 읽고 쓴다.
#[derive(Debug)]
pub struct ChildIo {
    /// provider에 요청을 쓰는 쪽.
    pub stdin: ChildStdin,
    /// provider 이벤트를 읽는 쪽. 한 줄에 JSON 하나.
    pub stdout: ChildStdout,
    /// 진단 출력. `secrets::Masker`로 가린 뒤에만 `tracing`으로 남긴다.
    pub stderr: ChildStderr,
}

/// 실행 결과.
#[derive(Debug)]
pub struct Spawned {
    /// 새 묶음 id.
    pub group: ProcessGroupId,
    /// 표준 입출력.
    pub io: ChildIo,
}

/// 중지 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopScope {
    /// 묶음 리더(provider 본 프로세스)는 남기고 자손만. 멈춤 뒤에도 session을 이어 쓸 때(Codex app-server, 유예 중 Claude).
    Descendants,
    /// 리더까지 전부. session 닫기, 연결 끊김, engine 종료 때.
    Whole,
}

/// 중지 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// 범위 안 프로세스와 묶음 밖으로 빠져나간 자손이 모두 끝났다.
    Stopped,
    /// 강제 종료 뒤에도 남았다. 대부분 묶음 밖으로 빠져나간 프로세스(`setsid` 등)다. 완료라고 하지 않는다.
    Unconfirmed {
        /// 남은 프로세스 수.
        remaining: usize,
    },
}

impl StopOutcome {
    /// 화면 문구. `Stopped`면 `None`, 아니면 `멈춤 확인 안 됨 · N개 남음`.
    pub fn notice(&self) -> Option<String> {
        todo!("#85")
    }
}

/// 프로세스가 끝난 모습.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// 종료 코드. 신호로 끝났으면 `None`.
    pub code: Option<i32>,
    /// 끝낸 신호 번호. 정상 종료면 `None`.
    pub signal: Option<i32>,
}

/// 등록된 묶음 하나의 감시 상태.
#[derive(Debug)]
struct Watched {
    /// 리더 프로세스. 종료 수거(reap)와 종료 코드 확인에 쓴다.
    child: tokio::process::Child,
    /// 감시 중 본 자손 pid. pgid가 묶음과 달라진 것이 묶음 밖으로 빠져나간 프로세스다.
    seen_descendants: Vec<u32>,
    /// 리더가 끝났으면 그 모습.
    exited: Option<ExitInfo>,
}

/// provider 프로세스 감시자. engine에 하나, 복제해 provider 연결마다 나눠 준다(내부는 공유).
#[derive(Debug, Clone, Default)]
pub struct Supervisor {
    groups: Arc<Mutex<HashMap<ProcessGroupId, Watched>>>,
}

impl Supervisor {
    /// 빈 감시자.
    pub fn new() -> Self {
        Self::default()
    }

    /// 새 묶음의 리더로 실행하고 감시를 시작한다.
    ///
    /// `env_clear` 뒤 `secrets::scrub(spec.env)` 결과와 `NESTED_MARKER_ENV=1`만 넣는다. 제외 목록(`secrets::CHILD_ENV_DENYLIST`)은 여기서 다시 보지 않고 `scrub`에 맡긴다.
    /// `process_group(0)`으로 새 묶음을 만들고 stdin, stdout, stderr를 파이프로 잇는다.
    /// 등록 뒤 자손 목록 갱신과 리더 종료 수거를 백그라운드 작업으로 돌린다. 갱신 주기는 1초 이하로 둔다.
    ///
    /// # Errors
    /// 실행 파일이 없거나 실행 권한이 없으면 `Spawn`.
    pub fn spawn(&self, spec: ProcessSpec) -> Result<Spawned, ProcessError> {
        todo!("#85")
    }

    /// 묶음 리더가 끝날 때까지 기다린다. 이미 끝났으면 바로 돌려준다.
    ///
    /// # Errors
    /// 등록되지 않은 묶음이면 `UnknownGroup`.
    pub async fn wait(&self, group: ProcessGroupId) -> Result<ExitInfo, ProcessError> {
        todo!("#85")
    }

    /// 리더가 아직 살아 있는지.
    pub fn is_running(&self, group: ProcessGroupId) -> bool {
        todo!("#85")
    }

    /// 단계별 중지. 호출자는 provider 멈춤 신호를 먼저 모두 보낸다.
    ///
    /// 1. 범위 안 프로세스가 모두 끝나기를 `STOP_GRACE`(10초)까지 기다린다.
    /// 2. 남았으면 묶음에 SIGTERM(`Descendants`면 리더 빼고 자손 pid마다).
    /// 3. `KILL_GRACE` 뒤에도 남았으면 SIGKILL.
    /// 4. 범위 안 프로세스와 묶음 밖으로 빠져나간 자손 중 살아 있는 수를 센다. 0이면 `Stopped`, 아니면 `Unconfirmed`.
    ///
    /// 묶음 밖 자손에는 신호를 보내지 않고 수만 보고한다. 트리 유휴(`core::agents`)까지 확인해 멈춤 완료를 알리는 것은 호출자다.
    ///
    /// # Errors
    /// 등록되지 않은 묶음이면 `UnknownGroup`, 신호 실패는 `Signal`, 남은 수를 셀 수 없으면 `Inspect`.
    pub async fn stop_tree(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
    ) -> Result<StopOutcome, ProcessError> {
        todo!("#85")
    }

    /// 등록된 모든 묶음을 `StopScope::Whole`로 멈춘다. engine 종료 때 부른다. 묶음마다 결과를 돌려준다.
    pub async fn stop_all(&self) -> Vec<(ProcessGroupId, Result<StopOutcome, ProcessError>)> {
        todo!("#85")
    }

    /// 끝난 묶음을 등록에서 뺀다. 리더가 끝났고 남은 자손이 없을 때만 뺀다. 뺐으면 참.
    pub fn release(&self, group: ProcessGroupId) -> bool {
        todo!("#85")
    }
}
