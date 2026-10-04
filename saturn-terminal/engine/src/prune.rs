//! 기록 정리: `retention.max_age_days`보다 오래 쓰지 않은 채팅의 미리보기와 삭제.
//! 설계: docs/design/records.md#보존과-정리

use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::time::{Duration, Instant, SystemTime};

use sha2::{Digest, Sha256};

use saturn_protocol::ids::ChatId;
use saturn_protocol::rpc::{
    Alert, ChatListItem, Notification, PruneSkipReason, PruneSkipped, QueryResult,
};

use crate::rpc::ClientId;
use crate::store::{
    PruneOutcome, PrunePlan, PruneRequest, PruneScope, RetentionPolicy, SkipReason,
};
use crate::{AutoPruneNotice, Engine, EngineError};

/// 미리보기마다 남기는 확인 번호의 유효 시간. 미리보기를 보고 결정하는 시간보다 길고 방치한 번호가 오래 남지 않을 만큼 짧다.
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);
/// 한꺼번에 남겨 두는 미리보기 수. 넘으면 가장 오래된 것부터 버린다.
const PLAN_LIMIT: usize = 8;

#[derive(Debug)]
struct SavedPlan {
    id: String,
    chats: Vec<ChatId>,
    saved_at: Instant,
}

/// 정리 미리보기가 정한 채팅 목록을 확인 번호로 기억한다. 확인 때 그 목록 밖의 채팅은 지우지 않는다.
/// 번호는 한 번만 쓸 수 있고 접속과 무관하게 engine 안에서만 유효하다. `saturn prune`의 미리보기와 `--yes`가
/// 서로 다른 접속이기 때문이다.
#[derive(Debug, Default)]
pub(crate) struct PrunePlans {
    issued: u64,
    saved: Vec<SavedPlan>,
}

impl PrunePlans {
    /// 새 번호를 만들어 `chats`와 함께 기억한다.
    pub(crate) fn save(&mut self, chats: Vec<ChatId>) -> String {
        self.save_at(chats, Instant::now())
    }

    fn save_at(&mut self, chats: Vec<ChatId>, now: Instant) -> String {
        self.saved
            .retain(|plan| now.saturating_duration_since(plan.saved_at) < PLAN_TTL);
        self.issued += 1;
        let id = plan_id(self.issued);
        self.saved.push(SavedPlan {
            id: id.clone(),
            chats,
            saved_at: now,
        });
        if self.saved.len() > PLAN_LIMIT {
            self.saved.remove(0);
        }
        id
    }

    /// 번호의 채팅 목록을 꺼내고 번호를 지운다. 모르거나 만료됐거나 이미 썼으면 `None`.
    pub(crate) fn take(&mut self, id: &str) -> Option<Vec<ChatId>> {
        self.take_at(id, Instant::now())
    }

    fn take_at(&mut self, id: &str, now: Instant) -> Option<Vec<ChatId>> {
        let index = self.saved.iter().position(|plan| plan.id == id)?;
        let plan = self.saved.remove(index);
        (now.saturating_duration_since(plan.saved_at) < PLAN_TTL).then_some(plan.chats)
    }
}

/// 맞히기 어려운 번호. 프로세스마다 다른 해시 키, 발급 순서, 시각을 섞어 앞 16바이트를 쓴다.
fn plan_id(issued: u64) -> String {
    let random = RandomState::new();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut seed = Vec::new();
    for round in 0..2_u64 {
        seed.extend_from_slice(&random.hash_one((issued, nanos, round)).to_le_bytes());
    }
    seed.extend_from_slice(&nanos.to_le_bytes());
    seed.extend_from_slice(&std::process::id().to_le_bytes());
    Sha256::digest(&seed)[..16]
        .iter()
        .fold(String::new(), |mut text, byte| {
            use std::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
            text
        })
}

impl Engine {
    /// `yes`가 거짓이면 아무것도 지우지 않고 지울 채팅과 확인 번호를 담은 `PrunePreview`를, 참이면 지우고 `Pruned`를
    /// 결과로 돌려준다. `plan`이 있으면 그 미리보기에 있던 채팅 가운데 지금도 지울 수 있는 것만 지우고, 없으면
    /// 요청 순간의 기준으로 정한 대상을 지운다. TUI가 붙어 있는 채팅은 열린 항목이 없어도 지우지 않고 `Attached`로 남긴다.
    ///
    /// # Errors
    /// 정리 기준 설정이 없으면 `NoRetention`, 모르는 `plan`이면 `PrunePlanUnknown`, 기록 저장소 실패면 `Store`나 `Settings`.
    pub(super) async fn prune_records(
        &mut self,
        client: ClientId,
        yes: bool,
        plan: Option<String>,
    ) -> Result<QueryResult, EngineError> {
        let before = self.prune_cutoff_or_tell(client).await?;
        let previewed = match (yes, plan) {
            (true, Some(id)) => Some(
                self.flow
                    .prune_plans
                    .take(&id)
                    .ok_or(EngineError::PrunePlanUnknown)?,
            ),
            _ => None,
        };
        let found = self
            .store
            .plan_prune(&PruneScope::InactiveBefore(before))
            .await?;
        let attached: HashSet<ChatId> = self.attachments.values().map(|a| a.chat).collect();
        let (kept, eligible): (Vec<ChatId>, Vec<ChatId>) =
            found.chats.iter().partition(|chat| attached.contains(chat));
        let mut skipped = skipped_of(&found.skipped);
        skipped.extend(kept.into_iter().map(|chat| PruneSkipped {
            chat,
            reasons: vec![PruneSkipReason::Attached],
        }));
        let candidates = match &previewed {
            Some(previewed) => {
                skipped.retain(|item| previewed.contains(&item.chat));
                let (still, gone): (Vec<ChatId>, Vec<ChatId>) =
                    previewed.iter().partition(|chat| eligible.contains(chat));
                let known: HashSet<ChatId> = skipped.iter().map(|item| item.chat).collect();
                for chat in gone {
                    if !known.contains(&chat) && self.store.chat_workdir(chat).await.is_ok() {
                        skipped.push(PruneSkipped {
                            chat,
                            reasons: vec![PruneSkipReason::UsedSincePreview],
                        });
                    }
                }
                still
            }
            None => eligible,
        };
        skipped.sort_by_key(|item| item.chat);
        let summaries = self.chat_summaries(&candidates).await?;
        if !yes {
            let counted = self.count_rows(&candidates).await?;
            let plan = self.flow.prune_plans.save(candidates);
            return Ok(QueryResult::PrunePreview {
                chats: with_rows(summaries, &counted.chat_rows),
                skipped,
                rows: counted.rows,
                plan,
            });
        }
        let request = PruneRequest {
            scope: PruneScope::Chats(candidates),
            yes: true,
        };
        let PruneOutcome::Deleted { plan: done, .. } = self.store.prune(&request).await? else {
            return Ok(QueryResult::Pruned {
                chats: Vec::new(),
                skipped,
                rows: 0,
            });
        };
        for chat in &done.chats {
            self.chats.remove(chat);
            self.chat_dirs.remove(chat);
        }
        skipped.extend(skipped_of(&done.skipped));
        skipped.sort_by_key(|item| item.chat);
        skipped.dedup_by_key(|item| item.chat);
        let deleted: HashSet<ChatId> = done.chats.iter().copied().collect();
        Ok(QueryResult::Pruned {
            chats: with_rows(
                summaries
                    .into_iter()
                    .filter(|item| deleted.contains(&item.chat))
                    .collect(),
                &done.chat_rows,
            ),
            skipped,
            rows: done.rows,
        })
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
    async fn count_rows(&self, chats: &[ChatId]) -> Result<PrunePlan, EngineError> {
        Ok(self
            .store
            .plan_prune(&PruneScope::Chats(chats.to_vec()))
            .await?)
    }

    /// 기준 설정이 없으면 요청한 TUI가 안내를 보일 수 있게 알리고 거절한다.
    async fn prune_cutoff_or_tell(&self, client: ClientId) -> Result<SystemTime, EngineError> {
        let cutoff = self.prune_cutoff().await;
        if matches!(cutoff, Err(EngineError::NoRetention)) {
            let alert = Alert::PruneNeedsRetention;
            self.send(client, Notification::Alert { alert }).await;
        }
        cutoff
    }
}

fn with_rows(mut chats: Vec<ChatListItem>, rows: &[(ChatId, u64)]) -> Vec<ChatListItem> {
    for item in &mut chats {
        item.rows = rows
            .iter()
            .find(|(chat, _)| *chat == item.chat)
            .map(|(_, count)| *count);
    }
    chats
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_plans_expire_are_single_use_and_keep_a_bounded_number() {
        let mut plans = PrunePlans::default();
        let start = Instant::now();
        let first = plans.save_at(vec![ChatId(1)], start);
        let second = plans.save_at(vec![ChatId(2)], start);
        assert_ne!(first, second);
        assert_eq!(plans.take_at(&first, start), Some(vec![ChatId(1)]));
        assert_eq!(plans.take_at(&first, start), None);
        assert_eq!(plans.take_at(&second, start + PLAN_TTL), None);

        let ids: Vec<String> = (0..PLAN_LIMIT as u64 + 1)
            .map(|n| plans.save_at(vec![ChatId(n)], start))
            .collect();
        assert_eq!(plans.take_at(&ids[0], start), None);
        assert_eq!(plans.take_at(&ids[1], start), Some(vec![ChatId(1)]));
    }
}
