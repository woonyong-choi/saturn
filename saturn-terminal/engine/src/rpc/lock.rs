//! 사용자당 engine 잠금.
//!
//! 설계: docs/design/engine-lifecycle.md(사용자당 engine 하나), docs/architecture.md(불변 조건).
//! 기록 저장소에 쓰는 프로세스를 하나로 두기 위해 `Store::open`보다 먼저 잡는다. 프로세스가 죽으면 OS가 잠금을 푼다.
//! TODO(#89): 잠금 방식(`std::fs::File::try_lock` 권고 잠금)과 잠금 파일 이름 미정. 소켓과 같은 `~/.saturn/` 아래에 둔다

use std::fs::File;
use std::path::{Path, PathBuf};

use super::RpcError;

/// 잡은 잠금. 버리면 풀린다.
#[derive(Debug)]
pub struct EngineLock {
    /// 잠금 파일 경로.
    path: PathBuf,
    /// 잠근 파일. 살아 있는 동안 잠금이 유지된다.
    file: File,
}

impl EngineLock {
    /// `home` 아래 잠금 파일을 만들고 기다리지 않고 잠근다.
    ///
    /// # Errors
    /// 다른 engine이 잡고 있으면 `AlreadyRunning`, 파일을 만들거나 잠그지 못하면 `Lock`.
    pub fn acquire(home: &Path) -> Result<Self, RpcError> {
        todo!("#89")
    }

    /// 잠금 파일 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
