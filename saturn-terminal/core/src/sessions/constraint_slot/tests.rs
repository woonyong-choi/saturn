use super::*;

const SEPARATOR: usize = 2;

fn constraint(id: u64, rule: &str, scope: &[&str]) -> SlotConstraint {
    SlotConstraint {
        id: ConstraintId(id),
        rule: rule.into(),
        scope: scope.iter().map(|path| (*path).to_string()).collect(),
    }
}

fn context<'a>(paths: &'a [String], input: &'a str) -> SlotContext<'a> {
    SlotContext {
        reference_paths: paths,
        last_input: input,
    }
}

fn ids(items: &[(ConstraintId, String)]) -> Vec<u64> {
    items.iter().map(|(id, _)| id.0).collect()
}

struct Case {
    name: &'static str,
    constraints: Vec<SlotConstraint>,
    reference_paths: Vec<String>,
    last_input: &'static str,
    cap: usize,
    included: Vec<u64>,
    omitted: Vec<u64>,
    tiers: Vec<ConstraintTier>,
}

fn slot_cases() -> Vec<Case> {
    use ConstraintTier::{All, Omitted, Relevance, Scope};

    let paths = |items: &[&str]| items.iter().map(|path| (*path).to_string()).collect();

    vec![
        Case {
            name: "all constraints fit in registration order",
            constraints: vec![
                constraint(1, "keep errors in English", &[]),
                constraint(2, "do not touch src/db.rs", &["src/db.rs"]),
            ],
            reference_paths: Vec::new(),
            last_input: "",
            cap: 1_000,
            included: vec![1, 2],
            omitted: vec![],
            tiers: vec![All, Relevance],
        },
        Case {
            name: "all then scope then relevance, with the newer all first",
            constraints: vec![
                constraint(1, "relevance: migrate database", &["db/schema.sql"]),
                constraint(2, "scope: api folder", &["src/api/"]),
                constraint(3, "all older", &[]),
                constraint(4, "all newer", &[]),
            ],
            reference_paths: paths(&["src/api/mod.rs"]),
            last_input: "migrate",
            cap: ("all newer".len() + SEPARATOR) + ("all older".len() + SEPARATOR),
            included: vec![3, 4],
            omitted: vec![1, 2],
            tiers: vec![Omitted, Omitted, All, All],
        },
        Case {
            name: "the newer all constraint wins a single place over the older one",
            constraints: vec![
                constraint(1, "all older", &[]),
                constraint(2, "all newer", &[]),
            ],
            reference_paths: Vec::new(),
            last_input: "",
            cap: "all newer".len() + SEPARATOR,
            included: vec![2],
            omitted: vec![1],
            tiers: vec![Omitted, All],
        },
        Case {
            name: "an older scope constraint beats a newer relevance one",
            constraints: vec![
                constraint(1, "scope: api folder", &["src/api/"]),
                constraint(2, "migrate api folder", &["db/schema.sql"]),
            ],
            reference_paths: paths(&["src/api/mod.rs"]),
            last_input: "migrate",
            cap: "migrate api folder".len() + SEPARATOR,
            included: vec![1],
            omitted: vec![2],
            tiers: vec![Scope, Omitted],
        },
    ]
}

fn ranking_cases() -> Vec<Case> {
    use ConstraintTier::{All, Omitted, Relevance, Scope};

    let paths = |items: &[&str]| items.iter().map(|path| (*path).to_string()).collect();

    vec![
        Case {
            name: "scope ranks by overlapping paths then newest",
            constraints: vec![
                constraint(1, "one path", &["a.rs"]),
                constraint(2, "two paths", &["a.rs", "b.rs"]),
                constraint(3, "one path newer", &["b.rs"]),
            ],
            reference_paths: paths(&["a.rs", "b.rs"]),
            last_input: "",
            cap: "two paths".len() + SEPARATOR + "one path newer".len() + SEPARATOR,
            included: vec![2, 3],
            omitted: vec![1],
            tiers: vec![Omitted, Scope, Scope],
        },
        Case {
            name: "an older multi-path scope constraint beats a newer single-path one",
            constraints: vec![
                constraint(1, "two paths", &["a.rs", "b.rs"]),
                constraint(2, "one path newer", &["b.rs"]),
            ],
            reference_paths: paths(&["a.rs", "b.rs"]),
            last_input: "",
            cap: "one path newer".len() + SEPARATOR,
            included: vec![1],
            omitted: vec![2],
            tiers: vec![Scope, Omitted],
        },
        Case {
            name: "relevance ranks by shared words, an older strong match beats a newer weak one",
            constraints: vec![
                constraint(1, "run migrations in db/schema.sql", &["db/schema.sql"]),
                constraint(2, "format docs/readme.md headings", &["docs/readme.md"]),
            ],
            reference_paths: Vec::new(),
            last_input: "please run the migrations",
            cap: "run migrations in db/schema.sql".len() + SEPARATOR,
            included: vec![1],
            omitted: vec![2],
            tiers: vec![Relevance, Omitted],
        },
        Case {
            name: "a constraint too big for the slot does not block smaller ones",
            constraints: vec![
                constraint(1, "small", &[]),
                constraint(2, &"big ".repeat(100), &[]),
                constraint(3, "tiny", &[]),
            ],
            reference_paths: Vec::new(),
            last_input: "",
            cap: 40,
            included: vec![1, 3],
            omitted: vec![2],
            tiers: vec![All, Omitted, All],
        },
        Case {
            name: "a mixed slot fills all then scope and omits the rest",
            constraints: vec![
                constraint(1, "x y", &["a.rs"]),
                constraint(2, "y z", &["b.rs"]),
                constraint(3, "z", &[]),
            ],
            reference_paths: paths(&["a.rs"]),
            last_input: "y",
            cap: 12,
            included: vec![1, 3],
            omitted: vec![2],
            tiers: vec![Scope, Omitted, All],
        },
    ]
}

#[test]
fn slot_selection_follows_tier_strength_and_registration_order() {
    for case in slot_cases().into_iter().chain(ranking_cases()) {
        let fill = || {
            fill_constraint_slot(
                &case.constraints,
                context(&case.reference_paths, case.last_input),
                case.cap,
                SEPARATOR,
            )
        };
        let slot = fill();

        assert_eq!(
            ids(&slot.included),
            case.included,
            "{}: included",
            case.name
        );
        assert_eq!(ids(&slot.omitted), case.omitted, "{}: omitted", case.name);
        let tiers: Vec<ConstraintTier> = slot.tiers.iter().map(|(_, tier)| *tier).collect();
        assert_eq!(tiers, case.tiers, "{}: tiers", case.name);
        assert_eq!(
            slot,
            fill(),
            "{}: same input gives the same slot",
            case.name
        );
    }
}

#[test]
fn folder_scope_overlaps_files_inside_only() {
    assert!(overlaps("src/api/", "src/api/mod.rs"));
    assert!(overlaps("./src/api", "src/api/mod.rs"));
    assert!(overlaps("src/db.rs", "src/db.rs"));
    assert!(overlaps("src/db.rs", "/work/repo/src/db.rs"));
    assert!(overlaps("src/api/", "/work/repo/src/api/mod.rs"));
    assert!(!overlaps("db.rs", "/work/repo/src/mydb.rs"));
    assert!(!overlaps("src/api/", "src/apiary/mod.rs"));
    assert!(!overlaps("src/db.rs", "src/db.rs.bak"));
}
