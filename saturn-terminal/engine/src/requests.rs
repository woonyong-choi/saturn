//! 채팅 붙기와 시작 창(router 키, 폴더 설정 신뢰)에 딸린 요청 처리. 설계: docs/design/engine-lifecycle.md

use std::path::{Path, PathBuf};

use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, LedgerSeq, TaskLabel};
use saturn_protocol::rpc::{Alert, ChatNotice, Notification, PROTOCOL_VERSION};
use saturn_protocol::state::TaskState;

use crate::rpc::ClientId;
use crate::secrets::{KeyInput, Masker};
use crate::settings::{Applied, FolderTrustPrompt};
use crate::store::{EventKind, EventReason, HistoryEntry, RunEnd};
use crate::{AutoPruneNotice, Engine, EngineError, RouterGate, masked_chain};

/// 초안. `LoadHistory` 한 번에 보내는 최대 기록 수.
const MAX_HISTORY: u32 = 500;

impl Engine {
    pub(super) fn start_info(&self, workdir: &Path, added_dirs: &[PathBuf]) -> Notification {
        let active = self.routers.active();
        let router_version = match self.router_gate {
            RouterGate::Open => active.model().to_owned(),
            RouterGate::KeyRequired { .. } => String::new(),
        };
        Notification::StartInfo {
            saturn_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: PROTOCOL_VERSION,
            providers: self.provider_infos(),
            router: active.router_id().to_owned(),
            router_version,
            folder: workdir.display().to_string(),
            added_dirs: added_dirs
                .iter()
                .map(|dir| dir.display().to_string())
                .collect(),
        }
    }

    /// `Attach` 전에도 보낸다. 클라이언트는 이 값으로 옛 engine을 교체할지 정한다.
    pub(super) async fn send_version(&self, client: ClientId) -> Result<(), EngineError> {
        let version = Notification::EngineVersion {
            saturn_version: env!("CARGO_PKG_VERSION").to_owned(),
            protocol_version: PROTOCOL_VERSION,
        };
        self.send(client, version).await;
        Ok(())
    }

    /// 붙은 모든 TUI에 engine이 업데이트로 끝난다고 알린다. 끝난 뒤 연결이 끊긴다.
    pub(super) async fn announce_restart(&self) {
        tracing::info!("engine is ending to apply an update");
        self.rpc
            .broadcast_attached(Notification::Alert {
                alert: Alert::EngineRestarting,
            })
            .await;
    }

    pub(super) async fn history_chunk(
        &self,
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    ) -> Result<Notification, EngineError> {
        let page = self.store.history_page(chat, before, limit).await?;
        Ok(Notification::HistoryChunk {
            chat,
            entries: page
                .entries
                .into_iter()
                .flat_map(|entry| self.history_notifications(chat, entry))
                .collect(),
            oldest: page.oldest,
            has_more: page.has_more,
        })
    }

    /// 기록에 없는 작업 글자와 처리 방식은 비운다. 실행은 TUI가 작업 상태를 알도록 시작과 끝 알림으로 감싼다.
    /// 허가와 입력 요청 이벤트는 이미 처리됐거나 접속 때 따로 보내므로 되살리지 않는다.
    /// 실행이 보고한 사용량은 이벤트 뒤에 이어 보내고, 앞 실행과 provider가 다르면 실행 앞에 전환 알림을 둔다.
    fn history_notifications(&self, chat: ChatId, entry: HistoryEntry) -> Vec<Notification> {
        match entry {
            HistoryEntry::Input {
                input,
                text,
                state,
                reason,
            } => vec![Notification::InputChanged {
                input,
                text,
                label: None,
                state,
                disposition: None,
                reason,
            }],
            HistoryEntry::Constraint { kind, reason, rule } => {
                let notice = match kind {
                    EventKind::Added => ChatNotice::ConstraintAdded {
                        rule,
                        unconfirmed: reason == Some(EventReason::Unconfirmed),
                    },
                    EventKind::Released => ChatNotice::ConstraintReleased { rule },
                };
                vec![Notification::ChatNotice {
                    chat,
                    task: None,
                    notice,
                }]
            }
            HistoryEntry::Run {
                task,
                provider,
                end,
                elapsed_ms,
                events,
                usage,
                switched_from,
            } => {
                let label = self.flow.tasks.label(task).unwrap_or(TaskLabel('?'));
                let changed = |state, elapsed_ms| Notification::TaskChanged {
                    task,
                    label,
                    state,
                    provider: Some(provider),
                    elapsed_ms,
                    failure: None,
                };
                let replayed = events
                    .into_iter()
                    .filter(|event| {
                        !matches!(
                            event,
                            ProviderEvent::PermissionRequested { .. }
                                | ProviderEvent::InputRequested { .. }
                        )
                    })
                    .chain(usage.into_iter().map(ProviderEvent::Usage))
                    .map(|event| Notification::TaskEvent { task, event });
                let finished = end.map(|end| {
                    let state = match end {
                        RunEnd::Failed => TaskState::Failed,
                        RunEnd::Completed | RunEnd::Stopped => TaskState::Done,
                    };
                    changed(state, elapsed_ms)
                });
                let switched = switched_from.map(|from| Notification::ChatNotice {
                    chat,
                    task: None,
                    notice: ChatNotice::ProviderSwitched { from, to: provider },
                });
                switched
                    .into_iter()
                    .chain(std::iter::once(changed(TaskState::Running, 0)))
                    .chain(replayed)
                    .chain(finished)
                    .collect()
            }
        }
    }

    /// 키 창이 떠 있으면 신뢰 창은 키를 받은 뒤 보낸다. TUI 창은 하나씩 뜬다.
    /// `applied`는 이 채팅의 설정 병합 결과로, 경고가 있을 때만 보낸다.
    pub(super) async fn send_start_notices(&mut self, client: ClientId, applied: Applied) {
        if std::mem::take(&mut self.notices.restarted) {
            let alert = Alert::EngineRestarted;
            self.send(client, Notification::Alert { alert }).await;
        }
        if let Some(notice) = self.notices.migration.take() {
            let alert = Alert::SchemaMigrated {
                from: notice.from,
                to: notice.to,
            };
            self.send(client, Notification::Alert { alert }).await;
        }
        for change in std::mem::take(&mut self.notices.provider_updates) {
            self.send(
                client,
                Notification::Alert {
                    alert: change.alert(),
                },
            )
            .await;
        }
        if let Some(notice) = self.notices.auto_prune.take() {
            let alert = match notice {
                AutoPruneNotice::Deleted { chats, rows } => Alert::AutoPruned { chats, rows },
                AutoPruneNotice::Failed => Alert::AutoPruneFailed,
            };
            self.send(client, Notification::Alert { alert }).await;
        }
        if applied.warning.is_some() {
            self.send(client, settings_notification(applied)).await;
        }
        if let RouterGate::KeyRequired { reason } = &self.router_gate {
            let reason = reason.clone();
            self.send(client, Notification::RouterKeyRequired { reason })
                .await;
            return;
        }
        self.send_folder_trust(client).await;
    }

    async fn send_folder_trust(&self, client: ClientId) {
        let prompt = self
            .attachments
            .get(&client)
            .and_then(|attachment| attachment.folder_trust.as_ref());
        if let Some(prompt) = prompt {
            self.send(client, trust_notification(prompt)).await;
        }
    }

    pub(super) async fn send(&self, client: ClientId, notification: Notification) {
        let _ = self.rpc.send_to(client, notification).await; // 끊긴 클라이언트는 건너뛴다
    }

    /// `before`는 앞서 받은 묶음의 `oldest`다. engine은 TUI별 위치를 기억하지 않는다.
    pub(super) async fn load_history(
        &self,
        client: ClientId,
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    ) -> Result<(), EngineError> {
        let history = self
            .history_chunk(chat, before, limit.min(MAX_HISTORY))
            .await?;
        self.send(client, history).await;
        Ok(())
    }

    /// 키 원문은 확인과 저장에만 쓰고 로그, 오류, 기록 저장소에 남기지 않는다.
    ///
    /// # Errors
    /// 키를 기다리지 않을 때면 `UnexpectedAnswer`, 다시 확인이 실패하면 `Routers`.
    pub(super) async fn submit_router_key(
        &mut self,
        client: ClientId,
        key: String,
    ) -> Result<(), EngineError> {
        if self.router_gate == RouterGate::Open {
            return Err(EngineError::UnexpectedAnswer { what: "router key" });
        }
        let accepted = self
            .routers
            .accept_key(KeyInput::Hidden(key), &self.secrets, &self.settings)
            .await;
        if let Err(error) = accepted {
            let reason = masked_chain(&self.masker, &error);
            self.router_gate = RouterGate::KeyRequired {
                reason: reason.clone(),
            };
            self.send(client, Notification::RouterKeyRequired { reason })
                .await;
            return Err(error.into());
        }
        self.masker = Masker::new(self.secrets.lock().await.mask_needles());
        self.router_gate = RouterGate::Open;
        tracing::info!("router key accepted");
        self.send_folder_trust(client).await;
        Ok(())
    }

    /// 그 TUI에 묻은 경로와 지문만 받는다. 적용을 고르지 않으면 그 채팅은 폴더 설정 없이 계속한다.
    /// 적용하면 그 채팅의 작업 폴더로 다시 병합해 같은 채팅에 붙은 모든 TUI에 `SettingsApplied`를 보낸다.
    ///
    /// # Errors
    /// 묻지 않은 경로나 지문이면 `UnexpectedAnswer`.
    pub(super) async fn answer_folder_trust(
        &mut self,
        client: ClientId,
        path: String,
        fingerprint: String,
        apply: bool,
    ) -> Result<(), EngineError> {
        let asked = self
            .attachments
            .get(&client)
            .and_then(|attachment| {
                attachment
                    .folder_trust
                    .as_ref()
                    .map(|prompt| (attachment.chat, prompt))
            })
            .filter(|(_, prompt)| {
                prompt.path == Path::new(&path) && prompt.fingerprint == fingerprint
            })
            .map(|(chat, _)| chat);
        let Some(chat) = asked else {
            return Err(EngineError::UnexpectedAnswer {
                what: "folder trust prompt",
            });
        };
        self.set_folder_trust(client, None);
        if !apply {
            return Ok(());
        }
        self.settings
            .trust_folder(Path::new(&path), &fingerprint)
            .await?;
        let workdir = self
            .chat_env(chat)
            .expect("attached chat should have an environment")
            .workdir()
            .to_path_buf();
        let (applied, changed) = self
            .settings
            .apply_trusted(&self.store, Some(chat), &workdir)
            .await?;
        let peers: Vec<ClientId> = self
            .attachments
            .iter()
            .filter(|(_, attachment)| attachment.chat == chat)
            .map(|(peer, _)| *peer)
            .collect();
        for peer in peers {
            self.send(peer, settings_notification(applied.clone()))
                .await;
        }
        if let Some(prompt) = changed {
            self.send(client, trust_notification(&prompt)).await;
            self.set_folder_trust(client, Some(prompt));
        }
        Ok(())
    }

    pub(crate) fn set_folder_trust(&mut self, client: ClientId, prompt: Option<FolderTrustPrompt>) {
        if let Some(attachment) = self.attachments.get_mut(&client) {
            attachment.folder_trust = prompt;
        }
    }
}

pub(crate) fn settings_notification(applied: Applied) -> Notification {
    Notification::SettingsApplied {
        revision: applied.revision,
        warning: applied.warning,
        keymap: applied.keymap,
    }
}

/// 바뀐 줄은 `줄 번호: 내용`.
pub(crate) fn trust_notification(prompt: &FolderTrustPrompt) -> Notification {
    Notification::FolderTrustRequested {
        path: prompt.path.display().to_string(),
        fingerprint: prompt.fingerprint.clone(),
        applied: prompt.applied.clone(),
        ignored: prompt.ignored.clone(),
        changed_lines: prompt
            .changed_lines
            .iter()
            .map(|(line, text)| format!("{line}: {text}"))
            .collect(),
    }
}
