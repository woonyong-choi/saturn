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
    let candidates: Vec<LedgerSeq> = (0..150).map(LedgerSeq).collect();

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
#[test]
fn compact_requests_large_state_splits_and_every_piece_carries_state() {
    let candidates: Vec<LedgerSeq> = (0..1_000).map(LedgerSeq).collect();
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
