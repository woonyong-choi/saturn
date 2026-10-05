//! 전달 패킷 근거: provider를 부르기 직전에 보낼 패킷 한 시도를 기록 저장소에 쓰고, 결과를 알면 확정한다.
//! 받은 응답(원시 기록)과 보낸 패킷은 다른 기록이다. 이 기록은 보낸 쪽만 맡고 본문은 해시로만 남긴다.
//! 설계: docs/design/records.md#전달-패킷-근거

use saturn_core::providers::ProviderError;
use saturn_protocol::ids::{ChatId, InputId, Provider, SessionId, SettingsRevision};

use crate::Engine;
use crate::handoff::PacketEvidence;
use crate::store::{NewPacket, PacketId, PacketKind, PacketState};

/// 패킷을 받는 session과 보내게 한 입력.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PacketTarget {
    pub(crate) chat: ChatId,
    pub(crate) kind: PacketKind,
    pub(crate) session: SessionId,
    pub(crate) input: Option<InputId>,
    pub(crate) provider: Provider,
    pub(crate) settings: SettingsRevision,
}

/// 결과로 시도의 상태를 정한다. 보내지 않음이 확정인 오류만 `NotSent`이고, 연결이 끊긴 경우를 포함해 나머지 오류는
/// 보냈는지 모른다.
pub(crate) fn state_of<T>(result: &Result<T, ProviderError>) -> PacketState {
    match result {
        Ok(_) => PacketState::Sent,
        Err(ProviderError::NotSent { .. } | ProviderError::ContextExceeded { .. }) => {
            PacketState::NotSent
        }
        Err(_) => PacketState::Unknown,
    }
}

impl Engine {
    /// 보낼 패킷 시도를 기록한다. 기록하지 못해도 전송은 막지 않고 로그만 남기며 `None`을 돌려준다.
    pub(crate) async fn record_packet_attempt(
        &self,
        target: PacketTarget,
        body: &str,
        evidence: &PacketEvidence,
    ) -> Option<PacketId> {
        let policy = self
            .policy_digest_at(target.settings)
            .await
            .unwrap_or_else(|_| "unavailable".to_owned());
        let recorded = async {
            let constraint_revision = self.store.constraint_revision(target.chat).await?;
            self.store
                .record_packet(&NewPacket {
                    chat: target.chat,
                    kind: target.kind,
                    attempt: evidence.attempt,
                    reduced_from: evidence.reduced_from,
                    session: target.session,
                    input: target.input,
                    provider: target.provider,
                    settings: target.settings.0,
                    chat_revision: evidence.up_to.0,
                    constraint_revision,
                    policy,
                    body: body.to_owned(),
                    estimated_tokens: evidence.tokens,
                    items: evidence.rows(),
                })
                .await
        }
        .await;
        match recorded {
            Ok(id) => Some(id),
            Err(error) => {
                self.warn_failure("failed to record a handoff packet", Err::<(), _>(error));
                None
            }
        }
    }

    /// 결과를 안 시도의 상태를 확정한다. 기록하지 못하면 로그만 남긴다.
    pub(crate) async fn settle_packet<T>(
        &self,
        packet: Option<PacketId>,
        result: &Result<T, ProviderError>,
        provider_session: Option<&str>,
    ) {
        let Some(packet) = packet else {
            return;
        };
        let settled = self
            .store
            .settle_packet(packet, state_of(result), provider_session)
            .await;
        self.warn_failure("failed to settle a handoff packet", settled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_confirmed_refusal_is_not_sent_and_every_other_failure_is_unknown() {
        let cases: [(Result<(), ProviderError>, PacketState); 5] = [
            (Ok(()), PacketState::Sent),
            (
                Err(ProviderError::NotSent {
                    reason: "write failed".to_owned(),
                }),
                PacketState::NotSent,
            ),
            (
                Err(ProviderError::ContextExceeded { limit_tokens: None }),
                PacketState::NotSent,
            ),
            (Err(ProviderError::Unknown), PacketState::Unknown),
            (Err(ProviderError::ConnectionLost), PacketState::Unknown),
        ];

        for (result, expected) in cases {
            assert_eq!(state_of(&result), expected, "{result:?}");
        }
    }
}
