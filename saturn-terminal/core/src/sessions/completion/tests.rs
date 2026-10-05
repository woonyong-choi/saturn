use saturn_protocol::event::ToolDetail;
use saturn_protocol::ids::SubagentId;

use super::*;
use crate::sessions::changes::{ChangeKind, FileChange};

const MAIN: AgentId = AgentId(1);

fn command(seq: u64, id: &str, command: &str) -> (u64, ProviderEvent) {
    (
        seq,
        ProviderEvent::ToolCall {
            agent: MAIN,
            subagent: None,
            call_id: id.to_owned(),
            activity: Activity::RunningCommand {
                command: command.to_owned(),
            },
            detail: ToolDetail::default(),
        },
    )
}

fn edit(seq: u64, id: &str) -> (u64, ProviderEvent) {
    (
        seq,
        ProviderEvent::ToolCall {
            agent: MAIN,
            subagent: None,
            call_id: id.to_owned(),
            activity: Activity::EditingFile,
            detail: ToolDetail {
                category: ToolCategory::FileEdit,
                ..ToolDetail::default()
            },
        },
    )
}

fn result(seq: u64, id: &str, exit_code: Option<i32>) -> (u64, ProviderEvent) {
    (
        seq,
        ProviderEvent::ToolResult {
            agent: MAIN,
            subagent: None,
            call_id: id.to_owned(),
            output: String::new(),
            exit_code,
        },
    )
}

fn changed(actors: &[&str], partial: bool) -> ChangeSet {
    ChangeSet {
        files: vec![FileChange {
            path: "/work/a.rs".to_owned(),
            kind: ChangeKind::Modified,
            actors: actors.iter().map(|actor| (*actor).to_owned()).collect(),
        }],
        is_partial: partial,
    }
}

fn judged(
    changes: Option<&ChangeSet>,
    events: &[(u64, ProviderEvent)],
    checks: &[&str],
) -> (EvidenceState, Option<UnverifiedReason>, Vec<u64>) {
    let checks: Vec<String> = checks.iter().map(|check| (*check).to_owned()).collect();
    let found = judge(EvidenceInput {
        changes,
        events,
        checks: &checks,
    });
    (found.state, found.reason, found.events)
}

use EvidenceState::{NotApplicable, Unverified, Verified};
use UnverifiedReason::{
    CheckFailed, EditedDuringCheck, NotChecked, OrderUnknown, PartialSnapshot, TreeNotIdle,
    Unmeasured,
};

type Verdict = (EvidenceState, Option<UnverifiedReason>, Vec<u64>);

/// 사례 하나: 이름, 수정 목록, 이벤트, 설정한 검사, 기대 판정.
struct Case {
    name: &'static str,
    changes: Option<ChangeSet>,
    events: Vec<(u64, ProviderEvent)>,
    checks: Vec<&'static str>,
    expected: Verdict,
}

fn case(
    name: &'static str,
    changes: Option<ChangeSet>,
    events: Vec<(u64, ProviderEvent)>,
    checks: &[&'static str],
    expected: Verdict,
) -> Case {
    Case {
        name,
        changes,
        events,
        checks: checks.to_vec(),
        expected,
    }
}

fn run(cases: Vec<Case>) {
    for case in cases {
        assert_eq!(
            judged(case.changes.as_ref(), &case.events, &case.checks),
            case.expected,
            "{}",
            case.name
        );
    }
}

fn pass(from: u64) -> Vec<(u64, ProviderEvent)> {
    vec![
        command(from, "t", "cargo test"),
        result(from + 1, "t", Some(0)),
    ]
}

fn edited() -> Option<ChangeSet> {
    Some(changed(&["main agent"], false))
}

fn edit_done(seq: u64) -> Vec<(u64, ProviderEvent)> {
    vec![edit(seq, "e"), result(seq + 1, "e", None)]
}

/// 판정이 검사까지 가기 전에 정해지는 사례와 검사를 설정하지 않은 사례.
#[test]
fn changes_decide_whether_a_check_is_needed_at_all() {
    let partial_empty = ChangeSet {
        files: vec![],
        is_partial: true,
    };
    run(vec![
        case(
            "a run that changed nothing",
            Some(ChangeSet::default()),
            vec![],
            &["cargo test"],
            (NotApplicable, None, vec![]),
        ),
        case(
            "a run whose changes were not measured",
            None,
            vec![],
            &["cargo test"],
            (Unverified, Some(Unmeasured), vec![]),
        ),
        case(
            "a partial snapshot even when nothing was listed",
            Some(partial_empty),
            vec![],
            &["cargo test"],
            (Unverified, Some(PartialSnapshot), vec![]),
        ),
        case(
            "a partial snapshot with a passing check",
            Some(changed(&["main agent"], true)),
            [edit_done(1), pass(3)].concat(),
            &["cargo test"],
            (Unverified, Some(PartialSnapshot), vec![]),
        ),
        case(
            "no check configured",
            edited(),
            [vec![edit(1, "e")], pass(2)].concat(),
            &[],
            (Unverified, Some(NotChecked), vec![]),
        ),
        case(
            "an unlisted change and no shell command to order it",
            Some(changed(&[], false)),
            pass(1),
            &["cargo test"],
            (Unverified, Some(OrderUnknown), vec![]),
        ),
    ]);
}

/// 마지막 수정과 검사의 순서, 종료 코드, 검사가 여럿인 경우.
#[test]
fn only_a_check_that_passed_after_the_last_edit_verifies() {
    let failing = || vec![command(2, "t", "cargo test"), result(3, "t", Some(101))];
    run(vec![
        case(
            "a check that passed after the last edit",
            edited(),
            [edit_done(1), pass(3)].concat(),
            &["cargo test"],
            (Verified, None, vec![4]),
        ),
        case(
            "extra arguments on the configured check",
            edited(),
            vec![
                edit(1, "e"),
                command(2, "t", "cargo test --workspace --locked"),
                result(3, "t", Some(0)),
            ],
            &["cargo   test"],
            (Verified, None, vec![3]),
        ),
        case(
            "the check never ran",
            edited(),
            edit_done(1),
            &["cargo test"],
            (Unverified, Some(NotChecked), vec![]),
        ),
        case(
            "the check ran before the last edit",
            edited(),
            [pass(1), edit_done(3)].concat(),
            &["cargo test"],
            (Unverified, Some(NotChecked), vec![]),
        ),
        case(
            "the check failed after the last edit",
            edited(),
            [vec![edit(1, "e")], failing()].concat(),
            &["cargo test"],
            (Unverified, Some(CheckFailed), vec![]),
        ),
        case(
            "a later passing run replaces an earlier failure",
            edited(),
            [
                vec![edit(1, "e")],
                failing(),
                vec![command(4, "t2", "cargo test"), result(5, "t2", Some(0))],
            ]
            .concat(),
            &["cargo test"],
            (Verified, None, vec![5]),
        ),
        case(
            "the check ended without an exit code",
            edited(),
            vec![
                edit(1, "e"),
                command(2, "t", "cargo test"),
                result(3, "t", None),
            ],
            &["cargo test"],
            (Unverified, Some(NotChecked), vec![]),
        ),
        case(
            "the check has no result yet",
            edited(),
            vec![edit(1, "e"), command(2, "t", "cargo test")],
            &["cargo test"],
            (Unverified, Some(NotChecked), vec![]),
        ),
        case(
            "every configured check must pass",
            edited(),
            [
                vec![edit(1, "e")],
                pass(2),
                vec![command(4, "c", "cargo clippy"), result(5, "c", Some(0))],
            ]
            .concat(),
            &["cargo test", "cargo clippy"],
            (Verified, None, vec![3, 5]),
        ),
        case(
            "one of two configured checks is missing",
            edited(),
            [vec![edit(1, "e")], pass(2)].concat(),
            &["cargo test", "cargo clippy"],
            (Unverified, Some(NotChecked), vec![]),
        ),
    ]);
}

/// 검사와 겹치는 수정, 검사 뒤의 읽기 전용 명령, 셸 수정.
#[test]
fn edits_that_overlap_or_follow_the_check_are_told_apart_from_harmless_commands() {
    run(vec![
        case(
            "an edit between the start and the end of the check",
            edited(),
            vec![
                command(1, "t", "cargo test"),
                edit(2, "e"),
                result(3, "t", Some(0)),
            ],
            &["cargo test"],
            (Unverified, Some(EditedDuringCheck), vec![]),
        ),
        case(
            "a shell command still running when the check starts",
            edited(),
            vec![
                command(1, "s", "sed -i s/a/b/ a.rs"),
                command(2, "t", "cargo test"),
                result(3, "t", Some(0)),
                result(4, "s", Some(0)),
            ],
            &["cargo test"],
            (Unverified, Some(EditedDuringCheck), vec![]),
        ),
        case(
            "a shell edit before the check",
            Some(changed(&[], false)),
            [
                vec![
                    command(1, "s", "sed -i s/a/b/ a.rs"),
                    result(2, "s", Some(0)),
                ],
                pass(3),
            ]
            .concat(),
            &["cargo test"],
            (Verified, None, vec![4]),
        ),
        case(
            "a read-only command after the check is not an edit",
            edited(),
            [
                vec![edit(1, "e")],
                pass(2),
                vec![command(4, "d", "git diff --stat"), result(5, "d", Some(0))],
            ]
            .concat(),
            &["cargo test"],
            (Verified, None, vec![3]),
        ),
        case(
            "a shell command after the check makes it stale",
            edited(),
            [
                vec![edit(1, "e")],
                pass(2),
                vec![command(4, "m", "cargo fmt"), result(5, "m", Some(0))],
            ]
            .concat(),
            &["cargo test"],
            (Unverified, Some(NotChecked), vec![]),
        ),
    ]);
}

/// 통과처럼 보이지만 종료 코드가 검사의 결과가 아닌 명령.
#[test]
fn commands_that_hide_a_failure_or_do_not_run_the_check_are_not_evidence() {
    let edited = changed(&["main agent"], false);
    let commands = [
        ("echo", "echo cargo test passed"),
        ("true after the check", "cargo test || true"),
        ("pipe", "cargo test | tee out.txt"),
        ("and then echo", "cargo test && echo ok"),
        ("semicolon", "cargo test; echo ok"),
        ("redirect", "cargo test > out.txt"),
        ("substitution", "echo $(cargo test)"),
        ("background", "cargo test &"),
        ("subshell", "(cargo test)"),
        ("other command", "cargo build"),
    ];
    for (name, text) in commands {
        let events = [edit(1, "e"), command(2, "t", text), result(3, "t", Some(0))];
        let found = judged(Some(&edited), &events, &["cargo test"]);
        assert_eq!(found.0, Unverified, "{name}");
        assert_eq!(found.1, Some(NotChecked), "{name}");
    }

    let events = [
        edit(1, "e"),
        command(2, "t", "echo ok"),
        result(3, "t", Some(0)),
    ];
    for trivial in [
        "echo ok",
        "true",
        "printf ok",
        "exit 0",
        "test -f a.rs",
        ":",
    ] {
        assert_eq!(
            judged(Some(&edited), &events, &[trivial]).1,
            Some(NotChecked),
            "{trivial}"
        );
    }
    let compound = ["cargo test || true", "cargo test | cat"];
    assert_eq!(
        judged(Some(&edited), &events, &compound).1,
        Some(NotChecked)
    );
}

#[test]
fn a_subagent_left_running_or_interrupted_is_never_verified() {
    let edited = changed(&["main agent"], false);
    let started = |seq| {
        (
            seq,
            ProviderEvent::SubagentStarted {
                agent: MAIN,
                subagent: SubagentId("child".to_owned()),
                parent: None,
            },
        )
    };
    let ended = |seq| {
        (
            seq,
            ProviderEvent::SubagentEnded {
                agent: MAIN,
                subagent: SubagentId("child".to_owned()),
            },
        )
    };
    let interrupted = |seq| {
        (
            seq,
            ProviderEvent::SubagentInterrupted {
                agent: MAIN,
                subagent: SubagentId("child".to_owned()),
            },
        )
    };
    let check = || [command(10, "t", "cargo test"), result(11, "t", Some(0))];
    let cases = [
        (
            "child still running",
            vec![edit(1, "e"), started(2)],
            TreeNotIdle,
        ),
        (
            "child interrupted by a crash",
            vec![edit(1, "e"), started(2), interrupted(3)],
            TreeNotIdle,
        ),
    ];
    for (name, mut events, reason) in cases {
        events.extend(check());
        assert_eq!(
            judged(Some(&edited), &events, &["cargo test"]),
            (Unverified, Some(reason), vec![]),
            "{name}"
        );
    }
    let mut finished = vec![edit(1, "e"), started(2), ended(3)];
    finished.extend(check());
    assert_eq!(
        judged(Some(&edited), &finished, &["cargo test"]).0,
        Verified
    );
}
