//! 허가 요청 판정: 모드, 개별 규칙, 항상 허용을 모아 `core` 판정에 넘기고 항상 허용을 저장한다.
//! 설계: docs/design/permissions.md#판정-흐름

use std::path::{Path, PathBuf};

use saturn_core::permission::{Mode, PermissionCall, PermissionTool, Policy, Rule, Verdict};
use saturn_core::queue::Permission;
use saturn_protocol::ids::{AgentId, ChatId, Provider, ProviderSessionId, SettingsRevision};
use saturn_protocol::rpc::ChatNotice;

use crate::providers::ProviderHandle;
use crate::secrets::{HookPolicy, HookVerdict, ToolCall};
use crate::settings;
use crate::{Engine, EngineError};

impl Engine {
    /// 접수하는 입력의 권한. 읽기 전용 모드이고 쓰기를 열 수 있는 규칙(읽기 규칙을 뺀 `allow`, `ask`)이 하나도 없을 때만 읽기 전용이라,
    /// 같은 폴더의 다른 읽기 작업과 병렬로 실행한다. `ask`는 사용자가 승인하면 쓰기가 되므로 `allow`처럼 쓰기로 둔다.
    /// 규칙을 읽지 못하면 쓰기로 둔다.
    pub(crate) async fn input_permission(
        &self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Permission {
        let read = async {
            let configured = self.settings.at(&self.store, revision).await?.permission();
            let mode = self.chat_mode(chat, revision).await?;
            let can_open_writes = configured
                .rules
                .iter()
                .any(|rule| rule.tool != PermissionTool::Read && rule.verdict != Verdict::Deny);
            Ok::<bool, EngineError>(mode == Mode::ReadOnly && !can_open_writes)
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

    /// 채팅 층 모드를 먼저 읽고 없으면 설정 모드다. 하위 채팅은 부모 채팅의 모드를 넘을 수 없으므로, 부모 쪽으로 올라가며
    /// 가장 낮은 모드를 쓴다. 부모가 모드를 낮추거나 설정이 바뀌면 하위 채팅도 다음 판정부터 따라간다.
    ///
    /// # Errors
    /// 설정이나 기록 저장소를 읽지 못하면 그 오류.
    pub(crate) async fn chat_mode(
        &self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<Mode, EngineError> {
        let configured = self
            .settings
            .at(&self.store, revision)
            .await?
            .permission()
            .mode;
        let mut lowest: Option<Mode> = None;
        let mut current = Some(chat);
        while let Some(id) = current {
            let layer = self.store.chat_layer(id).await?;
            let own = settings::chat_layer_mode(layer.as_deref()).unwrap_or(configured);
            lowest = Some(lowest.map_or(own, |lowest| lowest.min(own)));
            current = self.passes.parent_of(id);
        }
        Ok(lowest.unwrap_or(configured))
    }

    /// 채팅 층 모드를 먼저 읽고 없으면 설정 모드를 읽어 `full`인지 본다. 제약을 묻지 않고 지키는 쪽으로 등록할지 정하는 데 쓴다.
    /// 읽지 못하면 묻는 쪽(거짓)으로 둔다.
    pub(crate) async fn is_full_mode(&self, chat: ChatId, revision: SettingsRevision) -> bool {
        let read = async {
            let mode = self.chat_mode(chat, revision).await?;
            Ok::<bool, EngineError>(mode == Mode::Full)
        };
        match read.await {
            Ok(is_full) => is_full,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "permission mode not read, taking it as not full");
                false
            }
        }
    }

    /// 설정이 바뀌었는지 보고 바뀐 설정을 연결에 적용한다. `/permissions`로 모드를 바꿀 때와 입력을 접수하며 설정을
    /// 다시 읽을 때 부른다. 다시 시작이 필요한 연결(규칙이 연결을 시작할 때 고정되는 어댑터는 규칙 지문이나 질문
    /// 설정이, 그 밖의 어댑터는 질문 설정이 연결을 시작할 때와 다를 때)은 채팅에 실행 중인 작업이 없으면 바로 다시 시작하고, 있으면 턴 끝으로
    /// 미루며 미룬 것을 처음 알아챌 때 한 번 알린다. 입력 접수에서 부르면 그 입력은 다시 시작한 뒤의 새 연결로 나간다.
    pub(crate) async fn sync_provider_settings(
        &mut self,
        chat: ChatId,
        revision: SettingsRevision,
    ) {
        self.sync_provider_events(chat, revision).await;
        let (rules, questions) = match self.connection_settings(chat, revision).await {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "settings not read, keeping the provider connections");
                return;
            }
        };
        let differs = self.stale_providers(chat, &rules, questions);
        let deferred = self.mark_stale_connections(chat, differs);
        if self.chat_is_running(chat) {
            if deferred {
                self.notify_chat(chat, ChatNotice::PermissionsChanged).await;
            }
        } else {
            self.restart_stale_connections(chat).await;
        }
    }

    /// `debug.provider_events` 값을 채팅의 켜짐 값에 맞춘다. 연결을 다시 시작하지 않고 이미 열린 연결도 바로 따른다.
    /// 설정을 읽지 못하면 지금 값을 그대로 둔다.
    async fn sync_provider_events(&self, chat: ChatId, revision: SettingsRevision) {
        match self.settings.at(&self.store, revision).await {
            Ok(settings) => self
                .trace
                .set_enabled(chat, crate::launch::provider_events_on(&settings)),
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "settings not read, keeping the provider event trace as it is");
            }
        }
    }

    /// provider마다 연결을 시작할 때의 설정이 지금 설정과 다른지. 규칙 지문은 어댑터가 정한다.
    fn stale_providers(
        &self,
        chat: ChatId,
        rules: &[Rule],
        questions: bool,
    ) -> Vec<(Provider, bool)> {
        let flow = &self.flow;
        self.registry
            .ids()
            .into_iter()
            .map(|provider| {
                let rules_differ = self
                    .registry
                    .get(provider)
                    .and_then(|adapter| adapter.rules_fingerprint(rules))
                    .zip(flow.rules_of_connection.get(&(chat, provider)))
                    .is_some_and(|(current, started)| *started != current);
                let questions_differ = flow
                    .questions_of_connection
                    .get(&(chat, provider))
                    .is_some_and(|started| *started != questions);
                (provider, rules_differ || questions_differ)
            })
            .collect()
    }

    /// 다시 시작할 연결 표시를 맞춘다. 이번에 처음 표시한 연결이 있으면 참.
    fn mark_stale_connections(&mut self, chat: ChatId, differs: Vec<(Provider, bool)>) -> bool {
        let mut deferred = false;
        for (provider, is_stale) in differs {
            if is_stale {
                deferred |= self.flow.stale_connections.insert((chat, provider));
            } else {
                self.flow.stale_connections.remove(&(chat, provider));
            }
        }
        deferred
    }

    /// 채팅에 실행 중인 작업이 없을 때, 바뀐 설정을 적용하려고 표시한 연결을 모두 다시 시작한다. 작업 중이면 아무것도
    /// 하지 않고 턴 끝에서 다시 부른다. 열려 있던 session은 기록에 남아, 다음 입력이 새 설정으로 연결을 만들고 보관한
    /// provider session id로 이어 연다.
    pub(crate) async fn restart_stale_connections(&mut self, chat: ChatId) {
        if self.chat_is_running(chat) {
            return;
        }
        for provider in self.registry.ids() {
            if self.flow.stale_connections.remove(&(chat, provider)) {
                self.restart_connection(chat, provider).await;
            }
        }
    }

    /// 연결을 닫고 그 채팅에서 열려 있던 session을 흐름에서 뺀다. 기록의 session은 그대로 둬 다음 입력이 이어 연다.
    async fn restart_connection(&mut self, chat: ChatId, provider: Provider) {
        self.flow.stale_connections.remove(&(chat, provider));
        self.flow.questions_of_connection.remove(&(chat, provider));
        self.flow.rules_of_connection.remove(&(chat, provider));
        self.flow.extensions_of_connection.remove(&(chat, provider));
        let Some(connection) = self.providers.remove(&(chat, provider)) else {
            return;
        };
        let closed = self.open_sessions(chat, provider);
        self.close_connection(&connection, &closed).await;
        drop(connection);
        for (agent, _) in closed {
            self.flow.live.remove(&agent);
        }
        self.notify_chat(chat, ChatNotice::ProviderRestarted { provider })
            .await;
    }

    /// 그 채팅의 provider 연결에서 열려 있는 에이전트와 provider session.
    fn open_sessions(&self, chat: ChatId, provider: Provider) -> Vec<(AgentId, ProviderSessionId)> {
        self.flow
            .live
            .iter()
            .filter(|(_, live)| live.provider == provider)
            .filter(|(_, live)| {
                self.session_chat(live.session)
                    .is_ok_and(|owner| owner == chat)
            })
            .map(|(agent, live)| (*agent, live.provider_session.clone()))
            .collect()
    }

    /// 모든 session이 프로세스 묶음 하나를 같이 쓰는 연결은 그 묶음을 멈추고, 아니면 session마다 닫는다.
    async fn close_connection(
        &self,
        connection: &ProviderHandle,
        sessions: &[(AgentId, ProviderSessionId)],
    ) {
        if connection.shared_group().is_some() {
            self.stop_shared_connection(connection).await;
            return;
        }
        for (_, session) in sessions {
            connection.close_session_detached(session.clone());
        }
    }

    async fn stop_shared_connection(&self, connection: &ProviderHandle) {
        let Some(group) = connection.shared_group() else {
            return;
        };
        if let Err(error) = self
            .supervisor
            .stop_tree(group, crate::processes::StopScope::Whole)
            .await
        {
            tracing::warn!(error = %self.failure_line(&error), "failed to stop the shared connection for restart");
        }
        self.supervisor.release(group);
    }

    /// 에이전트 질문 기능을 켤지. 권한 모드가 `full`이면 끈다. 모드는 채팅 층에 쓴 값이 먼저다.
    ///
    /// # Errors
    /// 설정이나 기록 저장소를 읽지 못하면 그 오류.
    pub(crate) async fn agent_questions(
        &self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<bool, EngineError> {
        if self.children.contains_key(&chat) {
            return Ok(false); // 하위 채팅에는 답할 사용자가 없다
        }
        Ok(self.chat_mode(chat, revision).await? != Mode::Full)
    }

    /// 그 번호의 규칙 지문과 에이전트 질문 기능. 연결을 시작할 때 읽은 값과 비교한다.
    async fn connection_settings(
        &self,
        chat: ChatId,
        revision: SettingsRevision,
    ) -> Result<(Vec<Rule>, bool), EngineError> {
        let rules = self
            .settings
            .at(&self.store, revision)
            .await?
            .permission()
            .rules;
        Ok((rules, self.agent_questions(chat, revision).await?))
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
        let revision = self.revision_of_agent(chat, agent)?;
        let configured = self.settings.at(&self.store, revision).await?.permission();
        Ok(Policy {
            mode: self.chat_mode(chat, revision).await?,
            always: self.store.permission_allows(&key).await?,
            rules: configured.rules,
            extra_dirs: self
                .flow
                .session_dirs
                .get(&agent)
                .cloned()
                .unwrap_or_else(|| self.chat_dirs_of(chat))
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

    /// 규칙을 읽지 못하면 묻는다. 호출을 규칙으로 읽지 못한 요청(`call`이 없음)도 묻는다. 읽기 전용으로 접수한 실행은
    /// 쓰기 잠금 없이 도는 중이라 모드를 올려도 읽기 전용으로 판정하고, 그 때문에 판정이 달라지면 사용자에게 알린다.
    pub(crate) async fn router_permission(
        &self,
        chat: ChatId,
        agent: AgentId,
        call: Option<&PermissionCall>,
    ) -> Verdict {
        let Some(call) = call else {
            return Verdict::Ask;
        };
        let mut policy = match self.permission_policy(chat, agent).await {
            Ok(policy) => policy,
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "permission rules not read, asking the user");
                return Verdict::Ask;
            }
        };
        let call = resolved(&policy.workdir, call);
        // router 키 보호는 모드, 규칙, 항상 허용보다 먼저 보고 그 어느 것으로도 풀 수 없다
        if self.key_store_blocked(chat, &call) {
            return Verdict::Deny;
        }
        let verdict = ask_outside_sandbox(&call, policy.decide(&call));
        if verdict == Verdict::Deny
            || self.queue.running_permission(agent) != Some(Permission::ReadOnly)
        {
            return verdict;
        }
        policy.mode = Mode::ReadOnly;
        policy.always.clear();
        let kept = ask_outside_sandbox(&call, policy.decide(&call));
        if kept != verdict {
            self.notify_chat(chat, ChatNotice::ReadOnlyRunKept).await;
        }
        kept
    }

    /// 호출이 router 키 저장소(키체인 폴더, Saturn 키 파일)를 건드리는지. Claude 훅과 같은 판정이고, 샌드박스 밖에서 도는
    /// Codex 명령처럼 provider 샌드박스가 막아 주지 못하는 경우의 방어다. 명령을 해석하지 못하면 막는다.
    fn key_store_blocked(&self, chat: ChatId, call: &PermissionCall) -> bool {
        let user_home = self
            .chat_env(chat)
            .and_then(|env| {
                env.provider_env()
                    .into_iter()
                    .find(|(name, _)| name == "HOME")
                    .map(|(_, home)| PathBuf::from(home))
            })
            .unwrap_or_else(|| PathBuf::from("/"));
        let policy = HookPolicy::new(&self.options.home, &user_home);
        let blocked =
            |tool_call: &ToolCall| matches!(policy.check(tool_call), HookVerdict::Deny { .. });
        match call.tool {
            PermissionTool::Shell => blocked(&ToolCall::Command(call.target.clone())),
            PermissionTool::Edit | PermissionTool::Read => call
                .paths
                .iter()
                .any(|path| blocked(&ToolCall::Path(PathBuf::from(path)))),
            PermissionTool::Mcp | PermissionTool::Subagent => false,
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

    /// 다음 허가 요청부터 새 모드로 판정한다. 에이전트 질문 기능은 모드에 따라 연결을 다시 시작해 적용한다
    /// ([`Self::sync_provider_settings`]).
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
        if let Err(parent) = self.passes.check_mode(chat, mode) {
            return Err(EngineError::ChildRejected {
                reason: format!("requested mode is above the parent mode {}", parent.name()),
            });
        }
        let layer = self.store.chat_layer(chat).await?;
        let updated = settings::with_chat_layer_mode(layer.as_deref(), mode);
        self.store.set_chat_layer(chat, &updated).await?;
        self.passes.set_mode(chat, mode);
        if let Some(revision) = self.settings.latest_of(chat) {
            self.sync_provider_settings(chat, revision).await;
        }
        Ok(())
    }
}

/// provider 샌드박스 밖 실행 요청은 허용으로 판정돼도 묻는다. 샌드박스가 키 저장소 접근을 막지 못해 `full`의 자동 허용,
/// 모드 기본 규칙, 개별 `allow` 규칙, 항상 허용 어느 것도 이 요청을 대신 허용하지 못한다. 거부와 묻기는 그대로다.
fn ask_outside_sandbox(call: &PermissionCall, verdict: Verdict) -> Verdict {
    if call.outside_sandbox && verdict == Verdict::Allow {
        Verdict::Ask
    } else {
        verdict
    }
}

// cost: time O(p·d), heap O(p·d), stack O(1), alloc p, io p·d
// vars: p = 경로 수, d = 경로 깊이
// basis: estimate
/// 편집, 읽기, 읽기만 하는 셸 명령의 경로를 작업 폴더 기준 절대 경로로 바꾸고 링크를 풀어, 폴더 밖을 가리키는 링크가 안으로 보이지 않게 한다.
fn resolved(workdir: &Path, call: &PermissionCall) -> PermissionCall {
    if !matches!(
        call.tool,
        PermissionTool::Edit | PermissionTool::Read | PermissionTool::Shell
    ) {
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
