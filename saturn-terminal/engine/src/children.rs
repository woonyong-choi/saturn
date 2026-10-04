//! 하위 접속: 출입증으로 붙은 하위 채팅을 부모 채팅의 하위 작업으로 만들고, 끝내고, 부모와 함께 멈춘다.
//! 출입증 확인과 상한 대기는 연결 작업이 `PassGate`에서 끝내므로 여기에는 허용된 요청만 온다.
//! 설계: docs/design/child-sessions.md

use std::ffi::OsString;

use saturn_core::passes::Grant;
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{AgentId, ChatId, Provider, SubagentId};
use saturn_protocol::rpc::{PASS_ENV, SOCKET_ENV};

use crate::calls::Responder;
use crate::flow::LiveSession;
use crate::rpc::{ClientId, SOCKET_FILE};
use crate::settings::{self, SettingsError};
use crate::{AttachRequest, Engine, EngineError};

/// 하위 접속으로 만든 채팅과 부모 작업의 연결.
#[derive(Debug, Clone)]
pub(crate) struct ChildLink {
    /// 이 채팅을 하위 작업으로 거느린 채팅.
    parent: ChatId,
    /// 부모 채팅에서 하위 접속을 받은 작업의 에이전트. 트리에 하위 에이전트를 올린 곳이다.
    parent_agent: AgentId,
    parent_provider: Provider,
    subagent: SubagentId,
}

/// 하위 채팅을 만들기 전에 부모에게서 읽은 값.
struct Inherited {
    parent_agent: AgentId,
    parent_live: LiveSession,
    request: AttachRequest,
}

/// 트리에서 하위 접속을 가리키는 이름표의 앞부분. provider에게는 없는 이름이라 멈춤 신호를 보내지 않는다.
pub(crate) const SUBAGENT_PREFIX: &str = "saturn-child-";

fn rejected(reason: impl Into<String>) -> EngineError {
    EngineError::ChildRejected {
        reason: reason.into(),
    }
}

impl Engine {
    /// provider 실행 환경에 넣는 출입증과 소켓 경로. 채팅에 출입증이 없으면 만든다. 하위 채팅이면 하위 채팅의 출입증이다.
    ///
    /// # Errors
    /// 모드를 읽지 못하면 그 오류, 난수를 읽지 못하면 `Provider(NotSent)`.
    pub(crate) async fn pass_env(
        &self,
        chat: ChatId,
        revision: saturn_protocol::ids::SettingsRevision,
    ) -> Result<Vec<(OsString, OsString)>, EngineError> {
        let mode = self.chat_mode(chat, revision).await?;
        let token = self.passes.open_root(chat, mode).map_err(|error| {
            tracing::warn!(%error, "failed to issue a child pass");
            saturn_core::providers::ProviderError::NotSent {
                reason: "failed to issue a child pass".to_owned(),
            }
        })?;
        Ok(vec![
            (OsString::from(PASS_ENV), OsString::from(token.as_str())),
            (
                OsString::from(SOCKET_ENV),
                self.options.home.join(SOCKET_FILE).into_os_string(),
            ),
        ])
    }

    /// 허용된 하위 접속 요청을 처리해 응답한다.
    pub(crate) async fn on_child_request(
        &mut self,
        client: ClientId,
        id: saturn_protocol::envelope::RequestId,
        grant: Grant,
    ) {
        let result = self.attach_child(client, &grant).await;
        self.respond(Responder::Rpc(client, id), result).await;
    }

    /// 부모 채팅의 작업 폴더, 더한 폴더, 환경, 채팅 층(권한 규칙)을 물려받은 새 채팅을 만들어 이 접속에 붙이고, 부모 작업의
    /// 하위 에이전트로 트리에 올린다. 실패하면 자리를 돌려준다.
    ///
    /// # Errors
    /// 부모에 실행 중인 작업이 없거나 요청 모드가 부모 모드를 넘으면 `ChildRejected`, 그 밖에는 기록과 설정 오류.
    async fn attach_child(&mut self, client: ClientId, grant: &Grant) -> Result<(), EngineError> {
        let inherited = match self.prepare_child(grant).await {
            Ok(inherited) => inherited,
            Err(error) => {
                self.passes.abandon(grant);
                return Err(error);
            }
        };
        let Inherited {
            parent_agent,
            parent_live,
            request,
        } = inherited;
        let Some(child) = request.chat else {
            self.passes.abandon(grant);
            return Err(rejected("child chat was not created"));
        };
        if let Err(error) = self.passes.bind(grant, child) {
            tracing::warn!(%error, "failed to issue a child pass");
            return Err(rejected("failed to issue a child pass"));
        }
        if let Err(error) = self.attach(client, request).await {
            self.finish_child(child).await;
            return Err(error);
        }
        let subagent = SubagentId(format!("{SUBAGENT_PREFIX}{}", child.0));
        self.children.insert(
            child,
            ChildLink {
                parent: grant.parent,
                parent_agent,
                parent_provider: parent_live.provider,
                subagent: subagent.clone(),
            },
        );
        let started = ProviderEvent::SubagentStarted {
            agent: parent_agent,
            subagent,
            parent: None,
        };
        self.on_tree_event(parent_live.provider, started).await;
        Ok(())
    }

    /// 부모에서 물려받을 값을 읽고 하위 채팅을 만든다. 출입증을 묶기 전까지의 일이라 실패하면 호출자가 자리를 돌려준다.
    async fn prepare_child(&mut self, grant: &Grant) -> Result<Inherited, EngineError> {
        let parent = grant.parent;
        let (parent_agent, parent_live) = self
            .running_agents(parent)
            .into_iter()
            .find_map(|agent| Some((agent, self.flow.live.get(&agent)?.clone())))
            .ok_or_else(|| rejected("parent has no running task"))?;
        let revision = self
            .flow
            .settings_of
            .get(&parent_agent)
            .copied()
            .or(self.settings.latest_of(parent))
            .ok_or(SettingsError::NoPreviousRevision)?;
        let parent_mode = self.chat_mode(parent, revision).await?;
        if grant.mode > parent_mode {
            return Err(rejected(format!(
                "requested mode is above the parent mode {}",
                parent_mode.name()
            )));
        }
        let workdir = self.store.chat_workdir(parent).await?;
        let env = self
            .chat_env(parent)
            .ok_or(EngineError::ChatNotAttached { chat: parent })?
            .provider_env()
            .into_iter()
            .filter(|(name, _)| name != PASS_ENV && name != SOCKET_ENV)
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        let overrides = self
            .attachments
            .values()
            .find(|attachment| attachment.chat == parent)
            .map(|attachment| attachment.overrides.clone())
            .unwrap_or_default();
        let layer = self.store.chat_layer(parent).await?;
        let child = self.store.create_chat(workdir.clone()).await?;
        let layer = settings::with_chat_layer_mode(layer.as_deref(), grant.mode);
        self.store.set_chat_layer(child, &layer).await?;
        for dir in self.chat_dirs_of(parent) {
            self.register_dir(child, dir).await?;
        }
        Ok(Inherited {
            parent_agent,
            parent_live,
            request: AttachRequest {
                chat: Some(child),
                workdir,
                env,
                overrides,
                add_dirs: Vec::new(),
            },
        })
    }

    /// 하위 접속과 그 아래 하위 접속을 모두 끝낸다: 출입증을 지우고, 부모 트리에서 내리고, 작업을 멈추고, provider 연결을 닫는다.
    /// 호출한 쪽이 이미 출입증을 회수했어도 안전하다.
    pub(crate) async fn finish_child(&mut self, child: ChatId) {
        let mut pending = vec![child];
        while let Some(chat) = pending.pop() {
            pending.extend(self.passes.end(chat));
            if let Some(link) = self.children.remove(&chat) {
                self.end_child_work(chat, link).await;
            }
        }
    }

    /// 채팅의 모든 하위 접속을 회수하고 끝낸다. 채팅 자신의 출입증은 그대로다.
    pub(crate) async fn end_children_of(&mut self, chat: ChatId) {
        for child in self.passes.end_children(chat) {
            if let Some(link) = self.children.remove(&child) {
                self.end_child_work(child, link).await;
            }
        }
    }

    /// 출입증은 이미 회수했다. 부모 트리에서 내리고 작업을 멈추고 연결을 닫는다.
    async fn end_child_work(&mut self, child: ChatId, link: ChildLink) {
        let ended = ProviderEvent::SubagentEnded {
            agent: link.parent_agent,
            subagent: link.subagent,
        };
        self.on_tree_event(link.parent_provider, ended).await;
        let stopped = self.stop_chat_core(child).await;
        self.warn_failure("failed to stop a child chat", stopped);
        // 연결을 놓아 provider 프로세스를 닫는다. 상한은 프로세스 수를 막으려는 것이라 유예 없이 닫는다.
        self.providers.retain(|(chat, _), _| *chat != child);
    }

    /// 부모 트리에 하위 접속을 올리거나 내리는 이벤트. 부모 작업이 이미 끝났으면 버려지고 경고만 남는다.
    async fn on_tree_event(&mut self, provider: Provider, event: ProviderEvent) {
        // 이벤트 처리가 연결 끊김과 멈춤을 거쳐 이 함수로 되돌아올 수 있어 고정 크기로 만들려고 상자에 담는다
        if let Err(error) = Box::pin(self.on_provider_event(provider, event)).await {
            tracing::warn!(error = %self.failure_line(&error), "child was not shown in the parent tree");
        }
    }

    /// 하위 접속이 쓰기 잠금을 부모 작업과 나누는지. 부모 작업이 잠금을 쥔 채 하위 작업을 기다리므로, 같은 폴더의 쓰기가
    /// 서로를 기다리면 멈춘다. 하위 채팅은 쓰기 범위가 비어 부모의 잠금 아래에서 돈다.
    pub(crate) fn is_child_chat(&self, chat: ChatId) -> bool {
        self.children.contains_key(&chat)
    }
}
