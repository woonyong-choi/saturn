//! 패킷의 제약 칸 채우기. 상한 안에서 전체, 범위 겹침, 말 겹침 순으로 넣고 못 넣은 제약을 따로 돌려준다.
//! 설계: docs/design/constraints.md#패킷의-제약-칸

use std::collections::HashSet;

use saturn_protocol::ids::ConstraintId;

/// 제약이 패킷에 들어간 이유. `Omitted`는 칸이 차서 못 넣었다는 뜻이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintTier {
    /// 적용 범위가 전체인 제약.
    All,
    /// 범위가 지금 작업의 파일과 겹치는 제약.
    Scope,
    /// 마지막 입력과의 단어 겹침 순으로 넣은 나머지.
    Relevance,
    Omitted,
}

impl ConstraintTier {
    /// `packet_constraints.tier`에 쓰는 이름.
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Scope => "Scope",
            Self::Relevance => "Relevance",
            Self::Omitted => "Omitted",
        }
    }
}

/// 패킷에 넣을 후보인 유효 제약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotConstraint {
    pub id: ConstraintId,
    pub rule: String,
    /// 비어 있으면 적용 범위가 전체다.
    pub scope: Vec<String>,
}

/// 칸을 채우는 기준인 지금 작업.
#[derive(Debug, Clone, Copy)]
pub struct SlotContext<'a> {
    /// 마지막 입력에 나온 경로와 최근 턴이 건드린 파일.
    pub reference_paths: &'a [String],
    pub last_input: &'a str,
}

/// 칸 채우기의 결과. 둘 다 제약 번호 순서(등록 순)다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstraintSlot {
    pub included: Vec<(ConstraintId, String)>,
    /// 칸이 차서 못 넣은 제약의 규칙.
    pub omitted: Vec<(ConstraintId, String)>,
    /// 제약마다 단계. `Omitted` 포함, 제약 번호 순이다.
    pub tiers: Vec<(ConstraintId, ConstraintTier)>,
}

// cost: time O(c log c + c·(s·r + w)), heap O(c), stack O(1)
// vars: c = 제약 수, s = 범위 경로 수, r = 기준 경로 수, w = 규칙 단어 수
// basis: estimate
/// `cap_chars`는 항목 구분을 포함한 글자 수다. 칸에 들어가지 않는 제약은 건너뛰고 더 작은 제약은 계속 넣는다.
/// `constraints`는 등록 순서이고 같은 우선순위에서는 나중에 등록한 쪽이 먼저다.
pub fn fill_constraint_slot(
    constraints: &[SlotConstraint],
    context: SlotContext<'_>,
    cap_chars: usize,
    separator_chars: usize,
) -> ConstraintSlot {
    let words = words_of(context.last_input);
    let mut ranked: Vec<(usize, ConstraintTier, usize, &SlotConstraint)> = constraints
        .iter()
        .enumerate()
        .map(|(order, constraint)| {
            let (tier, strength) = classify(constraint, &context, &words);
            (order, tier, strength, constraint)
        })
        .collect();
    // 단계 순서, 단계 안의 강도가 센 쪽, 같으면 최신 순
    ranked.sort_by(|a, b| {
        tier_rank(a.1)
            .cmp(&tier_rank(b.1))
            .then(b.2.cmp(&a.2))
            .then(b.0.cmp(&a.0))
    });
    let mut left = cap_chars;
    let mut slot = ConstraintSlot::default();
    let mut tiers: Vec<(usize, ConstraintId, ConstraintTier)> = Vec::new();
    for (order, tier, _, constraint) in ranked {
        let size = constraint.rule.chars().count() + separator_chars;
        if size <= left {
            left -= size;
            slot.included.push((constraint.id, constraint.rule.clone()));
            tiers.push((order, constraint.id, tier));
        } else {
            slot.omitted.push((constraint.id, constraint.rule.clone()));
            tiers.push((order, constraint.id, ConstraintTier::Omitted));
        }
    }
    slot.included.sort_by_key(|(id, _)| *id);
    slot.omitted.sort_by_key(|(id, _)| *id);
    tiers.sort_by_key(|(order, _, _)| *order);
    slot.tiers = tiers.into_iter().map(|(_, id, tier)| (id, tier)).collect();
    slot
}

fn tier_rank(tier: ConstraintTier) -> u8 {
    match tier {
        ConstraintTier::All => 0,
        ConstraintTier::Scope => 1,
        ConstraintTier::Relevance | ConstraintTier::Omitted => 2,
    }
}

/// 단계와 단계 안의 강도(겹치는 경로 수 또는 겹치는 단어 수).
fn classify(
    constraint: &SlotConstraint,
    context: &SlotContext<'_>,
    input_words: &HashSet<String>,
) -> (ConstraintTier, usize) {
    if constraint.scope.is_empty() {
        return (ConstraintTier::All, 0);
    }
    let overlapping = constraint
        .scope
        .iter()
        .filter(|scope| {
            context
                .reference_paths
                .iter()
                .any(|path| overlaps(scope, path))
        })
        .count();
    if overlapping > 0 {
        return (ConstraintTier::Scope, overlapping);
    }
    let shared = words_of(&constraint.rule).intersection(input_words).count();
    (ConstraintTier::Relevance, shared)
}

/// 같은 경로이거나 범위가 폴더이고 그 안의 경로면 겹친다. 기준 경로는 절대 경로일 수 있어 범위가 경로의 뒷부분이어도 겹친다.
fn overlaps(scope: &str, path: &str) -> bool {
    let scope = scope.trim_start_matches("./").trim_end_matches('/');
    let path = path.trim_start_matches("./");
    if scope.is_empty() {
        return false;
    }
    path == scope
        || path.starts_with(&format!("{scope}/"))
        || path.ends_with(&format!("/{scope}"))
        || path.contains(&format!("/{scope}/"))
}

fn words_of(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests;
