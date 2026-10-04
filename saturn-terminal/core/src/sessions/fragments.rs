//! 단어 겹침 채널이 쓰는 단어 조각.
//! 설계: docs/design/context-selection.md#단어-조각

use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharKind {
    Hangul,
    /// 한자와 가나는 한 종류로 묶어 일본어의 한자·가나 혼용 단어를 나누지 않는다.
    HanKana,
    Latin,
    /// 라틴 밖 알파벳 문자는 라틴과 같은 규칙으로 자른다.
    OtherLetter,
    Digit,
    /// 기호는 경계, 공백·이모지·제어 문자는 버린다. 둘 다 조각을 만들지 않는다.
    Boundary,
}

impl CharKind {
    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    fn of(c: char) -> Self {
        if is_hangul(c) {
            return Self::Hangul;
        }
        if is_han_or_kana(c) {
            return Self::HanKana;
        }
        if c.is_numeric() {
            return Self::Digit;
        }
        if !c.is_alphabetic() {
            return Self::Boundary;
        }
        if is_latin(c) {
            return Self::Latin;
        }
        Self::OtherLetter
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 글자 수
// basis: estimate
/// 같은 조각이 여러 번 나오면 그 횟수만큼 들어 있다(BM25 단어 빈도).
pub fn fragments(text: &str) -> Vec<String> {
    let normalized: Vec<char> = text.nfc().collect();
    let mut result = Vec::new();
    let mut start = 0;
    while start < normalized.len() {
        let kind = CharKind::of(normalized[start]);
        let end = normalized[start..]
            .iter()
            .position(|&c| CharKind::of(c) != kind)
            .map_or(normalized.len(), |offset| start + offset);
        push_run(kind, &normalized[start..end], &mut result);
        start = end;
    }
    result
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 구간 글자 수
// basis: estimate
fn push_run(kind: CharKind, run: &[char], out: &mut Vec<String>) {
    match kind {
        CharKind::Hangul | CharKind::HanKana => push_bigrams(run, out),
        CharKind::Latin | CharKind::OtherLetter => push_identifier_words(run, out),
        CharKind::Digit => out.push(run.iter().collect()),
        CharKind::Boundary => {}
    }
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 구간 글자 수
// basis: estimate
/// 한 글자 구간은 그 글자 하나를 조각으로 둔다. 한 글자 단어를 버리지 않기 위해서다.
fn push_bigrams(run: &[char], out: &mut Vec<String>) {
    if run.len() == 1 {
        out.push(run.iter().collect());
        return;
    }
    out.extend(run.windows(2).map(|pair| pair.iter().collect::<String>()));
}

// cost: time O(l), heap O(l), stack O(1)
// vars: l = 구간 글자 수
// basis: estimate
/// camelCase는 소문자→대문자에서, `HTTPServer`처럼 이어진 대문자는 마지막 대문자 앞에서 나눈다.
fn push_identifier_words(run: &[char], out: &mut Vec<String>) {
    let mut word_start = 0;
    for index in 1..run.len() {
        if is_word_boundary(run, index) {
            out.push(lowercase(&run[word_start..index]));
            word_start = index;
        }
    }
    out.push(lowercase(&run[word_start..]));
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn is_word_boundary(run: &[char], index: usize) -> bool {
    let previous = run[index - 1];
    let current = run[index];
    if !current.is_uppercase() {
        return false;
    }
    if previous.is_lowercase() {
        return true;
    }
    let next_is_lower = run.get(index + 1).is_some_and(|next| next.is_lowercase());
    previous.is_uppercase() && next_is_lower
}

// cost: time O(l), heap O(l), stack O(1), alloc 1
// vars: l = 단어 글자 수
// basis: estimate
fn lowercase(word: &[char]) -> String {
    word.iter().flat_map(|c| c.to_lowercase()).collect()
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn is_hangul(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{D7B0}'..='\u{D7FF}'
    )
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn is_han_or_kana(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{30FF}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF66}'..='\u{FF9F}'
            | '\u{20000}'..='\u{3134F}'
    )
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
fn is_latin(c: char) -> bool {
    matches!(
        c,
        'A'..='Z'
            | 'a'..='z'
            | '\u{00C0}'..='\u{024F}'
            | '\u{1E00}'..='\u{1EFF}'
            | '\u{2C60}'..='\u{2C7F}'
            | '\u{A720}'..='\u{A7FF}'
            | '\u{AB30}'..='\u{AB6F}'
            | '\u{FF21}'..='\u{FF3A}'
            | '\u{FF41}'..='\u{FF5A}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    #[test]
    fn fragments_split_text_into_search_fragments() {
        // (사례, 원문, 예상 조각 목록)
        let cases: [(&str, &str, &[&str]); 12] = [
            (
                "hangul makes overlapping bigrams",
                "로그인실패",
                &["로그", "그인", "인실", "실패"],
            ),
            (
                "camel case and extension",
                "authLogin.rs",
                &["auth", "login", "rs"],
            ),
            ("snake case", "auth_login", &["auth", "login"]),
            ("kebab case", "auth-login", &["auth", "login"]),
            (
                "acronym splits before last capital",
                "HTTPServer",
                &["http", "server"],
            ),
            ("accented latin is lowercased", "Café", &["café"]),
            (
                "kind change splits hangul and latin",
                "로그인login",
                &["로그", "그인", "login"],
            ),
            ("digits stay one run", "404", &["404"]),
            ("letter then digit splits", "v2", &["v", "2"]),
            ("han makes a bigram", "設定", &["設定"]),
            ("kana makes bigrams", "ログイン", &["ログ", "グイ", "イン"]),
            ("single hangul char is kept", "왜 a", &["왜", "a"]),
        ];

        for (name, text, expected) in cases {
            assert_eq!(fragments(text), strings(expected), "{name}");
        }
        for (name, text) in [
            (
                "symbols, space, emoji and control",
                "/ . _ - :: \t\n\u{7}🚀",
            ),
            ("empty input", ""),
        ] {
            assert!(fragments(text).is_empty(), "{name}");
        }
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 테스트 데이터 크기
    // basis: estimate
    #[test]
    fn fragments_spacing_and_particle_share_hangul_bigrams() {
        let spaced = fragments("로그인 실패");
        let joined = fragments("로그인실패를");

        for fragment in ["로그", "그인", "실패"] {
            assert!(spaced.contains(&fragment.to_string()));
            assert!(joined.contains(&fragment.to_string()));
        }
    }

    #[test]
    fn fragments_nfd_hangul_matches_nfc() {
        let decomposed = "\u{1105}\u{1169}\u{1100}\u{1173}\u{110B}\u{1175}\u{11AB}.md";

        assert_eq!(fragments(decomposed), fragments("로그인.md"));
    }
}
