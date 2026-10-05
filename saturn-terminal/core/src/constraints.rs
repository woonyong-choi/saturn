//! 제약의 규칙 한 줄 자르기와 적용 범위 뽑기. 입력 원문에서 코드가 자르고 요약하거나 생성하지 않는다.
//! 설계: docs/design/constraints.md#제약의-모양

use saturn_protocol::ids::{ConstraintId, LedgerSeq};
use unicode_normalization::UnicodeNormalization;

use crate::sessions::ranking::{Candidate, DEFAULT_RRF_K, rank_candidates};

/// 이 글자 수를 넘고 여러 문장이면 문장으로 나눈다(초안).
pub const SPLIT_OVER_CHARS: usize = 200;

/// 문장이 이 수를 넘으면 나누지 않고 입력 전체를 한 건으로 쓴다(초안).
pub const MAX_SENTENCES: usize = 20;

/// 해제·예외 판단에 싣는 제약 수 상한. 전체를 실으면 요청이 크기 상한에 걸린다.
pub const CHANGE_CANDIDATES_MAX: usize = 10;

/// 경로 끝 이름의 확장자 최대 길이.
const MAX_EXTENSION_CHARS: usize = 8;

// cost: time O(L), heap O(L), stack O(1)
// vars: L = 입력 글자 수
// basis: estimate
/// 200자를 넘고 문장이 둘 이상이면 문장 목록(2~20개)을 돌려주고, 아니면 `None`이다.
/// `None`은 입력 전체가 규칙 한 줄이라는 뜻이다. 코드 블록(```)과 인용 줄(`>`)은 나누기 전에 뺀다.
/// 줄바꿈과 문장 끝(`.`, `?`, `!`, `。`)에서 나누고, `.`은 뒤에 공백이나 줄 끝이 올 때만 문장 끝으로 본다.
/// 그래야 `main.rs`나 `3.5`가 갈라지지 않는다.
pub fn split_sentences(text: &str) -> Option<Vec<String>> {
    if text.chars().count() <= SPLIT_OVER_CHARS {
        return None;
    }
    let mut sentences = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || trimmed.starts_with('>') {
            continue;
        }
        split_line(trimmed, &mut sentences);
    }
    (2..=MAX_SENTENCES)
        .contains(&sentences.len())
        .then_some(sentences)
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 줄 글자 수
// basis: estimate
fn split_line(line: &str, sentences: &mut Vec<String>) {
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        current.push(ch);
        let ends = match ch {
            '?' | '!' | '。' => true,
            '.' => chars.peek().is_none_or(|next| next.is_whitespace()),
            _ => false,
        };
        if ends {
            push_sentence(&mut current, sentences);
        }
    }
    push_sentence(&mut current, sentences);
}

fn push_sentence(current: &mut String, sentences: &mut Vec<String>) {
    let sentence = current.trim();
    if !sentence.is_empty() {
        sentences.push(sentence.to_owned());
    }
    current.clear();
}

// cost: time O(L), heap O(p), stack O(1)
// vars: L = 규칙 글자 수, p = 경로 수
// basis: estimate
/// 규칙 글에서 경로 모양 글자를 뽑는다. NFC로 정규화하고 앞의 `./`를 떼며 같은 경로는 한 번만 담는다.
/// 빈 목록이면 적용 범위가 `전체`다.
pub fn scope_of(rule: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for token in rule.split_whitespace() {
        let token = token
            .trim_matches(|ch: char| "\"'`()[]<>,;:".contains(ch))
            .trim_end_matches(['.', '?', '!', '。']);
        if !is_path_like(token) {
            continue;
        }
        let normalized: String = token.nfc().collect();
        let normalized = normalized
            .strip_prefix("./")
            .map_or_else(|| normalized.clone(), str::to_owned);
        if !paths.contains(&normalized) {
            paths.push(normalized);
        }
    }
    paths
}

/// 슬래시가 있고, 끝이 `/`이거나 `./`, `../`, `~/`, `/`로 시작하거나 끝 이름에 확장자가 있어야 경로로 본다.
/// `and/or` 같은 말을 경로로 읽어 범위를 좁히지 않기 위해서다.
fn is_path_like(token: &str) -> bool {
    if !token.contains('/') || token.contains("://") {
        return false;
    }
    let is_path_char = |ch: char| ch.is_alphanumeric() || "._-@~/".contains(ch);
    if !token.chars().all(is_path_char) {
        return false;
    }
    let has_marker = token.ends_with('/')
        || ["./", "../", "~/", "/"]
            .iter()
            .any(|prefix| token.starts_with(prefix));
    let name = token.rsplit('/').next().unwrap_or_default();
    let has_extension = name.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && (1..=MAX_EXTENSION_CHARS).contains(&extension.chars().count())
            && extension.chars().next().is_some_and(char::is_alphabetic)
            && extension.chars().all(char::is_alphanumeric)
    });
    token.chars().any(char::is_alphanumeric) && (has_marker || has_extension)
}

#[cfg(test)]
mod tests;

// cost: time O(L + c log c), heap O(L + c), stack O(1)
// vars: L = 규칙과 입력의 글자 수 합, c = 제약 수
// basis: estimate
/// 해제·예외 판단에 실을 유효 제약을 등록 순서로 돌려준다. 10개 이하면 전부, 넘으면 입력과의 파일 겹침, 단어 겹침, 최근성
/// 순위를 RRF로 합쳐 상위 10개다. 같은 점수면 나중에 등록한 쪽이 위다. `constraints`는 `(번호, 규칙, 범위)`다.
pub fn pick_change_candidates(
    constraints: &[(ConstraintId, String, Vec<String>)],
    input: &str,
) -> Vec<ConstraintId> {
    if constraints.len() <= CHANGE_CANDIDATES_MAX {
        let mut all: Vec<ConstraintId> = constraints.iter().map(|(id, _, _)| *id).collect();
        all.sort();
        return all;
    }
    let candidates: Vec<Candidate> = constraints
        .iter()
        .map(|(id, rule, scope)| Candidate {
            seq: LedgerSeq(id.0),
            text: rule.clone(),
            files: scope.clone(),
        })
        .collect();
    let mut picked: Vec<ConstraintId> =
        rank_candidates(&candidates, &scope_of(input), input, DEFAULT_RRF_K)
            .into_iter()
            .take(CHANGE_CANDIDATES_MAX)
            .map(|seq| ConstraintId(seq.0))
            .collect();
    picked.sort();
    picked
}
