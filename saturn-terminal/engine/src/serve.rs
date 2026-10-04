use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use saturn_protocol::envelope::{RequestId, Response};
use saturn_protocol::rpc::Request;

use super::{AttachRequest, Engine, EngineError, RouterGate, masked_chain, unsupported};
use crate::outcomes;
use crate::rpc::{ClientId, RpcEvent};
use crate::settings_watch::SETTINGS_WATCH_TICK;

/// 백그라운드에서 모든 작업이 끝났는지 보는 간격의 위쪽 한계. 초안.
const IDLE_TICK: Duration = Duration::from_secs(1);

impl Engine {
    /// # Errors
    /// 복구할 수 없는 오류만 돌려주고, 요청 하나의 오류는 그 클라이언트에 알리고 계속한다.
    #[expect(
        clippy::cognitive_complexity,
        reason = "select! 분기마다 한 줄씩 넘기는 루프라 나누면 대응이 흩어지고, 분기마다 점수가 오른다"
    )]
    pub(super) async fn serve(&mut self) -> Result<(), EngineError> {
        let mut tick = tokio::time::interval(outcomes::SETTLE_TICK);
        let mut watch = tokio::time::interval(SETTINGS_WATCH_TICK);
        let mut idle = tokio::time::interval(self.idle_grace.min(IDLE_TICK));
        loop {
            tokio::select! {
                event = self.rpc.next_event() => {
                    let Some(event) = event else { break };
                    self.handle_event(event).await?;
                }
                Some(done) = self.flow.router_rx.recv() => {
                    self.on_routed(done).await;
                }
                Some(message) = self.flow.provider_rx.recv() => {
                    self.on_provider_msg(message).await;
                }
                Some(done) = self.flow.stop_rx.recv() => {
                    self.on_stop_done(done).await;
                }
                _ = watch.tick() => {
                    self.watch_settings().await;
                }
                _ = idle.tick() => {
                    if self.check_idle(Instant::now()) {
                        break;
                    }
                }
                _ = tick.tick() => {
                    let settled = self.settle_signals(Instant::now()).await;
                    self.warn_failure("failed to settle judgment signals", settled);
                }
            }
        }
        Ok(())
    }

    /// 유휴 시각을 갱신하고, 유예가 지나 engine이 끝나야 하면 참.
    fn check_idle(&mut self, now: Instant) -> bool {
        self.refresh_idle(now);
        let is_expired = self.background_expired(now);
        if is_expired {
            tracing::info!("no tui and no work, engine is ending");
        }
        is_expired
    }

    pub(super) async fn handle_event(&mut self, event: RpcEvent) -> Result<(), EngineError> {
        match event {
            RpcEvent::Connected(_) => {}
            RpcEvent::Request(client, id, request) => {
                self.handle_request(client, id, request).await?;
            }
            RpcEvent::Disconnected(client) => {
                let detached = self.attachments.remove(&client);
                if let Some(attachment) = detached
                    && self.attachments.is_empty()
                {
                    self.on_last_detach(attachment.chat).await;
                }
            }
            // 유예 시계는 `serve`의 유휴 점검이 돌린다. 마지막 TUI 이탈은 `Disconnected`에서 처리한다
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

    #[expect(
        clippy::cognitive_complexity,
        reason = "요청 종류마다 한 줄씩 넘기는 분배라 나누면 대응표가 흩어지고, .await마다 점수가 오른다"
    )]
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
            Request::RenameChat { chat, name } => self.rename_chat(chat, &name).await,
            Request::SetChatGroup { chat, group } => {
                self.set_chat_group(chat, group.as_deref()).await
            }
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
            Request::AnswerStopConfirm { input, stop } => {
                self.answer_stop_confirm(client, input, stop).await
            }
            Request::CancelInput { input } => self.cancel_input(client, input).await,
            Request::Stop { chat } => self.stop_chat(chat).await,
            Request::StopAll => {
                self.stop_all_chats().await;
                Ok(())
            }
            Request::PrepareExit { chat } => self.prepare_exit(client, chat).await,
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
            Request::LatestChat { folder } => self.send_latest_chat(client, &folder).await,
            Request::ListChats { folder } => self.send_chat_list(client, folder.as_deref()).await,
            Request::SetModel { chat, model } => self.set_model(client, chat, &model).await,
            Request::ListModels { chat, provider } => {
                self.send_models(client, chat, provider).await
            }
            Request::ListTasks => self.send_task_list(client).await,
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
                Request::Attach { .. }
                    | Request::SubmitRouterKey { .. }
                    | Request::LatestChat { .. }
                    | Request::ListChats { .. }
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
