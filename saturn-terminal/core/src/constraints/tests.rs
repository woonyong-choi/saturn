use super::*;

fn long(sentences: &[&str]) -> String {
    let filler = "x".repeat(SPLIT_OVER_CHARS);
    format!("{} {filler}", sentences.join(" "))
}

#[test]
fn short_input_is_one_rule() {
    assert_eq!(split_sentences("Use English. Keep tests small."), None);
}

#[test]
fn long_single_sentence_is_one_rule() {
    assert_eq!(split_sentences(&"a".repeat(SPLIT_OVER_CHARS + 1)), None);
}

#[test]
fn long_input_splits_on_sentence_ends_and_newlines() {
    let text = long(&[
        "에러 메시지는 영어로 통일해.",
        "그리고 이 버그를 고쳐줘!",
        "왜 안 돼?",
    ]);

    let sentences = split_sentences(&text).unwrap();

    assert_eq!(sentences[0], "에러 메시지는 영어로 통일해.");
    assert_eq!(sentences[1], "그리고 이 버그를 고쳐줘!");
    assert_eq!(sentences[2], "왜 안 돼?");
    assert_eq!(sentences.len(), 4);
}

#[test]
fn dots_inside_names_and_numbers_do_not_split() {
    let text = long(&["edit src/main.rs with version 3.5 only."]);

    let sentences = split_sentences(&text).unwrap();

    assert_eq!(sentences[0], "edit src/main.rs with version 3.5 only.");
}

#[test]
fn code_blocks_and_quotes_are_dropped_before_splitting() {
    let filler = "x".repeat(SPLIT_OVER_CHARS);
    let text = format!(
        "Use tabs. Never use spaces.\n```\nlet a = 1. let b = 2.\n```\n> quoted rule. another.\n{filler}"
    );

    let sentences = split_sentences(&text).unwrap();

    assert_eq!(sentences[..2], ["Use tabs.", "Never use spaces."]);
    assert_eq!(sentences.len(), 3);
}

#[test]
fn more_than_twenty_sentences_are_not_split() {
    let many = vec!["Do it."; MAX_SENTENCES + 1].join(" ");
    let text = format!("{many} {}", "x".repeat(SPLIT_OVER_CHARS));

    assert_eq!(split_sentences(&text), None);
}

#[test]
fn rules_are_cut_from_the_original_text() {
    let text = long(&["Reply in English.", "Fix the build."]);

    let sentences = split_sentences(&text).unwrap();

    assert_eq!(sentences[..2], ["Reply in English.", "Fix the build."]);
    assert!(
        sentences
            .iter()
            .all(|sentence| text.contains(sentence.as_str()))
    );
}

#[test]
fn scope_is_empty_without_paths() {
    assert!(scope_of("에러 메시지는 영어로 통일해 and/or 아무거나").is_empty());
}

#[test]
fn scope_takes_path_like_words_normalized_and_unique() {
    let scope = scope_of("`./src/auth/` 아래에서는 unwrap 금지, src/auth/ 와 docs/a.md 도.");

    assert_eq!(scope, vec!["src/auth/".to_owned(), "docs/a.md".to_owned()]);
}

#[test]
fn scope_ignores_urls_and_bare_directories_without_markers() {
    assert!(scope_of("see https://example.com/a.html and src/auth").is_empty());
}

#[test]
fn scope_normalizes_decomposed_hangul_to_nfc() {
    let decomposed: String = "문서/한글.md".nfd().collect();

    let scope = scope_of(&decomposed);

    assert_eq!(scope, vec!["문서/한글.md".to_owned()]);
}

fn rules(count: u64) -> Vec<(ConstraintId, String, Vec<String>)> {
    (1..=count)
        .map(|id| (ConstraintId(id), format!("rule number {id}"), Vec::new()))
        .collect()
}

#[test]
fn change_candidates_are_all_constraints_up_to_the_cap() {
    let picked = pick_change_candidates(&rules(10), "아무 말");

    assert_eq!(picked, (1..=10).map(ConstraintId).collect::<Vec<_>>());
}

#[test]
fn change_candidates_over_the_cap_keep_the_word_match_and_the_newest() {
    let mut constraints = rules(14);
    constraints[1].1 = "always write the changelog in english".to_owned();

    let picked = pick_change_candidates(&constraints, "changelog english 규칙 풀어줘");

    assert_eq!(picked.len(), CHANGE_CANDIDATES_MAX);
    assert!(picked.contains(&ConstraintId(2)));
    assert!(picked.contains(&ConstraintId(14)));
    assert!(picked.windows(2).all(|pair| pair[0] < pair[1]));
}
