//! 후보 순위(파일·단어·최근성 채널과 RRF)와 router 판단 뒤의 최종 순서.
//! 설계: docs/design/context-selection.md

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use saturn_protocol::ids::LedgerSeq;
use unicode_normalization::UnicodeNormalization;

use super::fragments::fragments;

/// RRF 합치기 상수 `k`의 기본값(Cormack, Clarke, Büttcher 2009).
pub const DEFAULT_RRF_K: u32 = 60;

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

#[derive(Debug, Clone)]
pub struct Candidate {
    /// 후보끼리 겹치지 않는다.
    pub seq: LedgerSeq,
    pub text: String,
    /// 도구 호출 인자에서 꺼낸 경로. 없으면 파일 겹침 채널에서 빠진다.
    pub files: Vec<String>,
}

/// 여러 원문 조각을 대조할 동안 같은 질문의 정규화 결과를 재사용한다.
pub struct QueryTerms {
    terms: HashSet<String>,
}

impl QueryTerms {
    pub fn new(query: &str) -> Self {
        Self {
            terms: fragments(query).into_iter().collect(),
        }
    }
}

/// 같은 원문에 여러 질문을 대조할 때 단어 빈도와 문서 빈도를 다시 계산하지 않는다.
pub struct RelevanceIndex<'a> {
    candidates: &'a [Candidate],
    postings: HashMap<String, Vec<(usize, usize)>>,
    normalizers: Vec<f64>,
}

impl<'a> RelevanceIndex<'a> {
    // cost: time O(L), heap O(L), stack O(1)
    // vars: L = 후보 글자 수 합
    // basis: estimate
    pub fn new(candidates: &'a [Candidate]) -> Self {
        let mut postings: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        let mut lengths = Vec::with_capacity(candidates.len());
        for (index, candidate) in candidates.iter().enumerate() {
            let terms = term_counts(&candidate.text);
            lengths.push(terms.values().sum::<usize>());
            for (term, frequency) in terms {
                postings.entry(term).or_default().push((index, frequency));
            }
        }
        let total_length: usize = lengths.iter().sum();
        let average = (total_length as f64 / candidates.len().max(1) as f64).max(1.0);
        let normalizers = lengths
            .into_iter()
            .map(|length| BM25_K1 * (1.0 - BM25_B + BM25_B * length as f64 / average))
            .collect();
        Self {
            candidates,
            postings,
            normalizers,
        }
    }

    // cost: time O(q + p + c log c), heap O(q + c), stack O(1)
    // vars: q = 질문의 글자 수, p = 해당 단어가 있는 후보 항목 수 합, c = 후보 수
    // basis: estimate
    pub fn ranked(&self, query: &str) -> Vec<(LedgerSeq, f64)> {
        self.ranked_query(&QueryTerms::new(query))
    }

    pub fn ranked_query(&self, query: &QueryTerms) -> Vec<(LedgerSeq, f64)> {
        let scores = self.scores(query);
        let mut ranked: Vec<_> = self
            .candidates
            .iter()
            .zip(scores)
            .filter(|(_, score)| *score > 0.0)
            .collect();
        ranked.sort_by(|(left, a), (right, b)| b.total_cmp(a).then(left.seq.cmp(&right.seq)));
        ranked
            .into_iter()
            .map(|(candidate, score)| (candidate.seq, score))
            .collect()
    }

    fn scores(&self, query: &QueryTerms) -> Vec<f64> {
        let count = self.candidates.len() as f64;
        let idf: HashMap<&String, f64> = query
            .terms
            .iter()
            .map(|term| {
                let containing = self.postings.get(term).map_or(0, Vec::len) as f64;
                let weight = ((count - containing + 0.5) / (containing + 0.5) + 1.0).ln();
                (term, weight)
            })
            .collect();
        let mut scores = vec![0.0; self.candidates.len()];
        for (term, weight) in idf {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            for &(index, frequency) in postings {
                let frequency = frequency as f64;
                scores[index] +=
                    weight * frequency * (BM25_K1 + 1.0) / (frequency + self.normalizers[index]);
            }
        }
        scores
    }
}

// cost: time O(L + c log c), heap O(L + c), stack O(1)
// vars: L = 후보 글과 마지막 입력의 글자 수 합, c = 후보 수
// basis: estimate
/// 점수가 같으면 기록 번호가 큰 후보가 위다.
pub fn rank_candidates(
    candidates: &[Candidate],
    base_files: &[String],
    last_input: &str,
    k: u32,
) -> Vec<LedgerSeq> {
    let channels = [
        rank_by_score(candidates, &file_overlap_scores(candidates, base_files)),
        rank_by_score(candidates, &bm25_scores(candidates, last_input)),
        rank_by_score(candidates, &recency_scores(candidates)),
    ];
    let mut fused: Vec<(f64, LedgerSeq)> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let score = channels
                .iter()
                .filter_map(|ranks| ranks[index])
                .map(|rank| 1.0 / (f64::from(k) + rank as f64))
                .sum();
            (score, candidate.seq)
        })
        .collect();
    fused.sort_by(|left, right| right.0.total_cmp(&left.0).then(right.1.cmp(&left.1)));
    fused.into_iter().map(|(_, seq)| seq).collect()
}

// cost: time O(c log c), heap O(c), stack O(1), alloc 2
// vars: c = 후보 수
// basis: estimate
/// router가 답한 항목을 남길 확률이 높은 순으로 두고, 같은 확률이면 RRF 순이다.
/// 기준값이 없어 확률이 낮아도 빼지 않는다. 답이 없는 항목(실패한 조각 포함)은 RRF 순으로 뒤에 둔다.
/// `verdicts`가 비면 router가 전부 답하지 못한 것이라 RRF 순서 그대로다.
pub fn order_after_router(ranked: &[LedgerSeq], verdicts: &[(LedgerSeq, f64)]) -> Vec<LedgerSeq> {
    let answered: HashMap<LedgerSeq, f64> = verdicts.iter().copied().collect();
    let mut kept: Vec<(usize, f64, LedgerSeq)> = Vec::new();
    let mut unanswered: Vec<LedgerSeq> = Vec::new();
    for (position, seq) in ranked.iter().enumerate() {
        match answered.get(seq) {
            Some(&probability) => kept.push((position, probability, *seq)),
            None => unanswered.push(*seq),
        }
    }
    kept.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    kept.into_iter()
        .map(|(_, _, seq)| seq)
        .chain(unanswered)
        .collect()
}

// cost: time O(c log c), heap O(c), stack O(1), alloc 2
// vars: c = 후보 수
// basis: estimate
/// 점수가 0인 후보는 채널 순위에서 빠지고(`None`), 같은 점수는 같은 순위다.
fn rank_by_score(candidates: &[Candidate], scores: &[f64]) -> Vec<Option<usize>> {
    let mut order: Vec<usize> = (0..candidates.len())
        .filter(|&index| scores[index] > 0.0)
        .collect();
    order.sort_by(|&left, &right| scores[right].total_cmp(&scores[left]));
    let mut ranks = vec![None; candidates.len()];
    for (position, &index) in order.iter().enumerate() {
        let is_tied_with_previous = position > 0
            && scores[order[position - 1]].total_cmp(&scores[index]) == Ordering::Equal;
        ranks[index] = if is_tied_with_previous {
            ranks[order[position - 1]]
        } else {
            Some(position + 1)
        };
    }
    ranks
}

// cost: time O(F), heap O(b), stack O(1)
// vars: F = 후보 경로 글자 수 합, b = 기준 파일 수
// basis: estimate
fn file_overlap_scores(candidates: &[Candidate], base_files: &[String]) -> Vec<f64> {
    let base: HashSet<String> = base_files.iter().map(|path| normalize_path(path)).collect();
    candidates
        .iter()
        .map(|candidate| {
            let touched: HashSet<String> = candidate
                .files
                .iter()
                .map(|path| normalize_path(path))
                .collect();
            touched.intersection(&base).count() as f64
        })
        .collect()
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 경로 글자 수
// basis: estimate
fn normalize_path(path: &str) -> String {
    let path: String = path.nfc().collect();
    match path.strip_prefix("./") {
        Some(rest) => rest.to_string(),
        None => path,
    }
}

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 후보 글과 마지막 입력의 글자 수 합
// basis: estimate
/// 질의 조각은 중복 없이 한 번씩 센다.
fn bm25_scores(candidates: &[Candidate], last_input: &str) -> Vec<f64> {
    RelevanceIndex::new(candidates).scores(&QueryTerms::new(last_input))
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 글자 수
// basis: estimate
fn term_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for fragment in fragments(text) {
        *counts.entry(fragment).or_insert(0) += 1;
    }
    counts
}

// cost: time O(c), heap O(c), stack O(1), alloc 1
// vars: c = 후보 수
// basis: estimate
/// 기록 번호 0도 순위에 들도록 1을 더한다.
fn recency_scores(candidates: &[Candidate]) -> Vec<f64> {
    candidates
        .iter()
        .map(|candidate| candidate.seq.0 as f64 + 1.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    fn candidate(seq: u64, text: &str, files: &[&str]) -> Candidate {
        Candidate {
            seq: LedgerSeq(seq),
            text: text.into(),
            files: files.iter().map(|path| path.to_string()).collect(),
        }
    }

    // cost: time O(c), heap O(c), stack O(1)
    // vars: c = 후보 수
    // basis: estimate
    fn noise(count: u64) -> Vec<Candidate> {
        (0..count)
            .map(|seq| candidate(seq, &format!("unrelated output {seq}"), &["src/misc.rs"]))
            .collect()
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    fn seqs(values: &[u64]) -> Vec<LedgerSeq> {
        values.iter().map(|&value| LedgerSeq(value)).collect()
    }

    #[test]
    fn recall_excludes_unrelated_records_and_breaks_ties_by_source_order() {
        let candidates = vec![
            candidate(3, "greet", &[]),
            candidate(1, "greet", &[]),
            candidate(2, "cache", &[]),
        ];
        let index = RelevanceIndex::new(&candidates);
        let ranked: Vec<_> = index
            .ranked("greet")
            .into_iter()
            .map(|(seq, _)| seq)
            .collect();
        assert_eq!(ranked, seqs(&[1, 3]));
        assert!(index.ranked("deployment").is_empty());
        assert!(index.ranked("").is_empty());
    }

    #[test]
    fn rank_candidates_overlap_beats_recency_alone() {
        let mut candidates = noise(150);
        candidates[12] = candidate(12, "GET /v2/auth 로그인 실패 401", &["src/auth/login.rs"]);

        let ranked = rank_candidates(
            &candidates,
            &["src/auth/login.rs".into()],
            "로그인 실패 메시지 고쳐 줘",
            DEFAULT_RRF_K,
        );

        assert_eq!(ranked[0], LedgerSeq(12));
        assert_eq!(ranked.len(), 150);
    }

    #[test]
    fn rank_candidates_without_overlap_follows_recency() {
        let candidates = vec![
            candidate(1, "alpha", &[]),
            candidate(3, "beta", &[]),
            candidate(2, "gamma", &[]),
        ];

        let ranked = rank_candidates(&candidates, &[], "zeta", DEFAULT_RRF_K);

        assert_eq!(ranked, seqs(&[3, 2, 1]));
    }

    #[test]
    fn rank_candidates_matches_rrf_formula() {
        let candidates = vec![candidate(1, "login", &["a.rs"]), candidate(2, "other", &[])];

        let ranked = rank_candidates(&candidates, &["a.rs".into()], "login", 1);

        // seq 1: 1/(1+1)·2 + 1/(1+2) = 1.33, seq 2: 1/(1+1) = 0.5
        assert_eq!(ranked, seqs(&[1, 2]));
    }

    #[test]
    fn rank_candidates_path_prefix_and_nfd_still_overlap() {
        let candidates = vec![
            candidate(1, "x", &["./docs/\u{1105}\u{1169}\u{1100}\u{1173}.md"]),
            candidate(2, "y", &[]),
        ];

        let ranked = rank_candidates(&candidates, &["docs/로그.md".into()], "", DEFAULT_RRF_K);

        assert_eq!(ranked[0], LedgerSeq(1));
    }

    #[test]
    fn rank_by_score_ties_share_rank_and_zero_is_excluded() {
        let candidates = noise(4);

        let ranks = rank_by_score(&candidates, &[2.0, 0.0, 2.0, 1.0]);

        assert_eq!(ranks, vec![Some(1), None, Some(1), Some(3)]);
    }

    #[test]
    fn order_after_router_no_verdicts_keeps_rrf_order() {
        let ranked = seqs(&[5, 4, 3, 2, 1]);

        assert_eq!(order_after_router(&ranked, &[]), ranked);
    }

    #[test]
    fn order_after_router_orders_by_probability_and_keeps_low() {
        let ranked = seqs(&[5, 4, 3, 2, 1]);
        let verdicts = [
            (LedgerSeq(5), 0.6),
            (LedgerSeq(4), 0.9),
            (LedgerSeq(3), 0.2),
            (LedgerSeq(2), 0.7),
            (LedgerSeq(1), 0.8),
        ];

        let ordered = order_after_router(&ranked, &verdicts);

        assert_eq!(ordered, seqs(&[4, 1, 2, 5, 3]));
    }

    #[test]
    fn order_after_router_same_probability_follows_rrf_order() {
        let ranked = seqs(&[5, 4, 3]);
        let verdicts = [
            (LedgerSeq(3), 0.8),
            (LedgerSeq(4), 0.8),
            (LedgerSeq(5), 0.8),
        ];

        let ordered = order_after_router(&ranked, &verdicts);

        assert_eq!(ordered, seqs(&[5, 4, 3]));
    }

    #[test]
    fn order_after_router_unanswered_follow_answered_in_rrf_order() {
        let ranked = seqs(&[5, 4, 3, 2, 1]);
        let verdicts = [(LedgerSeq(2), 0.7), (LedgerSeq(4), 0.1)];

        let ordered = order_after_router(&ranked, &verdicts);

        assert_eq!(ordered, seqs(&[2, 4, 5, 3, 1]));
    }
}
