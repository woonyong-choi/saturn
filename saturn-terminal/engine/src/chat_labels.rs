//! 채팅 이름과 묶음 변경. 채팅에 붙지 않은 접속도 작업 목록에서 바꾼다.
//! 설계: docs/design/tui.md

use saturn_protocol::ids::ChatId;

use crate::{Engine, EngineError};

impl Engine {
    /// 앞뒤 공백을 지우고 비면 이름을 지운다.
    ///
    /// # Errors
    /// 제어 문자가 있으면 `InvalidLabel`, 없는 채팅이면 `Store(NotFound)`.
    pub(crate) async fn rename_chat(&self, chat: ChatId, name: &str) -> Result<(), EngineError> {
        let name = clean_label("name", name)?;
        Ok(self.store.set_chat_name(chat, name).await?)
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
        Ok(self.store.set_chat_group(chat, group).await?)
    }
}

fn clean_label<'a>(what: &'static str, text: &'a str) -> Result<Option<&'a str>, EngineError> {
    if text.chars().any(char::is_control) {
        return Err(EngineError::InvalidLabel { what });
    }
    let text = text.trim();
    Ok((!text.is_empty()).then_some(text))
}
