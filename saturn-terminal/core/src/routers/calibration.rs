//! 기준값 조정, 채점 라벨 게이트, 승격 게이트의 계산 규칙(실행은 engine `training`).
//! 설계: docs/design/router-training.md

pub const DEFAULT_TARGET_WRONG_RATE: f64 = 0.05;

/// 되돌릴 수 없는 행동 질문의 `bounds.0`으로 넘긴다.
pub const IRREVERSIBLE_FLOOR: f64 = 0.8;

/// 빠른 조정이 중심값에서 벗어날 수 있는 폭.
pub const FAST_RANGE: f64 = 0.05;

/// 느린 조정에 필요한 질문별 쓰인 결과 수(행동한 판단 + 물은 답)이고, 한 번의 이동을 `FAST_RANGE`로 제한하므로 일찍 적용한다.
pub const MIN_RECENTER_RESULTS: usize = 300;

pub const MAX_ASK_RATE: f64 = 0.05;

/// 단위는 pp이고, 신뢰구간 하한이 이 값보다 커야 한다.
pub const PROMOTION_ACCURACY_FLOOR_PP: f64 = -1.0;

/// 느린 조정이 중심값을 고르는 격자에서 1.0당 점 수이고, 간격은 0.005다.
const GRID_PER_UNIT: f64 = 200.0;

/// 신호 하나가 기준값을 옮기는 고정 폭이다.
const FAST_STEP: f64 = 0.002;

/// 묻는 빈도 상한에 닿았을 때도 쓰며, 1/q 가중의 최대값을 정한다(초안).
const MIN_ASK: f64 = 0.01;

/// 기준값 바로 위에서 묻는 확률(초안).
const MAX_ASK: f64 = 0.1;

/// 기준값에서 멀어질 때 묻는 확률이 줄어드는 거리 척도(초안).
const ASK_WIDTH: f64 = 0.1;

// 초안
const SPRT_ALPHA: f64 = 0.05;

// 초안
const SPRT_BETA: f64 = 0.2;

/// 나빠진 틀림 비율은 목표 × 이 값이다(초안).
const SPRT_WORSE_FACTOR: f64 = 2.0;

// 초안
const WEAK_LABEL_WEIGHT: f64 = 0.5;

/// 관찰 시간(다음 입력 3개 또는 10분)이 지나면 확정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// 행동 뒤 사용자가 뒤집거나 취소했다.
    Wrong,
    /// 행동하지 않았는데 사용자가 같은 행동을 직접 했다.
    Missed,
    /// 정답으로 보지 않는다.
    Unconfirmed,
}

/// 물은 판단에서 사용자가 한 답이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskedAnswer {
    /// router의 답이 맞았다고 답했다.
    Correct,
    /// router의 답이 틀렸다고 답했다.
    Wrong,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub question: String,
    pub probability: f64,
    /// 판단 당시 값.
    pub threshold: f64,
    /// 판단 당시 묻는 확률 q이고, 느린 조정이 1/q로 가중한다.
    pub asked_with: f64,
    /// 피드백 질문을 받은 판단이면 참이고, 빠른 조정이 이 신호만 1/q로 가중한다.
    pub is_asked: bool,
    pub signal: Signal,
    /// 물은 답이고, 행동하지 않은 판단은 이 값만 느린 조정의 결과로 쓴다.
    pub asked_answer: Option<AskedAnswer>,
}

impl Observation {
    /// 행동 신호는 항상 관찰되므로 1이고, 물은 답에서 온 신호만 `1/q`다.
    fn fast_weight(&self) -> f64 {
        if self.is_asked {
            1.0 / self.asked_with.max(MIN_ASK)
        } else {
            1.0
        }
    }
}

/// 빠른 조정 상태는 메모리에만 두어 engine을 다시 시작하면 중심값에서 다시 맞춘다.
#[derive(Debug, Clone)]
pub struct ThresholdState {
    pub question: String,
    /// 목표 틀림 비율을 지키는 가장 낮은 값.
    pub center: f64,
    /// `±FAST_RANGE` 안.
    pub fast_offset: f64,
    pub bounds: (f64, f64),
    pub target_wrong_rate: f64,
    /// `recenter` 직전의 (중심값, 빠른 조정 차이).
    previous: Option<(f64, f64)>,
}

impl ThresholdState {
    pub fn new(question: impl Into<String>, center: f64, bounds: (f64, f64)) -> Self {
        Self {
            question: question.into(),
            center,
            fast_offset: 0.0,
            bounds,
            target_wrong_rate: DEFAULT_TARGET_WRONG_RATE,
            previous: None,
        }
    }

    /// 틀림은 `(1 − α)`만큼 올리고 놓침은 `α`만큼 내려, 틀림 비율이 목표 α일 때 균형이 된다.
    pub fn observe(&mut self, observation: &Observation) {
        if observation.question != self.question {
            return;
        }
        let direction = match observation.signal {
            Signal::Wrong => 1.0 - self.target_wrong_rate,
            Signal::Missed => -self.target_wrong_rate,
            Signal::Unconfirmed => return,
        };
        self.fast_offset = (self.fast_offset + FAST_STEP * observation.fast_weight() * direction)
            .clamp(-FAST_RANGE, FAST_RANGE);
    }

    // cost: time O(n log n), heap O(n), stack O(1), alloc 1
    // vars: n = 판단 기록 수
    // basis: estimate
    /// `/train` 때 모든 판단 기록으로 부르며, 쓰인 결과가 `MIN_RECENTER_RESULTS`건 미만이면 하지 않고 중심값은 한 번에 이전 값 `±FAST_RANGE`까지만 움직인다.
    pub fn recenter(&mut self, records: &[Observation]) {
        let mut results = 0;
        let mut samples: Vec<(f64, f64)> = Vec::new();
        for observation in records
            .iter()
            .filter(|observation| observation.question == self.question)
        {
            let (wrong_weight, is_result) = risk_sample(observation);
            results += usize::from(is_result);
            samples.push((observation.probability, wrong_weight));
        }
        if results < MIN_RECENTER_RESULTS {
            return;
        }
        samples.sort_by(|left, right| right.0.total_cmp(&left.0));
        let new_center = lowest_safe_threshold(&samples, self.bounds, self.target_wrong_rate)
            .unwrap_or(self.bounds.1);
        let previous_center = self.center;
        self.previous = Some((previous_center, self.fast_offset));
        self.center = new_center
            .clamp(self.bounds.0, self.bounds.1)
            .clamp(previous_center - FAST_RANGE, previous_center + FAST_RANGE)
            .clamp(self.bounds.0, self.bounds.1);
        self.fast_offset = 0.0;
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 최근 판단 수
    // basis: estimate
    /// 행동한 판단으로 틀림 비율이 α인지 2α인지 가르는 SPRT를 해, 나빠졌으면 `recenter` 전 값으로 되돌리고 참을 돌려준다.
    pub fn rollback_if_worse(&mut self, recent: &[Observation]) -> bool {
        let Some((center, fast_offset)) = self.previous else {
            return false;
        };
        let base = self.target_wrong_rate.clamp(f64::EPSILON, 0.5);
        let worse = (base * SPRT_WORSE_FACTOR).min(1.0 - f64::EPSILON);
        let upper = ((1.0 - SPRT_BETA) / SPRT_ALPHA).ln();
        let lower = (SPRT_BETA / (1.0 - SPRT_ALPHA)).ln();
        let mut log_ratio = 0.0;
        let acted = recent.iter().filter(|observation| {
            observation.question == self.question
                && observation.probability >= observation.threshold
        });
        for observation in acted {
            log_ratio += if observation.signal == Signal::Wrong {
                (worse / base).ln()
            } else {
                ((1.0 - worse) / (1.0 - base)).ln()
            };
            if log_ratio <= lower {
                return false;
            }
            if log_ratio >= upper {
                self.center = center;
                self.fast_offset = fast_offset;
                self.previous = None;
                return true;
            }
        }
        false
    }

    pub fn current(&self) -> f64 {
        (self.center + self.fast_offset).clamp(self.bounds.0, self.bounds.1)
    }
}

/// 확실한 판단에서도 0이 아니며, 최근 물은 비율이 `MAX_ASK_RATE`에 닿으면 `MIN_ASK`다.
pub fn ask_probability(
    probability: f64,
    threshold: f64,
    recent_asks: u32,
    recent_judgments: u32,
) -> f64 {
    let allowed = MAX_ASK_RATE * (f64::from(recent_judgments) + 1.0);
    if f64::from(recent_asks) + 1.0 > allowed {
        return MIN_ASK;
    }
    let distance = (probability - threshold).abs();
    if distance.is_nan() {
        return MIN_ASK;
    }
    MIN_ASK + (MAX_ASK - MIN_ASK) * (-distance / ASK_WIDTH).exp()
}

/// 채점 모델마다 선택지 순서를 두 번 바꿔 풀고 같은 답만 채택한다.
#[derive(Debug, Clone)]
pub struct Label {
    pub question: String,
    pub grader_answers: Vec<String>,
    pub order_consistent: bool,
    /// 그때 알 수 있던 정보 기준의 사후 판정.
    pub hindsight: Option<String>,
    pub signal: Option<Signal>,
    /// AI 채점 치우침 보정에 쓴다.
    pub human: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelUse {
    Train,
    Eval,
    Drop,
}

// cost: time O(1), heap O(g), stack O(1), alloc 1
// vars: g = 격자 점 수
// basis: estimate
/// 하한과 0.005 간격의 점을 상한까지 오름차순으로 돌려준다.
fn threshold_grid(bounds: (f64, f64)) -> Vec<f64> {
    let first = (bounds.0 * GRID_PER_UNIT).floor() as u32 + 1;
    let last = (bounds.1 * GRID_PER_UNIT).floor() as u32;
    let mut grid = vec![bounds.0];
    grid.extend((first..=last).map(|step| f64::from(step) / GRID_PER_UNIT));
    grid
}

// cost: time O(g²), heap O(1), stack O(1)
// vars: g = 채점 모델 답 수
// basis: estimate
/// 사용자가 직접 답한 라벨은 순서 일관성과 사후 판정을 보지 않고 만장일치면 평가용이다.
pub fn gate_label(label: &Label) -> LabelUse {
    let Some(consensus) = majority(&label.grader_answers) else {
        return LabelUse::Drop;
    };
    let is_unanimous = label
        .grader_answers
        .iter()
        .all(|answer| answer == consensus);
    if label.human {
        return if is_unanimous {
            LabelUse::Eval
        } else {
            LabelUse::Drop
        };
    }
    if !label.order_consistent {
        return LabelUse::Drop;
    }
    match label.hindsight.as_deref() {
        Some(hindsight) if hindsight != consensus => LabelUse::Drop,
        Some(_) if is_unanimous => LabelUse::Eval,
        _ => LabelUse::Train,
    }
}

/// 뒤집기와 취소(틀림 신호)가 붙은 라벨은 약한 라벨이다.
pub fn label_weight(label: &Label) -> f64 {
    if label.signal == Some(Signal::Wrong) {
        WEAK_LABEL_WEIGHT
    } else {
        1.0
    }
}

/// 쌍은 모두 (새 모델, 현재 모델) 순서다.
#[derive(Debug, Clone)]
pub struct EvalReport {
    /// 정확도 차이(새 − 현재)의 95% 신뢰구간, pp 단위.
    pub accuracy_diff_ci: (f64, f64),
    pub brier: (f64, f64),
    pub ece: (f64, f64),
    pub coverage: (f64, f64),
    pub order_consistency: (f64, f64),
}

/// Brier와 ECE는 낮을수록, 처리 비율과 순서 일관성은 높을수록 좋고, 같은 값은 나빠지지 않은 것으로 본다.
pub fn should_promote(report: &EvalReport) -> bool {
    let (new_brier, current_brier) = report.brier;
    let (new_ece, current_ece) = report.ece;
    let (new_coverage, current_coverage) = report.coverage;
    let (new_order, current_order) = report.order_consistency;
    report.accuracy_diff_ci.0 > PROMOTION_ACCURACY_FLOOR_PP
        && new_brier <= current_brier
        && new_ece <= current_ece
        && new_coverage >= current_coverage
        && new_order >= current_order
}

/// 판단 하나의 (틀림 가중, 쓰인 결과 여부)이다. 행동한 판단은 틀림 신호를 가중 1로, 행동하지 않은 판단은 물은 답의 틀림만 `1/q`로 쓴다.
fn risk_sample(observation: &Observation) -> (f64, bool) {
    if observation.probability >= observation.threshold {
        let wrong = observation.signal == Signal::Wrong;
        return (f64::from(u8::from(wrong)), true);
    }
    match observation.asked_answer {
        Some(AskedAnswer::Wrong) => (1.0 / observation.asked_with.max(MIN_ASK), true),
        Some(AskedAnswer::Correct) => (0.0, true),
        None => (0.0, false),
    }
}

// cost: time O(n + g), heap O(g), stack O(1), alloc 1
// vars: n = 표본 수, g = 격자 점 수
// basis: estimate
/// `samples`는 (확률, 틀림 가중)이고 확률 내림차순이어야 한다. 격자의 각 값에서 확률이 그 값 이상인 판단 전체의 틀림 가중 평균이 목표 이하인 가장 낮은 값을 돌려준다.
fn lowest_safe_threshold(samples: &[(f64, f64)], bounds: (f64, f64), target: f64) -> Option<f64> {
    let mut wrong = 0.0;
    let mut count = 0_u32;
    let mut covered = 0;
    let mut lowest = None;
    for threshold in threshold_grid(bounds).into_iter().rev() {
        while let Some((probability, weight)) = samples.get(covered) {
            if *probability < threshold {
                break;
            }
            wrong += weight;
            count += 1;
            covered += 1;
        }
        if count > 0 && wrong / f64::from(count) <= target {
            lowest = Some(threshold);
        }
    }
    lowest
}

// cost: time O(g²), heap O(1), stack O(1)
// vars: g = 답 수
// basis: estimate
fn majority(answers: &[String]) -> Option<&str> {
    answers
        .iter()
        .find(|answer| 2 * answers.iter().filter(|other| other == answer).count() > answers.len())
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
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
    fn gate_label_unanimous_with_matching_hindsight_is_eval() {
        assert_eq!(
            gate_label(&label(&["yes", "yes"], Some("yes"))),
            LabelUse::Eval
        );
    }

    #[test]
    fn gate_label_majority_without_hindsight_is_train() {
        assert_eq!(
            gate_label(&label(&["yes", "yes", "no"], None)),
            LabelUse::Train
        );
        assert_eq!(gate_label(&label(&["yes", "yes"], None)), LabelUse::Train);
    }

    #[test]
    fn gate_label_order_inconsistent_is_dropped() {
        let mut inconsistent = label(&["yes", "yes"], Some("yes"));
        inconsistent.order_consistent = false;

        assert_eq!(gate_label(&inconsistent), LabelUse::Drop);
    }

    #[test]
    fn gate_label_no_majority_or_opposite_hindsight_is_dropped() {
        assert_eq!(gate_label(&label(&["yes", "no"], None)), LabelUse::Drop);
        assert_eq!(gate_label(&label(&[], None)), LabelUse::Drop);
        assert_eq!(
            gate_label(&label(&["yes", "yes"], Some("no"))),
            LabelUse::Drop
        );
    }

    #[test]
    fn gate_label_human_answer_is_eval() {
        let mut human = label(&["no"], None);
        human.human = true;
        human.order_consistent = false;

        assert_eq!(gate_label(&human), LabelUse::Eval);
    }

    #[test]
    fn label_weight_wrong_signal_is_weak() {
        let mut flipped = label(&["yes"], None);
        flipped.signal = Some(Signal::Wrong);

        assert!(label_weight(&flipped) < label_weight(&label(&["yes"], None)));
    }

    #[test]
    fn should_promote_passes_when_nothing_worse() {
        assert!(should_promote(&report()));
    }

    #[test]
    fn should_promote_rejects_low_accuracy_bound() {
        let worse = EvalReport {
            accuracy_diff_ci: (-1.0, 3.0),
            ..report()
        };

        assert!(!should_promote(&worse));
    }

    // cost: time O(1), heap O(1), stack O(1)
    // basis: estimate
    #[test]
    fn should_promote_rejects_any_regression() {
        let brier = EvalReport {
            brier: (0.13, 0.12),
            ..report()
        };
        let ece = EvalReport {
            ece: (0.05, 0.04),
            ..report()
        };
        let coverage = EvalReport {
            coverage: (0.89, 0.9),
            ..report()
        };
        let order = EvalReport {
            order_consistency: (0.95, 0.96),
            ..report()
        };

        for regressed in [brier, ece, coverage, order] {
            assert!(!should_promote(&regressed));
        }
    }

    #[test]
    fn should_promote_nan_is_rejected() {
        let nan = EvalReport {
            brier: (f64::NAN, 0.12),
            ..report()
        };

        assert!(!should_promote(&nan));
    }
}
