//! 요청 합계 알림: 채팅의 모든 일이 끝난 순간 그 요청의 provider 토큰, router 호출, 경과를 한 번 알린다.
//! 요청은 채팅이 쉬는 동안 접수한 첫 입력에서 시작해, 판단 중이거나 기다리는 입력과 실행 중인 작업이 모두 없어지면 끝난다.
//! 합계는 화면이 세던 값이 아니라 기록 저장소에서 읽어 `/usage`와 같은 계산으로 만든다.
//! 설계: docs/design/tui.md#요청-합계

use std::time::{Instant, SystemTime};

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::ChatNotice;
use saturn_protocol::state::InputState;

use crate::Engine;
use crate::store::to_millis;
use crate::usage::request_totals;

/// 열려 있는 요청 하나.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenRequest {
    /// 첫 입력을 접수하기 직전의 시각(unix 밀리초). 이 뒤에 시작한 실행과 router 호출이 이 요청의 것이다.
    since_ms: i64,
    started: Instant,
}

impl OpenRequest {
    pub(crate) fn now() -> Self {
        Self {
            since_ms: to_millis(SystemTime::now()),
            started: Instant::now(),
        }
    }
}

impl Engine {
    /// 입력을 접수하기 전에 부른다. 채팅에 열린 요청이 없으면 새로 열고, 있으면 그 요청에 들어간다.
    pub(crate) fn open_request(&mut self, chat: ChatId) {
        self.flow
            .requests
            .entry(chat)
            .or_insert_with(OpenRequest::now);
    }

    /// 판단 중이거나 기다리거나 전달 중인 입력, 실행 중인 작업, 멈추는 중이거나 맥락 정리 중인 일이 없다.
    /// 보류와 결과 모름은 사용자 확인을 기다리는 것이라 끝난 것으로 본다.
    fn is_quiet(&self, chat: ChatId) -> bool {
        let is_pending = [
            InputState::Judging,
            InputState::Queued,
            InputState::Delivering,
        ]
        .into_iter()
        .any(|state| !self.queue.inputs_in_state(chat, state).is_empty());
        let in_flight_router = self
            .flow
            .pending_lines
            .keys()
            .chain(self.flow.pending_change.keys())
            .any(|input| {
                self.queue
                    .input(*input)
                    .is_some_and(|input| input.chat == chat)
            });
        !is_pending
            && !in_flight_router
            && !self.chat_is_running(chat)
            && !self.flow.stopping.contains_key(&chat)
            && !self.flow.restarting.contains(&chat)
            && !self.flow.compact_waiting.contains_key(&chat)
    }

    /// 쉬게 된 채팅의 열린 요청을 닫고 합계를 한 번 알린다. 하위 채팅(에이전트 작업)은 알리지 않는다.
    /// engine 루프가 일을 처리할 때마다 부르고, 닫을 요청이 없으면 아무것도 하지 않는다.
    pub(crate) async fn finish_quiet_requests(&mut self) {
        let quiet: Vec<ChatId> = self
            .flow
            .requests
            .keys()
            .copied()
            .filter(|chat| self.is_quiet(*chat))
            .collect();
        for chat in quiet {
            let Some(request) = self.flow.requests.remove(&chat) else {
                continue;
            };
            if self.is_child_chat(chat) {
                continue;
            }
            match request_totals(&self.store, chat, request.since_ms).await {
                Ok(totals) if totals.provider_tokens.is_empty() && totals.router_calls == 0 => {}
                Ok(totals) => {
                    let notice = ChatNotice::RequestSummary {
                        provider_tokens: totals.provider_tokens,
                        router_calls: totals.router_calls,
                        router_tokens: totals.router_tokens,
                        elapsed_ms: u64::try_from(request.started.elapsed().as_millis())
                            .unwrap_or(u64::MAX),
                    };
                    self.notify_chat(chat, notice).await;
                }
                Err(error) => self.warn_failure::<()>("failed to total a request", Err(error)),
            }
        }
    }
}
