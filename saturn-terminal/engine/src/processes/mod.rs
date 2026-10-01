//! provider 프로세스 실행, 감시, 단계별 중지. provider 규약은 모르고 프로세스 묶음(process group)만 다룬다.
//! 설계: docs/design/engine-lifecycle.md

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::time::Instant;

use crate::secrets;

/// 멈춤 신호 뒤 묶음에 중지 신호를 보내기 전까지 기다리는 시간.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

/// SIGTERM 뒤 SIGKILL 전까지 기다리는 시간. 초안 값.
pub const KILL_GRACE: Duration = Duration::from_secs(5);

/// 초안 값.
pub const WATCH_INTERVAL: Duration = Duration::from_secs(1);

const STOP_POLL: Duration = Duration::from_millis(100);

/// SIGKILL 뒤 프로세스 표에서 사라지기를 기다리는 시간.
const KILL_SETTLE: Duration = Duration::from_secs(1);

/// 에이전트가 작업 중 실행한 `saturn`은 이 변수가 있으면 거절한다(판정은 `cli`).
/// TODO(#33): 표지 이름과 방식 미정. 자식 Saturn을 부모에 붙이는 방식이 정해지면 바꾼다
pub const NESTED_MARKER_ENV: &str = "SATURN_AGENT";

/// `{n}`은 남은 프로세스 수.
pub const UNCONFIRMED_NOTICE: &str = "멈춤 확인 안 됨 · {n}개 남음";

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// 프로세스가 뜨지 않았으므로 provider 입장에서는 보내기 전 실패다.
    #[error("failed to spawn process: {program}")]
    Spawn {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 등록되지 않았거나 이미 정리한 묶음.
    #[error("process group not found: {0:?}")]
    UnknownGroup(ProcessGroupId),
    /// 이미 끝난 프로세스(ESRCH)는 실패로 보지 않는다.
    #[error("failed to signal process group: {0:?}")]
    Signal(ProcessGroupId, #[source] std::io::Error),
    /// 남은 수를 모르므로 호출자는 완료라고 하지 않는다.
    #[error("failed to inspect process table")]
    Inspect(#[source] std::io::Error),
}

/// 묶음 리더(provider 본 프로세스)의 pid와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessGroupId(pub u32);

#[derive(Clone)]
pub struct ProcessSpec {
    pub program: PathBuf,
    /// Saturn 기본값 인자는 `providers`가 이미 넣어 둔다.
    pub args: Vec<String>,
    pub workdir: PathBuf,
    /// engine이 `secrets::scrub`으로 한 번 거른 부모 환경에 provider별 변수를 더한 것.
    pub env: Vec<(OsString, OsString)>,
}

#[derive(Debug)]
pub struct ChildIo {
    pub stdin: ChildStdin,
    /// 한 줄에 JSON 하나.
    pub stdout: ChildStdout,
    /// `secrets::Masker`로 가린 뒤에만 `tracing`으로 남긴다.
    pub stderr: ChildStderr,
}

#[derive(Debug)]
pub struct Spawned {
    pub group: ProcessGroupId,
    pub io: ChildIo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopScope {
    /// 리더는 남기고 자손만. 멈춤 뒤에도 session을 이어 쓸 때.
    Descendants,
    /// 리더까지 전부.
    Whole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// 범위 안 프로세스와 묶음 밖으로 빠져나간 자손이 모두 끝났다.
    Stopped,
    /// 강제 종료 뒤에도 남았다(대부분 `setsid` 등으로 묶음 밖으로 나간 프로세스).
    Unconfirmed {
        /// 남은 프로세스 수.
        remaining: usize,
    },
}

impl StopOutcome {
    /// `Stopped`면 `None`.
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Stopped => None,
            Self::Unconfirmed { remaining } => {
                Some(UNCONFIRMED_NOTICE.replace("{n}", &remaining.to_string()))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// 신호로 끝났으면 `None`.
    pub code: Option<i32>,
    /// 정상 종료면 `None`.
    pub signal: Option<i32>,
}

impl From<ExitStatus> for ExitInfo {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            signal: status.signal(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    pgid: u32,
}

/// 테스트가 줄여 넣는다.
#[derive(Debug, Clone, Copy)]
struct Grace {
    stop: Duration,
    kill: Duration,
}

#[derive(Debug)]
struct Watched {
    /// 종료 수거(reap)와 종료 코드 확인에 쓴다.
    child: tokio::process::Child,
    /// pgid가 묶음과 달라진 것이 묶음 밖으로 빠져나간 프로세스다.
    seen_descendants: Vec<u32>,
    exited: Option<ExitInfo>,
}

/// engine에 하나, 복제해 provider 연결마다 나눠 준다(내부는 공유).
#[derive(Debug, Clone, Default)]
pub struct Supervisor {
    groups: Arc<Mutex<HashMap<ProcessGroupId, Watched>>>,
}

impl Supervisor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 제외 목록은 여기서 다시 보지 않고 `secrets::scrub`에 맡긴다. tokio 실행기 안에서 불러야 한다.
    ///
    /// # Errors
    /// 실행 파일이 없거나 실행 권한이 없으면 `Spawn`.
    pub fn spawn(&self, spec: ProcessSpec) -> Result<Spawned, ProcessError> {
        let mut command = tokio::process::Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.workdir)
            .env_clear()
            .envs(secrets::scrub(spec.env))
            .env(NESTED_MARKER_ENV, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command.spawn().map_err(|source| ProcessError::Spawn {
            program: spec.program.clone(),
            source,
        })?;
        let pid = child
            .id()
            .expect("freshly spawned child should have a pid before it is reaped");
        let io = ChildIo {
            stdin: child.stdin.take().expect("stdin should be piped"),
            stdout: child.stdout.take().expect("stdout should be piped"),
            stderr: child.stderr.take().expect("stderr should be piped"),
        };
        let group = ProcessGroupId(pid);
        self.lock().insert(
            group,
            Watched {
                child,
                seen_descendants: Vec::new(),
                exited: None,
            },
        );
        tokio::spawn(self.clone().watch(group));
        Ok(Spawned { group, io })
    }

    /// 이미 끝났으면 바로 돌려준다.
    ///
    /// # Errors
    /// 등록되지 않은 묶음이면 `UnknownGroup`.
    pub async fn wait(&self, group: ProcessGroupId) -> Result<ExitInfo, ProcessError> {
        loop {
            if let Some(exit) = self.reap(group)? {
                return Ok(exit);
            }
            tokio::time::sleep(STOP_POLL).await;
        }
    }

    pub fn is_running(&self, group: ProcessGroupId) -> bool {
        matches!(self.reap(group), Ok(None))
    }

    /// 호출자는 provider 멈춤 신호를 먼저 모두 보낸다. 묶음 밖 자손에는 신호를 보내지 않고 수만 보고한다.
    ///
    /// # Errors
    /// 등록되지 않은 묶음이면 `UnknownGroup`, 신호 실패는 `Signal`, 남은 수를 셀 수 없으면 `Inspect`.
    pub async fn stop_tree(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
    ) -> Result<StopOutcome, ProcessError> {
        let grace = Grace {
            stop: STOP_GRACE,
            kill: KILL_GRACE,
        };
        self.stop_tree_with(group, scope, grace).await
    }

    /// engine 종료 때 부른다.
    pub async fn stop_all(&self) -> Vec<(ProcessGroupId, Result<StopOutcome, ProcessError>)> {
        let groups: Vec<ProcessGroupId> = self.lock().keys().copied().collect();
        let mut results = Vec::with_capacity(groups.len());
        for group in groups {
            results.push((group, self.stop_tree(group, StopScope::Whole).await));
        }
        results
    }

    /// 리더가 끝났고 마지막 감시 때 본 자손이 없을 때만 뺀다.
    pub fn release(&self, group: ProcessGroupId) -> bool {
        if !matches!(self.reap(group), Ok(Some(_))) {
            return false;
        }
        let mut groups = self.lock();
        let Some(watched) = groups.get(&group) else {
            return false;
        };
        if watched.seen_descendants.iter().any(|pid| pid_alive(*pid)) {
            return false;
        }
        groups.remove(&group);
        true
    }

    /// 테스트가 대기 시간을 줄여 넣는다.
    async fn stop_tree_with(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
        grace: Grace,
    ) -> Result<StopOutcome, ProcessError> {
        self.reap(group)?;
        if self.wait_until_clear(group, scope, grace.stop).await? {
            return self.outcome(group, scope).await;
        }
        self.signal_scope(group, scope, libc::SIGTERM).await?;
        if self.wait_until_clear(group, scope, grace.kill).await? {
            return self.outcome(group, scope).await;
        }
        self.signal_scope(group, scope, libc::SIGKILL).await?;
        self.wait_until_clear(group, scope, KILL_SETTLE).await?;
        self.outcome(group, scope).await
    }

    /// 끝났으면 참.
    async fn wait_until_clear(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
        limit: Duration,
    ) -> Result<bool, ProcessError> {
        let deadline = Instant::now() + limit;
        loop {
            self.reap(group)?;
            let rows = process_table().await?;
            self.remember_descendants(group, &rows);
            if in_scope(group, scope, &rows).is_empty() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(STOP_POLL).await;
        }
    }

    /// `Descendants`는 리더를 뺀 pid마다 보낸다.
    async fn signal_scope(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
        signal: libc::c_int,
    ) -> Result<(), ProcessError> {
        let failed = |source| ProcessError::Signal(group, source);
        match scope {
            StopScope::Whole => send_signal(-to_pid(group.0), signal).map_err(failed),
            StopScope::Descendants => {
                let rows = process_table().await?;
                for pid in in_scope(group, scope, &rows) {
                    send_signal(to_pid(pid), signal).map_err(failed)?;
                }
                Ok(())
            }
        }
    }

    /// 범위 안 프로세스와 묶음 밖으로 빠져나간 자손 중 살아 있는 수.
    async fn outcome(
        &self,
        group: ProcessGroupId,
        scope: StopScope,
    ) -> Result<StopOutcome, ProcessError> {
        self.reap(group)?;
        let rows = process_table().await?;
        self.remember_descendants(group, &rows);
        let mut remaining: HashSet<u32> = in_scope(group, scope, &rows).into_iter().collect();
        let seen = self.seen(group)?;
        let alive: HashSet<u32> = rows.iter().map(|row| row.pid).collect();
        remaining.extend(
            rows.iter()
                .filter(|row| seen.contains(&row.pid) && row.pgid != group.0)
                .map(|row| row.pid)
                .filter(|pid| alive.contains(pid)),
        );
        if remaining.is_empty() {
            Ok(StopOutcome::Stopped)
        } else {
            Ok(StopOutcome::Unconfirmed {
                remaining: remaining.len(),
            })
        }
    }

    /// 리더가 끝나고 산 자손이 없거나 등록이 빠지면 멈춘다.
    async fn watch(self, group: ProcessGroupId) {
        loop {
            tokio::time::sleep(WATCH_INTERVAL).await;
            let Ok(exit) = self.reap(group) else {
                return;
            };
            let Ok(rows) = process_table().await else {
                continue;
            };
            self.remember_descendants(group, &rows);
            let Ok(seen) = self.seen(group) else {
                return;
            };
            let any_alive = rows.iter().any(|row| seen.contains(&row.pid));
            if exit.is_some() && !any_alive {
                return;
            }
        }
    }

    fn reap(&self, group: ProcessGroupId) -> Result<Option<ExitInfo>, ProcessError> {
        let mut groups = self.lock();
        let watched = groups
            .get_mut(&group)
            .ok_or(ProcessError::UnknownGroup(group))?;
        if watched.exited.is_none()
            && let Ok(Some(status)) = watched.child.try_wait()
        {
            watched.exited = Some(status.into());
        }
        Ok(watched.exited)
    }

    /// 리더에서 부모 줄을 따라 내려간 자손과 묶음 구성원을 더한다.
    fn remember_descendants(&self, group: ProcessGroupId, rows: &[ProcessRow]) {
        let found = descendants(group, rows);
        if let Some(watched) = self.lock().get_mut(&group) {
            for pid in found {
                if !watched.seen_descendants.contains(&pid) {
                    watched.seen_descendants.push(pid);
                }
            }
        }
    }

    fn seen(&self, group: ProcessGroupId) -> Result<HashSet<u32>, ProcessError> {
        self.lock()
            .get(&group)
            .map(|watched| watched.seen_descendants.iter().copied().collect())
            .ok_or(ProcessError::UnknownGroup(group))
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ProcessGroupId, Watched>> {
        // 잠금을 쥔 채 panic하는 코드가 없으니 독이 든 잠금도 내용은 온전하다
        self.groups
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 좀비는 프로세스 표 읽기에서 이미 뺐다.
fn in_scope(group: ProcessGroupId, scope: StopScope, rows: &[ProcessRow]) -> Vec<u32> {
    rows.iter()
        .filter(|row| row.pgid == group.0)
        .filter(|row| scope == StopScope::Whole || row.pid != group.0)
        .map(|row| row.pid)
        .collect()
}

/// 리더 제외.
fn descendants(group: ProcessGroupId, rows: &[ProcessRow]) -> Vec<u32> {
    let mut found: Vec<u32> = rows
        .iter()
        .filter(|row| row.pgid == group.0 && row.pid != group.0)
        .map(|row| row.pid)
        .collect();
    let mut frontier = vec![group.0];
    frontier.extend(found.iter().copied());
    while let Some(parent) = frontier.pop() {
        for row in rows.iter().filter(|row| row.ppid == parent) {
            if row.pid != group.0 && !found.contains(&row.pid) {
                found.push(row.pid);
                frontier.push(row.pid);
            }
        }
    }
    found
}

/// 좀비(`Z`)는 끝난 것으로 보고 뺀다.
async fn process_table() -> Result<Vec<ProcessRow>, ProcessError> {
    let output = tokio::process::Command::new("/bin/ps")
        .args(["-A", "-o", "pid=,ppid=,pgid=,stat="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(ProcessError::Inspect)?;
    if !output.status.success() {
        return Err(ProcessError::Inspect(std::io::Error::other(format!(
            "ps exited with {}",
            output.status
        ))));
    }
    Ok(parse_table(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_table(text: &str) -> Vec<ProcessRow> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid = fields.next()?.parse().ok()?;
            let pgid = fields.next()?.parse().ok()?;
            let stat = fields.next().unwrap_or_default();
            (!stat.starts_with('Z')).then_some(ProcessRow { pid, ppid, pgid })
        })
        .collect()
}

/// 음수 pid는 묶음이다. 이미 없는 대상(ESRCH)은 성공으로 본다.
fn send_signal(target: libc::pid_t, signal: libc::c_int) -> std::io::Result<()> {
    // SAFETY: `kill`은 메모리를 건드리지 않는 시스템 호출이고 인자는 정수다
    let result = unsafe { libc::kill(target, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(error)
}

/// 권한이 없어도 존재하면 참.
fn pid_alive(pid: u32) -> bool {
    // SAFETY: 신호 0은 존재 확인만 하는 `kill` 호출이다
    let result = unsafe { libc::kill(to_pid(pid), 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn to_pid(pid: u32) -> libc::pid_t {
    libc::pid_t::try_from(pid).expect("os pids should fit in pid_t")
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    use super::*;

    const SHORT: Grace = Grace {
        stop: Duration::from_millis(300),
        kill: Duration::from_millis(300),
    };

    fn shell(script: &str, extra_env: &[(&str, &str)]) -> ProcessSpec {
        let mut env: Vec<(OsString, OsString)> = vec![("PATH".into(), "/usr/bin:/bin".into())];
        env.extend(
            extra_env
                .iter()
                .map(|(name, value)| ((*name).into(), (*value).into())),
        );
        ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), script.to_owned()],
            workdir: std::env::temp_dir(),
            env,
        }
    }

    async fn first_line(io: &mut ChildIo) -> String {
        let mut line = String::new();
        BufReader::new(&mut io.stdout)
            .read_line(&mut line)
            .await
            .unwrap();
        line.trim().to_owned()
    }

    #[test]
    fn design_grace_values() {
        assert_eq!(STOP_GRACE, Duration::from_secs(10));
        assert_eq!(StopOutcome::Stopped.notice(), None);
        assert_eq!(
            StopOutcome::Unconfirmed { remaining: 2 }
                .notice()
                .as_deref(),
            Some("멈춤 확인 안 됨 · 2개 남음")
        );
    }

    #[test]
    fn table_parsing_skips_zombies_and_finds_descendants() {
        let rows =
            parse_table("  10   1  10 Ss\n  11  10  10 S\n  12  11  12 S\n  13  10  10 Z\n bad\n");

        assert_eq!(rows.len(), 3);
        let group = ProcessGroupId(10);
        let mut found = descendants(group, &rows);
        found.sort_unstable();
        assert_eq!(found, vec![11, 12]);
        assert_eq!(in_scope(group, StopScope::Whole, &rows), vec![10, 11]);
        assert_eq!(in_scope(group, StopScope::Descendants, &rows), vec![11]);
    }

    #[tokio::test]
    async fn child_env_is_scrubbed_and_marked() {
        let supervisor = Supervisor::new();
        let script = format!(
            "echo \"${{{NESTED_MARKER_ENV}}}:${{{key}:-none}}:${{KEEP}}:${{HOME:-unset}}\"",
            key = secrets::JUDGE_KEY_ENV
        );
        let spec = shell(
            &script,
            &[(secrets::JUDGE_KEY_ENV, "sk-secret"), ("KEEP", "yes")],
        );

        let mut spawned = supervisor.spawn(spec).unwrap();

        assert_eq!(first_line(&mut spawned.io).await, "1:none:yes:unset");
        let exit = supervisor.wait(spawned.group).await.unwrap();
        assert_eq!(
            exit,
            ExitInfo {
                code: Some(0),
                signal: None
            }
        );
    }

    #[tokio::test]
    async fn missing_program_is_spawn_error() {
        let supervisor = Supervisor::new();
        let mut spec = shell("true", &[]);
        spec.program = PathBuf::from("/nonexistent/provider");

        let error = supervisor.spawn(spec).unwrap_err();

        assert!(matches!(error, ProcessError::Spawn { .. }));
        assert!(matches!(
            supervisor.wait(ProcessGroupId(1)).await,
            Err(ProcessError::UnknownGroup(_))
        ));
    }

    #[tokio::test]
    async fn stop_escalates_to_kill_when_term_is_ignored() {
        let supervisor = Supervisor::new();
        let mut spawned = supervisor
            .spawn(shell(
                "trap '' TERM; echo ready; while :; do sleep 1; done",
                &[],
            ))
            .unwrap();
        assert_eq!(first_line(&mut spawned.io).await, "ready");

        let outcome = supervisor
            .stop_tree_with(spawned.group, StopScope::Whole, SHORT)
            .await
            .unwrap();

        assert_eq!(outcome, StopOutcome::Stopped);
        assert!(!supervisor.is_running(spawned.group));
        let exit = supervisor.wait(spawned.group).await.unwrap();
        assert_eq!(exit.signal, Some(libc::SIGKILL));
        let rows = process_table().await.unwrap();
        assert!(rows.iter().all(|row| row.pgid != spawned.group.0));
        assert!(supervisor.release(spawned.group));
    }

    #[tokio::test]
    async fn stop_sends_term_after_grace() {
        let supervisor = Supervisor::new();
        let mut spawned = supervisor
            .spawn(shell("echo ready; sleep 30", &[]))
            .unwrap();
        assert_eq!(first_line(&mut spawned.io).await, "ready");
        let started = Instant::now();

        let outcome = supervisor
            .stop_tree_with(spawned.group, StopScope::Whole, SHORT)
            .await
            .unwrap();

        assert_eq!(outcome, StopOutcome::Stopped);
        assert!(started.elapsed() >= SHORT.stop);
        assert_eq!(
            supervisor.wait(spawned.group).await.unwrap().signal,
            Some(libc::SIGTERM)
        );
    }

    #[tokio::test]
    async fn descendants_scope_keeps_leader() {
        let supervisor = Supervisor::new();
        let mut spawned = supervisor
            .spawn(shell("sleep 30 & echo ready; read line", &[]))
            .unwrap();
        assert_eq!(first_line(&mut spawned.io).await, "ready");

        let outcome = supervisor
            .stop_tree_with(spawned.group, StopScope::Descendants, SHORT)
            .await
            .unwrap();

        assert_eq!(outcome, StopOutcome::Stopped);
        assert!(supervisor.is_running(spawned.group));
        assert!(!supervisor.release(spawned.group));
        drop(spawned.io.stdin);
        let all = supervisor.stop_all().await;
        assert_eq!(all.len(), 1);
        assert!(matches!(all[0].1, Ok(StopOutcome::Stopped)));
    }

    #[tokio::test]
    async fn escaped_process_is_reported_not_signalled() {
        let supervisor = Supervisor::new();
        let script = "/usr/bin/perl -e 'use POSIX qw(setsid); setsid(); sleep 20' & \
                      sleep 0.3; echo ready; read line";
        let mut spawned = supervisor.spawn(shell(script, &[])).unwrap();
        assert_eq!(first_line(&mut spawned.io).await, "ready");

        let outcome = supervisor
            .stop_tree_with(spawned.group, StopScope::Whole, SHORT)
            .await
            .unwrap();

        assert_eq!(outcome, StopOutcome::Unconfirmed { remaining: 1 });
        assert!(!supervisor.release(spawned.group));
        let rows = process_table().await.unwrap();
        let seen = supervisor.seen(spawned.group).unwrap();
        for row in rows.iter().filter(|row| seen.contains(&row.pid)) {
            send_signal(to_pid(row.pid), libc::SIGKILL).unwrap();
        }
        let mut rest = String::new();
        let _ = spawned.io.stdout.read_to_string(&mut rest).await; // 리더가 끝나 빈 출력이다
    }
}
