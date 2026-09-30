//! 기준값 조정, 채점 라벨 게이트, 승격 게이트. 계산 규칙만 두고 실행(`/train`)은 engine `training`이 맡는다.
//!
//! 설계: docs/design/judge-training.md.

/// 판단 뒤 관찰한 결과 신호. 관찰 시간(다음 입력 3개 또는 10분)이 지나면 확정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// 행동 뒤 사용자가 뒤집거나 취소했다(틀림).
    Wrong,
    /// 행동하지 않았는데 사용자가 같은 행동을 직접 했다(놓침).
    Missed,
    /// 반응이 없다. 정답으로 보지 않는다.
    Unconfirmed,
}

/// 판단 하나의 조정 입력. 기록마다 judge 버전, 그때 기준값, 물은 확률 q를 남긴다.
#[derive(Debug, Clone)]
pub struct Observation {
    /// 질문 id.
    pub question: String,
    /// 판단 확률.
    pub probability: f64,
    /// 판단 때 기준값.
    pub threshold: f64,
    /// 맞았는지 물은 확률. 계산 때 1/q로 가중한다.
    pub asked_with: f64,
    /// 관찰 신호.
    pub signal: Signal,
}

/// 질문 하나의 기준값 상태. 느린 조정이 중심값, 빠른 조정이 ±0.05 안의 차이.
#[derive(Debug, Clone)]
pub struct ThresholdState {
    /// 질문 id.
    pub question: String,
    /// 느린 조정 중심값. 목표 틀림 비율(기본 5%)을 지키는 가장 낮은 값.
    pub center: f64,
    /// 빠른 조정 차이. 중심값 ±0.05 안.
    pub fast_offset: f64,
    /// 질문별 최저값과 최고값. 되돌릴 수 없는 행동은 최저 0.8.
    pub bounds: (f64, f64),
    /// 목표 틀림 비율.
    pub target_wrong_rate: f64,
}

impl ThresholdState {
    /// 판단 하나로 빠른 조정. 폭은 판단이 쌓일수록 줄고 최근 100번이 ±0.01 안이면 멈춘다. 틀림 비율이 급변하면 폭을 다시 키운다.
    pub fn observe(&mut self, observation: &Observation) {
        todo!("#81")
    }

    /// 느린 조정. `/train` 때 채점된 판단 전부로 중심값을 다시 계산하고 빠른 조정 차이를 0으로 되돌린다.
    /// 채점된 판단이 200건 미만이면 하지 않는다.
    pub fn recenter(&mut self, labeled: &[Observation]) {
        todo!("#81")
    }

    /// 순차 검정으로 바꾼 기준값이 확실히 나빠졌으면 이전 값으로 되돌린다.
    pub fn rollback_if_worse(&mut self, recent: &[Observation]) -> bool {
        todo!("#81")
    }

    /// 지금 적용할 기준값(중심값 + 빠른 조정 차이, 최저·최고로 자른 값).
    pub fn current(&self) -> f64 {
        todo!("#81")
    }
}

/// 판단 하나에 피드백 질문을 할 확률 q. 기준값 근처에서 높고 확실한 판단에서 낮되 0이 아니다.
/// 전체 빈도는 판단 20번에 1번을 넘지 않는다.
pub fn ask_probability(
    probability: f64,
    threshold: f64,
    recent_asks: u32,
    recent_judgments: u32,
) -> f64 {
    todo!("#81")
}

/// 채점 라벨 하나. 채점 모델마다 선택지 순서를 두 번 바꿔 풀고 같은 답만 채택한다.
#[derive(Debug, Clone)]
pub struct Label {
    /// 질문 id.
    pub question: String,
    /// 채점 모델들의 답.
    pub grader_answers: Vec<String>,
    /// 순서를 바꿔도 같은 답이었는지.
    pub order_consistent: bool,
    /// 사후 판정(그때 알 수 있던 정보 기준).
    pub hindsight: Option<String>,
    /// 결과 신호.
    pub signal: Option<Signal>,
    /// 사용자가 직접 답한 라벨이면 참. AI 채점 치우침 보정에 쓴다.
    pub human: bool,
}

/// 라벨 용도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelUse {
    /// 학습용: 합의, 순서 일관, 반대 결과 없음 통과.
    Train,
    /// 평가용: 만장일치와 결과 일치만.
    Eval,
    /// 버린다.
    Drop,
}

/// label model로 채점 모델, 사후 판정, 결과 신호를 결합해 라벨 용도를 정한다. 뒤집기와 취소는 약한 라벨로 가중한다.
pub fn gate_label(label: &Label) -> LabelUse {
    todo!("#81")
}

/// 새 judge 모델과 현재 모델의 평가 결과.
#[derive(Debug, Clone)]
pub struct EvalReport {
    /// 정확도 차이(새 − 현재)의 95% 신뢰구간(하한, 상한), pp 단위.
    pub accuracy_diff_ci: (f64, f64),
    /// Brier 점수(새, 현재).
    pub brier: (f64, f64),
    /// ECE(새, 현재).
    pub ece: (f64, f64),
    /// 처리 비율(새, 현재).
    pub coverage: (f64, f64),
    /// 선택지 순서 일관성(새, 현재).
    pub order_consistency: (f64, f64),
}

/// 승격 게이트. 정확도 차이 하한이 −1pp보다 크고 Brier, ECE, 처리 비율, 순서 일관성이 나빠지지 않아야 한다.
pub fn should_promote(report: &EvalReport) -> bool {
    todo!("#81")
}
