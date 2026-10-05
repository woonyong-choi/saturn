//! provider 원시 응답 수집: 연결의 읽기 작업이 받은 줄을 변환하기 전 모습 그대로(router 키만 가려) engine 루프로 보낸다.
//! 저장은 engine 루프 하나가 한다. 설계: docs/design/records.md#provider-원시-응답-수집

use saturn_protocol::ids::{AgentId, ChatId, Provider};
use serde_json::Value;
use tokio::sync::mpsc;

use super::{ProviderMsg, mask_values};
use crate::secrets::Masker;

/// provider가 보낸 줄 하나. `bytes`는 줄바꿈을 뺀 글자이고 router 키는 이미 가렸다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawLine {
    /// 줄이 속한 에이전트. 어댑터가 provider의 session·thread 식별자로 찾았을 때만 있고, 못 찾으면 비어 있다.
    pub(crate) agent: Option<AgentId>,
    /// 줄에 적힌 provider의 session 또는 thread 식별자. 없으면 비어 있다.
    pub(crate) provider_session: Option<String>,
    /// JSON으로 읽었는지. 읽지 못한 줄도 버리지 않고 보낸다.
    pub(crate) is_json: bool,
    pub(crate) bytes: Vec<u8>,
}

/// 연결 하나가 받은 줄을 engine으로 보내는 손잡이. 어댑터는 줄을 읽는 즉시, 그 줄이 만드는 이벤트를 보내기 전에 보낸다.
/// 둘은 같은 메시지 줄에 서므로 engine은 줄을 먼저 받고 그 줄의 이벤트(턴 끝으로 실행을 닫는 이벤트 포함)를 나중에 받는다.
#[derive(Debug, Clone, Default)]
pub struct RawTap {
    link: Option<Link>,
}

#[derive(Debug, Clone)]
struct Link {
    chat: ChatId,
    provider: Provider,
    msgs: mpsc::UnboundedSender<ProviderMsg>,
    masker: Masker,
}

impl RawTap {
    /// 수집하지 않는 손잡이.
    pub fn off() -> Self {
        Self::default()
    }

    pub(crate) fn new(
        chat: ChatId,
        provider: Provider,
        msgs: mpsc::UnboundedSender<ProviderMsg>,
        masker: &Masker,
    ) -> Self {
        Self {
            link: Some(Link {
                chat,
                provider,
                msgs,
                masker: masker.clone(),
            }),
        }
    }

    /// `message`는 `line`을 읽어 router 키를 가린 값이고, 줄이 JSON이 아니면 `None`이다. 줄에서 키를 가린 결과를 JSON으로
    /// 읽은 값이 `message`와 다르면(이스케이프로 줄 글자에서는 못 찾은 키가 있다) 가린 `message`를 다시 써서 보낸다.
    pub(crate) fn send(
        &self,
        line: &str,
        message: Option<&Value>,
        (agent, provider_session): (Option<AgentId>, Option<String>),
    ) {
        let Some(link) = &self.link else {
            return;
        };
        let masked = link.masker.mask(line).as_str().to_owned();
        let bytes = match message {
            Some(message)
                if serde_json::from_str::<Value>(&masked).ok().as_ref() != Some(message) =>
            {
                message.to_string()
            }
            _ => masked,
        };
        let raw = RawLine {
            agent,
            provider_session,
            is_json: message.is_some(),
            bytes: bytes.into_bytes(),
        };
        let _ = link.msgs.send(ProviderMsg::Raw {
            chat: link.chat,
            provider: link.provider,
            raw,
        }); // engine이 끝난 뒤에는 받을 곳이 없다
    }

    /// 읽은 줄이 JSON이면 키를 가린 값을, 아니면 `None`을 돌려준다. 어댑터가 변환에 쓰는 값이다.
    pub(crate) fn parse_masked(line: &str, masker: &Masker) -> Option<Value> {
        let mut message = serde_json::from_str::<Value>(line).ok()?;
        mask_values(&mut message, masker);
        Some(message)
    }
}
