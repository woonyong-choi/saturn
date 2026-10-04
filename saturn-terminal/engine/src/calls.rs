//! 입력 전달이 아닌 provider 요청: 허가·입력 답, `/model` 목록의 연결과 조회, 맥락 정리의 새 session 열기.
//! 요청은 연결 작업이 실행하고, engine 루프는 번호를 붙여 맡긴 뒤 결과 메시지가 올 때 이어 간다.
//! 설계: docs/design/providers-and-sessions.md#provider-요청-작업

use saturn_core::providers::ProviderError;
use saturn_protocol::envelope::{RequestId, Response};
use saturn_protocol::ids::{ChatId, Provider};
use saturn_protocol::rpc::QueryResult;

use crate::events::{PermissionAnswering, RuleAnswering};
use crate::inputs::InputAnswering;
use crate::launch::ConnectionSeed;
use crate::providers::{CallResult, ProviderHandle};
use crate::rpc::ClientId;
use crate::switch::Restart;
use crate::{Engine, EngineError, masked_chain};

/// 요청에 응답을 돌려줄 곳. 결과를 기다려야 하는 요청은 맡긴 뒤 이 값을 들고 있다가 결과가 오면 응답한다.
#[derive(Debug)]
pub(crate) enum Responder {
    Rpc(ClientId, RequestId),
    /// 소켓 없이 부르는 시험이 결과를 받는다.
    #[cfg(test)]
    Local(tokio::sync::oneshot::Sender<Result<(), EngineError>>),
}

/// 결과를 기다리는 요청 하나와 그 요청이 간 연결.
#[derive(Debug)]
pub(crate) struct PendingCall {
    pub(crate) chat: ChatId,
    pub(crate) provider: Provider,
    pub(crate) kind: CallKind,
}

/// 요청 종류마다 결과를 받아 이어 갈 값.
#[derive(Debug)]
pub(crate) enum CallKind {
    /// 사용자가 한 허가 답.
    Permission(PermissionAnswering),
    /// 사용자가 한 입력 요청 답.
    Input(InputAnswering),
    /// Saturn 규칙이 한 허가 답.
    Rule(RuleAnswering),
    /// `/model` 요청의 모델 조회 하나.
    ModelList { query: u64 },
    /// `/model` 요청의 연결 하나. 연결을 맺고 모델 목록까지 받아 온다.
    ModelConnect { query: u64, seed: ConnectionSeed },
    /// 맥락 정리의 새 session 열기.
    Restart(Box<Restart>),
}

impl CallKind {
    /// 연결 작업이 끝나 결과가 오지 않을 때 대신 쓰는 결과.
    fn lost(&self) -> CallResult {
        let lost = ProviderError::ConnectionLost;
        match self {
            Self::Permission(_) | Self::Input(_) | Self::Rule(_) => CallResult::Done(Err(lost)),
            Self::ModelList { .. } => CallResult::Models(Err(lost)),
            Self::ModelConnect { .. } => CallResult::Connected(Err(lost)),
            Self::Restart(_) => CallResult::Opened(Err(lost)),
        }
    }
}

#[cfg(test)]
impl Responder {
    /// 소켓 없이 응답을 받는 시험용 응답처.
    pub(crate) fn local() -> (
        Self,
        tokio::sync::oneshot::Receiver<Result<(), EngineError>>,
    ) {
        let (reply, receive) = tokio::sync::oneshot::channel();
        (Self::Local(reply), receive)
    }
}

impl Engine {
    /// 요청을 연결 작업에 맡기고 기다리지 않는다. 호출자는 그 연결이 있는지 먼저 확인해야 한다. 결과가 요청보다
    /// 먼저 처리되지 않도록 번호를 먼저 등록하고 맡긴다.
    pub(crate) fn start_call(
        &mut self,
        (chat, provider): (ChatId, Provider),
        kind: CallKind,
        send: impl FnOnce(&ProviderHandle, u64),
    ) {
        let tag = self.flow.issue_call();
        self.flow.calls.insert(
            tag,
            PendingCall {
                chat,
                provider,
                kind,
            },
        );
        match self.providers.get(&(chat, provider)) {
            Some(handle) => send(handle, tag),
            None => self.lose_call(tag),
        }
    }

    /// 연결이 없어 맡기지 못한 요청의 결과를 연결 끊김으로 돌려준다.
    fn lose_call(&self, tag: u64) {
        let Some(call) = self.flow.calls.get(&tag) else {
            return;
        };
        let message = crate::providers::ProviderMsg::Reply {
            chat: call.chat,
            provider: call.provider,
            reply: crate::providers::Reply::Call {
                tag,
                result: call.kind.lost(),
            },
        };
        let _ = self.flow.provider_tx.send(message); // 받는 쪽은 이 engine의 루프다
    }

    /// 맡긴 요청의 결과를 그 요청이 이어 갈 곳으로 넘긴다.
    #[expect(
        clippy::cognitive_complexity,
        reason = "요청 종류마다 한 줄씩 넘기는 분배라 나누면 대응표가 흩어지고, .await마다 점수가 오른다"
    )]
    pub(crate) async fn on_call_reply(&mut self, tag: u64, result: CallResult) {
        let Some(PendingCall {
            chat,
            provider,
            kind,
        }) = self.flow.calls.remove(&tag)
        else {
            tracing::warn!(tag, "provider reply without a waiting request");
            return;
        };
        match (kind, result) {
            (CallKind::Permission(answering), CallResult::Done(result)) => {
                self.finish_permission_answer(answering, result).await;
            }
            (CallKind::Input(answering), CallResult::Done(result)) => {
                self.finish_input_answer(answering, result).await;
            }
            (CallKind::Rule(answering), CallResult::Done(result)) => {
                self.finish_rule_answer(chat, answering, result).await;
            }
            (CallKind::ModelList { query }, CallResult::Models(listed)) => {
                self.on_models_listed(query, (chat, provider), listed).await;
            }
            (CallKind::ModelConnect { query, seed }, CallResult::Connected(connected)) => {
                self.on_models_connected(query, (chat, provider), seed, connected)
                    .await;
            }
            (CallKind::Restart(restart), CallResult::Opened(opened)) => {
                self.finish_restart(chat, *restart, opened).await;
            }
            (kind, result) => {
                tracing::warn!(?kind, ?result, "provider reply does not match the request");
            }
        }
    }

    /// 연결 작업이 끝났다. 그 연결에 맡긴 요청은 결과가 오지 않으므로 연결 끊김으로 이어 간다.
    pub(crate) async fn on_calls_lost(&mut self, chat: ChatId, provider: Provider) {
        let lost: Vec<(u64, CallResult)> = self
            .flow
            .calls
            .iter()
            .filter(|(_, call)| call.chat == chat && call.provider == provider)
            .map(|(tag, call)| (*tag, call.kind.lost()))
            .collect();
        for (tag, result) in lost {
            self.on_call_reply(tag, result).await;
        }
    }

    /// 요청에 응답한다. 이미 끊긴 클라이언트에는 응답할 곳이 없다.
    pub(crate) async fn respond(&self, responder: Responder, result: Result<(), EngineError>) {
        self.respond_with(responder, result.map(|()| None)).await;
    }

    /// 조회 요청이면 결과를 응답의 `result`에 담아 응답한다.
    pub(crate) async fn respond_with(
        &self,
        responder: Responder,
        result: Result<Option<QueryResult>, EngineError>,
    ) {
        match responder {
            Responder::Rpc(client, id) => {
                let response = self.response_of(client, id, result);
                let _ = self.rpc.respond(client, response).await; // 이미 끊긴 클라이언트에는 응답할 곳이 없다
            }
            #[cfg(test)]
            Responder::Local(reply) => {
                let _ = reply.send(result.map(|_| ())); // 결과를 기다리지 않는 시험도 있다
            }
        }
    }

    /// 요청 결과를 응답으로 바꾼다. 오류는 가려서 알린다.
    pub(crate) fn response_of(
        &self,
        client: ClientId,
        id: RequestId,
        result: Result<Option<QueryResult>, EngineError>,
    ) -> Response {
        match result {
            Ok(None) => Response::ok(id),
            Ok(Some(result)) => Response::result(id, result),
            Err(error) => {
                let message = masked_chain(&self.masker, &error);
                tracing::warn!(client = client.0, error = %message, "request failed");
                Response::error_of_kind(Some(id), error.code(), error.kind(), message)
            }
        }
    }
}
