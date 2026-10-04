//! 실행 경계의 폴더 상태 차이로 수정 파일 목록을 센다. 시작 때 찍고, 트리 유휴와 멈춤 확인에서 비교한다.
//! 설계: docs/design/providers-and-sessions.md#수정-파일-목록

use std::path::{Path, PathBuf};

use saturn_core::sessions::changes::{ChangeSet, Edit, Snapshot, attribute, diff};
use saturn_protocol::event::{ProviderEvent, ToolCategory};
use saturn_protocol::ids::{AgentId, ChatId, RunId};

use crate::Engine;
use crate::workspace::{self, Limits};

/// 실행 시작 때 폴더 상태와 그때 훑은 폴더들.
#[derive(Debug)]
pub(crate) struct Baseline {
    folders: Vec<PathBuf>,
    snapshot: Snapshot,
}

impl Engine {
    /// 실행이 시작될 때 작업 폴더와 더한 폴더의 상태를 찍어 둔다. 찍지 못해도 실행은 막지 않는다.
    pub(crate) async fn take_baseline(&mut self, run: RunId, chat: ChatId, workdir: &Path) {
        let folders = self.change_folders(chat, workdir);
        let limits = self.flow.change_limits;
        let scanned = scan(folders.clone(), limits).await;
        match scanned {
            Some(snapshot) => {
                self.flow
                    .baselines
                    .insert(run, Baseline { folders, snapshot });
            }
            None => tracing::warn!(run = run.0, "failed to take the folder state at run start"),
        }
    }

    /// 실행이 끝나는 경계에서 지금 상태와 시작 상태를 비교해 기록 저장소에 남기고 돌려준다.
    /// 시작 상태가 없으면(engine이 죽었다 되살린 실행) 목록을 만들지 못해 `None`이다.
    /// 기록하지 못해도 흐름은 막지 않는다.
    pub(crate) async fn settle_changes(
        &mut self,
        run: RunId,
        chat: ChatId,
        main: AgentId,
    ) -> Option<ChangeSet> {
        let baseline = self.flow.baselines.remove(&run)?;
        let limits = self.flow.change_limits;
        let folders = baseline.folders.clone();
        let compared = tokio::task::spawn_blocking(move || {
            let after = workspace::capture(&baseline.folders, &limits);
            let committed =
                workspace::committed_since(&baseline.snapshot, &baseline.folders, &limits);
            diff(&baseline.snapshot, &after, &committed)
        })
        .await;
        let mut changes = match compared {
            Ok(changes) => changes,
            Err(error) => {
                tracing::warn!(run = run.0, %error, "failed to compare the folder state");
                return None;
            }
        };
        let events = self.store.run_events(run).await.unwrap_or_default();
        attribute(&mut changes, &edits_of(&events, main, &folders));
        let written = self.store.record_run_changes(run, chat, &changes).await;
        self.warn_failure("failed to record the changed files", written);
        Some(changes)
    }

    /// 작업 폴더를 링크를 푼 경로로 쓰고, 더한 폴더가 이미 들어 있으면 겹쳐 훑지 않는다.
    fn change_folders(&self, chat: ChatId, workdir: &Path) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = Vec::new();
        for dir in self.write_scope_of(chat, workdir) {
            if !folders.iter().any(|kept| dir.starts_with(kept)) {
                folders.push(dir);
            }
        }
        folders
    }
}

async fn scan(folders: Vec<PathBuf>, limits: Limits) -> Option<Snapshot> {
    tokio::task::spawn_blocking(move || workspace::capture(&folders, &limits))
        .await
        .ok()
}

/// provider 이벤트가 알린 파일 수정. 하위 에이전트가 한 수정은 그 하위 에이전트로 적는다.
/// 상대 경로는 훑은 폴더들 중 파일이 실제로 있는 곳 기준으로 풀지 못하므로 첫 폴더 기준으로 푼다.
fn edits_of(events: &[ProviderEvent], main: AgentId, folders: &[PathBuf]) -> Vec<Edit> {
    let base = folders.first();
    events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::ToolCall {
                agent,
                subagent,
                detail,
                ..
            } if detail.category == ToolCategory::FileEdit => {
                let actor = match subagent {
                    Some(subagent) => format!("subagent {}", subagent.0),
                    None if *agent == main => "main agent".to_owned(),
                    None => format!("agent {}", agent.0),
                };
                Some(detail.paths.iter().map(move |path| (path, actor.clone())))
            }
            _ => None,
        })
        .flatten()
        .map(|(path, actor)| {
            let path = Path::new(path);
            let absolute = match base {
                Some(base) if path.is_relative() => base.join(path),
                _ => path.to_path_buf(),
            };
            Edit {
                path: absolute.to_string_lossy().into_owned(),
                actor,
            }
        })
        .collect()
}
