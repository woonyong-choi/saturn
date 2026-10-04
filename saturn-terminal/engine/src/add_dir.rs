//! 채팅 폴더 추가 등록: 더한 폴더를 기록 저장소에 저장하고 provider session을 열 때 넘긴다.
//! 설계: docs/design/engine-lifecycle.md#채팅-폴더와-이어-열기

use std::path::{Path, PathBuf};

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::ChatNotice;

use crate::rpc::ClientId;
use crate::{Engine, EngineError};

impl Engine {
    /// 채팅의 더한 폴더. 기본 폴더는 들어 있지 않고, 붙은 적 없는 채팅은 비어 있다.
    pub(crate) fn chat_dirs_of(&self, chat: ChatId) -> Vec<PathBuf> {
        self.chat_dirs.get(&chat).cloned().unwrap_or_default()
    }

    /// 쓰기 잠금이 겹침을 보는 범위. 작업 폴더와 더한 폴더를 링크를 푼 경로로 만든다. 허가 판정이 경로를 푸는 방식과
    /// 같아, 링크로 우회한 경로도 같은 폴더로 본다. 폴더가 없으면 푸는 대신 적은 그대로 쓴다.
    pub(crate) fn write_scope_of(&self, chat: ChatId, workdir: &Path) -> Vec<PathBuf> {
        if self.is_child_chat(chat) {
            // 하위 채팅은 부모 작업이 쥔 쓰기 잠금 아래에서 돈다. 부모는 트리가 유휴가 될 때까지 잠금을 놓지 않는다
            return Vec::new();
        }
        std::iter::once(workdir.to_path_buf())
            .chain(self.chat_dirs_of(chat))
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .collect()
    }

    /// 폴더를 더한 뒤 아직 보내지 않은 입력의 쓰기 범위를 다시 잡는다. 새 session이 더한 폴더를 받을 수 있어서다.
    /// 이미 보낸 입력의 범위는 그대로다. `AddDir` 요청과 `Attach`의 `--add-dir`이 함께 쓴다.
    pub(crate) fn rescope_unsent_inputs(&mut self, chat: ChatId) {
        let scopes: std::collections::HashMap<_, _> = self
            .queue
            .inputs_in_state(chat, saturn_protocol::state::InputState::Judging)
            .into_iter()
            .chain(
                self.queue
                    .inputs_in_state(chat, saturn_protocol::state::InputState::Queued),
            )
            .chain(
                self.queue
                    .inputs_in_state(chat, saturn_protocol::state::InputState::Held),
            )
            .filter_map(|id| self.queue.input(id))
            .map(|record| (record.id, self.write_scope_of(chat, &record.workdir)))
            .collect();
        self.queue.rescope_unsent(chat, |record| {
            scopes.get(&record.id).cloned().unwrap_or_default()
        });
    }

    /// `Attach`가 받은 `--add-dir`을 채팅에 더한다. 새로 더한 폴더가 있으면 `AddDir`과 같게 보내기 전 입력의 쓰기 범위를 다시 잡는다.
    pub(crate) async fn register_attach_dirs(
        &mut self,
        chat: ChatId,
        dirs: Vec<PathBuf>,
    ) -> Result<(), EngineError> {
        let mut added = false;
        for dir in dirs {
            added |= self.register_dir(chat, dir).await?;
        }
        if added {
            self.rescope_unsent_inputs(chat);
        }
        Ok(())
    }

    /// `AddDir` 요청. 새로 더했으면 그 채팅에 붙은 TUI에 알린다.
    ///
    /// # Errors
    /// 이 클라이언트가 붙지 않은 채팅이면 `ChatNotAttached`, 폴더가 아니면 `InvalidFolder`, 저장 실패면 `Store`.
    pub(crate) async fn add_dir(
        &mut self,
        client: ClientId,
        chat: ChatId,
        path: &str,
    ) -> Result<(), EngineError> {
        self.require_attached(client, chat)?;
        let dir = resolve_folder(path)?;
        if self.register_dir(chat, dir.clone()).await? {
            self.rescope_unsent_inputs(chat);
            let applies_from_next_session = self.sessions.live_main(chat).is_some();
            self.notify_chat(
                chat,
                ChatNotice::FolderAdded {
                    path: dir.display().to_string(),
                    applies_from_next_session,
                },
            )
            .await;
        }
        Ok(())
    }

    /// 이미 더했거나 기본 폴더면 `false`.
    ///
    /// # Errors
    /// 저장 실패나 없는 채팅이면 `Store`.
    pub(crate) async fn register_dir(
        &mut self,
        chat: ChatId,
        dir: PathBuf,
    ) -> Result<bool, EngineError> {
        let base = self.store.chat_workdir(chat).await?;
        let base = base.canonicalize().unwrap_or(base);
        if dir == base || !self.store.add_chat_dir(chat, &dir).await? {
            return Ok(false);
        }
        self.chat_dirs.entry(chat).or_default().push(dir);
        Ok(true)
    }

    /// 기록 저장소의 더한 폴더를 메모리에 올린다. 이어 연 채팅도 이전에 더한 폴더를 되살린다.
    pub(crate) async fn load_chat_dirs(&mut self, chat: ChatId) -> Result<(), EngineError> {
        let dirs = self.store.chat_dirs(chat).await?;
        self.chat_dirs.insert(chat, dirs);
        Ok(())
    }
}

// cost: time O(p·d), heap O(p·d), stack O(1), io p·d
// vars: p = 경로 수, d = 경로 깊이
// basis: estimate
/// 붙을 때 받은 `--add-dir` 경로를 모두 확인한다. 채팅을 만들기 전에 불러 틀린 경로가 빈 채팅을 남기지 않게 한다.
///
/// # Errors
/// 하나라도 폴더가 아니면 `InvalidFolder`.
pub(crate) fn resolve_folders(paths: &[String]) -> Result<Vec<PathBuf>, EngineError> {
    paths.iter().map(|path| resolve_folder(path)).collect()
}

// cost: time O(d), heap O(d), stack O(1), io d
// vars: d = 경로 깊이
// basis: estimate
/// 이미 있는 폴더의 절대 경로를 링크를 푼 값으로 돌려준다. 같은 폴더를 다른 이름으로 두 번 더하지 않기 위해서다.
fn resolve_folder(path: &str) -> Result<PathBuf, EngineError> {
    let invalid = |reason: &'static str| EngineError::InvalidFolder {
        path: path.to_owned(),
        reason,
    };
    if !Path::new(path).is_absolute() {
        return Err(invalid("path should be absolute"));
    }
    let dir = Path::new(path)
        .canonicalize()
        .map_err(|_| invalid("folder does not exist"))?;
    if !dir.is_dir() {
        return Err(invalid("path is not a folder"));
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_folder_follows_links_and_rejects_files_and_relative_paths() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, root.path().join("link")).unwrap();
        let file = root.path().join("file");
        std::fs::write(&file, "x").unwrap();
        let text = |path: &Path| path.display().to_string();

        let linked = resolve_folder(&text(&root.path().join("link"))).unwrap();

        assert_eq!(linked, real.canonicalize().unwrap());
        for bad in [
            text(&file),
            text(&root.path().join("missing")),
            "relative/dir".to_owned(),
        ] {
            assert!(matches!(
                resolve_folder(&bad),
                Err(EngineError::InvalidFolder { .. })
            ));
        }
    }
}
