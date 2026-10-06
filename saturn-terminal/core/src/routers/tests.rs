use super::*;

const REVISION: ChatRevision = ChatRevision(3);
const SETTINGS: SettingsRevision = SettingsRevision(2);

fn models() -> Vec<String> {
    vec!["model-a".to_string(), "model-b".to_string()]
}

fn request(running: bool, has_held: bool) -> RouterRequest {
    RouterRequest {
        model: "router".into(),
        state: "state".into(),
        sets: questions_for_input(
            running,
            false,
            has_held,
            &models(),
            ConstraintQuestion::Without,
        ),
    }
}

// cost: time O(a), heap O(a), stack O(1)
// vars: a = 답 수
// basis: estimate
fn response(answers: Vec<(&str, Answer)>) -> RouterResponse {
    RouterResponse {
        model: "router".into(),
        answers: answers
            .into_iter()
            .map(|(id, answer)| (id.to_string(), answer))
            .collect(),
        tokens: (0, 0),
    }
}

fn decide(request: &RouterRequest, response: &RouterResponse, method: Method) -> RouteDecision {
    decide_route(
        (request, response),
        &Thresholds::default(),
        method,
        REVISION,
        SETTINGS,
    )
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 질문 수
// basis: estimate
fn ids(sets: &[(QuestionSetId, Vec<Question>)]) -> Vec<String> {
    sets.iter()
        .flat_map(|(_, questions)| questions.iter().map(|question| question.id.clone()))
        .collect()
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 질문 수
// basis: estimate
fn asks(sets: &[(QuestionSetId, Vec<Question>)], id: &str) -> bool {
    ids(sets).iter().any(|asked| asked == id)
}

// cost: time O(f), heap O(1), stack O(1)
// vars: f = 대체 규칙 기록 수
// basis: estimate
fn has_fallback(decision: &RouteDecision, id: &str) -> bool {
    decision.fallbacks.iter().any(|fallback| fallback == id)
}

#[test]
fn confidence_follows_the_formula() {
    // (사례, 답, 예상 확신도)
    let cases = [
        ("uniform choice", Answer::Choice(vec![0.25; 4]), 0.0),
        ("certain choice", Answer::Choice(vec![0.0, 1.0, 0.0]), 1.0),
        (
            "formula",
            Answer::Choice(vec![0.7, 0.2, 0.1]),
            (3.0 * 0.7 - 1.0) / 2.0,
        ),
        ("noul high", Answer::Noul(0.9), 0.8),
        ("noul low", Answer::Noul(0.1), 0.8),
        ("noul even", Answer::Noul(0.5), 0.0),
        ("nan choice", Answer::Choice(vec![f64::NAN, 0.5]), 0.0),
        ("empty choice", Answer::Choice(Vec::new()), 0.0),
        ("nan noul", Answer::Noul(f64::NAN), 0.0),
    ];

    for (name, answer, expected) in cases {
        assert!(
            (answer.confidence() - expected).abs() < 1e-12,
            "{name}: {} != {expected}",
            answer.confidence()
        );
    }
}

#[test]
fn default_thresholds_match_design_table() {
    let thresholds = Thresholds::default();

    assert_eq!(thresholds.keep_current, 0.8);
    assert_eq!(thresholds.is_actionable, 0.7);
    assert_eq!(thresholds.min_confidence, 0.6);
    assert_eq!(thresholds.resume_held, 0.85);
    assert_eq!(thresholds.file_relevant, (0.7, 0.35));
    assert_eq!(thresholds.context_gate, 0.3);
    assert_eq!(thresholds.injection, 0.7);
    assert_eq!(thresholds.progressing, 0.2);
    assert_eq!(thresholds.feedback_cause, 0.7);
    assert_eq!(thresholds.is_constraint, 0.8);
    assert_eq!(thresholds.constraint_ask, 0.7);
    assert_eq!(thresholds.constraint_release, 0.8);
}

#[test]
fn questions_for_input_idle_asks_route_only() {
    let sets = questions_for_input(false, false, false, &models(), ConstraintQuestion::Without);

    assert_eq!(
        ids(&sets),
        vec!["keep_current", "is_actionable", "target_model"]
    );
    assert_eq!((sets[0].0.major, sets[0].0.minor), (1, 0));
}

// cost: time O(q), heap O(q), stack O(1)
// vars: q = 질문 수
// basis: estimate
#[test]
fn questions_for_input_running_adds_relation_and_send_opt() {
    let sets = questions_for_input(true, false, true, &models(), ConstraintQuestion::Without);

    let names: Vec<&str> = sets.iter().map(|(set, _)| set.name.as_str()).collect();
    assert_eq!(names, vec!["route", "relation", "send-opt"]);
    assert!(asks(&sets, "resume_held"));
}

#[test]
fn questions_for_input_pinned_model_skips_target_model() {
    let sets = questions_for_input(false, true, false, &models(), ConstraintQuestion::Without);

    assert!(!asks(&sets, "target_model"));
}

// cost: time O(q·n), heap O(1), stack O(1)
// vars: q = 질문 수, n = 선택지 수
// basis: estimate
#[test]
fn questions_for_input_choice_has_other_option() {
    let sets = questions_for_input(true, false, false, &models(), ConstraintQuestion::Without);

    let choices: Vec<&Vec<String>> = sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .filter_map(|question| match &question.kind {
            AnswerKind::Choice { options } => Some(options),
            _ => None,
        })
        .collect();
    assert!(!choices.is_empty());
    assert!(
        choices
            .iter()
            .all(|options| options.iter().any(|option| option == "other"))
    );
}

#[test]
fn decide_route_keep_current_high_keeps_agent() {
    let request = request(false, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.91)),
        ("is_actionable", Answer::Noul(0.9)),
        ("target_model", Answer::Choice(vec![0.9, 0.05, 0.05])),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::Queue);
    assert!(decision.keep_current);
    assert_eq!(decision.model.as_deref(), Some("model-a"));
    assert!(decision.fallbacks.is_empty());
}

#[test]
fn decide_route_keep_current_low_starts_new_task() {
    let request = request(false, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.1)),
        ("is_actionable", Answer::Noul(0.9)),
        ("target_model", Answer::Choice(vec![0.05, 0.9, 0.05])),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::NewTask);
    assert!(!decision.keep_current);
}

#[test]
fn decide_route_keep_current_middle_falls_back_to_keep() {
    let request = request(false, false);
    let response = response(vec![("keep_current", Answer::Noul(0.5))]);

    let decision = decide(&request, &response, Method::Jev);

    assert!(decision.keep_current);
    assert!(has_fallback(&decision, "keep_current"));
    assert!(has_fallback(&decision, "is_actionable"));
    assert!(has_fallback(&decision, "target_model"));
}

#[test]
fn decide_route_low_confidence_model_falls_back_to_current() {
    let request = request(false, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.9)),
        ("target_model", Answer::Choice(vec![0.4, 0.35, 0.25])),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.model, None);
    assert!(has_fallback(&decision, "target_model"));
}

#[test]
fn decide_route_relation_low_confidence_queues() {
    let request = request(true, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.9)),
        (
            "relation_to_running",
            Answer::Choice(vec![0.25, 0.25, 0.2, 0.2, 0.1]),
        ),
        (
            "steer_or_spawn",
            Answer::Choice(vec![0.97, 0.01, 0.01, 0.01]),
        ),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::Queue);
    assert!(!decision.is_conflict);
    assert!(has_fallback(&decision, "relation_to_running"));
}

#[test]
fn decide_route_refines_and_steer_steers() {
    let request = request(true, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.91)),
        (
            "relation_to_running",
            Answer::Choice(vec![0.96, 0.01, 0.01, 0.01, 0.01]),
        ),
        (
            "steer_or_spawn",
            Answer::Choice(vec![0.97, 0.01, 0.01, 0.01]),
        ),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::Steer);
    assert!(decision.keep_current);
    assert!(!decision.is_conflict);
}

// 근거: #36 결정, 충돌은 대기시키지 않고 끼워 넣는다
#[test]
fn decide_route_conflicts_steers_and_marks_the_conflict() {
    let request = request(true, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.91)),
        (
            "relation_to_running",
            Answer::Choice(vec![0.01, 0.01, 0.01, 0.96, 0.01]),
        ),
        (
            "steer_or_spawn",
            Answer::Choice(vec![0.01, 0.97, 0.01, 0.01]),
        ),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::Steer);
    assert!(decision.is_conflict);
    assert!(decision.keep_current);
}

#[test]
fn decide_route_independent_starts_new_task() {
    let request = request(true, false);
    let response = response(vec![
        ("keep_current", Answer::Noul(0.1)),
        (
            "relation_to_running",
            Answer::Choice(vec![0.01, 0.01, 0.96, 0.01, 0.01]),
        ),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::NewTask);
    assert!(!decision.keep_current);
}

#[test]
fn decide_route_send_low_confidence_queues_to_current() {
    let request = request(true, false);
    let response = response(vec![
        (
            "relation_to_running",
            Answer::Choice(vec![0.96, 0.01, 0.01, 0.01, 0.01]),
        ),
        ("steer_or_spawn", Answer::Choice(vec![0.4, 0.3, 0.3, 0.0])),
    ]);

    let decision = decide(&request, &response, Method::Jev);

    assert_eq!(decision.disposition, Disposition::Queue);
    assert!(decision.keep_current);
    assert!(has_fallback(&decision, "steer_or_spawn"));
}

#[test]
fn decide_route_resume_held_needs_threshold() {
    let request = request(false, true);
    let below = response(vec![("resume_held", Answer::Noul(0.84))]);
    let above = response(vec![("resume_held", Answer::Noul(0.9))]);

    assert!(!decide(&request, &below, Method::Jev).resume_held);
    assert!(decide(&request, &above, Method::Jev).resume_held);
}

#[test]
fn decide_route_saturn_method_ignores_unsure_noul() {
    let request = request(false, false);
    let response = response(vec![("keep_current", Answer::Noul(0.25))]);

    let jev = decide(&request, &response, Method::Jev);
    let saturn = decide(&request, &response, Method::Saturn);

    assert_eq!(jev.disposition, Disposition::NewTask);
    assert_eq!(saturn.disposition, Disposition::Queue);
    assert!(has_fallback(&saturn, "keep_current"));
}

#[test]
fn validate_accepts_only_well_formed_answers() {
    let request = request(false, false);
    let keep = || ("keep_current", Answer::Noul(0.9));
    let actionable = || ("is_actionable", Answer::Noul(0.8));
    let model = |probabilities: Vec<f64>| ("target_model", Answer::Choice(probabilities));
    // (사례, 응답, 통과해야 하는지)
    let cases = [
        (
            "well formed",
            vec![keep(), actionable(), model(vec![0.6, 0.3, 0.1])],
            true,
        ),
        ("missing answer", vec![keep()], false),
        (
            "unknown question",
            vec![
                keep(),
                actionable(),
                model(vec![0.6, 0.3, 0.1]),
                ("model-c", Answer::Noul(0.5)),
            ],
            false,
        ),
        (
            "nan",
            vec![
                ("keep_current", Answer::Noul(f64::NAN)),
                actionable(),
                model(vec![0.6, 0.3, 0.1]),
            ],
            false,
        ),
        (
            "wrong length",
            vec![keep(), actionable(), model(vec![0.6, 0.4])],
            false,
        ),
        (
            "sum not one",
            vec![keep(), actionable(), model(vec![0.6, 0.6, 0.1])],
            false,
        ),
        (
            "sum 0.99 rounded",
            vec![keep(), actionable(), model(vec![0.6, 0.3, 0.09])],
            true,
        ),
        (
            "sum 1.01 rounded",
            vec![keep(), actionable(), model(vec![0.6, 0.3, 0.11])],
            true,
        ),
        (
            "sum beyond tolerance",
            vec![keep(), actionable(), model(vec![0.6, 0.3, 0.12])],
            false,
        ),
        (
            "sum below tolerance",
            vec![keep(), actionable(), model(vec![0.6, 0.3, 0.08])],
            false,
        ),
        (
            "kind mismatch",
            vec![
                ("keep_current", Answer::Choice(vec![0.5, 0.5])),
                actionable(),
                model(vec![0.6, 0.3, 0.1]),
            ],
            false,
        ),
    ];

    for (name, answers, is_valid) in cases {
        let result = validate(&request, &response(answers));

        if is_valid {
            assert!(result.is_ok(), "{name}");
        } else {
            assert!(
                matches!(result, Err(RouterError::Invalid { .. })),
                "{name}: {result:?}"
            );
        }
    }
}

// cost: time O(c), heap O(c), stack O(1)
// vars: c = 후보 수
// basis: estimate
#[test]
fn compact_questions_150_candidates_ask_all() {
    let candidates: Vec<CompactCandidate> = (0..150).map(candidate).collect();

    let (set, questions) = compact_questions(&candidates);

    assert_eq!(set.name, SET_COMPACT);
    assert_eq!(questions.len(), 300);
    assert!(
        questions
            .iter()
            .any(|question| question.id == "call_0_keep")
    );
    assert!(
        questions
            .iter()
            .any(|question| question.id == "result_149_keep")
    );
}

// cost: time O(c), heap O(c), stack O(1)
// vars: c = 후보 수
// basis: estimate
fn candidate(seq: u64) -> CompactCandidate {
    CompactCandidate {
        seq: LedgerSeq(seq),
        call: format!("Read src/file_{seq}.rs"),
        result: "x".repeat(10),
    }
}

// cost: time O(1), heap O(1), stack O(1)
// vars: 없음
// basis: estimate
#[test]
fn compact_questions_carry_candidate_content_and_clip_long_results() {
    let long = CompactCandidate {
        seq: LedgerSeq(4),
        call: "Bash cargo test".into(),
        result: "Z".repeat(COMPACT_RESULT_CHARS + 500),
    };

    let (_, questions) = compact_questions(&[long]);

    let call = &questions[0].text;
    let result = &questions[1].text;
    assert!(call.ends_with("Bash cargo test"));
    assert!(result.contains("Bash cargo test\n\nResult:\n"));
    assert_eq!(result.matches('Z').count(), COMPACT_RESULT_CHARS);
}

#[test]
fn compact_state_keeps_every_input_whole_and_states_the_candidate_scope() {
    // 첫 입력이 정정이고 한 입력이 2,000자를 넘어도 목표와 정정은 줄이지 않는다
    let long = format!("correction {}", "z".repeat(COMPACT_INPUT_CHARS * 2));
    let mut inputs = vec![long.clone()];
    inputs.extend((2..=6).map(|n| format!("input {n}")));

    let state = compact_state(&inputs, 7);

    assert!(state.starts_with("Latest user request:\ninput 6"));
    assert!(state.contains(&format!(
        "- {long}\n- input 2\n- input 3\n- input 4\n- input 5"
    )));
    assert!(state.contains("All 7 tool call records"));
    assert!(state.contains("up to 2000 characters and a result up to 4000 characters"));
    assert!(state.contains("Only tool records are selected"));
    assert_eq!(compact_state(&[], 7), "");
}

#[test]
fn compact_questions_mark_a_cut_and_the_omitted_bytes_are_counted() {
    let long = CompactCandidate {
        seq: LedgerSeq(4),
        call: "Bash cargo test".into(),
        // 한글은 글자당 3바이트다. 근거가 앞 4,000자 밖 중간에 있다
        result: format!(
            "{}중간근거{}",
            "가".repeat(COMPACT_RESULT_CHARS),
            "나".repeat(10)
        ),
    };
    let short = candidate(5);

    let (_, questions) = compact_questions(&[long.clone(), short.clone()]);

    let result = &questions[1].text;
    assert!(!result.contains("중간근거"));
    assert!(result.ends_with("\n[14 more characters not shown]"));
    assert!(!questions[3].text.contains("not shown"));
    assert_eq!(compact_omitted_bytes(&[long, short]), 14 * 3);
}

#[test]
fn compact_overflow_counts_the_bytes_over_the_request_limit() {
    let candidates = [candidate(1)];
    let question = compact_questions(&candidates)
        .1
        .iter()
        .map(split::question_bytes)
        .max()
        .unwrap();
    let room = split::MAX_STATE_AND_QUESTION_BYTES - "m".len() - question;

    assert_eq!(
        compact_overflow_bytes("m", &"s".repeat(room), &candidates),
        0
    );
    assert_eq!(
        compact_overflow_bytes("m", &"s".repeat(room + 9), &candidates),
        9
    );
}

#[test]
fn compact_requests_large_state_splits_and_every_piece_carries_state() {
    let candidates: Vec<CompactCandidate> = (0..1_000).map(candidate).collect();
    let state = "s".repeat(20_000);

    let requests = compact_requests("jev-test", &state, &candidates).unwrap();

    assert!(requests.len() > 1);
    assert!(requests.iter().all(|request| request.state == state));
}

#[test]
fn compact_verdicts_merges_pieces_and_skips_failed_piece() {
    let candidates = [LedgerSeq(7), LedgerSeq(3), LedgerSeq(1)];
    let first = response(vec![
        ("call_7_keep", Answer::Noul(0.9)),
        ("result_7_keep", Answer::Noul(0.1)),
    ]);
    let second = response(vec![("call_1_keep", Answer::Noul(0.4))]);

    let verdicts = compact_verdicts(&candidates, &[first, second]);

    assert_eq!(verdicts, vec![(LedgerSeq(7), 0.9), (LedgerSeq(1), 0.4)]);
}

#[test]
fn compact_verdicts_takes_larger_of_call_and_result() {
    let candidates = [LedgerSeq(7), LedgerSeq(3)];
    let answers = response(vec![
        ("call_7_keep", Answer::Noul(0.2)),
        ("result_7_keep", Answer::Noul(0.65)),
        ("result_3_keep", Answer::Noul(0.3)),
    ]);

    let verdicts = compact_verdicts(&candidates, &[answers]);

    assert_eq!(verdicts, vec![(LedgerSeq(7), 0.65), (LedgerSeq(3), 0.3)]);
}

#[test]
fn compact_verdicts_no_responses_is_empty() {
    assert!(compact_verdicts(&[LedgerSeq(1)], &[]).is_empty());
}

fn constraint_request() -> RouterRequest {
    RouterRequest {
        model: "router".into(),
        state: "state".into(),
        sets: questions_for_input(false, false, false, &models(), ConstraintQuestion::With),
    }
}

fn constraint_response(yes: f64) -> RouterResponse {
    response(vec![
        ("keep_current", Answer::Noul(0.9)),
        ("is_actionable", Answer::Noul(0.9)),
        ("target_model", Answer::Choice(vec![1.0, 0.0, 0.0])),
        ("is_constraint", Answer::Noul(yes)),
    ])
}

fn registration(yes: f64, method: Method) -> constraint::RegistrationAction {
    constraint::read_registration(
        &constraint_request(),
        &constraint_response(yes),
        &Thresholds::default(),
        method,
    )
    .action
}

#[test]
fn questions_with_constraint_use_route_1_1_and_ask_is_constraint() {
    let sets = questions_for_input(false, false, false, &models(), ConstraintQuestion::With);

    assert!(asks(&sets, "is_constraint"));
    assert_eq!((sets[0].0.major, sets[0].0.minor), (1, 1));
    assert!(validate(&constraint_request(), &constraint_response(0.9)).is_ok());
}

#[test]
fn registration_follows_the_three_bands() {
    use constraint::RegistrationAction::{Ask, Auto, Skip};

    assert_eq!(registration(0.85, Method::Jev), Auto);
    assert_eq!(registration(0.8, Method::Jev), Auto);
    assert_eq!(registration(0.75, Method::Jev), Ask);
    assert_eq!(registration(0.7, Method::Jev), Ask);
    assert_eq!(registration(0.65, Method::Jev), Skip);
}

#[test]
fn saturn_method_treats_a_low_confidence_answer_as_ask() {
    use constraint::RegistrationAction::{Ask, Skip};

    assert_eq!(registration(0.5, Method::Saturn), Ask);
    assert_eq!(registration(0.5, Method::Jev), Skip);
}

#[test]
fn registration_without_an_answer_is_skipped_and_noted_as_fallback() {
    let request = constraint_request();
    let empty = response(Vec::new());

    let verdict =
        constraint::read_registration(&request, &empty, &Thresholds::default(), Method::Jev);
    let decision = decide(&request, &empty, Method::Jev);

    assert_eq!(verdict.action, constraint::RegistrationAction::Skip);
    assert_eq!(verdict.probability, None);
    assert!(decision.fallbacks.iter().any(|id| id == "is_constraint"));
}

#[test]
fn registration_ignores_a_question_that_was_not_asked() {
    let verdict = constraint::read_registration(
        &request(false, false),
        &constraint_response(0.95),
        &Thresholds::default(),
        Method::Jev,
    );

    assert_eq!(verdict.action, constraint::RegistrationAction::Skip);
}

#[test]
fn line_answers_pick_sentences_at_or_above_the_ask_threshold() {
    let answers = response(vec![
        ("line_1_is_constraint", Answer::Noul(0.9)),
        ("line_2_is_constraint", Answer::Noul(0.3)),
        ("line_3_is_constraint", Answer::Noul(0.7)),
    ]);

    let picked = constraint::read_lines(&answers, 3, &Thresholds::default());

    assert_eq!(picked, Some(vec![1, 3]));
    let (set, questions) = constraint::line_questions(3);
    assert_eq!(
        (set.name.as_str(), set.major, set.minor),
        ("constraint", 1, 0)
    );
    assert_eq!(questions.len(), 3);
}

#[test]
fn a_missing_line_answer_fails_the_whole_line_question() {
    let answers = response(vec![("line_1_is_constraint", Answer::Noul(0.9))]);

    assert_eq!(
        constraint::read_lines(&answers, 2, &Thresholds::default()),
        None
    );
}

fn change_verdict(
    count: usize,
    probabilities: Vec<f64>,
    method: Method,
) -> constraint::ChangeVerdict {
    let (set, questions) = constraint::change_question(count);
    let request = RouterRequest {
        model: "router".into(),
        state: "state".into(),
        sets: vec![(set, questions)],
    };
    let answers = response(vec![("constraint_change", Answer::Choice(probabilities))]);
    assert!(validate(&request, &answers).is_ok());
    constraint::read_change(&request, &answers, count, &Thresholds::default(), method)
}

#[test]
fn change_answer_reads_target_and_kind_from_the_top_option() {
    use constraint::ChangeKind::{Once, Release, Scoped};
    use constraint::ChangeVerdict::{Apply, None, UnsureKind};

    // 선택지 순서: none, release_1, once_1, scoped_1, release_2, once_2, scoped_2
    let table = [
        (
            vec![0.05, 0.9, 0.02, 0.01, 0.01, 0.01, 0.0],
            Apply {
                target: 0,
                kind: Release,
            },
        ),
        (
            vec![0.02, 0.0, 0.0, 0.0, 0.02, 0.94, 0.02],
            Apply {
                target: 1,
                kind: Once,
            },
        ),
        (
            vec![0.02, 0.0, 0.0, 0.0, 0.0, 0.0, 0.98],
            Apply {
                target: 1,
                kind: Scoped,
            },
        ),
        // 요청 확률은 0.85지만 종류 몫이 0.5/0.85 미만이라 종류를 확신하지 못한다
        (
            vec![0.15, 0.42, 0.38, 0.05, 0.0, 0.0, 0.0],
            UnsureKind {
                target: 0,
                kind: Release,
            },
        ),
        // 요청 확률 0.7은 기준 미만
        (vec![0.3, 0.7, 0.0, 0.0, 0.0, 0.0, 0.0], None),
        (vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], None),
    ];

    for (probabilities, expected) in table {
        assert_eq!(
            change_verdict(2, probabilities.clone(), Method::Jev),
            expected,
            "{probabilities:?}"
        );
    }
}

#[test]
fn change_answer_with_a_wrong_shape_or_low_confidence_changes_nothing() {
    use constraint::ChangeVerdict::None;

    let (set, questions) = constraint::change_question(1);
    let request = RouterRequest {
        model: "router".into(),
        state: "state".into(),
        sets: vec![(set, questions)],
    };
    let thresholds = Thresholds::default();
    let short = response(vec![("constraint_change", Answer::Choice(vec![0.1, 0.9]))]);
    let noul = response(vec![("constraint_change", Answer::Noul(0.9))]);
    let missing = response(Vec::new());
    for answers in [&short, &noul, &missing] {
        assert_eq!(
            constraint::read_change(&request, answers, 1, &thresholds, Method::Jev),
            None
        );
    }
    let unasked = constraint::read_change(
        &constraint_request(),
        &response(vec![(
            "constraint_change",
            Answer::Choice(vec![0.0, 1.0, 0.0, 0.0]),
        )]),
        1,
        &thresholds,
        Method::Jev,
    );
    assert_eq!(unasked, None);
    // 분포가 퍼져 확신도가 낮은 saturn 답은 요청 확률이 높아도 적용하지 않는다
    assert_eq!(
        change_verdict(1, vec![0.0, 0.34, 0.33, 0.33], Method::Saturn),
        None
    );
}

#[test]
fn scoped_condition_is_a_contiguous_span_of_the_input() {
    let input = "  tests 폴더에서만 영어 강제를 풀어줘  ";
    let condition = constraint::scoped_condition(input).unwrap();
    assert!(input.contains(&condition));
    assert_eq!(condition, "tests 폴더에서만 영어 강제를 풀어줘");

    let long = "가".repeat(500);
    let cut = constraint::scoped_condition(&long).unwrap();
    assert_eq!(cut.chars().count(), 200);
    assert!(long.starts_with(&cut));
    assert_eq!(constraint::scoped_condition("   "), None);
}
