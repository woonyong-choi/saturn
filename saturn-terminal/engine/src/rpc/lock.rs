//! 사용자당 engine 잠금. 기록 저장소에 쓰는 프로세스를 하나로 두려고 `Store::open`보다 먼저 잡는다.
//! 설계: docs/design/engine-lifecycle.md
//! TODO(#89): 잠금 방식과 잠금 파일 이름 미정, 소켓과 같은 `~/.saturn/` 아래에 둔다

use std::fs::File;
use std::path::{Path, PathBuf};

use super::RpcError;

/// 버리거나 프로세스가 죽으면 풀린다.
#[derive(Debug)]
pub struct EngineLock {
    path: PathBuf,
    file: File,
}

impl EngineLock {
    /// 기다리지 않고 잠근다.
    ///
    /// # Errors
    /// 다른 engine이 잡고 있으면 `AlreadyRunning`, 파일을 만들거나 잠그지 못하면 `Lock`.
    pub fn acquire(home: &Path) -> Result<Self, RpcError> {
        todo!("#89")
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
