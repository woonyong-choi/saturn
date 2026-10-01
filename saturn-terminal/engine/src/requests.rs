//! 채팅 붙기와 시작 창(judge 키, 폴더 설정 신뢰)에 딸린 요청 처리. 설계: docs/design/engine-lifecycle.md

use std::path::Path;

use saturn_protocol::ids::{ChatId, LedgerSeq, Provider};
use saturn_protocol::rpc::Notification;

use crate::rpc::ClientId;
use crate::secrets::{KeyInput, Masker};
use crate::settings::{Applied, FolderTrustPrompt};
use crate::store::HistoryEntry;
use crate::{Engine, EngineError, JudgeGate, masked_chain};

/// 초안. `LoadHistory` 한 번에 보내는 최대 기록 수.
const MAX_HISTORY: u32 = 500;

impl Engine {
    pub(super) fn start_info(&self, workdir: &Path) -> Notification {
        let active = self.judges.active();
        let judge_version = match self.judge_gate {
            JudgeGate::Open => active.model().to_owned(),
            JudgeGate::KeyRequired { .. } => String::new(),
        };
        Notification::StartInfo {
            saturn_version: env!("CARGO_PKG_VERSION").to_owned(),
            // provider는 첫 입력 때 연결해 버전을 아직 모른다.
            providers: [Provider::Codex, Provider::Claude]
                .into_iter()
                .map(|provider| (provider, String::new()))
                .collect(),
            judge: active.judge_id().to_owned(),
            judge_version,
            folder: workdir.display().to_string(),
        }
    }

    pub(super) async fn history_chunk(
        &self,
        chat: ChatId,
        limit: u32,
    ) -> Result<Notification, EngineError> {
        let (entries, has_more) = self.store.recent_history(chat, limit).await?;
        Ok(Notification::HistoryChunk {
            chat,
            entries: entries.into_iter().map(history_notification).collect(),
            has_more,
        })
    }

    /// 키 창이 떠 있으면 신뢰 창은 키를 받은 뒤 보낸다. TUI 창은 하나씩 뜬다.
    pub(super) async fn send_start_notices(&mut self, client: ClientId) {
        if let Some(applied) = self.notices.settings.take() {
            self.send(client, settings_notification(applied)).await;
        }
        if let JudgeGate::KeyRequired { reason } = &self.judge_gate {
            let reason = reason.clone();
            self.send(client, Notification::JudgeKeyRequired { reason })
                .await;
            return;
        }
        if let Some(prompt) = &self.notices.folder_trust {
            let request = trust_notification(prompt);
            self.send(client, request).await;
        }
    }

    pub(super) async fn send(&self, client: ClientId, notification: Notification) {
        let _ = self.rpc.send_to(client, notification).await; // 끊긴 클라이언트는 건너뛴다
    }

    /// TODO(#110): 이전 기록 요청의 기준 위치 `before`
    pub(super) async fn load_history(
        &self,
        client: ClientId,
        chat: ChatId,
        before: Option<LedgerSeq>,
        limit: u32,
    ) -> Result<(), EngineError> {
        let history = self.history_chunk(chat, limit.min(MAX_HISTORY)).await?;
        self.send(client, history).await;
        Ok(())
    }

    /// 키 원문은 확인과 저장에만 쓰고 로그, 오류, 기록 저장소에 남기지 않는다.
    ///
    /// # Errors
    /// 키를 기다리지 않을 때면 `UnexpectedAnswer`, 다시 확인이 실패하면 `Judges`.
    pub(super) async fn submit_judge_key(
        &mut self,
        client: ClientId,
        key: String,
    ) -> Result<(), EngineError> {
        if self.judge_gate == JudgeGate::Open {
            return Err(EngineError::UnexpectedAnswer { what: "judge key" });
        }
        let accepted = self
            .judges
            .accept_key(KeyInput::Hidden(key), &self.secrets, &self.settings)
            .await;
        if let Err(error) = accepted {
            let reason = masked_chain(&self.masker, &error);
            self.judge_gate = JudgeGate::KeyRequired {
                reason: reason.clone(),
            };
            self.send(client, Notification::JudgeKeyRequired { reason })
                .await;
            return Err(error.into());
        }
        self.masker = Masker::new(self.secrets.lock().await.mask_needles());
        self.judge_gate = JudgeGate::Open;
        tracing::info!("judge key accepted");
        if let Some(prompt) = &self.notices.folder_trust {
            let request = trust_notification(prompt);
            self.send(client, request).await;
        }
        Ok(())
    }

    /// 묻은 경로와 지문만 받는다. 적용을 고르지 않으면 이번 실행 동안 폴더 설정 없이 계속한다.
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
        let asked = self.notices.folder_trust.as_ref().is_some_and(|prompt| {
            prompt.path == Path::new(&path) && prompt.fingerprint == fingerprint
        });
        if !asked {
            return Err(EngineError::UnexpectedAnswer {
                what: "folder trust prompt",
            });
        }
        self.notices.folder_trust = None;
        if !apply {
            return Ok(());
        }
        self.settings
            .trust_folder(Path::new(&path), &fingerprint)
            .await?;
        let (applied, changed) = self.settings.apply_trusted(&self.store, None).await?;
        self.rpc
            .broadcast(None, settings_notification(applied))
            .await;
        if let Some(prompt) = changed {
            self.send(client, trust_notification(&prompt)).await;
            self.notices.folder_trust = Some(prompt);
        }
        Ok(())
    }
}

/// 기록에 없는 작업 글자와 처리 방식은 비워 둔다.
fn history_notification(entry: HistoryEntry) -> Notification {
    match entry {
        HistoryEntry::Input {
            input,
            text,
            state,
            reason,
        } => Notification::InputChanged {
            input,
            text,
            label: None,
            state,
            disposition: None,
            reason,
        },
        HistoryEntry::Event { task, event, .. } => Notification::TaskEvent { task, event },
    }
}

fn settings_notification(applied: Applied) -> Notification {
    Notification::SettingsApplied {
        revision: applied.revision,
        warning: applied.warning,
    }
}

/// 바뀐 줄은 `줄 번호: 내용`.
fn trust_notification(prompt: &FolderTrustPrompt) -> Notification {
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
