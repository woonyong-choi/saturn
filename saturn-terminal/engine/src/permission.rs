//! 허가 요청 판정: 모드, 개별 규칙, 항상 허용을 모아 `core` 판정에 넘기고 항상 허용을 저장한다.
//! 설계: docs/design/permissions.md#판정-흐름

use std::path::{Path, PathBuf};

use saturn_core::permission::{Mode, PermissionCall, PermissionTool, Policy, Verdict};
use saturn_core::queue::Permission;
use saturn_protocol::ids::{AgentId, ChatId, Provider, SettingsRevision};
use saturn_protocol::rpc::ChatNotice;

use crate::providers::rules_fingerprint;
use crate::settings::{self, SettingsError};
use crate::{Engine, EngineError};

impl Engine {
    /// 접수하는 입력의 권한. 읽기 전용 모드이고 쓰기를 허용하는 규칙이 하나도 없을 때만 읽기 전용이라, 같은 폴더의
    /// 다른 읽기 작업과 병렬로 실행한다. 규칙을 읽지 못하면 쓰기로 둔다.
    pub(crate) async fn input_permission(
        &self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Permission {
        let read = async {
            let configured = self.settings.at(&self.store, revision).await?.permission();
            let layer = self.store.chat_layer(chat).await?;
            let mode = settings::chat_layer_mode(layer.as_deref()).unwrap_or(configured.mode);
            let allows = configured
                .rules
                .iter()
                .any(|rule| rule.verdict == Verdict::Allow);
            Ok::<bool, EngineError>(mode == Mode::ReadOnly && !allows)
        };
        match read.await {
            Ok(true) => Permission::ReadOnly,
            Ok(false) => Permission::Write,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "input permission not read, taking it as write");
                Permission::Write
            }
        }
    }

    /// 입력을 접수할 때 그 번호의 Codex 규칙이 연결을 시작할 때 읽은 규칙과 다른지 본다. 다르면 다음 턴이 끝난 뒤
    /// 연결을 다시 시작하도록 표시하며 처음 표시할 때 한 번 알리고, 다시 같아졌으면 표시를 지운다.
    pub(crate) async fn note_rules_revision(&mut self, chat: ChatId, revision: SettingsRevision) {
        let Some(started) = self.flow.rules_of_connection.get(&chat) else {
            return;
        };
        let current = match self.settings.at(&self.store, revision).await {
            Ok(settings) => rules_fingerprint(&settings.permission().rules),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "rules not read, keeping the codex connection");
                return;
            }
        };
        if *started == current {
            self.flow.rules_stale.remove(&chat);
        } else if self.flow.rules_stale.insert(chat) {
            self.notify_chat(chat, ChatNotice::PermissionsChanged).await;
        }
    }

    /// 턴 끝에서 부른다. 규칙이 바뀐 Codex 연결은 채팅에 실행 중인 작업이 없을 때 통째로 닫는다. 열려 있던 session은
    /// 기록에 그대로 남아, 다음 입력이 새 번호의 규칙으로 연결을 만들고 보관한 provider session id로 이어 연다.
    pub(crate) async fn restart_stale_codex(&mut self, chat: ChatId) {
        if !self.flow.rules_stale.contains(&chat) || self.chat_is_running(chat) {
            return;
        }
        self.flow.rules_stale.remove(&chat);
        self.flow.rules_of_connection.remove(&chat);
        let Some(connection) = self.providers.remove(&(chat, Provider::Codex)) else {
            return;
        };
        if let Some(group) = connection.shared_group() {
            if let Err(error) = self
                .supervisor
                .stop_tree(group, crate::processes::StopScope::Whole)
                .await
            {
                tracing::warn!(error = %self.failure_line(&error), "failed to stop the codex connection for restart");
            }
            self.supervisor.release(group);
        }
        drop(connection);
        let closed: Vec<_> = self
            .flow
            .live
            .iter()
            .filter(|(_, live)| live.provider == Provider::Codex)
            .filter(|(_, live)| {
                self.session_chat(live.session)
                    .is_ok_and(|owner| owner == chat)
            })
            .map(|(agent, _)| *agent)
            .collect();
        for agent in closed {
            self.flow.live.remove(&agent);
        }
        self.notify_chat(
            chat,
            ChatNotice::ProviderRestarted {
                provider: Provider::Codex,
            },
        )
        .await;
    }

    /// 규칙은 에이전트가 가장 나중에 시작한 입력에 고정한 설정 번호의 값이고, 모드는 채팅 층에 쓴 값이 있으면 그것이
    /// 먼저다. 채팅 층의 모드는 `/permissions`로 실행 중에 바뀌어 다음 허가 요청부터 적용된다.
    ///
    /// # Errors
    /// 붙은 적 없는 채팅이면 `ChatNotAttached`, 설정이나 기록 저장소를 읽지 못하면 그 오류.
    pub(crate) async fn permission_policy(
        &self,
        chat: ChatId,
        agent: AgentId,
    ) -> Result<Policy, EngineError> {
        let key = self.workdir_key(chat)?;
        let revision = self
            .flow
            .settings_of
            .get(&agent)
            .copied()
            .or(self.settings.current())
            .ok_or(SettingsError::NoPreviousRevision)?;
        let configured = self.settings.at(&self.store, revision).await?.permission();
        let layer = self.store.chat_layer(chat).await?;
        Ok(Policy {
            mode: settings::chat_layer_mode(layer.as_deref()).unwrap_or(configured.mode),
            always: self.store.permission_allows(&key).await?,
            rules: configured.rules,
            extra_dirs: self
                .chat_dirs_of(chat)
                .into_iter()
                .map(|dir| dir.canonicalize().unwrap_or(dir))
                .collect(),
            workdir: key.canonicalize().unwrap_or(key),
        })
    }

    /// 항상 허용을 저장하고 찾는 키. 채팅의 작업 폴더를 푼 경로가 아닌 기록한 그대로 쓴다.
    fn workdir_key(&self, chat: ChatId) -> Result<PathBuf, EngineError> {
        let env = self
            .chat_env(chat)
            .ok_or(EngineError::ChatNotAttached { chat })?;
        Ok(env.workdir().to_path_buf())
    }

    /// 규칙을 읽지 못하면 묻는다. 호출을 규칙으로 읽지 못한 요청(`call`이 없음)도 묻는다.
    pub(crate) async fn router_permission(
        &self,
        chat: ChatId,
        agent: AgentId,
        call: Option<&PermissionCall>,
    ) -> Verdict {
        let Some(call) = call else {
            return Verdict::Ask;
        };
        match self.permission_policy(chat, agent).await {
            Ok(policy) => policy.decide(&resolved(&policy.workdir, call)),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "permission rules not read, asking the user");
                Verdict::Ask
            }
        }
    }

    /// `항상 허용` 답이 온 호출의 허용 규칙을 작업 폴더에 저장한다. 저장하지 못해도 이번 허용은 이미 나갔으므로
    /// 로그만 남긴다.
    pub(crate) async fn save_always_allow(
        &self,
        chat: ChatId,
        agent: AgentId,
        call: &PermissionCall,
    ) {
        let saved = async {
            let policy = self.permission_policy(chat, agent).await?;
            let key = self.workdir_key(chat)?;
            for rule in policy.always_rules(&resolved(&policy.workdir, call)) {
                self.store.add_permission_allow(&key, &rule).await?;
            }
            Ok::<(), EngineError>(())
        };
        if let Err(error) = saved.await {
            tracing::warn!(error = %self.failure_line(&error), "always allow not saved");
        }
    }

    /// 다음 허가 요청부터 새 모드로 판정하고 provider는 다시 시작하지 않는다.
    ///
    /// # Errors
    /// 모르는 모드 이름이면 `UnknownPermissionMode`, 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn set_permission_mode(
        &mut self,
        chat: ChatId,
        mode: &str,
    ) -> Result<(), EngineError> {
        let mode = Mode::parse(mode).ok_or_else(|| EngineError::UnknownPermissionMode {
            mode: mode.to_owned(),
        })?;
        let layer = self.store.chat_layer(chat).await?;
        let updated = settings::with_chat_layer_mode(layer.as_deref(), mode);
        self.store.set_chat_layer(chat, &updated).await?;
        Ok(())
    }
}

// cost: time O(p·d), heap O(p·d), stack O(1), alloc p, io p·d
// vars: p = 경로 수, d = 경로 깊이
// basis: estimate
/// 편집 경로를 작업 폴더 기준 절대 경로로 바꾸고 링크를 풀어, 폴더 밖을 가리키는 링크가 안으로 보이지 않게 한다.
fn resolved(workdir: &Path, call: &PermissionCall) -> PermissionCall {
    if call.tool != PermissionTool::Edit {
        return call.clone();
    }
    PermissionCall {
        paths: call
            .paths
            .iter()
            .map(|path| {
                resolve_links(&workdir.join(path))
                    .to_string_lossy()
                    .into_owned()
            })
            .collect(),
        ..call.clone()
    }
}

// cost: time O(d), heap O(d), stack O(1), io d
// vars: d = 경로 깊이
// basis: estimate
/// 있는 앞부분은 실제 경로로 풀고 아직 없는 뒷부분은 그대로 붙인다.
fn resolve_links(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = current.canonicalize() {
            return missing
                .iter()
                .rev()
                .fold(real, |resolved, name| resolved.join(name));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests;
