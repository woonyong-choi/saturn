//! 근거 후보 집합과 선택 계약. 순위(RRF)와 Jev, 작업 LLM이 같은 후보와 같은 전문 예산을 받고 선택 ID만 달라진다.
//! 설계: docs/design/context-selection.md#근거-검색과-원문-조회

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use saturn_protocol::ids::LedgerSeq;
use sha2::{Digest, Sha256};

use super::packet::DIGEST_HEAD_CHARS;

/// Jev 선택이 후보를 쓰는 남김 확률의 하한. 첫 실험을 위한 가정이고 최적값이 아니다.
/// 하한을 넘는 후보가 하나도 없으면 낮은 확신으로 보고 순위 선택으로 돌아간다.
pub const JEV_MIN_PROBABILITY: f64 = 0.5;

/// 후보 하나. 원문은 기록 저장소에만 있고 후보는 어디서 읽을지와 바뀌었는지만 가진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceCandidate {
    /// 채팅 안에서 겹치지 않는 기록 번호.
    pub id: LedgerSeq,
    /// 기록을 만든 채팅의 작업 폴더.
    pub project: String,
    /// 기록 시각. unix 밀리초.
    pub at_ms: i64,
    /// 원문 글자 수. 원문 범위는 `0..chars`다.
    pub chars: usize,
    /// 원문 앞부분.
    pub excerpt: String,
    /// 원문의 SHA-256(16진수 소문자).
    pub hash: String,
}

impl EvidenceCandidate {
    // cost: time O(L), heap O(1), stack O(1), alloc 3
    // vars: L = 원문 글자 수
    // basis: estimate
    /// 원문에서 후보를 만든다. 발췌와 해시, 글자 수가 모두 이 원문에서 나온다.
    pub fn from_text(id: LedgerSeq, project: &str, at_ms: i64, text: &str) -> Self {
        Self {
            id,
            project: project.to_owned(),
            at_ms,
            chars: text.chars().count(),
            excerpt: text.chars().take(DIGEST_HEAD_CHARS).collect(),
            hash: text_hash(text),
        }
    }
}

// cost: time O(L), heap O(1), stack O(1), alloc 1
// vars: L = 글자 바이트 수
// basis: estimate
/// 원문의 SHA-256(16진수 소문자). 후보 해시와 조회 때 받은 해시를 이 값으로 견준다.
pub fn text_hash(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}"); // String에 쓰기는 실패하지 않는다
        out
    })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SetError {
    #[error("duplicate candidate id: {0:?}")]
    DuplicateId(LedgerSeq),
}

/// 후보 집합. 후보는 번호 순으로 보관해 입력 순서가 바뀌어도 같은 집합이고 같은 해시다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSet {
    candidates: Vec<EvidenceCandidate>,
    hash: String,
}

impl CandidateSet {
    // cost: time O(c log c), heap O(c), stack O(1), alloc c
    // vars: c = 후보 수
    // basis: estimate
    /// # Errors
    /// 같은 번호의 후보가 둘 이상이면 `DuplicateId`.
    pub fn new(mut candidates: Vec<EvidenceCandidate>) -> Result<Self, SetError> {
        candidates.sort_by_key(|candidate| candidate.id);
        if let Some(pair) = candidates.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(SetError::DuplicateId(pair[0].id));
        }
        let mut digest = Sha256::new();
        for candidate in &candidates {
            digest.update(
                format!(
                    "{}\0{}\0{}\0{}\0{}\n",
                    candidate.id.0,
                    candidate.project,
                    candidate.at_ms,
                    candidate.chars,
                    candidate.hash
                )
                .as_bytes(),
            );
        }
        Ok(Self {
            hash: hex(&digest.finalize()),
            candidates,
        })
    }

    /// 후보 번호 순서.
    pub fn candidates(&self) -> &[EvidenceCandidate] {
        &self.candidates
    }

    /// 후보 목록이 같으면 같다. 선택 조건들이 같은 후보를 받았는지 이 값으로 대조한다.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    // cost: time O(log c), heap O(1), stack O(1)
    // vars: c = 후보 수
    // basis: estimate
    pub fn get(&self, id: LedgerSeq) -> Option<&EvidenceCandidate> {
        self.candidates
            .binary_search_by_key(&id, |candidate| candidate.id)
            .ok()
            .map(|index| &self.candidates[index])
    }

    // cost: time O(log c), heap O(1), stack O(1)
    // vars: c = 후보 수
    // basis: estimate
    /// 원문을 읽으려는 번호가 이 집합과 맞는지 본다. `hash`를 주면 후보를 본 때의 원문과 같은지도 본다.
    ///
    /// # Errors
    /// 번호가 없으면 `Unknown`, 다른 폴더의 기록이면 `OtherProject`, 해시가 다르면 `Stale`.
    pub fn resolve(
        &self,
        id: LedgerSeq,
        project: &str,
        hash: Option<&str>,
    ) -> Result<&EvidenceCandidate, RefError> {
        let candidate = self.get(id).ok_or(RefError::Unknown)?;
        if candidate.project != project {
            return Err(RefError::OtherProject);
        }
        match hash {
            Some(hash) if hash != candidate.hash => Err(RefError::Stale),
            _ => Ok(candidate),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefError {
    #[error("no such record in this chat")]
    Unknown,
    #[error("the record belongs to another folder")]
    OtherProject,
    #[error("the record changed since the candidate was listed")]
    Stale,
}

/// 선택 방법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    Rank,
    Jev,
    Llm,
}

/// 선택 방법이 판정하지 못한 이유. 이 경우 순위 선택으로 돌아간다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Undecided {
    /// 호출이나 응답 읽기에 실패했다.
    Error,
    /// 확률이 하한을 넘는 후보가 없다.
    LowConfidence,
    /// 아무것도 고르지 않았다.
    Empty,
    UnknownId(LedgerSeq),
    DuplicateId(LedgerSeq),
    /// 확률이 0과 1 사이가 아니다.
    BadProbability(LedgerSeq),
}

impl Undecided {
    /// 기록에 쓰는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::LowConfidence => "low_confidence",
            Self::Empty => "empty",
            Self::UnknownId(_) => "unknown_id",
            Self::DuplicateId(_) => "duplicate_id",
            Self::BadProbability(_) => "bad_probability",
        }
    }
}

/// 선택 방법이 낸 제안. 오류는 호출한 쪽이 `Err(())`로 알린다.
#[derive(Debug, Clone)]
pub enum Proposal {
    /// 순위 그대로.
    Rank,
    /// 후보마다 남김 확률. 답이 없는 후보는 빠진다.
    Jev(Result<Vec<(LedgerSeq, f64)>, ()>),
    /// 고른 번호. 고른 순서다.
    Llm(Result<Vec<LedgerSeq>, ()>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// 고른 후보. 고른 순서이고 예산 안이다.
    pub ids: Vec<LedgerSeq>,
    pub requested: SelectionSource,
    /// 실제 적용한 방법. 판정하지 못했으면 `Rank`다.
    pub applied: SelectionSource,
    pub undecided: Option<Undecided>,
}

// cost: time O(c log c), heap O(c), stack O(1), alloc c
// vars: c = 후보 수
// basis: estimate
/// 같은 `set`, `ranked`, `budget_chars`에서 `proposal`만 바꿔 부른다. 선택 방법이 판정하지 못하면(오류, 낮은 확신, 없는
/// 번호, 겹친 번호, 빈 선택) 순위 선택으로 돌아가고 추가 모델을 부르지 않는다. 후보는 원문 글자 수로 예산을 채우며 예산에
/// 들지 않는 후보는 건너뛴다. `ranked`는 순위가 높은 순이고, 집합에 없는 번호는 버리고 빠진 후보는 번호가 큰 순으로 뒤에 붙인다.
pub fn select(
    set: &CandidateSet,
    ranked: &[LedgerSeq],
    budget_chars: usize,
    proposal: Proposal,
) -> Selection {
    let order = full_order(set, ranked);
    let (requested, outcome) = match proposal {
        Proposal::Rank => (SelectionSource::Rank, Ok(order.clone())),
        Proposal::Jev(response) => (SelectionSource::Jev, jev_order(set, &order, response)),
        Proposal::Llm(response) => (SelectionSource::Llm, llm_order(set, response)),
    };
    let (applied, undecided, picked) = match outcome {
        Ok(picked) => (requested, None, picked),
        Err(reason) => (SelectionSource::Rank, Some(reason), order),
    };
    Selection {
        ids: fill(set, &picked, budget_chars),
        requested,
        applied,
        undecided,
    }
}

fn full_order(set: &CandidateSet, ranked: &[LedgerSeq]) -> Vec<LedgerSeq> {
    let mut seen: HashSet<LedgerSeq> = HashSet::new();
    let mut order: Vec<LedgerSeq> = ranked
        .iter()
        .copied()
        .filter(|id| set.get(*id).is_some() && seen.insert(*id))
        .collect();
    order.extend(
        set.candidates()
            .iter()
            .rev()
            .map(|candidate| candidate.id)
            .filter(|id| !seen.contains(id)),
    );
    order
}

fn jev_order(
    set: &CandidateSet,
    order: &[LedgerSeq],
    response: Result<Vec<(LedgerSeq, f64)>, ()>,
) -> Result<Vec<LedgerSeq>, Undecided> {
    let verdicts = response.map_err(|()| Undecided::Error)?;
    let mut probabilities: HashMap<LedgerSeq, f64> = HashMap::new();
    for (id, probability) in verdicts {
        if set.get(id).is_none() {
            return Err(Undecided::UnknownId(id));
        }
        if !(0.0..=1.0).contains(&probability) {
            return Err(Undecided::BadProbability(id));
        }
        if probabilities.insert(id, probability).is_some() {
            return Err(Undecided::DuplicateId(id));
        }
    }
    let mut eligible: Vec<(usize, f64, LedgerSeq)> = order
        .iter()
        .enumerate()
        .filter_map(|(position, id)| Some((position, *probabilities.get(id)?, *id)))
        .filter(|(_, probability, _)| *probability >= JEV_MIN_PROBABILITY)
        .collect();
    if eligible.is_empty() {
        return Err(Undecided::LowConfidence);
    }
    eligible.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    Ok(eligible.into_iter().map(|(_, _, id)| id).collect())
}

fn llm_order(
    set: &CandidateSet,
    response: Result<Vec<LedgerSeq>, ()>,
) -> Result<Vec<LedgerSeq>, Undecided> {
    let ids = response.map_err(|()| Undecided::Error)?;
    let mut seen: HashSet<LedgerSeq> = HashSet::new();
    for id in &ids {
        if set.get(*id).is_none() {
            return Err(Undecided::UnknownId(*id));
        }
        if !seen.insert(*id) {
            return Err(Undecided::DuplicateId(*id));
        }
    }
    if ids.is_empty() {
        return Err(Undecided::Empty);
    }
    Ok(ids)
}

fn fill(set: &CandidateSet, order: &[LedgerSeq], budget_chars: usize) -> Vec<LedgerSeq> {
    let mut remaining = budget_chars;
    let mut picked = Vec::new();
    for id in order {
        let Some(candidate) = set.get(*id) else {
            continue;
        };
        if candidate.chars <= remaining {
            remaining -= candidate.chars;
            picked.push(*id);
        }
    }
    picked
}

// cost: time O(L), heap O(l), stack O(1), alloc 1
// vars: L = 원문 글자 수, l = 돌려주는 글자 수
// basis: estimate
/// 원문에서 `offset`부터 `limit`글자를 잘라 돌려준다. 남은 글이 있으면 다음 `offset`도 돌려준다.
pub fn slice_chars(text: &str, offset: usize, limit: usize) -> (String, Option<usize>) {
    let slice: String = text.chars().skip(offset).take(limit).collect();
    let end = offset.saturating_add(slice.chars().count());
    let next = (end < text.chars().count()).then_some(end);
    (slice, next)
}

#[cfg(test)]
mod tests;
