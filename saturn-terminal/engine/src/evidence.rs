//! 근거 검색과 원문 조회. 에이전트 작업 안의 `saturn evidence`가 출입증으로 자기 채팅의 기록만 찾고 다시 읽는다.
//! 후보와 원문은 패킷의 경쟁 구역과 같은 도구 호출 기록이고, 순위는 `sessions::ranking`을 그대로 쓴다.
//! 설계: docs/design/context-selection.md#근거-검색과-원문-조회

use std::path::{Path, PathBuf};

use saturn_core::permission::{PermissionTool, Verdict, rules_verdict};
use saturn_core::sessions::evidence::{CandidateSet, EvidenceCandidate, RefError, slice_chars};
use saturn_core::sessions::ranking::{Candidate, rank_candidates};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, LedgerSeq};
use saturn_protocol::rpc::{EVIDENCE_UNREACHABLE_MARKER, EvidenceItem, QueryResult};

use crate::handoff::{ToolRecord, tool_records};
use crate::permission::resolve_links;
use crate::settings::SettingsError;
use crate::store::{LookupKind, LookupOutcome};
use crate::{Engine, EngineError, EvidenceRefusal};

/// 한 번에 돌려주는 후보 수의 상한. 초안.
pub(crate) const MAX_SEARCH_ITEMS: usize = 50;

/// 한 번에 돌려주는 원문 글자 수의 상한. 더 있으면 다음 `offset`을 알린다. 초안.
pub(crate) const MAX_READ_CHARS: usize = 20_000;

/// 한 채팅의 후보. `readable`은 읽기 범위 안 기록이고 `unreadable`은 번호만 가진다.
struct Material {
    project: String,
    readable: Vec<ToolRecord>,
    unreadable: Vec<LedgerSeq>,
    rrf_k: u32,
}

impl Engine {
    /// 출입증을 준 채팅의 후보를 순위 순으로 `limit`개(상한 `MAX_SEARCH_ITEMS`) 돌려준다.
    ///
    /// # Errors
    /// 출입증이 없거나 회수됐으면 `Evidence`, 기록이나 설정을 읽지 못하면 그 오류.
    pub(crate) async fn evidence_search(
        &self,
        pass: &str,
        query: &str,
        limit: u32,
    ) -> Result<QueryResult, EngineError> {
        let chat = self.evidence_chat(pass)?;
        let material = self.evidence_material(chat).await?;
        let set = candidate_set(&material)?;
        let ranking: Vec<Candidate> = material
            .readable
            .iter()
            .map(|record| Candidate {
                seq: record.seq,
                text: record.text.clone(),
                files: record.files.clone(),
            })
            .collect();
        let take = usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .min(MAX_SEARCH_ITEMS);
        let items: Vec<EvidenceItem> = rank_candidates(&ranking, &[], query, material.rrf_k)
            .into_iter()
            .filter_map(|seq| set.get(seq))
            .take(take)
            .map(|candidate| EvidenceItem {
                id: candidate.id,
                at_ms: candidate.at_ms,
                chars: candidate.chars as u64,
                excerpt: candidate.excerpt.clone(),
                hash: candidate.hash.clone(),
            })
            .collect();
        self.note_lookup(
            chat,
            (LookupKind::Search, None),
            (LookupOutcome::Ok, items.len() as u64),
        )
        .await;
        Ok(QueryResult::EvidenceCandidates {
            set_hash: set.hash().to_owned(),
            total: u32::try_from(set.candidates().len()).unwrap_or(u32::MAX),
            items,
        })
    }

    /// 기록 한 건의 원문을 `offset`글자부터 최대 `limit`글자(상한 `MAX_READ_CHARS`) 돌려준다.
    ///
    /// # Errors
    /// 출입증이 없거나 회수됐거나, 번호가 이 채팅에 없거나, 읽기 범위 밖 파일의 기록이거나, 해시가 다르면 `Evidence`.
    pub(crate) async fn evidence_read(
        &self,
        pass: &str,
        (id, hash): (LedgerSeq, Option<&str>),
        (offset, limit): (u64, u64),
    ) -> Result<QueryResult, EngineError> {
        let chat = self.evidence_chat(pass)?;
        let material = self.evidence_material(chat).await?;
        let set = candidate_set(&material)?;
        let refusal = match set.resolve(id, &material.project, hash) {
            Ok(candidate) => {
                let text = material
                    .readable
                    .iter()
                    .find(|record| record.seq == id)
                    .map(|record| record.text.as_str())
                    .unwrap_or_default();
                return self
                    .evidence_record(chat, candidate, text, (offset, limit))
                    .await;
            }
            Err(RefError::Stale) => (EvidenceRefusal::Stale, LookupOutcome::Stale),
            Err(RefError::Unknown) if material.unreadable.contains(&id) => {
                (EvidenceRefusal::Scope, LookupOutcome::Scope)
            }
            Err(RefError::Unknown | RefError::OtherProject) => {
                (EvidenceRefusal::NotFound, LookupOutcome::NotFound)
            }
        };
        self.note_lookup(chat, (LookupKind::Read, Some(id)), (refusal.1, 0))
            .await;
        Err(EngineError::Evidence { refusal: refusal.0 })
    }

    async fn evidence_record(
        &self,
        chat: ChatId,
        candidate: &EvidenceCandidate,
        text: &str,
        (offset, limit): (u64, u64),
    ) -> Result<QueryResult, EngineError> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let length = usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .clamp(1, MAX_READ_CHARS);
        let (slice, next) = slice_chars(text, start, length);
        self.note_lookup(
            chat,
            (LookupKind::Read, Some(candidate.id)),
            (LookupOutcome::Ok, slice.chars().count() as u64),
        )
        .await;
        Ok(QueryResult::EvidenceRecord {
            id: candidate.id,
            hash: candidate.hash.clone(),
            chars: candidate.chars as u64,
            offset,
            next_offset: next.map(|next| next as u64),
            text: slice,
        })
    }

    fn evidence_chat(&self, pass: &str) -> Result<ChatId, EngineError> {
        self.passes.chat_of(pass).ok_or(EngineError::Evidence {
            refusal: EvidenceRefusal::Pass,
        })
    }

    /// 명령이 engine에 닿지 못했다고 알리는 도구 결과면 그 시도를 `Unreachable`로 센다. 요청이 engine에 오지 않은 시도는
    /// 이 표지로만 알 수 있다. 표지가 결과의 첫머리에 있을 때만 읽어, 기록이나 문서 안의 같은 글이 시도로 세어지지 않는다.
    pub(crate) async fn note_unreachable_lookup(&self, chat: ChatId, event: &ProviderEvent) {
        let ProviderEvent::ToolResult { output, .. } = event else {
            return;
        };
        let Some(which) = unreachable_attempt(output) else {
            return;
        };
        self.note_lookup(chat, which, (LookupOutcome::Unreachable, 0))
            .await;
    }

    async fn note_lookup(
        &self,
        chat: ChatId,
        which: (LookupKind, Option<LedgerSeq>),
        result: (LookupOutcome, u64),
    ) {
        let recorded = self.store.record_evidence_lookup(chat, which, result).await;
        self.warn_failure("failed to record an evidence lookup", recorded);
    }

    /// 채팅의 도구 호출 기록을 읽기 범위로 나눈다. 범위는 작업 폴더와 더한 폴더이고, 읽기 규칙이 거부하는 경로는 뺀다.
    async fn evidence_material(&self, chat: ChatId) -> Result<Material, EngineError> {
        let project = self.store.chat_workdir(chat).await?;
        let rows = self.store.ledger_since(chat, LedgerSeq(0)).await?;
        let records = tool_records(&rows);
        let revision = self
            .settings
            .latest_of(chat)
            .ok_or(SettingsError::NoPreviousRevision)?;
        let settings = self.settings.at(&self.store, revision).await?;
        let rules = settings.permission().rules;
        let scope: Vec<PathBuf> = std::iter::once(project.clone())
            .chain(self.chat_dirs_of(chat))
            .map(|dir| resolve_links(&dir))
            .collect();
        let is_readable = |record: &ToolRecord| {
            record.files.iter().all(|file| {
                let path = resolve_links(&project.join(file));
                is_inside(&path, &scope)
                    && rules_verdict(&rules, PermissionTool::Read, &path.to_string_lossy())
                        != Some(Verdict::Deny)
            })
        };
        let (readable, unreadable): (Vec<_>, Vec<_>) =
            records.iter().cloned().partition(is_readable);
        Ok(Material {
            project: project.to_string_lossy().into_owned(),
            readable,
            unreadable: unreadable.into_iter().map(|record| record.seq).collect(),
            rrf_k: settings.rrf_k(),
        })
    }
}

/// 도구 결과의 첫머리가 `Error: <표지> (read 13)`이나 `(search)`이면 그 조회의 종류와 번호.
fn unreachable_attempt(output: &str) -> Option<(LookupKind, Option<LedgerSeq>)> {
    let rest = output
        .trim_start()
        .strip_prefix("Error: ")?
        .strip_prefix(EVIDENCE_UNREACHABLE_MARKER)?
        .strip_prefix(" (")?;
    let what = rest.split_once(')')?.0;
    match what.split_once(' ') {
        None if what == "search" => Some((LookupKind::Search, None)),
        Some(("read", id)) => Some((LookupKind::Read, Some(LedgerSeq(id.parse().ok()?)))),
        _ => None,
    }
}

fn is_inside(path: &Path, scope: &[PathBuf]) -> bool {
    scope.iter().any(|dir| path.starts_with(dir))
}

fn candidate_set(material: &Material) -> Result<CandidateSet, EngineError> {
    let candidates = material
        .readable
        .iter()
        .map(|record| {
            EvidenceCandidate::from_text(record.seq, &material.project, record.at_ms, &record.text)
        })
        .collect();
    CandidateSet::new(candidates).map_err(|_| EngineError::Evidence {
        refusal: EvidenceRefusal::NotFound,
    })
}
