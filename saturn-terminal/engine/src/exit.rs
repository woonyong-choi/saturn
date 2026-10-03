//! TUI를 닫은 뒤 처리: `on_exit` 값에 따라 계속하거나 모든 채팅의 작업을 멈춤과 같게 보류한다.
//! 설계: docs/design/engine-lifecycle.md#tui-종료-뒤-동작

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ExitPlan, Notification};
use saturn_protocol::state::OnExit;

use crate::rpc::ClientId;
use crate::{Engine, EngineError, Presence};

impl Engine {
    /// `PrepareExit` 요청. 닫은 뒤의 처리를 `ExitPlan`으로 보낸다.
    ///
    /// # Errors
    /// 이 클라이언트가 붙지 않은 채팅이면 `ChatNotAttached`, 설정을 읽지 못하면 `Settings`나 `Store`.
    pub(crate) async fn prepare_exit(
        &mut self,
        client: ClientId,
        chat: ChatId,
    ) -> Result<(), EngineError> {
        self.require_attached(client, chat)?;
        let plan = self.exit_plan(client, chat).await?;
        self.send(client, Notification::ExitPlan { plan }).await;
        Ok(())
    }

    /// 다른 TUI가 붙어 있으면 `on_exit`는 적용되지 않으므로 묻지 않는다.
    async fn exit_plan(&mut self, client: ClientId, chat: ChatId) -> Result<ExitPlan, EngineError> {
        let others_attached = self.attachments.keys().any(|other| *other != client);
        let running = self.continuing_work_total();
        if others_attached || running == 0 {
            return Ok(ExitPlan::Close);
        }
        Ok(match self.on_exit_of(chat).await? {
            OnExit::Stop => ExitPlan::Close,
            OnExit::Ask => ExitPlan::Ask { running },
            OnExit::Background => ExitPlan::Notice { running },
        })
    }

    /// `StopAll` 요청. 작업이 있는 모든 채팅에 멈춤을 적용한다.
    /// 한 채팅의 실패는 경고로 남기고 나머지 채팅을 이어서 멈춘다.
    pub(crate) async fn stop_all_chats(&mut self) {
        let mut chats: Vec<ChatId> = self.chats.keys().copied().collect();
        chats.sort();
        for chat in chats {
            if self.continuing_work(chat) > 0 {
                let stopped = self.stop_chat(chat).await;
                self.warn_failure("failed to stop chat on exit", stopped);
            }
        }
    }

    /// 마지막 TUI가 떨어졌다. `stop`이면 모든 채팅을 멈춤과 같게 보류하고, `background`와 `ask`는 계속한다.
    /// `ask`의 질문은 TUI가 닫히기 전에 끝나 있어서, 질문 없이 떨어진 `ask`는 계속으로 본다.
    pub(crate) async fn on_last_detach(&mut self, chat: ChatId) {
        self.presence = Presence::Background { idle_since: None };
        let on_exit = self.on_exit_of(chat).await;
        match on_exit {
            Ok(OnExit::Stop) => self.stop_all_chats().await,
            Ok(OnExit::Background | OnExit::Ask) => {}
            Err(error) => {
                tracing::warn!(error = %self.failure_line(&error), "failed to read on_exit; continuing in background");
            }
        }
    }

    /// 채팅의 폴더 층과 채팅 층까지 합친 `on_exit`.
    async fn on_exit_of(&mut self, chat: ChatId) -> Result<OnExit, EngineError> {
        let workdir = self
            .chat_env(chat)
            .ok_or(EngineError::ChatNotAttached { chat })?
            .workdir()
            .to_path_buf();
        let (applied, _) = self
            .settings
            .apply_trusted(&self.store, Some(chat), &workdir)
            .await?;
        Ok(self
            .settings
            .at(&self.store, applied.revision)
            .await?
            .on_exit())
    }

    /// 모든 채팅에서 계속 처리될 작업 수.
    fn continuing_work_total(&self) -> u32 {
        let total: usize = self
            .chats
            .keys()
            .map(|chat| self.continuing_work(*chat))
            .sum();
        u32::try_from(total).unwrap_or(u32::MAX)
    }
}
