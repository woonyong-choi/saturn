use std::path::{Path, PathBuf};
use std::time::Instant;

use saturn_protocol::envelope::{RequestId, Response};
use saturn_protocol::rpc::Request;

use super::{AttachRequest, Engine, EngineError, RouterGate, masked_chain, unsupported};
use crate::rpc::{ClientId, RpcEvent};
use crate::{events, outcomes};

impl Engine {
    /// # Errors
    /// 복구할 수 없는 오류만 돌려주고, 요청 하나의 오류는 그 클라이언트에 알리고 계속한다.
    pub(super) async fn serve(&mut self) -> Result<(), EngineError> {
        let mut tick = tokio::time::interval(outcomes::SETTLE_TICK);
        loop {
            tokio::select! {
                event = self.rpc.next_event() => {
                    let Some(event) = event else { break };
                    self.handle_event(event).await?;
                }
                Some(done) = self.flow.router_rx.recv() => {
                    self.on_routed(done).await;
                }
                arrival = events::next_arrival(&mut self.providers) => {
                    self.on_arrival(arrival).await;
                }
                Some(done) = self.flow.stop_rx.recv() => {
                    self.on_stop_done(done).await;
                }
                _ = tick.tick() => {
                    let settled = self.settle_signals(Instant::now()).await;
                    self.warn_failure("failed to settle judgment signals", settled);
                }
            }
        }
        Ok(())
    }

    async fn handle_event(&mut self, event: RpcEvent) -> Result<(), EngineError> {
        match event {
            RpcEvent::Connected(_) => {}
            RpcEvent::Request(client, id, request) => {
                self.handle_request(client, id, request).await?;
            }
            RpcEvent::Disconnected(client) => {
                self.attachments.remove(&client);
            }
            // TODO(#150): 마지막 TUI가 떨어진 뒤 `on_last_detach`와 유예 시계
            RpcEvent::LastDetached => {}
        }
        Ok(())
    }

    /// 요청마다 응답 하나를 돌려준다. `SubmitRouterKey` 메시지는 기록하지 않는다.
    async fn handle_request(
        &mut self,
        client: ClientId,
        id: RequestId,
        request: Request,
    ) -> Result<(), EngineError> {
        let response = match self.route(client, request).await {
            Ok(()) => Response::ok(id),
            Err(error) => {
                let message = masked_chain(&self.masker, &error);
                tracing::warn!(client = client.0, error = %message, "request failed");
                Response::error(Some(id), error.code(), message)
            }
        };
        let _ = self.rpc.respond(client, response).await; // 이미 끊긴 클라이언트에는 응답할 곳이 없다
        Ok(())
    }

    async fn route(&mut self, client: ClientId, request: Request) -> Result<(), EngineError> {
        self.ensure_router_open(&request)?;
        match request {
            Request::Attach {
                chat,
                workdir,
                env,
                overrides,
                add_dirs,
            } => {
                let request = AttachRequest {
                    chat,
                    workdir: PathBuf::from(workdir),
                    env,
                    overrides,
                    add_dirs,
                };
                self.attach(client, request).await
            }
            Request::AddDir { chat, path } => self.add_dir(client, chat, &path).await,
            Request::LoadHistory {
                chat,
                before,
                limit,
            } => self.load_history(client, chat, before, limit).await,
            // TODO(#161): 채팅 이름과 묶음
            Request::RenameChat { .. } => Err(unsupported("RenameChat")),
            Request::SetChatGroup { .. } => Err(unsupported("SetChatGroup")),
            // 서버가 응답하고 끊김으로 바꿔 여기까지 오지 않는다.
            Request::Detach => Ok(()),
            Request::SubmitInput {
                chat,
                client_ref,
                text,
                skip_relation,
            } => {
                self.submit_input(client, chat, client_ref, text, skip_relation)
                    .await
            }
            Request::RunAsNewTask { input } => self.run_as_new_task(client, input).await,
            Request::SendNow { input } => self.send_now(client, input).await,
            Request::CancelInput { input } => self.cancel_input(client, input).await,
            Request::Stop { chat } => self.stop_chat(chat).await,
            Request::Continue { chat, task } => self.continue_held(chat, task).await,
            Request::ContinueInput { input } => self.continue_input(input).await,
            Request::CloseHeld { chat, task } => self.close_held(chat, task).await,
            Request::AnswerPermission { request_id, answer } => {
                self.answer_permission(client, request_id, answer).await
            }
            Request::AnswerInput { request_id, answer } => {
                self.answer_input(client, request_id, answer).await
            }
            Request::AnswerFeedback { judgment, correct } => {
                self.answer_feedback(judgment, correct).await
            }
            Request::SubmitRouterKey { key } => self.submit_router_key(client, key).await,
            Request::AnswerFolderTrust {
                path,
                fingerprint,
                apply,
            } => {
                self.answer_folder_trust(client, path, fingerprint, apply)
                    .await
            }
            Request::SetRecording { chat, on } => Ok(self.store.set_recording(chat, on).await?),
            Request::SetPermissionMode { chat, mode } => {
                self.set_permission_mode(chat, &mode).await
            }
            Request::Usage { scope, folder } => {
                self.send_usage(client, scope, folder.as_deref()).await
            }
            Request::SetModel { chat, model } => self.set_model(client, chat, &model).await,
            Request::ListModels { chat, provider } => {
                self.send_models(client, chat, provider).await
            }
            // TODO(#161): 작업 목록
            Request::ListTasks => Err(unsupported("ListTasks")),
            // TODO(#91): 학습과 router 버전
            Request::Train { .. } => Err(unsupported("Train")),
            Request::ConfirmTrain { .. } => Err(unsupported("ConfirmTrain")),
            Request::ListRouterVersions => Err(unsupported("ListRouterVersions")),
            Request::UseRouterVersion { .. } => Err(unsupported("UseRouterVersion")),
            // TODO(#161): 기록 정리 미리보기
            Request::Prune { .. } => Err(unsupported("Prune")),
            Request::ExportJudgments { path } => self.export_judgments(&path).await,
        }
    }

    /// router 키를 기다리는 동안은 `Attach`와 `SubmitRouterKey`만 받는다.
    fn ensure_router_open(&self, request: &Request) -> Result<(), EngineError> {
        if let RouterGate::KeyRequired { reason } = &self.router_gate
            && !matches!(
                request,
                Request::Attach { .. } | Request::SubmitRouterKey { .. }
            )
        {
            return Err(EngineError::RouterKeyRequired {
                reason: reason.clone(),
            });
        }
        Ok(())
    }

    async fn export_judgments(&self, path: &str) -> Result<(), EngineError> {
        let count = self.store.export_judgments(Path::new(path)).await?;
        tracing::info!(count, "judgments exported");
        Ok(())
    }
}
