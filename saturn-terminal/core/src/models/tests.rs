use std::collections::HashMap;

use saturn_protocol::ids::Provider;
use saturn_protocol::rpc::ModelChoice;

use super::*;

const TODAY: &str = "2026-10-06";

fn choice(provider: &'static str, model: &str) -> ModelChoice {
    ModelChoice {
        provider: Provider::from_static(provider),
        model: model.to_owned(),
    }
}

fn available(provider: &'static str, model: &str, revision: Option<&str>) -> AvailableModel {
    AvailableModel {
        choice: choice(provider, model),
        revision: revision.map(str::to_owned),
    }
}

fn evidence(kind: EvidenceKind, version: Option<&str>, revision: &str) -> Evidence {
    Evidence {
        kind,
        name: "bench".to_owned(),
        source_url: "https://example.com/bench".to_owned(),
        collected: "2026-10-01".to_owned(),
        version: version.map(str::to_owned),
        score: Some(0.5),
        sample: None,
        environment: None,
        counterexample: None,
        model_revision: Some(revision.to_owned()),
    }
}

fn entry(
    provider: &str,
    model: &str,
    alias: Option<&str>,
    revision: Option<&str>,
    evidence: Vec<Evidence>,
) -> CatalogEntry {
    CatalogEntry {
        provider: provider.to_owned(),
        model_id: model.to_owned(),
        alias: alias.map(str::to_owned),
        revision: revision.map(str::to_owned),
        efforts: Vec::new(),
        features: Vec::new(),
        price: None,
        uncertainty: None,
        evidence,
    }
}

fn catalog() -> ModelCatalog {
    ModelCatalog {
        version: "2026-10-06.1".to_owned(),
        collected: "2026-10-06".to_owned(),
        expires: "2026-12-31".to_owned(),
        entries: vec![
            entry(
                "claude",
                "claude-opus-5-5",
                Some("opus"),
                Some("r1"),
                vec![evidence(EvidenceKind::Benchmark, Some("v1"), "r1")],
            ),
            entry(
                "codex",
                "gpt-6.1-sol",
                None,
                Some("r1"),
                vec![evidence(EvidenceKind::OwnExperiment, Some("e1"), "r1")],
            ),
            entry(
                "claude",
                "claude-haiku-4-5",
                None,
                Some("r1"),
                vec![evidence(EvidenceKind::PublicOpinion, Some("v1"), "r1")],
            ),
        ],
    }
}

// 여론, 공식 사양, 날짜·버전 없는 점수는 품질을 확정하지 못하고, 근거를 잰 revision이 바뀌면 이전 성능을 쓰지 않는다
#[test]
fn only_dated_versioned_measurements_of_the_same_revision_confirm_quality() {
    let mut undated = catalog();
    undated.entries[0].evidence[0].version = None;
    let mut spec_only = catalog();
    spec_only.entries[0].evidence[0].kind = EvidenceKind::OfficialSpec;
    let mut expired = catalog();
    expired.expires = "2026-10-05".to_owned();
    let cases = [
        (catalog(), "opus", Some("r1"), Quality::Verified),
        (catalog(), "claude-opus-5-5", Some("r1"), Quality::Verified),
        (
            catalog(),
            "claude-haiku-4-5",
            Some("r1"),
            Quality::Unverified(Unverified::NoDatedEvidence),
        ),
        (
            undated,
            "opus",
            Some("r1"),
            Quality::Unverified(Unverified::NoDatedEvidence),
        ),
        (
            spec_only,
            "opus",
            Some("r1"),
            Quality::Unverified(Unverified::NoDatedEvidence),
        ),
        (
            catalog(),
            "opus",
            Some("r2"),
            Quality::Unverified(Unverified::RevisionChanged),
        ),
        (
            catalog(),
            "opus",
            None,
            Quality::Unverified(Unverified::RevisionUnknown),
        ),
        (
            catalog(),
            "claude-new",
            Some("r1"),
            Quality::Unverified(Unverified::NotInCatalog),
        ),
        (
            expired,
            "opus",
            Some("r1"),
            Quality::Unverified(Unverified::CatalogExpired),
        ),
    ];
    for (catalog, model, revision, expected) in cases {
        assert_eq!(
            catalog.quality("claude", model, revision, TODAY),
            expected,
            "{model} {revision:?}"
        );
    }
}

#[test]
fn validation_rejects_missing_sources_bad_dates_and_duplicates() {
    assert_eq!(catalog().validate(), Ok(()));
    let mut no_source = catalog();
    no_source.entries[0].evidence[0].source_url.clear();
    assert_eq!(
        no_source.validate(),
        Err(CatalogError::MissingSource("bench".to_owned()))
    );
    let mut bad_date = catalog();
    bad_date.expires = "2026/12/31".to_owned();
    assert_eq!(
        bad_date.validate(),
        Err(CatalogError::BadDate("2026/12/31".to_owned()))
    );
    let mut twice = catalog();
    twice.entries.push(twice.entries[0].clone());
    assert!(matches!(
        twice.validate(),
        Err(CatalogError::DuplicateEntry(_))
    ));
}

struct Case {
    available: Vec<AvailableModel>,
    limits: Vec<(&'static str, LimitState)>,
    pinned: Option<ModelChoice>,
    preferences: Vec<ModelChoice>,
}

fn run(case: &Case) -> CandidateSet {
    let catalog = catalog();
    let limits: HashMap<String, LimitState> = case
        .limits
        .iter()
        .map(|(provider, state)| ((*provider).to_owned(), *state))
        .collect();
    candidate_set(&CandidateInput {
        catalog: &catalog,
        available: &case.available,
        limits: &limits,
        pinned: case.pinned.as_ref(),
        preferences: &case.preferences,
        today: TODAY,
    })
}

fn names(set: &CandidateSet) -> Vec<String> {
    set.candidates
        .iter()
        .map(|candidate| candidate.choice.model.clone())
        .collect()
}

// Claude만, Codex만, 양쪽, 후보 없음, 한도 미상, 한도 소진, 명시 고정에서 후보와 선호 처리 결과
#[test]
fn candidate_sets_follow_availability_limits_pins_and_preferences() {
    let claude = || available("claude", "opus", Some("r1"));
    let codex = || available("codex", "gpt-6.1-sol", Some("r1"));
    let ok = [
        ("claude", LimitState::Available),
        ("codex", LimitState::Available),
    ];

    let claude_only = run(&Case {
        available: vec![claude()],
        limits: ok.to_vec(),
        pinned: None,
        preferences: vec![choice("codex", "gpt-6.1-sol")],
    });
    assert_eq!(names(&claude_only), ["opus"]);
    assert_eq!(
        claude_only.skipped_preferences,
        [SkippedPreference {
            choice: choice("codex", "gpt-6.1-sol"),
            reason: SkippedReason::NotAvailable
        }]
    );

    let codex_only = run(&Case {
        available: vec![codex()],
        limits: ok.to_vec(),
        pinned: None,
        preferences: Vec::new(),
    });
    assert_eq!(names(&codex_only), ["gpt-6.1-sol"]);

    // 선호는 품질을 확정한 후보의 순서만 앞당긴다
    let both = run(&Case {
        available: vec![claude(), codex()],
        limits: ok.to_vec(),
        pinned: None,
        preferences: vec![
            choice("codex", "gpt-6.1-sol"),
            choice("codex", "gpt-6.1-sol"),
        ],
    });
    assert_eq!(names(&both), ["gpt-6.1-sol", "opus"]);
    assert!(both.skipped_preferences.is_empty());

    // 미검증 후보는 남지만 선호로 앞당기지 않는다
    let unverified = run(&Case {
        available: vec![
            available("claude", "claude-haiku-4-5", Some("r1")),
            claude(),
        ],
        limits: ok.to_vec(),
        pinned: None,
        preferences: vec![choice("claude", "claude-haiku-4-5")],
    });
    assert_eq!(names(&unverified), ["claude-haiku-4-5", "opus"]);
    assert_eq!(
        unverified.skipped_preferences[0].reason,
        SkippedReason::Unverified
    );

    let none = run(&Case {
        available: Vec::new(),
        limits: Vec::new(),
        pinned: None,
        preferences: Vec::new(),
    });
    assert!(none.candidates.is_empty() && none.pinned.is_none());

    // 한도를 모르는 provider는 후보에 두고 표시한다. 한도를 다 쓴 provider는 뺀다
    let limits = run(&Case {
        available: vec![claude(), codex()],
        limits: vec![("codex", LimitState::Exhausted)],
        pinned: None,
        preferences: vec![choice("codex", "gpt-6.1-sol")],
    });
    assert_eq!(names(&limits), ["opus"]);
    assert_eq!(limits.candidates[0].limit, LimitState::Unknown);
    assert_eq!(
        limits.skipped_preferences[0].reason,
        SkippedReason::LimitExhausted
    );

    // 명시 고정은 후보를 대신하고 품질·한도·선호가 덮지 못한다
    let pinned = run(&Case {
        available: vec![claude()],
        limits: vec![("claude", LimitState::Exhausted)],
        pinned: Some(choice("codex", "gpt-new")),
        preferences: vec![choice("claude", "opus")],
    });
    assert_eq!(pinned.pinned, Some(choice("codex", "gpt-new")));
    assert!(pinned.candidates.is_empty());
}
