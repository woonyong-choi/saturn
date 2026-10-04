use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use saturn_protocol::envelope::RequestId;
use saturn_protocol::rpc::{QueryResult, Request};

use super::{AttachRequest, Engine, EngineError, RouterGate, unsupported};
use crate::calls::Responder;
use crate::outcomes;
use crate::rpc::{ClientId, RpcEvent};
use crate::settings_watch::SETTINGS_WATCH_TICK;

/// 백그라운드에서 모든 작업이 끝났는지 보는 간격의 위쪽 한계. 초안.
const IDLE_TICK: Duration = Duration::from_secs(1);

/// `route_deferred`의 결과.
enum Deferred {
    /// 응답을 이미 했거나 결과가 오면 한다.
    Responded,
    /// 이 요청은 `route`가 처리한다.
    Route(RequestId, Request),
}

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
                    if self.upgrade_requested {
                        self.announce_restart().await;
                        break;
                    }
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

    /// 요청마다 응답 하나를 돌려준다. `SubmitRouterKey` 메시지는 기록하지 않는다. provider 응답을 기다리는
    /// 요청(허가·입력 답, 모델 목록)은 루프가 기다리지 않고 맡긴 뒤 결과가 오면 응답한다.
    async fn handle_request(
        &mut self,
        client: ClientId,
        id: RequestId,
        request: Request,
    ) -> Result<(), EngineError> {
        let (id, request) = match self.route_deferred(client, id, request).await {
            Deferred::Responded => return Ok(()),
            Deferred::Route(id, request) => (id, request),
        };
        let result = self.dispatch(client, request).await;
        self.respond_with(Responder::Rpc(client, id), result).await;
        Ok(())
    }

    /// 조회 요청은 결과를, 명령 요청은 `None`을 돌려준다. 결과는 응답의 `result`에 실려 요청한 접속에만 간다.
    async fn dispatch(
        &mut self,
        client: ClientId,
        request: Request,
    ) -> Result<Option<QueryResult>, EngineError> {
        self.ensure_router_open(&request)?;
        let result = match request {
            Request::LoadHistory {
                chat,
                before,
                limit,
            } => self.load_history(chat, before, limit).await?,
            Request::PrepareExit { chat } => self.prepare_exit(client, chat).await?,
            Request::Usage { scope, folder } => {
                self.usage_result(client, scope, folder.as_deref()).await?
            }
            Request::LatestChat { folder } => self.latest_chat_result(&folder).await?,
            Request::ListChats { folder } => self.chat_list_result(folder.as_deref()).await?,
            Request::ListTasks => self.task_list_result().await?,
            Request::ListRouterVersions => return Err(unsupported("ListRouterVersions")),
            Request::Prune { yes } => self.prune_records(client, yes).await?,
            command => {
                self.route(client, command).await?;
                return Ok(None);
            }
        };
        Ok(Some(result))
    }

    /// provider 응답을 기다리는 요청은 여기서 맡기고 응답은 나중에 한다. 그 밖의 요청은 `route`로 돌려준다.
    async fn route_deferred(
        &mut self,
        client: ClientId,
        id: RequestId,
        request: Request,
    ) -> Deferred {
        if !matches!(
            request,
            Request::AnswerPermission { .. }
                | Request::AnswerInput { .. }
                | Request::ListModels { .. }
        ) {
            return Deferred::Route(id, request);
        }
        let responder = Responder::Rpc(client, id);
        if let Err(error) = self.ensure_router_open(&request) {
            self.respond(responder, Err(error)).await;
            return Deferred::Responded;
        }
        match request {
            Request::AnswerPermission { request_id, answer } => {
                self.answer_permission(client, request_id, answer, responder)
                    .await;
            }
            Request::AnswerInput { request_id, answer } => {
                self.answer_input(client, request_id, answer, responder)
                    .await;
            }
            Request::ListModels { chat, provider } => {
                self.send_models(client, chat, provider, responder).await;
            }
            _ => {}
        }
        Deferred::Responded
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
            Request::RenameChat { chat, name } => self.rename_chat(chat, &name).await,
            Request::SetChatGroup { chat, group } => {
                self.set_chat_group(chat, group.as_deref()).await
            }
            // 서버가 응답하고 끊김으로 바꿔 여기까지 오지 않는다.
            Request::Detach => Ok(()),
            Request::Version => self.send_version(client).await,
            Request::Shutdown => {
                self.upgrade_requested = true;
                Ok(())
            }
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
            Request::Continue { chat, task } => self.continue_held(chat, task).await,
            Request::ContinueInput { input } => self.continue_input(input).await,
            Request::CloseHeld { chat, task } => self.close_held(chat, task).await,
            // 응답을 기다리는 요청은 `route_deferred`가 맡아 여기까지 오지 않는다.
            Request::AnswerPermission { .. } => Err(unsupported("AnswerPermission")),
            Request::AnswerInput { .. } => Err(unsupported("AnswerInput")),
            Request::ListModels { .. } => Err(unsupported("ListModels")),
            Request::AnswerFeedback { judgment, correct } => {
                self.answer_feedback(judgment, correct).await
            }
            Request::AnswerConstraintAsk { ask, answer } => {
                self.answer_constraint_ask(client, ask, answer).await
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
            Request::SetModel { chat, model } => self.set_model(client, chat, &model).await,
            Request::SetDefaultModel { chat, model } => {
                self.set_default_model(client, chat, &model).await
            }
            Request::SetModelMode { chat, mode } => self.set_model_mode(client, chat, mode).await,
            // TODO(#91): 학습과 router 버전
            Request::Train { .. } => Err(unsupported("Train")),
            Request::ConfirmTrain { .. } => Err(unsupported("ConfirmTrain")),
            Request::UseRouterVersion { .. } => Err(unsupported("UseRouterVersion")),
            Request::ExportJudgments { path } => self.export_judgments(&path).await,
            Request::LoadHistory { .. }
            | Request::PrepareExit { .. }
            | Request::Usage { .. }
            | Request::LatestChat { .. }
            | Request::ListChats { .. }
            | Request::ListTasks
            | Request::ListRouterVersions
            | Request::Prune { .. } => unreachable!("query requests are answered by dispatch"),
        }
    }

    /// router 키를 기다리는 동안은 `Attach`와 `SubmitRouterKey`만 받는다.
    fn ensure_router_open(&self, request: &Request) -> Result<(), EngineError> {
        if let RouterGate::KeyRequired { reason } = &self.router_gate
            && !matches!(
                request,
                Request::Attach { .. }
                    | Request::Version
                    | Request::Shutdown
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
