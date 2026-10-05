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

#[test]
fn all_constraints_fit_in_registration_order() {
    let constraints = [
        constraint(1, "keep errors in English", &[]),
        constraint(2, "do not touch src/db.rs", &["src/db.rs"]),
    ];

    let slot = fill_constraint_slot(&constraints, context(&[], ""), 1_000, SEPARATOR);

    assert_eq!(ids(&slot.included), [1, 2]);
    assert!(slot.omitted.is_empty());
}

#[test]
fn tiers_go_all_then_scope_then_relevance_and_newer_first() {
    let paths = vec!["src/api/mod.rs".to_string()];
    let constraints = [
        constraint(1, "relevance: migrate database", &["db/schema.sql"]),
        constraint(2, "scope: api folder", &["src/api/"]),
        constraint(3, "all older", &[]),
        constraint(4, "all newer", &[]),
    ];
    // 두 칸만 들어가는 상한
    let cap = ("all newer".len() + SEPARATOR) + ("all older".len() + SEPARATOR);

    let slot = fill_constraint_slot(&constraints, context(&paths, "migrate"), cap, SEPARATOR);

    assert_eq!(ids(&slot.included), [3, 4]);
    assert_eq!(ids(&slot.omitted), [1, 2]);
    assert_eq!(
        slot.tiers,
        [
            (ConstraintId(1), ConstraintTier::Omitted),
            (ConstraintId(2), ConstraintTier::Omitted),
            (ConstraintId(3), ConstraintTier::All),
            (ConstraintId(4), ConstraintTier::All),
        ]
    );
}

#[test]
fn scope_tier_ranks_by_overlapping_paths_then_newest() {
    let paths = vec!["a.rs".to_string(), "b.rs".to_string()];
    let constraints = [
        constraint(1, "one path", &["a.rs"]),
        constraint(2, "two paths", &["a.rs", "b.rs"]),
        constraint(3, "one path newer", &["b.rs"]),
    ];
    let cap = "two paths".len() + SEPARATOR + "one path newer".len() + SEPARATOR;

    let slot = fill_constraint_slot(&constraints, context(&paths, ""), cap, SEPARATOR);

    assert_eq!(ids(&slot.included), [2, 3]);
    assert_eq!(ids(&slot.omitted), [1]);
}

#[test]
fn relevance_tier_ranks_by_shared_words_with_the_last_input() {
    let constraints = [
        constraint(1, "format docs/readme.md headings", &["docs/readme.md"]),
        constraint(2, "run migrations in db/schema.sql", &["db/schema.sql"]),
    ];
    let cap = constraints[1].rule.len() + SEPARATOR;

    let slot = fill_constraint_slot(
        &constraints,
        context(&[], "please run the migrations"),
        cap,
        SEPARATOR,
    );

    assert_eq!(ids(&slot.included), [2]);
    assert_eq!(slot.tiers[1], (ConstraintId(2), ConstraintTier::Relevance));
}

#[test]
fn a_constraint_too_big_for_the_slot_does_not_block_smaller_ones() {
    let constraints = [
        constraint(1, "small", &[]),
        constraint(2, &"big ".repeat(100), &[]),
        constraint(3, "tiny", &[]),
    ];

    let slot = fill_constraint_slot(&constraints, context(&[], ""), 40, SEPARATOR);

    assert_eq!(ids(&slot.included), [1, 3]);
    assert_eq!(ids(&slot.omitted), [2]);
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

#[test]
fn same_input_gives_same_slot() {
    let paths = vec!["a.rs".to_string()];
    let constraints = [
        constraint(1, "x y", &["a.rs"]),
        constraint(2, "y z", &["b.rs"]),
        constraint(3, "z", &[]),
    ];

    let first = fill_constraint_slot(&constraints, context(&paths, "y"), 12, SEPARATOR);
    let second = fill_constraint_slot(&constraints, context(&paths, "y"), 12, SEPARATOR);

    assert_eq!(first, second);
}
