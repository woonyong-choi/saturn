use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use saturn_core::providers::{ProviderCommand, ProviderError};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

use super::config::{default_args, read_user_config, with_env_overrides};
use super::permission::mcp_check;
use super::stream::{log_stderr, read_loop};
use super::{
    Approvals, COMMAND_DESCRIPTION_PREFIX, COMMAND_METHODS, CodexClient, EVENT_BUFFER,
    MCP_READY_POLL, MCP_READY_TIMEOUT, OPEN_METHODS, OPEN_REPLY_TIMEOUT, Pending, REPLY_TIMEOUT,
    Threads, error_message, lock,
};
use crate::processes::{ProcessSpec, Supervisor};
use crate::providers::{LaunchSpec, UserProviderConfig};

impl CodexClient {
    /// `launch.hook_settings`는 쓰지 않는다. `launch.permission.env`의 `CODEX_HOME`으로 환경을 덮어써
    /// 사용자 설정과 규칙이 끼어들지 못하게 한다.
    ///
    /// # Errors
    /// 실행 실패나 `initialize` 응답 없이 stdout이 닫히면 `ConnectionLost`, 지원하지 않는 버전이면 `NotSent`.
    pub(crate) async fn start(
        launch: LaunchSpec,
        supervisor: Supervisor,
    ) -> Result<Self, ProviderError> {
        let launch = with_env_overrides(launch);
        let found = read_user_config(&launch);
        let user = UserProviderConfig {
            has_auto_compact: launch.user_config.has_auto_compact || found.has_auto_compact,
        };
        let mut args = vec!["app-server".to_owned()];
        args.extend(default_args(user, &launch));
        let spawned = supervisor
            .spawn(ProcessSpec {
                program: launch.program.clone(),
                args,
                workdir: launch.workdir.clone(),
                env: launch.env.clone(),
            })
            .map_err(|error| {
                tracing::warn!(error = %error, "failed to start codex app-server");
                ProviderError::ConnectionLost
            })?;
        let (tx, events) = mpsc::channel(EVENT_BUFFER);
        let pending = Pending::default();
        let threads = Threads::default();
        let approvals = Approvals::default();
        tokio::spawn(read_loop(
            spawned.io.stdout,
            Arc::clone(&pending),
            Arc::clone(&threads),
            Arc::clone(&approvals),
            tx,
            launch.masker.clone(),
        ));
        tokio::spawn(log_stderr(spawned.io.stderr, launch.masker.clone()));
        let mut client = Self {
            supervisor,
            group: spawned.group,
            stdin: spawned.io.stdin,
            next_request_id: 1,
            pending,
            threads,
            approvals,
            events,
            own_events: VecDeque::new(),
            commands: Vec::new(),
            skill_paths: HashMap::new(),
            mcp_servers: launch.permission.mcp_servers.clone(),
            is_mcp_ready: false,
            mcp_ready_timeout: MCP_READY_TIMEOUT,
            reply_timeout: REPLY_TIMEOUT,
            open_reply_timeout: OPEN_REPLY_TIMEOUT,
        };
        if let Err(error) = client.initialize().await {
            client.abort_start().await;
            return Err(error);
        }
        client.load_commands(&launch.workdir).await;
        Ok(client)
    }

    /// 열지 않기로 한 app-server를 남기지 않는다.
    async fn abort_start(&mut self) {
        let stopped = self
            .supervisor
            .stop_tree(self.group, crate::processes::StopScope::Whole)
            .await;
        if let Err(error) = stopped {
            tracing::warn!(%error, "failed to stop rejected codex app-server");
        }
        self.supervisor.release(self.group);
    }

    async fn initialize(&mut self) -> Result<(), ProviderError> {
        let params = json!({
            "clientInfo": {
                "name": "saturn",
                "title": "Saturn",
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        check_initialize_reply(self.request("initialize", params).await)?;
        self.write_line(&json!({ "method": "initialized" }))
            .await
            .map_err(|_| ProviderError::ConnectionLost)
    }

    /// 스킬 목록을 못 받으면 대응표만 둔다.
    async fn load_commands(&mut self, workdir: &Path) {
        self.commands = COMMAND_METHODS
            .iter()
            .map(|(name, method)| ProviderCommand {
                name: (*name).to_owned(),
                description: format!("{COMMAND_DESCRIPTION_PREFIX}{method}"),
                is_skill: false,
            })
            .collect();
        let params = json!({ "cwds": [workdir.to_string_lossy()] });
        let Ok(Ok(result)) = self.request("skills/list", params).await else {
            return;
        };
        let skills = result["data"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|entry| entry["skills"].as_array().into_iter().flatten());
        for skill in skills {
            let (Some(name), Some(path)) = (skill["name"].as_str(), skill["path"].as_str()) else {
                continue;
            };
            if skill["enabled"].as_bool() == Some(false) {
                continue;
            }
            let description = skill["shortDescription"]
                .as_str()
                .or_else(|| skill["description"].as_str())
                .unwrap_or_default();
            self.skill_paths.insert(name.to_owned(), path.to_owned());
            self.commands.push(ProviderCommand {
                name: name.to_owned(),
                description: description.to_owned(),
                is_skill: true,
            });
        }
    }

    /// 쓰기 전에 실패하면 `NotSent`, 쓴 뒤 응답 없이 연결이 끊기거나 요청 종류별 제한 시간(`OPEN_METHODS`이면 `open_reply_timeout`, 아니면 `reply_timeout`) 안에 응답이 오지 않으면 `Unknown`,
    /// JSON-RPC 오류 응답은 `Err(오류 객체)`. 응답이 늦어도 app-server와 턴은 끊지 않고 이 요청만 포기한다.
    /// 포기한 요청의 늦은 응답은 읽기 작업이 찾을 곳이 없어 버린다.
    pub(super) async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<Result<serde_json::Value, serde_json::Value>, ProviderError> {
        let id = self.next_request_id;
        self.next_request_id += 1;
        let (reply, receive) = oneshot::channel();
        lock(&self.pending).insert(id, reply);
        let message = json!({ "id": id, "method": method, "params": params });
        if let Err(error) = self.write_line(&message).await {
            lock(&self.pending).remove(&id);
            return Err(ProviderError::NotSent {
                reason: format!("failed to write request: {}", error.kind()),
            });
        }
        let limit = if OPEN_METHODS.contains(&method) {
            self.open_reply_timeout
        } else {
            self.reply_timeout
        };
        match tokio::time::timeout(limit, receive).await {
            Ok(reply) => reply.map_err(|_| ProviderError::Unknown),
            Err(_) => {
                lock(&self.pending).remove(&id);
                tracing::warn!(method, "codex app-server did not reply in time");
                Err(ProviderError::Unknown)
            }
        }
    }

    pub(super) async fn write_line(&mut self, message: &Value) -> std::io::Result<()> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await
    }

    // cost: time O(t/p·s), heap O(s), stack O(1), io t/p
    // vars: t = 제한 시간, p = 확인 간격, s = 서버 수
    // basis: estimate
    /// 첫 session을 열기 전에 대상 MCP 서버의 시작이 끝날 때까지 `mcpServerStatus/list`로 확인한다.
    /// 시작에 실패한 서버와 제한 시간까지 준비를 알 수 없던 서버는 그 서버의 도구만 쓸 수 없는 것으로 보고
    /// 이유를 한 줄 남긴 채 통과시킨다. 첫 턴은 막지 않는다. 쓸 수 없는 서버의 도구는 모델이 부르면 승인 요청으로 온다.
    ///
    /// # Errors
    /// 연결이 끊기면 `ConnectionLost`.
    pub(super) async fn wait_for_mcp(&mut self) -> Result<(), ProviderError> {
        if self.is_mcp_ready || self.mcp_servers.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + self.mcp_ready_timeout;
        let mut unavailable;
        loop {
            let (waiting, failed) = match self.request("mcpServerStatus/list", json!({})).await? {
                Ok(result) => {
                    let check = mcp_check(&result, &self.mcp_servers);
                    (check.waiting, check.unavailable)
                }
                Err(error) => (vec![error_message(&error)], Vec::new()),
            };
            unavailable = failed;
            if waiting.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                unavailable.extend(waiting);
                break;
            }
            tokio::time::sleep(MCP_READY_POLL).await;
        }
        for reason in &unavailable {
            tracing::warn!(reason = %reason, "mcp server tools are unavailable, sending the first turn anyway");
        }
        self.is_mcp_ready = true;
        Ok(())
    }
}

fn check_initialize_reply(
    reply: Result<Result<Value, Value>, ProviderError>,
) -> Result<(), ProviderError> {
    let result = reply
        .map_err(|_| ProviderError::ConnectionLost)?
        .map_err(|error| {
            tracing::warn!(error = %error, "codex app-server rejected initialize");
            ProviderError::ConnectionLost
        })?;
    // 버전으로 열기를 막지 않는다. 적용된 정책은 thread를 열 때 확인한다
    tracing::info!(
        user_agent = result["userAgent"].as_str().unwrap_or_default(),
        "codex app-server initialized"
    );
    Ok(())
}
