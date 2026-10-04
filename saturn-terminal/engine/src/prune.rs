//! 기록 정리: `retention.max_age_days`보다 오래 쓰지 않은 채팅의 미리보기와 삭제.
//! 설계: docs/design/records.md#보존과-정리

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{ChatListItem, Notification, PruneSkipReason, PruneSkipped};

use crate::rpc::ClientId;
use crate::store::{PruneOutcome, PruneRequest, PruneScope, RetentionPolicy, SkipReason};
use crate::{AutoPruneNotice, Engine, EngineError};

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

    /// 시작 때 한 번 `retention.auto_prune`이 참이면 오래 쓰지 않은 채팅을 지운다.
    /// 실패해도 시작은 이어 가고, 지웠거나 실패했으면 첫 TUI에 알린다.
    pub(super) async fn auto_prune_on_start(&mut self) {
        let Some(policy) = self.auto_prune_policy().await else {
            return;
        };
        self.notices.auto_prune = self.run_auto_prune(policy).await;
    }

    /// 스위치가 꺼져 있거나 기준 기한이 없으면 `None`.
    async fn auto_prune_policy(&self) -> Option<RetentionPolicy> {
        let policy = self.user_retention().await.ok()?;
        if !policy.auto_prune {
            return None;
        }
        if policy.max_age.is_none() {
            tracing::warn!("자동 정리를 건너뜀: retention.max_age_days가 없음");
            return None;
        }
        Some(policy)
    }

    async fn run_auto_prune(&mut self, policy: RetentionPolicy) -> Option<AutoPruneNotice> {
        let outcome = self.store.prune_on_start(policy, SystemTime::now()).await;
        match outcome {
            Ok(Some(PruneOutcome::Deleted { plan, .. })) => {
                self.forget_pruned(&plan.chats, plan.rows)
            }
            Ok(_) => None,
            Err(error) => {
                tracing::error!(%error, "auto prune failed, starting without deleting");
                Some(AutoPruneNotice::Failed)
            }
        }
    }

    /// 지운 채팅의 메모리 사본을 버리고, 지운 채팅이 있으면 알림을 돌려준다.
    fn forget_pruned(&mut self, deleted: &[ChatId], rows: u64) -> Option<AutoPruneNotice> {
        for chat in deleted {
            self.chats.remove(chat);
            self.chat_dirs.remove(chat);
        }
        if deleted.is_empty() {
            return None;
        }
        let chats = u32::try_from(deleted.len()).unwrap_or(u32::MAX);
        tracing::warn!(chats, rows, "auto prune deleted old chats");
        Some(AutoPruneNotice::Deleted { chats, rows })
    }

    async fn user_retention(&self) -> Result<RetentionPolicy, EngineError> {
        let Some(revision) = self.settings.current() else {
            return Ok(RetentionPolicy::default());
        };
        Ok(self.settings.at(&self.store, revision).await?.retention())
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
