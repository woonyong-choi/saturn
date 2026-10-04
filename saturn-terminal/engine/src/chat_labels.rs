//! 채팅 이름과 묶음 변경. 채팅에 붙지 않은 접속도 작업 목록에서 바꾼다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::Notification;

use crate::{Engine, EngineError};

impl Engine {
    /// 앞뒤 공백을 지우고 비면 이름을 지운다.
    ///
    /// # Errors
    /// 제어 문자가 있으면 `InvalidLabel`, 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn rename_chat(&self, chat: ChatId, name: &str) -> Result<(), EngineError> {
        let name = clean_label("name", name)?;
        self.store.set_chat_name(chat, name).await?;
        self.announce_labels(chat).await
    }

    /// `None`이거나 공백뿐이면 묶음에서 뺀다.
    ///
    /// # Errors
    /// 제어 문자가 있으면 `InvalidLabel`, 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn set_chat_group(
        &self,
        chat: ChatId,
        group: Option<&str>,
    ) -> Result<(), EngineError> {
        let group = clean_label("group", group.unwrap_or_default())?;
        self.store.set_chat_group(chat, group).await?;
        self.announce_labels(chat).await
    }

    /// 저장한 이름과 묶음을 붙은 모든 TUI에 알린다.
    async fn announce_labels(&self, chat: ChatId) -> Result<(), EngineError> {
        let (name, group) = self.store.chat_labels(chat).await?;
        self.rpc
            .broadcast_attached(Notification::ChatLabeled { chat, name, group })
            .await;
        Ok(())
    }
}

fn clean_label<'a>(what: &'static str, text: &'a str) -> Result<Option<&'a str>, EngineError> {
    if text.chars().any(char::is_control) {
        return Err(EngineError::InvalidLabel { what });
    }
    let text = text.trim();
    Ok((!text.is_empty()).then_some(text))
}
