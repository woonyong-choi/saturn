//! 근거 검색과 원문 조회. 에이전트 작업 안의 `saturn evidence`가 출입증으로 자기 채팅의 기록만 찾고 다시 읽는다.
//! 후보와 원문은 도구 호출 기록(패킷의 경쟁 구역과 같은 것), 저장된 사용자 입력, 메인 에이전트가 보인 글이고,
//! 순위는 `sessions::ranking`을 그대로 쓴다. 번호는 종류마다 따로 세므로 종류와 채팅을 함께 적은 참조로 읽는다.
//! 설계: docs/design/context-selection.md#근거-검색과-원문-조회

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use saturn_core::permission::{PermissionTool, Verdict, rules_verdict};
use saturn_core::sessions::evidence::{CandidateSet, EvidenceCandidate, RefError, slice_chars};
use saturn_core::sessions::ranking::{Candidate, rank_candidates};
use saturn_protocol::event::ProviderEvent;
use saturn_protocol::ids::{ChatId, LedgerSeq, RunId};
use saturn_protocol::rpc::{EVIDENCE_UNREACHABLE_MARKER, EvidenceItem, EvidenceKind, QueryResult};

use crate::handoff::{ToolRecord, tool_records};
use crate::permission::resolve_links;
use crate::settings::SettingsError;
use crate::store::{LedgerRow, LookupKind, LookupOutcome, SteeredInput, sha256_hex};
use crate::{Engine, EngineError, EvidenceRefusal};

/// 한 번에 돌려주는 후보 수의 상한. 초안.
pub(crate) const MAX_SEARCH_ITEMS: usize = 50;

/// 한 번에 돌려주는 원문 글자 수의 상한. 더 있으면 다음 `offset`을 알린다. 초안.
pub(crate) const MAX_READ_CHARS: usize = 20_000;

/// 읽을 기록 한 건의 지정. `kind`가 없으면 옛 형식이라 도구 호출 기록 번호이고 `chat`은 보지 않는다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EvidenceTarget<'a> {
    pub(crate) kind: Option<EvidenceKind>,
    pub(crate) chat: Option<ChatId>,
    pub(crate) id: LedgerSeq,
    pub(crate) hash: Option<&'a str>,
}

impl<'a> EvidenceTarget<'a> {
    /// 옛 형식: 도구 호출 기록 번호와 선택 해시.
    pub(crate) fn tool(id: LedgerSeq, hash: Option<&'a str>) -> Self {
        Self {
            kind: None,
            chat: None,
            id,
            hash,
        }
    }
}

/// 후보가 될 원문 하나. 번호는 `kind` 안에서만 겹치지 않는다.
struct Record {
    kind: EvidenceKind,
    id: u64,
    /// 시간 순서의 자리. 기록 번호 기준이고 같은 자리에서는 입력, 도구, 글, 끼워 넣은 입력 순이다.
    anchor: u64,
    at_ms: i64,
    text: String,
    files: Vec<String>,
}

impl Record {
    fn order(&self) -> (u64, u8) {
        let rank = match self.kind {
            EvidenceKind::Input => 0,
            EvidenceKind::Tool => 1,
            EvidenceKind::Text => 2,
            EvidenceKind::Steer => 3,
        };
        (self.anchor, rank)
    }
}

/// 한 채팅의 후보. `readable`은 읽기 범위 안 기록이고 `unreadable`은 종류와 번호만 가진다.
struct Material {
    project: String,
    readable: Vec<Record>,
    unreadable: Vec<(EvidenceKind, u64)>,
    rrf_k: u32,
    /// 후보를 읽은 때 채팅 기록의 마지막 번호. 채팅 revision이다.
    tail: LedgerSeq,
}

impl Material {
    /// 질문과 맞는 순서의 읽을 수 있는 기록. 종류마다 번호가 겹치므로 순위는 시간순 자리(`index`)를 번호로 삼아 매긴다. 순위는 크기 순서만 쓴다.
    fn ranked(&self, query: &str) -> Vec<&Record> {
        let ranking: Vec<Candidate> = self
            .readable
            .iter()
            .enumerate()
            .map(|(index, record)| Candidate {
                seq: LedgerSeq(index as u64),
                text: record.text.clone(),
                files: record.files.clone(),
            })
            .collect();
        rank_candidates(&ranking, &[], query, self.rrf_k)
            .into_iter()
            .filter_map(|slot| self.readable.get(usize::try_from(slot.0).ok()?))
            .collect()
    }
}

/// 관련 원문 선주입이 순위로 본 기록 한 건. 종류, 번호, 원문 해시가 근거 검색 후보와 같고 원문 전체를 가진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelatedFound {
    pub(crate) kind: EvidenceKind,
    pub(crate) id: u64,
    pub(crate) hash: String,
    pub(crate) text: String,
}

/// 관련 원문 검색의 결과. `ranked`는 순위 순서이고 `tail`은 후보를 읽은 때의 채팅 revision이다.
#[derive(Debug, Clone)]
pub(crate) struct RelatedSearch {
    pub(crate) chat: ChatId,
    pub(crate) tail: LedgerSeq,
    pub(crate) ranked: Vec<RelatedFound>,
}

/// 종류마다 하나인 후보 집합. 번호가 종류 안에서만 겹치지 않으므로 집합을 나눈다.
struct Sets(Vec<(EvidenceKind, CandidateSet)>);

impl Sets {
    fn of(&self, kind: EvidenceKind) -> Option<&CandidateSet> {
        self.0
            .iter()
            .find(|(owner, _)| *owner == kind)
            .map(|(_, set)| set)
    }

    fn total(&self) -> usize {
        self.0.iter().map(|(_, set)| set.candidates().len()).sum()
    }

    /// 후보가 있는 종류의 집합 해시를 종류 이름과 함께 이은 값의 해시.
    fn hash(&self) -> String {
        let joined: String = self
            .0
            .iter()
            .filter(|(_, set)| !set.candidates().is_empty())
            .map(|(kind, set)| format!("{}\0{}\n", kind.name(), set.hash()))
            .collect();
        sha256_hex(joined.as_bytes())
    }
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
        let sets = candidate_sets(&material)?;
        let take = usize::try_from(limit)
            .unwrap_or(usize::MAX)
            .min(MAX_SEARCH_ITEMS);
        let items: Vec<EvidenceItem> = material
            .ranked(query)
            .into_iter()
            .filter_map(|record| {
                let candidate = sets.of(record.kind)?.get(LedgerSeq(record.id))?;
                Some(EvidenceItem {
                    id: candidate.id,
                    kind: record.kind,
                    chat: Some(chat),
                    at_ms: candidate.at_ms,
                    chars: candidate.chars as u64,
                    excerpt_start: 0,
                    excerpt_end: candidate.excerpt.chars().count() as u64,
                    excerpt: candidate.excerpt.clone(),
                    hash: candidate.hash.clone(),
                })
            })
            .take(take)
            .collect();
        self.note_lookup(
            chat,
            (LookupKind::Search, None),
            (LookupOutcome::Ok, items.len() as u64),
        )
        .await;
        Ok(QueryResult::EvidenceCandidates {
            set_hash: sets.hash(),
            total: u32::try_from(sets.total()).unwrap_or(u32::MAX),
            items,
        })
    }

    /// 기록 한 건의 원문을 `offset`글자부터 최대 `limit`글자(상한 `MAX_READ_CHARS`) 돌려준다.
    /// 읽을 때마다 출입증, 채팅 범위, 현재 읽기 권한, 해시를 다시 본다.
    ///
    /// # Errors
    /// 출입증이 없거나 회수됐거나, 번호가 이 채팅에 없거나(종류가 있으면 채팅도 같아야 한다), 읽기 범위 밖 파일의 기록이거나,
    /// 해시가 다르거나(종류가 있으면 해시가 필수다) 비밀이 든 글이면 `Evidence`.
    pub(crate) async fn evidence_read(
        &self,
        pass: &str,
        target: EvidenceTarget<'_>,
        (offset, limit): (u64, u64),
    ) -> Result<QueryResult, EngineError> {
        let chat = self.evidence_chat(pass)?;
        let id = target.id;
        let kind = target.kind.unwrap_or_default();
        let typed_refusal = match (target.kind, target.chat, target.hash) {
            (None, _, _) => None,
            (Some(_), chat_of_ref, _) if chat_of_ref != Some(chat) => {
                Some((EvidenceRefusal::NotFound, LookupOutcome::NotFound))
            }
            (Some(_), _, None) => Some((EvidenceRefusal::Stale, LookupOutcome::Stale)),
            _ => None,
        };
        if let Some(refusal) = typed_refusal {
            return self.evidence_refused(chat, id, refusal).await;
        }
        let material = self.evidence_material(chat).await?;
        let sets = candidate_sets(&material)?;
        let resolved = match sets.of(kind) {
            Some(set) => set.resolve(id, &material.project, target.hash),
            None => Err(RefError::Unknown),
        };
        let refusal = match resolved {
            Ok(candidate) => {
                let text = material
                    .readable
                    .iter()
                    .find(|record| record.kind == kind && record.id == id.0)
                    .map(|record| record.text.as_str())
                    .unwrap_or_default();
                return self
                    .evidence_record(chat, (kind, candidate), text, (offset, limit))
                    .await;
            }
            Err(RefError::Stale) => (EvidenceRefusal::Stale, LookupOutcome::Stale),
            Err(RefError::Unknown) if material.unreadable.contains(&(kind, id.0)) => {
                (EvidenceRefusal::Scope, LookupOutcome::Scope)
            }
            Err(RefError::Unknown | RefError::OtherProject) => {
                (EvidenceRefusal::NotFound, LookupOutcome::NotFound)
            }
        };
        self.evidence_refused(chat, id, refusal).await
    }

    /// 새 session의 첫 작업 입력 앞에 미리 넣을 후보를 근거 검색과 같은 후보 집합, 같은 권한 범위, 같은 순위로 `query`에 맞춰 돌려준다.
    /// 출입증 없이 engine 안에서 부르므로 `chat`은 호출한 쪽이 입력의 채팅으로 정한다.
    ///
    /// # Errors
    /// 기록이나 설정을 읽지 못하면 그 오류.
    pub(crate) async fn related_search(
        &self,
        chat: ChatId,
        query: &str,
    ) -> Result<RelatedSearch, EngineError> {
        let material = self.evidence_material(chat).await?;
        let sets = candidate_sets(&material)?;
        let ranked = material
            .ranked(query)
            .into_iter()
            .filter_map(|record| {
                let candidate = sets.of(record.kind)?.get(LedgerSeq(record.id))?;
                Some(RelatedFound {
                    kind: record.kind,
                    id: record.id,
                    hash: candidate.hash.clone(),
                    text: record.text.clone(),
                })
            })
            .collect();
        Ok(RelatedSearch {
            chat,
            tail: material.tail,
            ranked,
        })
    }

    /// 고른 기록을 적용 직전에 현재 기록과 권한으로 다시 맞춘다. 순서는 `picked`와 같고 `None`이면 지금도 같은 원문이다.
    /// 읽기 범위 밖이면 `scope`, 기록에서 사라졌으면 `deleted`다. 읽을 수 있는 기록의 채팅 revision이
    /// `tail`과 다르면 `stale_revision`, 원문이 바뀌었으면 `changed`다.
    ///
    /// # Errors
    /// 기록이나 설정을 읽지 못하면 그 오류.
    pub(crate) async fn related_recheck(
        &self,
        chat: ChatId,
        tail: LedgerSeq,
        picked: &[(EvidenceKind, u64, &str)],
    ) -> Result<Vec<Option<&'static str>>, EngineError> {
        let material = self.evidence_material(chat).await?;
        let sets = candidate_sets(&material)?;
        Ok(picked
            .iter()
            .map(|(kind, id, hash)| {
                if material.unreadable.contains(&(*kind, *id)) {
                    return Some("scope");
                }
                match sets.of(*kind).and_then(|set| set.get(LedgerSeq(*id))) {
                    Some(_) if material.tail != tail => Some("stale_revision"),
                    Some(candidate) if candidate.hash == *hash => None,
                    Some(_) => Some("changed"),
                    None => Some("deleted"),
                }
            })
            .collect())
    }

    async fn evidence_refused(
        &self,
        chat: ChatId,
        id: LedgerSeq,
        (refusal, outcome): (EvidenceRefusal, LookupOutcome),
    ) -> Result<QueryResult, EngineError> {
        self.note_lookup(chat, (LookupKind::Read, Some(id)), (outcome, 0))
            .await;
        Err(EngineError::Evidence { refusal })
    }

    async fn evidence_record(
        &self,
        chat: ChatId,
        (kind, candidate): (EvidenceKind, &EvidenceCandidate),
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
            kind,
            chat: Some(chat),
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

    /// 채팅의 도구 호출, 사용자 입력, 메인 에이전트 글을 읽기 범위로 나눈다. 도구 호출의 범위는 작업 폴더와 더한 폴더이고,
    /// 읽기 규칙이 거부하는 경로는 뺀다. 입력과 글은 파일을 가리키지 않아 파일 규칙을 받지 않고, router 키나 출입증이 든 글은 뺀다.
    /// 읽을 때마다 현재 설정으로 다시 나눈다.
    async fn evidence_material(&self, chat: ChatId) -> Result<Material, EngineError> {
        let project = self.store.chat_workdir(chat).await?;
        let rows = self.store.ledger_since(chat, LedgerSeq(0)).await?;
        let steers = self.store.steered_inputs(chat).await?;
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
        // 비밀이 든 도구 기록도 대화 본문과 같이 후보에서 완전히 뺀다.
        let (readable_tools, unreadable_tools): (Vec<_>, Vec<_>) = records
            .into_iter()
            .filter(|record| self.masker.mask(&record.text).as_str() == record.text)
            .partition(is_readable);
        let mut unreadable: Vec<(EvidenceKind, u64)> = unreadable_tools
            .into_iter()
            .map(|record| (EvidenceKind::Tool, record.seq.0))
            .collect();
        let mut readable: Vec<Record> = readable_tools
            .into_iter()
            .map(|record| Record {
                kind: EvidenceKind::Tool,
                id: record.seq.0,
                anchor: record.seq.0,
                at_ms: record.at_ms,
                text: record.text,
                files: record.files,
            })
            .collect();
        // 비밀이 든 글은 있다는 사실도 알리지 않으려 `unreadable`에 넣지 않는다
        readable.extend(
            dialogue_records(&rows, &steers)
                .into_iter()
                .filter(|record| self.masker.mask(&record.text).as_str() == record.text),
        );
        readable.sort_by_key(Record::order);
        unreadable.sort();
        Ok(Material {
            project: project.to_string_lossy().into_owned(),
            readable,
            unreadable,
            rrf_k: settings.rrf_k(),
            tail: rows.last().map_or(LedgerSeq(0), |row| row.seq),
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

// cost: time O(r + s·r), heap O(L), stack O(1)
// vars: r = 기록 행 수, s = 끼워 넣은 입력 수, L = 대화 글자 수
// basis: estimate
/// 패킷의 대화 본문과 같은 번호를 쓴다. 실행을 연 입력은 그 실행의 첫 기록 번호, 메인 에이전트 글(`PacketReply`와
/// 하위 에이전트 글은 아님)은 그 글의 기록 번호, 끼워 넣어 적용한 입력은 입력 번호다. 빈 글은 후보가 아니다.
fn dialogue_records(rows: &[LedgerRow], steers: &[SteeredInput]) -> Vec<Record> {
    let mut opened: HashSet<RunId> = HashSet::new();
    let mut records = Vec::new();
    for row in rows {
        if let Some(input) = row.input.as_ref().filter(|_| opened.insert(row.run)) {
            records.push(Record {
                kind: EvidenceKind::Input,
                id: row.seq.0,
                anchor: row.seq.0,
                at_ms: row.at_ms,
                text: input.clone(),
                files: Vec::new(),
            });
        }
        if let ProviderEvent::Text {
            subagent: None,
            text,
            ..
        } = &row.event
        {
            records.push(Record {
                kind: EvidenceKind::Text,
                id: row.seq.0,
                anchor: row.seq.0,
                at_ms: row.at_ms,
                text: text.clone(),
                files: Vec::new(),
            });
        }
    }
    records.extend(steers.iter().map(|steer| {
        Record {
            kind: EvidenceKind::Steer,
            id: steer.input.0,
            anchor: steer.after.0,
            at_ms: rows
                .iter()
                .find(|row| row.seq == steer.after)
                .map_or(0, |row| row.at_ms),
            text: steer.text.clone(),
            files: Vec::new(),
        }
    }));
    records.retain(|record| !record.text.trim().is_empty());
    records
}

fn candidate_sets(material: &Material) -> Result<Sets, EngineError> {
    EvidenceKind::ALL
        .into_iter()
        .map(|kind| {
            let candidates = material
                .readable
                .iter()
                .filter(|record| record.kind == kind)
                .map(|record| {
                    EvidenceCandidate::from_text(
                        LedgerSeq(record.id),
                        &material.project,
                        record.at_ms,
                        &record.text,
                    )
                })
                .collect();
            CandidateSet::new(candidates).map(|set| (kind, set))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Sets)
        .map_err(|_| EngineError::Evidence {
            refusal: EvidenceRefusal::NotFound,
        })
}
