//! 기록 정리: `retention.max_age_days`보다 오래 쓰지 않은 채팅의 미리보기와 삭제.
//! 설계: docs/design/records.md#보존과-정리

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ChatListItem, Notification, PruneSkipReason, PruneSkipped};

use crate::rpc::ClientId;
use crate::store::{PruneOutcome, PruneRequest, PruneScope, SkipReason};
use crate::{Engine, EngineError};

impl Engine {
    /// `yes`가 거짓이면 아무것도 지우지 않고 `PrunePreview`를, 참이면 지우고 `Pruned`를 보낸다.
    /// TUI가 붙어 있는 채팅은 열린 항목이 없어도 지우지 않고 `Attached`로 남긴다.
    ///
    /// # Errors
    /// 정리 기준 설정이 없으면 `NoRetention`, 기록 저장소 실패면 `Store`나 `Settings`.
    pub(super) async fn prune_records(
        &mut self,
        client: ClientId,
        yes: bool,
    ) -> Result<(), EngineError> {
        let before = self.prune_cutoff().await?;
        let plan = self
            .store
            .plan_prune(&PruneScope::InactiveBefore(before))
            .await?;
        let attached: HashSet<ChatId> = self.attachments.values().map(|a| a.chat).collect();
        let (kept, candidates): (Vec<ChatId>, Vec<ChatId>) =
            plan.chats.iter().partition(|chat| attached.contains(chat));
        let summaries = self.chat_summaries(&candidates).await?;
        let mut skipped = skipped_of(&plan.skipped);
        skipped.extend(kept.into_iter().map(|chat| PruneSkipped {
            chat,
            reasons: vec![PruneSkipReason::Attached],
        }));
        skipped.sort_by_key(|item| item.chat);
        if !yes {
            let notification = Notification::PrunePreview {
                chats: summaries,
                skipped,
                rows: self.rows_of(&candidates).await?,
            };
            self.send(client, notification).await;
            return Ok(());
        }
        let request = PruneRequest {
            scope: PruneScope::Chats(candidates),
            yes: true,
        };
        let PruneOutcome::Deleted { plan: done, .. } = self.store.prune(&request).await? else {
            return Ok(());
        };
        for chat in &done.chats {
            self.chats.remove(chat);
            self.chat_dirs.remove(chat);
        }
        skipped.extend(skipped_of(&done.skipped));
        skipped.sort_by_key(|item| item.chat);
        skipped.dedup_by_key(|item| item.chat);
        let deleted: HashSet<ChatId> = done.chats.iter().copied().collect();
        let notification = Notification::Pruned {
            chats: summaries
                .into_iter()
                .filter(|item| deleted.contains(&item.chat))
                .collect(),
            skipped,
            rows: done.rows,
        };
        self.send(client, notification).await;
        Ok(())
    }

    /// 이 시각보다 오래 쓰지 않은 채팅이 정리 대상이다. 사용자 설정의 `retention.max_age_days`로 정한다.
    async fn prune_cutoff(&self) -> Result<SystemTime, EngineError> {
        let revision = self.settings.current().ok_or(EngineError::NoRetention)?;
        let max_age = self
            .settings
            .at(&self.store, revision)
            .await?
            .retention()
            .max_age
            .ok_or(EngineError::NoRetention)?;
        Ok(SystemTime::now()
            .checked_sub(max_age)
            .unwrap_or(SystemTime::UNIX_EPOCH))
    }

    async fn chat_summaries(&self, chats: &[ChatId]) -> Result<Vec<ChatListItem>, EngineError> {
        let wanted: HashSet<ChatId> = chats.iter().copied().collect();
        let mut found: HashMap<ChatId, ChatListItem> = self
            .store
            .list_chats(None)
            .await?
            .into_iter()
            .filter(|item| wanted.contains(&item.chat))
            .map(|item| (item.chat, item))
            .collect();
        Ok(chats.iter().filter_map(|chat| found.remove(chat)).collect())
    }

    /// 지울 채팅의 행 수. 판단 기록과 설정 스냅샷은 세지 않는다.
    async fn rows_of(&self, chats: &[ChatId]) -> Result<u64, EngineError> {
        let plan = self
            .store
            .plan_prune(&PruneScope::Chats(chats.to_vec()))
            .await?;
        Ok(plan.rows)
    }
}

fn skipped_of(skipped: &[(ChatId, Vec<SkipReason>)]) -> Vec<PruneSkipped> {
    skipped
        .iter()
        .map(|(chat, reasons)| PruneSkipped {
            chat: *chat,
            reasons: reasons.iter().map(|reason| reason_of(*reason)).collect(),
        })
        .collect()
}

fn reason_of(reason: SkipReason) -> PruneSkipReason {
    match reason {
        SkipReason::OpenInput => PruneSkipReason::OpenInput,
        SkipReason::OpenRun => PruneSkipReason::OpenRun,
        SkipReason::PendingStop => PruneSkipReason::PendingStop,
        SkipReason::ActiveSession => PruneSkipReason::ActiveSession,
        SkipReason::WaitingSession => PruneSkipReason::WaitingSession,
    }
}
