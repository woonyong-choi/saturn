//! router 요청을 만들고 답을 행동으로 바꾸는 순수 규칙(구현은 `saturn-engine`의 `routers`).
//! 설계: docs/design/router.md

pub mod calibration;
pub mod failure;
pub mod split;

use std::future::Future;

use saturn_protocol::ids::{ChatRevision, LedgerSeq, SettingsRevision};
use saturn_protocol::state::Disposition;

use self::split::{SplitError, split_request};

/// 이 값 미만이면 새 작업, 이 값부터 유지 기준 미만까지는 현재 에이전트 유지.
pub const KEEP_CURRENT_FLOOR: f64 = 0.3;

/// 확률 합이 1에서 벗어나도 되는 폭(초안).
pub const PROBABILITY_TOLERANCE: f64 = 0.01;

pub const SET_ROUTE: &str = "route";
pub const SET_RELATION: &str = "relation";
pub const SET_SEND_OPT: &str = "send-opt";
pub const SET_COMPACT: &str = "compact";

/// 판단 기록과 대체 규칙 기록에 그대로 남는다.
pub mod question_ids {
    pub const KEEP_CURRENT: &str = "keep_current";
    pub const IS_ACTIONABLE: &str = "is_actionable";
    pub const TARGET_MODEL: &str = "target_model";
    pub const RESUME_HELD: &str = "resume_held";
    pub const RELATION_TO_RUNNING: &str = "relation_to_running";
    pub const STEER_OR_SPAWN: &str = "steer_or_spawn";
}

/// 순서가 답의 확률 순서다.
pub const RELATION_OPTIONS: [&str; 5] =
    ["refines", "continues", "independent", "conflicts", "other"];

/// 순서가 답의 확률 순서다.
pub const SEND_OPTIONS: [&str; 4] = ["steer", "queue", "spawn", "other"];

/// 답이 하나인 `choice` 질문에 넣는다.
const OTHER: &str = "other";

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    /// 재시도가 끝나도 응답이 없으면 현재 에이전트와 현재 모델로 진행한다.
    #[error("router did not respond")]
    NoResponse,
    /// `cost-unknown`으로 기록하고 다시 보낸다.
    #[error("router timed out after send")]
    TimedOutAfterSend,
    /// 연결 실패와 구분해 키 입력 창을 띄운다.
    #[error("router rejected the key")]
    Unauthorized,
    /// 기다렸다가 다시 보낸다.
    #[error("router rate limited")]
    RateLimited,
    /// 후보 밖 선택, NaN, 확률 누락이면 `invalid`로 기록하고 대체 규칙을 적용한다.
    #[error("router answer is invalid: {reason}")]
    Invalid { reason: String },
    /// `superseded`로 기록한다.
    #[error("chat revision changed during judgment")]
    Superseded,
}

/// 기록에 `route@3.1`처럼 남는다.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct QuestionSetId {
    pub name: String,
    /// 뜻이나 선택지가 바뀌면 올리고 옛 선택지 대응표를 함께 둔다.
    pub major: u16,
    /// 뜻이 같은 작은 변경이라 옛 답과 라벨을 그대로 쓴다.
    pub minor: u16,
}

/// 문장과 선택지는 영어로 쓰고 사용자 원문은 `state`에 그대로 넣는다.
#[derive(Debug, Clone)]
pub struct Question {
    pub id: String,
    pub text: String,
    pub kind: AnswerKind,
}

#[derive(Debug, Clone)]
pub enum AnswerKind {
    /// 255개 이하이고, 답이 하나인 질문에는 `other`를 넣는다.
    Choice { options: Vec<String> },
    /// 여러 개가 맞을 수 있는 속성은 `noul`로 하나씩 묻는다.
    Noul,
    /// 2~10단계.
    Score { levels: u8 },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Answer {
    /// 순서는 `options`와 같다.
    Choice(Vec<f64>),
    /// P(yes).
    Noul(f64),
    Score(Vec<f64>),
}

impl Answer {
    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 선택지 수
    // basis: estimate
    /// `(N·pmax − 1)/(N − 1)`을 0~1로 자르고, `noul`은 두 선택지로, NaN이나 빈 답은 0으로 본다.
    pub fn confidence(&self) -> f64 {
        let (count, max) = match self {
            Self::Noul(yes) => (2, yes.max(1.0 - yes)),
            Self::Choice(probabilities) | Self::Score(probabilities) => (
                probabilities.len(),
                probabilities
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max),
            ),
        };
        let has_nan = match self {
            Self::Noul(yes) => yes.is_nan(),
            Self::Choice(probabilities) | Self::Score(probabilities) => {
                probabilities.iter().any(|probability| probability.is_nan())
            }
        };
        if count == 0 || has_nan {
            return 0.0;
        }
        if count == 1 {
            return 1.0;
        }
        let count = count as f64;
        ((count * max - 1.0) / (count - 1.0)).clamp(0.0, 1.0)
    }

    // cost: time O(n), heap O(1), stack O(1)
    // vars: n = 선택지 수
    // basis: estimate
    fn top_index(&self) -> Option<usize> {
        let (Self::Choice(probabilities) | Self::Score(probabilities)) = self else {
            return None;
        };
        probabilities
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
    }
}

/// 판단 기록에 남는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JudgmentOutcome {
    Ok,
    Invalid,
    Superseded,
    CostUnknown,
    NoResponse,
}

/// 입력당 한 번 부르고 필요한 질문을 모두 묶는다.
#[derive(Debug, Clone)]
pub struct RouterRequest {
    /// 고정 버전 이름을 쓰고, 별칭이면 응답의 `model`을 기록한다.
    pub model: String,
    /// 비밀값, 절대 경로, 다른 대화 원문은 넣지 않는다.
    pub state: String,
    pub sets: Vec<(QuestionSetId, Vec<Question>)>,
}

#[derive(Debug, Clone)]
pub struct RouterResponse {
    pub model: String,
    pub answers: Vec<(String, Answer)>,
    /// (입력, 출력) 토큰.
    pub tokens: (u64, u64),
}

/// 모든 router는 이 trait의 구현으로만 붙는다.
pub trait RouterClient: Send + Sync {
    /// # Errors
    /// 실패하면 호출자는 키를 다시 받거나 Saturn을 실행하지 않는다.
    fn check(&self) -> impl Future<Output = Result<(), RouterError>> + Send;

    /// 64K를 넘거나 `state`와 가장 긴 질문 합이 32K를 넘으면 구현이 `split::split_request`로 나눠 보낸다.
    ///
    /// # Errors
    /// 호출자는 `RouterError` 종류별로 대기, 대체 규칙, 재전송을 고른다.
    fn router(
        &self,
        request: RouterRequest,
    ) -> impl Future<Output = Result<RouterResponse, RouterError>> + Send;
}

/// 판단 기록은 방식과 관계없이 전부 남긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// 외부 API 기준 router.
    Jev,
    /// 확신도가 기준보다 낮으면 행동하지 않고 대체 규칙으로 간다.
    Saturn,
    /// TODO(#40): `collect`의 뜻 미정
    Collect,
}

/// 기본값은 docs/design/router.md 표를 따른다.
#[derive(Debug, Clone)]
pub struct Thresholds {
    pub keep_current: f64,
    pub is_actionable: f64,
    /// `choice`와 `score` 확신도 기준.
    pub min_confidence: f64,
    /// 미만이면 무시 횟수가 1 오른다.
    pub resume_held: f64,
    /// (존재 기준, 없음 기준).
    pub file_relevant: (f64, f64),
    /// `context-select` 게이트 평균 없음 기준.
    pub context_gate: f64,
    /// `doc-filter`의 `injection` 제외 기준.
    pub injection: f64,
    /// `loop`의 `is_progressing`이 이 값 미만이면 루프.
    pub progressing: f64,
    pub feedback_cause: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            keep_current: 0.8,
            is_actionable: 0.7,
            min_confidence: 0.6,
            resume_held: 0.85,
            file_relevant: (0.7, 0.35),
            context_gate: 0.3,
            injection: 0.7,
            progressing: 0.2,
            feedback_cause: 0.7,
        }
    }
}

/// `queue`가 적용 직전에 `revision`을 비교한다.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// 판단 시점 값.
    pub revision: ChatRevision,
    pub settings: SettingsRevision,
    pub disposition: Disposition,
    /// 거짓이면 새 작업이나 보조 에이전트.
    pub keep_current: bool,
    /// `None`이면 사용자 고정 모델이나 현재 모델.
    pub model: Option<String>,
    pub resume_held: bool,
    /// 기록에 사유와 함께 남긴다.
    pub fallbacks: Vec<String>,
}

// cost: time O(m), heap O(m), stack O(1)
// vars: m = 허용 모델 후보 수
// basis: estimate
/// 제어 명령이 아닌 입력에만 부르고, `models`가 비었거나 모델이 고정됐으면 `target_model`을 묻지 않는다.
pub fn questions_for_input(
    running: bool,
    model_pinned: bool,
    has_held: bool,
    models: &[String],
) -> Vec<(QuestionSetId, Vec<Question>)> {
    let mut route = vec![
        noul(
            question_ids::KEEP_CURRENT,
            "Does this input continue the work of the agent that handled the previous input?",
        ),
        noul(
            question_ids::IS_ACTIONABLE,
            "Is this input clear enough to act on without exploring the codebase first?",
        ),
    ];
    if !model_pinned && !models.is_empty() {
        let options = models.iter().cloned().chain([OTHER.to_string()]).collect();
        route.push(choice(
            question_ids::TARGET_MODEL,
            "Which model should handle this input?",
            options,
        ));
    }
    if has_held {
        route.push(noul(
            question_ids::RESUME_HELD,
            "Does the user want to resume the work that was stopped and held?",
        ));
    }
    let mut sets = vec![(set_id(SET_ROUTE), route)];
    if running {
        sets.push((
            set_id(SET_RELATION),
            vec![choice(
                question_ids::RELATION_TO_RUNNING,
                "How does this input relate to the work that is running now?",
                RELATION_OPTIONS.map(String::from).to_vec(),
            )],
        ));
        sets.push((
            set_id(SET_SEND_OPT),
            vec![choice(
                question_ids::STEER_OR_SPAWN,
                "Should this input be added to the running turn (steer), wait until the turn ends (queue), \
                 or start a separate agent (spawn)?",
                SEND_OPTIONS.map(String::from).to_vec(),
            )],
        ));
    }
    sets
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 후보 수
// basis: estimate
/// 후보마다 `call_<id>_keep`과 `result_<id>_keep`을 묻는다. 후보를 줄이지 않고 전체를 넘긴다.
pub fn compact_questions(candidates: &[LedgerSeq]) -> (QuestionSetId, Vec<Question>) {
    let questions = candidates
        .iter()
        .flat_map(|seq| {
            [
                noul(
                    &compact_call_id(*seq),
                    &format!("Should tool call {} stay in the handoff context?", seq.0),
                ),
                noul(
                    &compact_result_id(*seq),
                    &format!(
                        "Should the result of tool call {} stay in the handoff context?",
                        seq.0
                    ),
                ),
            ]
        })
        .collect();
    (set_id(SET_COMPACT), questions)
}

// cost: time O(n + q), heap O(n + q·s), stack O(1), alloc 1
// vars: n = 후보 수, q = 질문 수, s = state 글자 수
// basis: estimate
/// 후보 전체를 묻는 요청을 크기 한도에 맞게 나눈 목록이다. 조각마다 같은 `state`를 싣는다.
///
/// # Errors
/// `state`가 너무 커서 질문 하나도 담을 수 없으면 `SplitError::QuestionTooLarge`.
pub fn compact_requests(
    model: &str,
    state: &str,
    candidates: &[LedgerSeq],
) -> Result<Vec<RouterRequest>, SplitError> {
    split_request(RouterRequest {
        model: model.to_string(),
        state: state.to_string(),
        sets: vec![compact_questions(candidates)],
    })
}

// cost: time O(n·r·a), heap O(n), stack O(1)
// vars: n = 후보 수, r = 응답 수, a = 응답당 답 수
// basis: estimate
/// 조각마다 받은 응답에서 `call_<id>_keep`과 `result_<id>_keep` 답 중 큰 값을 `(기록 번호, P(yes))`로 모은다.
/// 실패한 조각의 응답은 넘기지 않으며, 두 답이 모두 없거나 `noul`이 아닌 후보는 뺀다.
pub fn compact_verdicts(
    candidates: &[LedgerSeq],
    responses: &[RouterResponse],
) -> Vec<(LedgerSeq, f64)> {
    let answers: Vec<&(String, Answer)> = responses
        .iter()
        .flat_map(|response| &response.answers)
        .collect();
    let yes_of = |id: &str| {
        answers.iter().find_map(|(answer_id, answer)| match answer {
            Answer::Noul(yes) if answer_id == id => Some(*yes),
            _ => None,
        })
    };
    candidates
        .iter()
        .filter_map(|seq| {
            let call = yes_of(&compact_call_id(*seq));
            let result = yes_of(&compact_result_id(*seq));
            match (call, result) {
                (Some(call), Some(result)) => Some((*seq, call.max(result))),
                (Some(yes), None) | (None, Some(yes)) => Some((*seq, yes)),
                (None, None) => None,
            }
        })
        .collect()
}

/// `keep_current`를 `is_actionable`보다 먼저 읽어 이어 가는 입력이 파일 탐색으로 빠지지 않게 한다.
pub fn decide_route(
    routerd: (&RouterRequest, &RouterResponse),
    thresholds: &Thresholds,
    method: Method,
    revision: ChatRevision,
    settings: SettingsRevision,
) -> RouteDecision {
    let mut decision = RouteDecision {
        revision,
        settings,
        disposition: Disposition::Queue,
        keep_current: true,
        model: None,
        resume_held: false,
        fallbacks: Vec::new(),
    };
    let (request, response) = routerd;
    let reader = AnswerReader {
        request,
        response,
        thresholds,
        method,
    };
    let keep_current = reader.keep_current(&mut decision.fallbacks);
    reader.note_actionable(&mut decision.fallbacks);
    decision.model = reader.target_model(&mut decision.fallbacks);
    decision.resume_held = reader.resume_held(&mut decision.fallbacks);
    decision.disposition = if reader.is_running() {
        reader.running_disposition(&mut decision.fallbacks)
    } else if keep_current {
        Disposition::Queue
    } else {
        Disposition::NewTask
    };
    decision.keep_current = decision.disposition != Disposition::NewTask;
    decision
}

// cost: time O(q·a + q·n), heap O(q), stack O(1), alloc 1
// vars: q = 요청 질문 수, a = 답 수, n = 질문당 선택지 수
// basis: estimate
/// # Errors
/// 요청과 답의 질문이 일대일로 맞지 않거나, 확률이 0~1 밖이거나, 분포 합이 1이 아니면 `RouterError::Invalid`.
pub fn validate(request: &RouterRequest, response: &RouterResponse) -> Result<(), RouterError> {
    let questions: Vec<&Question> = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .collect();
    for (id, _) in &response.answers {
        if !questions.iter().any(|question| question.id == *id) {
            return Err(invalid(format!("unknown question: {id}")));
        }
    }
    for question in questions {
        let mut answers = response
            .answers
            .iter()
            .filter(|(id, _)| *id == question.id)
            .map(|(_, answer)| answer);
        let Some(answer) = answers.next() else {
            return Err(invalid(format!("missing answer: {}", question.id)));
        };
        if answers.next().is_some() {
            return Err(invalid(format!("duplicate answer: {}", question.id)));
        }
        check_answer(question, answer)?;
    }
    Ok(())
}

struct AnswerReader<'a> {
    request: &'a RouterRequest,
    response: &'a RouterResponse,
    thresholds: &'a Thresholds,
    method: Method,
}

impl AnswerReader<'_> {
    // cost: time O(q + a), heap O(1), stack O(1)
    // vars: q = 요청 질문 수, a = 답 수
    // basis: estimate
    fn question(&self, id: &str) -> Option<&Question> {
        self.request
            .sets
            .iter()
            .flat_map(|(_, questions)| questions)
            .find(|question| question.id == id)
    }

    // cost: time O(a), heap O(1), stack O(1)
    // vars: a = 답 수
    // basis: estimate
    fn answer(&self, id: &str) -> Option<&Answer> {
        self.response
            .answers
            .iter()
            .find(|(answer_id, _)| answer_id == id)
            .map(|(_, answer)| answer)
    }

    fn is_asked(&self, id: &str) -> bool {
        self.question(id).is_some()
    }

    fn is_running(&self) -> bool {
        self.is_asked(question_ids::RELATION_TO_RUNNING)
    }

    // cost: time O(a), heap O(1), stack O(1)
    // vars: a = 답 수
    // basis: estimate
    /// `Saturn` 방식에서는 확신도가 기준 미만이어도 `None`이다.
    fn yes(&self, id: &str) -> Option<f64> {
        let answer = self.answer(id)?;
        let Answer::Noul(yes) = answer else {
            return None;
        };
        if !(0.0..=1.0).contains(yes) {
            return None;
        }
        let is_unsure = answer.confidence() < self.thresholds.min_confidence;
        if self.method == Method::Saturn && is_unsure {
            return None;
        }
        Some(*yes)
    }

    // cost: time O(q + a + n), heap O(1), stack O(1)
    // vars: q = 요청 질문 수, a = 답 수, n = 선택지 수
    // basis: estimate
    /// 확신도가 기준 미만이거나 길이가 선택지 수와 다르면 `None`.
    fn picked(&self, id: &str) -> Option<&str> {
        let question = self.question(id)?;
        let AnswerKind::Choice { options } = &question.kind else {
            return None;
        };
        let answer = self.answer(id)?;
        let Answer::Choice(probabilities) = answer else {
            return None;
        };
        if probabilities.len() != options.len() {
            return None;
        }
        if answer.confidence() < self.thresholds.min_confidence {
            return None;
        }
        answer
            .top_index()
            .and_then(|index| options.get(index))
            .map(String::as_str)
    }

    /// 유지 기준과 `KEEP_CURRENT_FLOOR` 사이, 판단 없음은 대체 규칙으로 유지한다.
    fn keep_current(&self, fallbacks: &mut Vec<String>) -> bool {
        match self.yes(question_ids::KEEP_CURRENT) {
            Some(yes) if yes >= self.thresholds.keep_current => true,
            Some(yes) if yes < KEEP_CURRENT_FLOOR => false,
            _ => {
                fallbacks.push(question_ids::KEEP_CURRENT.to_string());
                true
            }
        }
    }

    /// 판단이 없으면 명확으로 보고 탐색을 생략하며, 대체 규칙 기록만 남긴다.
    fn note_actionable(&self, fallbacks: &mut Vec<String>) {
        let is_asked = self.is_asked(question_ids::IS_ACTIONABLE);
        if is_asked && self.yes(question_ids::IS_ACTIONABLE).is_none() {
            fallbacks.push(question_ids::IS_ACTIONABLE.to_string());
        }
    }

    /// 허용 후보 밖이면 대체 규칙으로 `None`(사용자 고정 모델이나 현재 모델).
    fn target_model(&self, fallbacks: &mut Vec<String>) -> Option<String> {
        if !self.is_asked(question_ids::TARGET_MODEL) {
            return None;
        }
        match self.picked(question_ids::TARGET_MODEL) {
            Some(model) if model != OTHER => Some(model.to_string()),
            _ => {
                fallbacks.push(question_ids::TARGET_MODEL.to_string());
                None
            }
        }
    }

    /// 판단이 없으면 재개하지 않는다.
    fn resume_held(&self, fallbacks: &mut Vec<String>) -> bool {
        if !self.is_asked(question_ids::RESUME_HELD) {
            return false;
        }
        match self.yes(question_ids::RESUME_HELD) {
            Some(yes) => yes >= self.thresholds.resume_held,
            None => {
                fallbacks.push(question_ids::RESUME_HELD.to_string());
                false
            }
        }
    }

    /// 관계가 `conflicts`이거나 확신도 미달이면 대기로 둔다.
    /// TODO(#36): `conflicts`일 때 바로 멈출지, 사용자에게 확인할지
    fn running_disposition(&self, fallbacks: &mut Vec<String>) -> Disposition {
        let relation = self.picked(question_ids::RELATION_TO_RUNNING);
        match relation {
            Some("refines" | "continues") => self.send_disposition(fallbacks),
            Some("independent") => Disposition::NewTask,
            Some("conflicts") => Disposition::Queue,
            _ => {
                fallbacks.push(question_ids::RELATION_TO_RUNNING.to_string());
                Disposition::Queue
            }
        }
    }

    /// 확신도 미달이면 현재 에이전트에 대기 뒤 전송.
    fn send_disposition(&self, fallbacks: &mut Vec<String>) -> Disposition {
        match self.picked(question_ids::STEER_OR_SPAWN) {
            Some("steer") => Disposition::Steer,
            Some("queue") => Disposition::Queue,
            Some("spawn") => Disposition::NewTask,
            _ => {
                fallbacks.push(question_ids::STEER_OR_SPAWN.to_string());
                Disposition::Queue
            }
        }
    }
}

/// 초안: 모든 세트를 1.0에서 시작한다.
fn compact_call_id(seq: LedgerSeq) -> String {
    format!("call_{}_keep", seq.0)
}

fn compact_result_id(seq: LedgerSeq) -> String {
    format!("result_{}_keep", seq.0)
}

fn set_id(name: &str) -> QuestionSetId {
    QuestionSetId {
        name: name.to_string(),
        major: 1,
        minor: 0,
    }
}

fn noul(id: &str, text: &str) -> Question {
    Question {
        id: id.to_string(),
        text: text.to_string(),
        kind: AnswerKind::Noul,
    }
}

fn choice(id: &str, text: &str, options: Vec<String>) -> Question {
    Question {
        id: id.to_string(),
        text: text.to_string(),
        kind: AnswerKind::Choice { options },
    }
}

fn invalid(reason: String) -> RouterError {
    RouterError::Invalid { reason }
}

// cost: time O(n), heap O(1), stack O(1)
// vars: n = 선택지 수
// basis: estimate
fn check_answer(question: &Question, answer: &Answer) -> Result<(), RouterError> {
    let id = &question.id;
    let probabilities: &[f64] = match (&question.kind, answer) {
        (AnswerKind::Noul, Answer::Noul(yes)) => std::slice::from_ref(yes),
        (AnswerKind::Choice { options }, Answer::Choice(probabilities)) => {
            check_length(id, options.len(), probabilities.len())?;
            probabilities
        }
        (AnswerKind::Score { levels }, Answer::Score(probabilities)) => {
            check_length(id, usize::from(*levels), probabilities.len())?;
            probabilities
        }
        _ => return Err(invalid(format!("answer kind mismatch: {id}"))),
    };
    let is_in_range = probabilities
        .iter()
        .all(|probability| (0.0..=1.0).contains(probability));
    if !is_in_range {
        return Err(invalid(format!("probability out of range: {id}")));
    }
    let is_distribution = !matches!(answer, Answer::Noul(_));
    let sum: f64 = probabilities.iter().sum();
    if is_distribution && (sum - 1.0).abs() > PROBABILITY_TOLERANCE {
        return Err(invalid(format!("probabilities do not sum to 1: {id}")));
    }
    Ok(())
}

fn check_length(id: &str, expected: usize, actual: usize) -> Result<(), RouterError> {
    if expected == actual {
        return Ok(());
    }
    Err(invalid(format!(
        "probability count mismatch: {id} expected {expected}, got {actual}"
    )))
}

#[cfg(test)]
mod tests {
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
            sets: questions_for_input(running, false, has_held, &models()),
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
    fn confidence_uniform_is_zero_and_certain_is_one() {
        assert_eq!(Answer::Choice(vec![0.25; 4]).confidence(), 0.0);
        assert_eq!(Answer::Choice(vec![0.0, 1.0, 0.0]).confidence(), 1.0);
    }

    #[test]
    fn confidence_matches_formula() {
        let answer = Answer::Choice(vec![0.7, 0.2, 0.1]);

        let expected = (3.0 * 0.7 - 1.0) / 2.0;
        assert!((answer.confidence() - expected).abs() < 1e-12);
    }

    #[test]
    fn confidence_noul_is_two_choices() {
        assert!((Answer::Noul(0.9).confidence() - 0.8).abs() < 1e-12);
        assert!((Answer::Noul(0.1).confidence() - 0.8).abs() < 1e-12);
        assert_eq!(Answer::Noul(0.5).confidence(), 0.0);
    }

    #[test]
    fn confidence_nan_or_empty_is_zero() {
        assert_eq!(Answer::Choice(vec![f64::NAN, 0.5]).confidence(), 0.0);
        assert_eq!(Answer::Choice(Vec::new()).confidence(), 0.0);
        assert_eq!(Answer::Noul(f64::NAN).confidence(), 0.0);
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
    }

    #[test]
    fn questions_for_input_idle_asks_route_only() {
        let sets = questions_for_input(false, false, false, &models());

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
        let sets = questions_for_input(true, false, true, &models());

        let names: Vec<&str> = sets.iter().map(|(set, _)| set.name.as_str()).collect();
        assert_eq!(names, vec!["route", "relation", "send-opt"]);
        assert!(asks(&sets, "resume_held"));
    }

    #[test]
    fn questions_for_input_pinned_model_skips_target_model() {
        let sets = questions_for_input(false, true, false, &models());

        assert!(!asks(&sets, "target_model"));
    }

    // cost: time O(q·n), heap O(1), stack O(1)
    // vars: q = 질문 수, n = 선택지 수
    // basis: estimate
    #[test]
    fn questions_for_input_choice_has_other_option() {
        let sets = questions_for_input(true, false, false, &models());

        let has_other = sets
            .iter()
            .flat_map(|(_, questions)| questions)
            .all(|question| match &question.kind {
                AnswerKind::Choice { options } => options.iter().any(|option| option == "other"),
                _ => true,
            });
        assert!(has_other);
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
    fn validate_well_formed_answer_passes() {
        let request = request(false, false);
        let response = response(vec![
            ("keep_current", Answer::Noul(0.9)),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.3, 0.1])),
        ]);

        assert!(validate(&request, &response).is_ok());
    }

    #[test]
    fn validate_missing_answer_is_invalid() {
        let request = request(false, false);
        let response = response(vec![("keep_current", Answer::Noul(0.9))]);

        assert!(matches!(
            validate(&request, &response),
            Err(RouterError::Invalid { .. })
        ));
    }

    #[test]
    fn validate_unknown_question_is_invalid() {
        let request = request(false, false);
        let response = response(vec![
            ("keep_current", Answer::Noul(0.9)),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.3, 0.1])),
            ("model-c", Answer::Noul(0.5)),
        ]);

        assert!(validate(&request, &response).is_err());
    }

    #[test]
    fn validate_nan_or_wrong_length_is_invalid() {
        let request = request(false, false);
        let nan = response(vec![
            ("keep_current", Answer::Noul(f64::NAN)),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.3, 0.1])),
        ]);
        let short = response(vec![
            ("keep_current", Answer::Noul(0.9)),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.4])),
        ]);

        assert!(validate(&request, &nan).is_err());
        assert!(validate(&request, &short).is_err());
    }

    #[test]
    fn validate_sum_not_one_is_invalid() {
        let request = request(false, false);
        let response = response(vec![
            ("keep_current", Answer::Noul(0.9)),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.6, 0.1])),
        ]);

        assert!(validate(&request, &response).is_err());
    }

    #[test]
    fn validate_kind_mismatch_is_invalid() {
        let request = request(false, false);
        let response = response(vec![
            ("keep_current", Answer::Choice(vec![0.5, 0.5])),
            ("is_actionable", Answer::Noul(0.8)),
            ("target_model", Answer::Choice(vec![0.6, 0.3, 0.1])),
        ]);

        assert!(validate(&request, &response).is_err());
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
}
