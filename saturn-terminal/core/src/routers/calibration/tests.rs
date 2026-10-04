const IRREVERSIBLE_FLOOR: f64 = 0.8;

use super::*;

const QUESTION: &str = "keep_current";

fn state() -> ThresholdState {
    ThresholdState::new(QUESTION, 0.8, (0.5, 0.95))
}

fn observation(probability: f64, signal: Signal) -> Observation {
    Observation {
        question: QUESTION.to_string(),
        probability,
        threshold: 0.8,
        asked_with: 1.0,
        is_asked: false,
        signal,
        asked_answer: None,
    }
}

fn asked(probability: f64, signal: Signal, q: f64) -> Observation {
    Observation {
        asked_with: q,
        is_asked: true,
        ..observation(probability, signal)
    }
}

fn acted(probability: f64, signal: Signal) -> Observation {
    Observation {
        threshold: 0.9,
        ..observation(probability, signal)
    }
}

fn observation_below(probability: f64, signal: Signal) -> Observation {
    Observation {
        threshold: 0.9,
        ..observation(probability, signal)
    }
}

fn skipped_asked(probability: f64, answer: AskedAnswer, q: f64) -> Observation {
    Observation {
        threshold: 0.9,
        asked_answer: Some(answer),
        ..asked(probability, Signal::Unconfirmed, q)
    }
}

// cost: time O(g), heap O(g), stack O(1)
// vars: g = 답 수
// basis: estimate
fn label(answers: &[&str], hindsight: Option<&str>) -> Label {
    Label {
        question: QUESTION.to_string(),
        grader_answers: answers.iter().map(|answer| (*answer).to_string()).collect(),
        order_consistent: true,
        hindsight: hindsight.map(String::from),
        signal: None,
        human: false,
    }
}

fn report() -> EvalReport {
    EvalReport {
        accuracy_diff_ci: (-0.5, 2.0),
        brier: (0.10, 0.12),
        ece: (0.03, 0.04),
        coverage: (0.9, 0.9),
        order_consistency: (0.97, 0.96),
    }
}

#[test]
fn new_starts_at_center_with_default_target() {
    let state = state();

    assert_eq!(state.current(), 0.8);
    assert_eq!(state.target_wrong_rate, DEFAULT_TARGET_WRONG_RATE);
}

#[test]
fn current_is_clamped_to_bounds() {
    let mut state = ThresholdState::new(QUESTION, 0.94, (0.5, 0.95));
    state.fast_offset = 0.05;

    assert_eq!(state.current(), 0.95);
}

#[test]
fn observe_wrong_raises_and_missed_lowers() {
    let mut raised = state();
    let mut lowered = state();

    raised.observe(&observation(0.85, Signal::Wrong));
    lowered.observe(&observation(0.7, Signal::Missed));

    assert!(raised.current() > 0.8);
    assert!(lowered.current() < 0.8);
}

#[test]
fn observe_unconfirmed_or_other_question_is_ignored() {
    let mut state = state();
    let mut other = observation(0.85, Signal::Wrong);
    other.question = "is_actionable".into();

    state.observe(&observation(0.85, Signal::Unconfirmed));
    state.observe(&other);

    assert_eq!(state.current(), 0.8);
}

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 넣는 신호 수
// basis: estimate
#[test]
fn observe_never_leaves_fast_range() {
    let mut state = state();
    let wrong = asked(0.85, Signal::Wrong, 0.0001);

    for _ in 0..1_000 {
        state.observe(&wrong);
    }

    assert!((state.current() - (0.8 + FAST_RANGE)).abs() < 1e-12);
}

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 넣는 신호 수
// basis: estimate
#[test]
fn observe_irreversible_floor_holds() {
    let mut state = ThresholdState::new(QUESTION, 0.82, (IRREVERSIBLE_FLOOR, 0.99));

    for _ in 0..1_000 {
        state.observe(&observation(0.5, Signal::Missed));
    }

    assert!(state.current() >= IRREVERSIBLE_FLOOR);
}

#[test]
fn observe_behavior_signal_moves_fixed_step_without_ask_weight() {
    let mut wrong = state();
    let mut missed = state();
    let mut rarely_asked = observation(0.85, Signal::Wrong);
    rarely_asked.asked_with = 0.01;

    wrong.observe(&rarely_asked);
    missed.observe(&observation(0.7, Signal::Missed));

    assert!((wrong.fast_offset - FAST_STEP * 0.95).abs() < 1e-12);
    assert!((missed.fast_offset + FAST_STEP * 0.05).abs() < 1e-12);
}

#[test]
fn observe_asked_answer_is_weighted_by_inverse_q() {
    let mut state = state();

    state.observe(&asked(0.85, Signal::Wrong, 0.1));

    assert!((state.fast_offset - FAST_STEP * 10.0 * 0.95).abs() < 1e-12);
}

#[test]
fn observe_asked_weight_is_capped_by_min_ask() {
    let mut state = state();

    state.observe(&asked(0.7, Signal::Missed, 0.0001));

    assert!((state.fast_offset + FAST_STEP / MIN_ASK * 0.05).abs() < 1e-12);
}

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 넣는 신호 수
// basis: estimate
#[test]
fn observe_step_stays_fixed_after_many_signals() {
    let mut state = state();
    for _ in 0..300 {
        state.observe(&observation(0.7, Signal::Missed));
    }
    let before = state.fast_offset;

    state.observe(&observation(0.85, Signal::Wrong));

    assert!((state.fast_offset - before - FAST_STEP * 0.95).abs() < 1e-12);
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn observe_follows_simulation_b_on_same_signals() {
    // 시뮬레이션 B(고정 폭 0.002)가 같은 입력 열에서 낸 이동 후 값이다.
    let signals = [
        (Signal::Wrong, None, 0.0019),
        (Signal::Missed, None, 0.0018),
        (Signal::Wrong, Some(0.1), 0.0208),
        (Signal::Missed, Some(0.04), 0.0183),
        (Signal::Missed, Some(0.01), 0.0083),
        (Signal::Wrong, None, 0.0102),
        (Signal::Missed, None, 0.0101),
        (Signal::Wrong, Some(0.05), 0.0481),
    ];
    let mut state = state();

    for (signal, q, expected) in signals {
        let observation = match q {
            Some(q) => asked(0.85, signal, q),
            None => observation(0.85, signal),
        };
        state.observe(&observation);

        assert!((state.fast_offset - expected).abs() < 1e-9);
    }
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
fn results(count: usize, probability: f64, signal: Signal) -> Vec<Observation> {
    (0..count).map(|_| acted(probability, signal)).collect()
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
/// 확률 0.50~0.99 50단계에 1,000건씩이고 틀림 비율이 `0.6 × (1 − p)`다. 0.9 이상만 행동했고, 아래는 10건에 1건(q = 0.1)만 물었다.
fn population() -> Vec<Observation> {
    let mut records = Vec::new();
    for level in 50..100_u32 {
        let probability = f64::from(level) / 100.0;
        let wrong_count = 10 * (60.0 * (1.0 - probability)).round() as u32;
        for index in 0..1_000_u32 {
            let is_wrong = index * 7 % 1_000 < wrong_count;
            let is_asked = index % 10 == 0;
            let record = match (probability >= 0.9, is_asked) {
                (true, _) if is_wrong => acted(probability, Signal::Wrong),
                (true, _) => acted(probability, Signal::Unconfirmed),
                (false, true) => {
                    let answer = if is_wrong {
                        AskedAnswer::Wrong
                    } else {
                        AskedAnswer::Correct
                    };
                    skipped_asked(probability, answer, 0.1)
                }
                (false, false) => Observation {
                    asked_with: 0.1,
                    ..acted(probability, Signal::Unconfirmed)
                },
            };
            records.push(record);
        }
    }
    records
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
/// 틀림 비율은 `(1000 − p)² / 800`, 기준값은 판단마다 0.75, 0.80, 0.85이고 q는 0.1, 0.05, 0.02, 0.01을 돈다. 시뮬레이션 S1q와 같은 입력이다.
fn simulated_records() -> Vec<Observation> {
    let asked_milli = [100_u64, 50, 20, 10];
    (0..8_000_u64)
        .map(|index| {
            let probability_milli = 500 + index * 7_919 % 500;
            let threshold_milli = 750 + 50 * (index % 3);
            let q_milli = asked_milli[(index % 4) as usize];
            let wrong_milli = (1_000 - probability_milli).pow(2) / 800;
            let is_wrong = ((index * 2_654_435_761) >> 8) % 1_000 < wrong_milli;
            let is_asked = ((index * 2_246_822_519) >> 8) % 1_000 < q_milli;
            let is_acted = probability_milli >= threshold_milli;
            let answer = if is_wrong {
                AskedAnswer::Wrong
            } else {
                AskedAnswer::Correct
            };
            Observation {
                question: QUESTION.to_string(),
                probability: probability_milli as f64 / 1_000.0,
                threshold: threshold_milli as f64 / 1_000.0,
                asked_with: q_milli as f64 / 1_000.0,
                is_asked,
                signal: if is_acted && is_wrong {
                    Signal::Wrong
                } else {
                    Signal::Unconfirmed
                },
                asked_answer: (!is_acted && is_asked).then_some(answer),
            }
        })
        .collect()
}

#[test]
fn recenter_below_min_results_does_nothing() {
    let mut state = state();
    let records = results(MIN_RECENTER_RESULTS - 1, 0.6, Signal::Unconfirmed);

    state.recenter(&records);

    assert_eq!(state.center, 0.8);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
#[test]
fn recenter_unasked_skipped_judgments_are_not_results() {
    let mut state = state();
    let mut records = results(MIN_RECENTER_RESULTS - 1, 0.95, Signal::Unconfirmed);
    records.extend((0..5_000).map(|_| observation_below(0.7, Signal::Missed)));

    state.recenter(&records);

    assert_eq!(state.center, 0.8);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
#[test]
fn recenter_picks_lowest_threshold_meeting_target() {
    let mut state = ThresholdState::new(QUESTION, 0.71, (0.5, 0.95));
    state.fast_offset = 0.03;
    // 0.9는 모두 맞고, 0.7은 절반이 틀린다.
    let mut records = results(1_500, 0.9, Signal::Unconfirmed);
    records.extend((0..1_500).map(|index| {
        let signal = if index % 2 == 0 {
            Signal::Wrong
        } else {
            Signal::Unconfirmed
        };
        Observation {
            threshold: 0.6,
            ..observation(0.7, signal)
        }
    }));

    state.recenter(&records);

    // 0.7 위 첫 격자점은 0.9만 포함한다.
    assert_eq!(state.center, 0.705);
    assert_eq!(state.fast_offset, 0.0);
}

#[test]
fn recenter_all_wrong_uses_upper_bound() {
    let mut state = state();
    let records = results(MIN_RECENTER_RESULTS, 0.9, Signal::Wrong);

    state.recenter(&records);

    // 하한·상한은 0.95이고, 한 번에 0.05만 움직인다.
    assert!((state.center - 0.85).abs() < 1e-12);
}

#[test]
fn recenter_acted_without_reaction_counts_as_not_wrong() {
    let mut state = state();
    let mut records = results(MIN_RECENTER_RESULTS, 0.92, Signal::Unconfirmed);
    records.extend(results(10, 0.92, Signal::Wrong));

    state.recenter(&records);

    assert_eq!(state.center, 0.75);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
#[test]
fn recenter_skipped_judgment_uses_only_asked_answer_with_inverse_q() {
    // 행동하지 않은 판단 1,000건이 모두 물은 답이고 q는 0.1이다.
    let skipped = |wrong_count: usize| {
        let mut records = results(2_000, 0.9, Signal::Unconfirmed);
        records.extend((0..1_000).map(|index| {
            let answer = if index < wrong_count {
                AskedAnswer::Wrong
            } else {
                AskedAnswer::Correct
            };
            skipped_asked(0.7, answer, 0.1)
        }));
        records
    };
    let mut few_wrong = ThresholdState::new(QUESTION, 0.6, (0.5, 0.95));
    let mut many_wrong = ThresholdState::new(QUESTION, 0.6, (0.5, 0.95));

    // 틀림 10건 x 10 = 100 / 3,000 = 3.3%, 20건 x 10 = 200 / 3,000 = 6.7%
    few_wrong.recenter(&skipped(10));
    many_wrong.recenter(&skipped(20));

    assert!((few_wrong.center - 0.55).abs() < 1e-12);
    assert!((many_wrong.center - 0.65).abs() < 1e-12);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 판단 기록 수
// basis: estimate
#[test]
fn recenter_skipped_judgment_with_missed_signal_is_not_used() {
    let mut state = state();
    let mut records = results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed);
    records.extend((0..1_000).map(|_| observation_below(0.6, Signal::Missed)));

    state.recenter(&records);

    // 0.6 판단은 결과가 없어 틀림 가중 0으로 분모에만 들어가므로 평균이 낮아져 한 번에 갈 수 있는 곳까지 내려간다.
    assert_eq!(state.center, 0.75);
}

#[test]
fn recenter_where_actions_stop_above_oracle_lands_near_oracle() {
    let mut state = state();

    state.recenter(&population());

    // 0.85 이상의 틀림 비율이 4.8%, 0.84 이상이 5.1%라 oracle은 0.85다.
    assert!((state.center - 0.85).abs() <= 0.02, "{}", state.center);
}

#[test]
fn recenter_matches_simulation_s1q_on_same_records() {
    let mut state = ThresholdState::new(QUESTION, 0.7, (0.5, 0.95));

    state.recenter(&simulated_records());

    assert_eq!(state.center, 0.695);
}

#[test]
fn recenter_matches_simulation_t1_on_same_records() {
    // S1q가 0.695를 고르고, 시작값 0.8에서 한 번에 0.05만 내려간다.
    let mut state = state();

    state.recenter(&simulated_records());

    assert_eq!(state.center, 0.75);
}

#[test]
fn recenter_at_min_results_acts_and_below_does_not() {
    let mut below = state();
    let mut at = state();

    below.recenter(&results(MIN_RECENTER_RESULTS - 1, 0.9, Signal::Unconfirmed));
    at.recenter(&results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed));

    assert_eq!(below.center, 0.8);
    assert_eq!(at.center, 0.75);
}

#[test]
fn recenter_moves_at_most_fast_range_per_call() {
    let mut state = state();
    let records = results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed);

    state.recenter(&records);
    assert_eq!(state.center, 0.75);
    state.recenter(&records);
    assert_eq!(state.center, 0.7);
    state.recenter(&results(MIN_RECENTER_RESULTS, 0.9, Signal::Wrong));
    assert_eq!(state.center, 0.75);
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn threshold_grid_default_bounds_spans_bounds_by_half_percent() {
    let grid = threshold_grid((0.5, 0.95));

    assert_eq!(grid.len(), 91);
    assert_eq!(grid.first(), Some(&0.5));
    assert_eq!(grid.last(), Some(&0.95));
    assert!(grid.contains(&0.805));
}

#[test]
fn recenter_keeps_center_within_irreversible_floor() {
    let mut state = ThresholdState::new(QUESTION, 0.85, (IRREVERSIBLE_FLOOR, 0.95));
    let records = results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed);

    state.recenter(&records);

    assert_eq!(state.center, IRREVERSIBLE_FLOOR);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 표본 수
// basis: estimate
#[test]
fn rollback_if_worse_restores_previous_on_many_wrongs() {
    let mut state = state();
    state.recenter(&results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed));
    assert_eq!(state.center, 0.75);
    let recent: Vec<Observation> = (0..20)
        .map(|_| Observation {
            threshold: 0.5,
            ..observation(0.65, Signal::Wrong)
        })
        .collect();

    let rolled_back = state.rollback_if_worse(&recent);

    assert!(rolled_back);
    assert_eq!(state.center, 0.8);
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 표본 수
// basis: estimate
#[test]
fn rollback_if_worse_keeps_value_when_fine() {
    let mut state = state();
    state.recenter(&results(MIN_RECENTER_RESULTS, 0.9, Signal::Unconfirmed));
    let recent: Vec<Observation> = (0..100)
        .map(|_| Observation {
            threshold: 0.5,
            ..observation(0.65, Signal::Unconfirmed)
        })
        .collect();

    let rolled_back = state.rollback_if_worse(&recent);

    assert!(!rolled_back);
    assert_eq!(state.center, 0.75);
}

#[test]
fn rollback_if_worse_without_previous_returns_false() {
    let mut state = state();

    assert!(!state.rollback_if_worse(&[observation(0.9, Signal::Wrong)]));
}

#[test]
fn ask_probability_higher_near_threshold_and_never_zero() {
    let near = ask_probability(0.81, 0.8, 0, 100);
    let far = ask_probability(0.01, 0.8, 0, 100);

    assert!(near > far);
    assert!(far > 0.0);
}

#[test]
fn ask_probability_at_rate_limit_uses_minimum() {
    let limited = ask_probability(0.8, 0.8, 5, 100);

    assert_eq!(limited, MIN_ASK);
}

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 판단 수
// basis: estimate
#[test]
fn ask_probability_overall_rate_stays_under_limit() {
    // 확률을 누적해 1을 넘을 때마다 묻는 결정적 표본 추출.
    let judgments = 10_000_u32;
    let mut asks = 0_u32;
    let mut accumulated = 0.0;

    for routed in 0..judgments {
        accumulated += ask_probability(0.8, 0.8, asks, routed);
        if accumulated >= 1.0 {
            accumulated -= 1.0;
            asks += 1;
        }
    }

    let rate = f64::from(asks) / f64::from(judgments);
    assert!(rate <= MAX_ASK_RATE + 1.0 / f64::from(judgments));
}

#[test]
fn gate_label_decides_the_label_use() {
    let inconsistent = {
        let mut label = label(&["yes", "yes"], Some("yes"));
        label.order_consistent = false;
        label
    };
    let human = {
        let mut label = label(&["no"], None);
        label.human = true;
        label.order_consistent = false;
        label
    };
    // (사례, 라벨, 예상 사용처)
    let cases = [
        (
            "unanimous with matching hindsight",
            label(&["yes", "yes"], Some("yes")),
            LabelUse::Eval,
        ),
        (
            "majority without hindsight",
            label(&["yes", "yes", "no"], None),
            LabelUse::Train,
        ),
        (
            "unanimous without hindsight",
            label(&["yes", "yes"], None),
            LabelUse::Train,
        ),
        ("order inconsistent", inconsistent, LabelUse::Drop),
        ("no majority", label(&["yes", "no"], None), LabelUse::Drop),
        ("no answers", label(&[], None), LabelUse::Drop),
        (
            "opposite hindsight",
            label(&["yes", "yes"], Some("no")),
            LabelUse::Drop,
        ),
        ("human answer", human, LabelUse::Eval),
    ];

    for (name, label, expected) in cases {
        assert_eq!(gate_label(&label), expected, "{name}");
    }
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
#[test]
fn should_promote_only_when_nothing_is_worse() {
    // (사례, 보고서, 승격 여부)
    let cases = [
        ("nothing worse", report(), true),
        (
            "low accuracy bound",
            EvalReport {
                accuracy_diff_ci: (-1.0, 3.0),
                ..report()
            },
            false,
        ),
        (
            "brier regression",
            EvalReport {
                brier: (0.13, 0.12),
                ..report()
            },
            false,
        ),
        (
            "ece regression",
            EvalReport {
                ece: (0.05, 0.04),
                ..report()
            },
            false,
        ),
        (
            "coverage regression",
            EvalReport {
                coverage: (0.89, 0.9),
                ..report()
            },
            false,
        ),
        (
            "order consistency regression",
            EvalReport {
                order_consistency: (0.95, 0.96),
                ..report()
            },
            false,
        ),
        (
            "nan",
            EvalReport {
                brier: (f64::NAN, 0.12),
                ..report()
            },
            false,
        ),
    ];

    for (name, report, expected) in cases {
        assert_eq!(should_promote(&report), expected, "{name}");
    }
}
