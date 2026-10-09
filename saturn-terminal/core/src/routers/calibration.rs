//! 기준값 조정, 채점 라벨 게이트, 승격 게이트의 계산 규칙.
//! 설계: docs/design/router-training.md

pub const DEFAULT_TARGET_WRONG_RATE: f64 = 0.05;

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
    /// 모든 판단 기록으로 부르며, 쓰인 결과가 `MIN_RECENTER_RESULTS`건 미만이면 하지 않고 중심값은 한 번에 이전 값 `±FAST_RANGE`까지만 움직인다.
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
mod tests;
