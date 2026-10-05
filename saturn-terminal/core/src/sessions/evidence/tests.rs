use super::*;

fn candidate(id: u64, project: &str, text: &str) -> EvidenceCandidate {
    EvidenceCandidate::from_text(LedgerSeq(id), project, 1_000 + id as i64, text)
}

fn set_of(sizes: &[(u64, usize)]) -> CandidateSet {
    CandidateSet::new(
        sizes
            .iter()
            .map(|(id, chars)| candidate(*id, "/work", &"x".repeat(*chars)))
            .collect(),
    )
    .unwrap()
}

fn ids(values: &[u64]) -> Vec<LedgerSeq> {
    values.iter().map(|value| LedgerSeq(*value)).collect()
}

#[test]
fn set_hash_ignores_input_order_and_follows_any_candidate_change() {
    let forward = set_of(&[(1, 10), (2, 20), (3, 30)]);
    let shuffled = set_of(&[(3, 30), (1, 10), (2, 20)]);
    let longer = set_of(&[(1, 10), (2, 21), (3, 30)]);

    assert_eq!(forward.hash(), shuffled.hash());
    assert_eq!(forward, shuffled);
    assert_ne!(forward.hash(), longer.hash());
}

#[test]
fn set_rejects_a_duplicate_id() {
    let error = CandidateSet::new(vec![candidate(4, "/work", "a"), candidate(4, "/work", "b")])
        .unwrap_err();

    assert_eq!(error, SetError::DuplicateId(LedgerSeq(4)));
}

#[test]
fn resolve_checks_the_id_the_folder_and_the_hash() {
    let set = CandidateSet::new(vec![candidate(7, "/work", "body")]).unwrap();
    let fresh = text_hash("body");
    let rows = [
        (7, "/work", None, Ok(())),
        (7, "/work", Some(fresh.as_str()), Ok(())),
        (8, "/work", None, Err(RefError::Unknown)),
        (7, "/other", None, Err(RefError::OtherProject)),
        (7, "/work", Some("old-hash"), Err(RefError::Stale)),
        (7, "/other", Some("old-hash"), Err(RefError::OtherProject)),
    ];

    for (id, project, hash, expected) in rows {
        let got = set.resolve(LedgerSeq(id), project, hash).map(|_| ());
        assert_eq!(got, expected, "{id} {project} {hash:?}");
    }
}

#[test]
fn candidate_carries_range_excerpt_and_hash_of_the_text() {
    let text = "가".repeat(DIGEST_HEAD_CHARS + 5);

    let candidate = candidate(1, "/work", &text);

    assert_eq!(candidate.chars, DIGEST_HEAD_CHARS + 5);
    assert_eq!(candidate.excerpt.chars().count(), DIGEST_HEAD_CHARS);
    assert_eq!(candidate.hash, text_hash(&text));
}

#[test]
fn the_three_methods_get_the_same_set_and_budget_and_differ_only_in_ids() {
    let set = set_of(&[(1, 40), (2, 40), (3, 40), (4, 40)]);
    let ranked = ids(&[4, 3, 2, 1]);

    let rank = select(&set, &ranked, 80, Proposal::Rank);
    let jev = select(
        &set,
        &ranked,
        80,
        Proposal::Jev(Ok(vec![
            (LedgerSeq(1), 0.9),
            (LedgerSeq(2), 0.8),
            (LedgerSeq(3), 0.1),
        ])),
    );
    let llm = select(&set, &ranked, 80, Proposal::Llm(Ok(ids(&[3, 1]))));

    assert_eq!(rank.ids, ids(&[4, 3]));
    assert_eq!(jev.ids, ids(&[1, 2]));
    assert_eq!(llm.ids, ids(&[3, 1]));
    for selection in [&rank, &jev, &llm] {
        assert_eq!(selection.undecided, None);
        assert_eq!(selection.applied, selection.requested);
    }
}

#[test]
fn a_selection_that_cannot_be_decided_falls_back_to_the_rank_selection() {
    let set = set_of(&[(1, 10), (2, 10), (3, 10)]);
    let ranked = ids(&[2, 3, 1]);
    let rank = select(&set, &ranked, 20, Proposal::Rank).ids;
    let rows: [(Proposal, Undecided); 9] = [
        (Proposal::Jev(Err(())), Undecided::Error),
        (
            Proposal::Jev(Ok(vec![(LedgerSeq(1), 0.49), (LedgerSeq(2), 0.1)])),
            Undecided::LowConfidence,
        ),
        (Proposal::Jev(Ok(vec![])), Undecided::LowConfidence),
        (
            Proposal::Jev(Ok(vec![(LedgerSeq(9), 0.9)])),
            Undecided::UnknownId(LedgerSeq(9)),
        ),
        (
            Proposal::Jev(Ok(vec![(LedgerSeq(1), 0.9), (LedgerSeq(1), 0.8)])),
            Undecided::DuplicateId(LedgerSeq(1)),
        ),
        (
            Proposal::Jev(Ok(vec![(LedgerSeq(1), f64::NAN)])),
            Undecided::BadProbability(LedgerSeq(1)),
        ),
        (Proposal::Llm(Err(())), Undecided::Error),
        (
            Proposal::Llm(Ok(ids(&[1, 9]))),
            Undecided::UnknownId(LedgerSeq(9)),
        ),
        (Proposal::Llm(Ok(ids(&[]))), Undecided::Empty),
    ];

    for (proposal, reason) in rows {
        let selection = select(&set, &ranked, 20, proposal.clone());

        assert_eq!(selection.applied, SelectionSource::Rank, "{proposal:?}");
        assert_eq!(selection.ids, rank, "{proposal:?}");
        assert_eq!(
            selection.undecided.map(Undecided::name),
            Some(reason.name())
        );
    }
    let duplicate = select(&set, &ranked, 20, Proposal::Llm(Ok(ids(&[3, 3]))));
    assert_eq!(
        duplicate.undecided,
        Some(Undecided::DuplicateId(LedgerSeq(3)))
    );
}

#[test]
fn jev_ties_follow_rank_order_and_candidate_input_order_changes_nothing() {
    let forward = set_of(&[(1, 10), (2, 10), (3, 10)]);
    let shuffled = set_of(&[(2, 10), (3, 10), (1, 10)]);
    let ranked = ids(&[3, 1, 2]);
    let tied = || {
        Proposal::Jev(Ok(vec![
            (LedgerSeq(1), 0.7),
            (LedgerSeq(2), 0.7),
            (LedgerSeq(3), 0.7),
        ]))
    };

    let first = select(&forward, &ranked, 100, tied());
    let second = select(&shuffled, &ranked, 100, tied());

    assert_eq!(first.ids, ids(&[3, 1, 2]));
    assert_eq!(first, second);
}

#[test]
fn rank_order_drops_unknown_ids_and_appends_unranked_candidates_newest_first() {
    let set = set_of(&[(1, 10), (2, 10), (3, 10)]);

    let selection = select(&set, &ids(&[2, 99, 2]), 100, Proposal::Rank);

    assert_eq!(selection.ids, ids(&[2, 3, 1]));
}

#[test]
fn a_candidate_over_the_remaining_budget_is_skipped_and_smaller_ones_still_fit() {
    let set = set_of(&[(1, 60), (2, 60), (3, 30)]);

    let selection = select(&set, &ids(&[1, 2, 3]), 100, Proposal::Rank);

    assert_eq!(selection.ids, ids(&[1, 3]));
}

#[test]
fn slice_chars_returns_the_next_offset_until_the_text_ends() {
    let text = "가나다라마";

    assert_eq!(slice_chars(text, 0, 2), ("가나".to_owned(), Some(2)));
    assert_eq!(slice_chars(text, 2, 2), ("다라".to_owned(), Some(4)));
    assert_eq!(slice_chars(text, 4, 2), ("마".to_owned(), None));
    assert_eq!(slice_chars(text, 9, 2), (String::new(), None));
}
