//! 기준값 조정, 채점 라벨 게이트, 승격 게이트의 계산 규칙(실행은 engine `training`).
//! 설계: docs/design/judge-training.md

use std::collections::VecDeque;

pub const DEFAULT_TARGET_WRONG_RATE: f64 = 0.05;

/// 되돌릴 수 없는 행동 질문의 `bounds.0`으로 넘긴다.
pub const IRREVERSIBLE_FLOOR: f64 = 0.8;

/// 빠른 조정이 중심값에서 벗어날 수 있는 폭.
pub const FAST_RANGE: f64 = 0.05;

/// 느린 조정에 필요한 질문별 채점된 판단 수.
pub const MIN_LABELED: usize = 200;

pub const MAX_ASK_RATE: f64 = 0.05;

/// 단위는 pp이고, 신뢰구간 하한이 이 값보다 커야 한다.
pub const PROMOTION_ACCURACY_FLOOR_PP: f64 = -1.0;

/// n번째 판단의 폭은 `FAST_STEP / sqrt(n + 1)`이다(초안).
const FAST_STEP: f64 = 0.01;

/// 이 수의 최근 기준값이 `STABLE_BAND` 안이면 빠른 조정을 멈춘다.
const STABLE_WINDOW: usize = 100;

/// 한쪽 폭(±).
const STABLE_BAND: f64 = 0.01;

/// 빠른·느린 지수 평균 차이가 이 값을 넘으면 폭을 다시 키운다(초안).
const RATE_SHIFT: f64 = 0.25;

// 초안
const FAST_RATE_WEIGHT: f64 = 0.2;

// 초안
const SLOW_RATE_WEIGHT: f64 = 0.02;

// 초안
const RATE_SHIFT_MIN_SIGNALS: u32 = 20;

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

#[derive(Debug, Clone)]
pub struct Observation {
    pub question: String,
    pub probability: f64,
    /// 판단 당시 값.
    pub threshold: f64,
    /// 계산 때 1/q로 가중한다.
    pub asked_with: f64,
    pub signal: Signal,
}

/// 빠른 조정 상태는 메모리에만 두어 engine을 다시 시작하면 처음 폭으로 다시 맞춘다.
#[derive(Debug, Clone)]
pub struct ThresholdState {
    pub question: String,
    /// 목표 틀림 비율을 지키는 가장 낮은 값.
    pub center: f64,
    /// `±FAST_RANGE` 안.
    pub fast_offset: f64,
    pub bounds: (f64, f64),
    pub target_wrong_rate: f64,
    adaptation: Adaptation,
    /// `recenter` 직전의 (중심값, 빠른 조정 차이).
    previous: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
struct Adaptation {
    /// 마지막 재시작 뒤 반영한 신호 수.
    signals: u32,
    /// 최대 `STABLE_WINDOW`개.
    recent: VecDeque<f64>,
    is_frozen: bool,
    /// (빠른, 느린)이고 첫 신호 전에는 `None`.
    wrong_rate: Option<(f64, f64)>,
}

impl ThresholdState {
    pub fn new(question: impl Into<String>, center: f64, bounds: (f64, f64)) -> Self {
        Self {
            question: question.into(),
            center,
            fast_offset: 0.0,
            bounds,
            target_wrong_rate: DEFAULT_TARGET_WRONG_RATE,
            adaptation: Adaptation::default(),
            previous: None,
        }
    }

    // cost: time O(w), heap O(1) amortized, stack O(1)
    // vars: w = STABLE_WINDOW
    // basis: estimate
    /// 틀림은 `(1 − α)`만큼 올리고 놓침은 `α`만큼 내려, 틀림 비율이 목표 α일 때 균형이 된다.
    pub fn observe(&mut self, observation: &Observation) {
        if observation.question != self.question {
            return;
        }
        let is_wrong = match observation.signal {
            Signal::Wrong => true,
            Signal::Missed => false,
            Signal::Unconfirmed => return,
        };
        if self.has_rate_shifted(is_wrong) {
            self.adaptation = Adaptation {
                wrong_rate: self.adaptation.wrong_rate,
                ..Adaptation::default()
            };
        }
        if self.adaptation.is_frozen {
            return;
        }
        let step = FAST_STEP / f64::from(self.adaptation.signals + 1).sqrt();
        let weight = 1.0 / observation.asked_with.max(MIN_ASK);
        let direction = if is_wrong {
            1.0 - self.target_wrong_rate
        } else {
            -self.target_wrong_rate
        };
        self.fast_offset =
            (self.fast_offset + step * weight * direction).clamp(-FAST_RANGE, FAST_RANGE);
        self.adaptation.signals += 1;
        self.remember_current();
    }

    // cost: time O(n log n), heap O(n), stack O(1), alloc 1
    // vars: n = 채점된 판단 수
    // basis: estimate
    /// `/train` 때 부르며, 채점된 판단이 `MIN_LABELED`건 미만이면 하지 않는다.
    pub fn recenter(&mut self, labeled: &[Observation]) {
        let mut samples: Vec<(f64, f64, bool)> = labeled
            .iter()
            .filter(|observation| observation.question == self.question)
            .filter_map(|observation| {
                let weight = 1.0 / observation.asked_with.max(MIN_ASK);
                match observation.signal {
                    Signal::Wrong => Some((observation.probability, weight, true)),
                    Signal::Missed => Some((observation.probability, weight, false)),
                    Signal::Unconfirmed => None,
                }
            })
            .collect();
        if samples.len() < MIN_LABELED {
            return;
        }
        samples.sort_by(|left, right| right.0.total_cmp(&left.0));
        let new_center =
            lowest_safe_threshold(&samples, self.target_wrong_rate).unwrap_or(self.bounds.1);
        self.previous = Some((self.center, self.fast_offset));
        self.center = new_center.clamp(self.bounds.0, self.bounds.1);
        self.fast_offset = 0.0;
        self.adaptation = Adaptation::default();
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
                self.adaptation = Adaptation::default();
                return true;
            }
        }
        false
    }

    pub fn current(&self) -> f64 {
        (self.center + self.fast_offset).clamp(self.bounds.0, self.bounds.1)
    }

    /// 판정하면서 틀림 비율 지수 평균도 갱신한다.
    fn has_rate_shifted(&mut self, is_wrong: bool) -> bool {
        let value = if is_wrong { 1.0 } else { 0.0 };
        let (fast, slow) = self.adaptation.wrong_rate.unwrap_or((value, value));
        let fast = fast + FAST_RATE_WEIGHT * (value - fast);
        let slow = slow + SLOW_RATE_WEIGHT * (value - slow);
        self.adaptation.wrong_rate = Some((fast, slow));
        self.adaptation.signals >= RATE_SHIFT_MIN_SIGNALS && (fast - slow).abs() > RATE_SHIFT
    }

    // cost: time O(w), heap O(1) amortized, stack O(1)
    // vars: w = STABLE_WINDOW
    // basis: estimate
    /// 최근 `STABLE_WINDOW`개가 ±`STABLE_BAND` 안이면 멈춘다.
    fn remember_current(&mut self) {
        let current = self.current();
        let recent = &mut self.adaptation.recent;
        recent.push_back(current);
        if recent.len() > STABLE_WINDOW {
            recent.pop_front();
        }
        if recent.len() < STABLE_WINDOW {
            return;
        }
        let min = recent.iter().copied().fold(f64::INFINITY, f64::min);
        let max = recent.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        self.adaptation.is_frozen = max - min <= 2.0 * STABLE_BAND;
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

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 표본 수
// basis: estimate
/// `samples`는 확률 내림차순이어야 한다.
fn lowest_safe_threshold(samples: &[(f64, f64, bool)], target: f64) -> Option<f64> {
    let mut wrong = 0.0;
    let mut total = 0.0;
    let mut lowest = None;
    for (index, (probability, weight, is_wrong)) in samples.iter().enumerate() {
        total += weight;
        if *is_wrong {
            wrong += weight;
        }
        let is_last_of_value = samples
            .get(index + 1)
            .is_none_or(|next| next.0 < *probability);
        if is_last_of_value && wrong / total <= target {
            lowest = Some(*probability);
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
            signal,
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
        let mut wrong = observation(0.85, Signal::Wrong);
        wrong.asked_with = 0.0001;

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
    fn observe_step_shrinks_with_signals() {
        let mut state = state();

        state.observe(&observation(0.85, Signal::Wrong));
        let first = state.fast_offset;
        state.observe(&observation(0.85, Signal::Wrong));
        let second = state.fast_offset - first;

        assert!(second < first);
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 넣는 신호 수
    // basis: estimate
    #[test]
    fn observe_stops_after_stable_window() {
        let mut state = state();
        for _ in 0..STABLE_WINDOW {
            state.observe(&observation(0.7, Signal::Missed));
        }
        let frozen_at = state.fast_offset;

        state.observe(&observation(0.7, Signal::Missed));

        assert!(state.adaptation.is_frozen);
        assert_eq!(state.fast_offset, frozen_at);
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 넣는 신호 수
    // basis: estimate
    #[test]
    fn observe_rate_shift_restarts_adaptation() {
        let mut state = state();
        for _ in 0..STABLE_WINDOW {
            state.observe(&observation(0.7, Signal::Missed));
        }
        assert!(state.adaptation.is_frozen);

        for _ in 0..3 {
            state.observe(&observation(0.85, Signal::Wrong));
        }

        assert!(!state.adaptation.is_frozen);
        assert!(state.adaptation.signals < 10);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 표본 수
    // basis: estimate
    #[test]
    fn recenter_below_min_labeled_does_nothing() {
        let mut state = state();
        let labeled: Vec<Observation> = (0..MIN_LABELED - 1)
            .map(|_| observation(0.6, Signal::Missed))
            .collect();

        state.recenter(&labeled);

        assert_eq!(state.center, 0.8);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 표본 수
    // basis: estimate
    #[test]
    fn recenter_picks_lowest_threshold_meeting_target() {
        let mut state = state();
        state.fast_offset = 0.03;
        // 0.9 이상은 모두 맞고, 0.7은 절반이 틀린다.
        let mut labeled: Vec<Observation> =
            (0..150).map(|_| observation(0.9, Signal::Missed)).collect();
        labeled.extend((0..50).map(|n| {
            let signal = if n % 2 == 0 {
                Signal::Wrong
            } else {
                Signal::Missed
            };
            observation(0.7, signal)
        }));

        state.recenter(&labeled);

        assert_eq!(state.center, 0.9);
        assert_eq!(state.fast_offset, 0.0);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 표본 수
    // basis: estimate
    #[test]
    fn recenter_all_wrong_uses_upper_bound() {
        let mut state = state();
        let labeled: Vec<Observation> = (0..MIN_LABELED)
            .map(|_| observation(0.9, Signal::Wrong))
            .collect();

        state.recenter(&labeled);

        assert_eq!(state.center, 0.95);
    }

    // cost: time O(n), heap O(n), stack O(1)
    // vars: n = 표본 수
    // basis: estimate
    #[test]
    fn rollback_if_worse_restores_previous_on_many_wrongs() {
        let mut state = state();
        let labeled: Vec<Observation> = (0..MIN_LABELED)
            .map(|_| observation(0.6, Signal::Missed))
            .collect();
        state.recenter(&labeled);
        assert_eq!(state.center, 0.6);
        let recent: Vec<Observation> = (0..20)
            .map(|_| Observation {
                threshold: 0.6,
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
        let labeled: Vec<Observation> = (0..MIN_LABELED)
            .map(|_| observation(0.6, Signal::Missed))
            .collect();
        state.recenter(&labeled);
        let recent: Vec<Observation> = (0..100)
            .map(|_| Observation {
                threshold: 0.6,
                ..observation(0.65, Signal::Unconfirmed)
            })
            .collect();

        let rolled_back = state.rollback_if_worse(&recent);

        assert!(!rolled_back);
        assert_eq!(state.center, 0.6);
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

        for judged in 0..judgments {
            accumulated += ask_probability(0.8, 0.8, asks, judged);
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
