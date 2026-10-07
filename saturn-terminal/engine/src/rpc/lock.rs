//! 사용자당 engine 잠금. 기록 저장소에 쓰는 프로세스를 하나로 두려고 `Store::open`보다 먼저 잡는다.
//! 설계: docs/design/engine-lifecycle.md

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::RpcError;

/// 초안. 소켓과 같은 `~/.saturn/` 아래.
pub(crate) const LOCK_FILE: &str = "engine.lock";

/// `flock` 잠금이라 버리거나 프로세스가 죽으면 풀린다.
#[derive(Debug)]
pub(crate) struct EngineLock {
    // 잠금은 열린 파일에 묶여 있어 살아 있는 동안 들고 있어야 한다.
    #[allow(dead_code)]
    file: File,
}

impl EngineLock {
    /// 기다리지 않고 잠근다.
    ///
    /// # Errors
    /// 다른 engine이 잡고 있으면 `AlreadyRunning`, 파일을 만들거나 잠그지 못하면 `Lock`.
    pub(crate) fn acquire(home: &Path) -> Result<Self, RpcError> {
        let path = home.join(LOCK_FILE);
        let lock_error = |source| RpcError::Lock {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(home).map_err(lock_error)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(lock_error)?;
        // SAFETY: `file`이 살아 있는 동안 유효한 fd에 `flock`만 부른다.
        #[expect(unsafe_code, reason = "libc flock 호출")]
        let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                return Err(RpcError::AlreadyRunning { lock: path });
            }
            return Err(lock_error(error));
        }
        Ok(Self { file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_second_lock_returns_already_running() {
        let home = tempfile::tempdir().unwrap();
        let _first = EngineLock::acquire(home.path()).unwrap();

        let second = EngineLock::acquire(home.path());

        assert!(matches!(second, Err(RpcError::AlreadyRunning { .. })));
    }

    #[test]
    fn acquire_after_drop_succeeds() {
        let home = tempfile::tempdir().unwrap();
        drop(EngineLock::acquire(home.path()).unwrap());

        let again = EngineLock::acquire(home.path());

        assert!(again.is_ok());
    }

    #[test]
    fn acquire_creates_missing_home() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("saturn");

        let _lock = EngineLock::acquire(&home).unwrap();

        assert!(home.join(LOCK_FILE).is_file());
    }
}
