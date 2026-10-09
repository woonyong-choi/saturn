//! 제약 등록 질문을 만들고 답을 읽는 순수 규칙.
//! 설계: docs/design/constraints.md, docs/design/router.md

use super::{
    Answer, Method, Question, QuestionSetId, RouterRequest, RouterResponse, Thresholds,
    question_ids, set_id,
};
use super::{SET_CONSTRAINT, choice, noul};

/// `route@1.1`의 `is_constraint` 질문 문장.
pub(super) const IS_CONSTRAINT_TEXT: &str = "Does the user's latest input set a rule that applies beyond this single request \
     and limits how the work is done (language, tool, format, or prohibition) rather than what to do?";

/// 등록 판단이 정한 행동.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationAction {
    /// 등록하지 않는다. 판단이 없거나 `invalid`인 답도 같다.
    Skip,
    /// 묻지 않고 등록한다.
    Auto,
    /// 등록할지 사용자에게 묻는다.
    Ask,
}

/// `is_constraint` 답을 읽은 결과.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegistrationVerdict {
    pub action: RegistrationAction,
    /// 읽은 P(yes). 판단이 없거나 `invalid`면 `None`.
    pub probability: Option<f64>,
}

// cost: time O(q + a), heap O(1), stack O(1)
// vars: q = 요청 질문 수, a = 답 수
// basis: estimate
/// `is_constraint` 확률이 `is_constraint` 기준값 이상이면 자동, `constraint_ask` 이상 미만이면 묻기, 그 미만이면 미등록이다.
/// 질문하지 않았거나 답이 없거나 범위 밖이면 미등록이다. `saturn` 방식에서 확신도가 `min_confidence` 미만인 답은
/// 판단 없음이 아니라 묻는 구간으로 본다.
pub fn read_registration(
    request: &RouterRequest,
    response: &RouterResponse,
    thresholds: &Thresholds,
    method: Method,
) -> RegistrationVerdict {
    let none = RegistrationVerdict {
        action: RegistrationAction::Skip,
        probability: None,
    };
    let is_asked = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .any(|question| question.id == question_ids::IS_CONSTRAINT);
    let answer = response
        .answers
        .iter()
        .find(|(id, _)| id == question_ids::IS_CONSTRAINT)
        .map(|(_, answer)| answer);
    let (true, Some(answer @ Answer::Noul(yes))) = (is_asked, answer) else {
        return none;
    };
    if !(0.0..=1.0).contains(yes) {
        return none;
    }
    let is_unsure = answer.confidence() < thresholds.min_confidence;
    let action = if method == Method::Saturn && is_unsure {
        RegistrationAction::Ask
    } else if *yes >= thresholds.is_constraint {
        RegistrationAction::Auto
    } else if *yes >= thresholds.constraint_ask {
        RegistrationAction::Ask
    } else {
        RegistrationAction::Skip
    };
    RegistrationVerdict {
        action,
        probability: Some(*yes),
    }
}

/// 문장 번호 `line`(1부터)의 질문 ID.
pub fn line_question_id(line: usize) -> String {
    format!("line_{line}_is_constraint")
}

// cost: time O(n), heap O(n), stack O(1)
// vars: n = 문장 수
// basis: estimate
/// 문장마다 제약인지 묻는 `constraint@1.0` 질문. 문장은 `state`에 `[k] 글` 줄로 싣는다.
pub fn line_questions(count: usize) -> (QuestionSetId, Vec<Question>) {
    let questions = (1..=count)
        .map(|line| {
            noul(
                &line_question_id(line),
                &format!(
                    "Does sentence [{line}] of the user's latest input set a rule that applies beyond this single request \
                     and limits how the work is done (language, tool, format, or prohibition) rather than what to do?"
                ),
            )
        })
        .collect();
    (set_id(SET_CONSTRAINT), questions)
}

// cost: time O(n·a), heap O(n), stack O(1)
// vars: n = 문장 수, a = 답 수
// basis: estimate
/// 확률이 `constraint_ask` 이상인 문장 번호(1부터)를 순서대로 돌려준다.
/// 문장 하나라도 답이 없거나 `noul`이 아니거나 범위 밖이면 질문이 실패한 것이라 `None`이다.
pub fn read_lines(
    response: &RouterResponse,
    count: usize,
    thresholds: &Thresholds,
) -> Option<Vec<usize>> {
    let mut picked = Vec::new();
    for line in 1..=count {
        let id = line_question_id(line);
        let (_, answer) = response
            .answers
            .iter()
            .find(|(answered, _)| *answered == id)?;
        let Answer::Noul(yes) = answer else {
            return None;
        };
        if !(0.0..=1.0).contains(yes) {
            return None;
        }
        if *yes >= thresholds.constraint_ask {
            picked.push(line);
        }
    }
    Some(picked)
}

/// 해제와 예외의 종류. `constraint_change`의 선택지 접두어와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// 영구 해제.
    Release,
    /// 그 작업 하나 동안만 멈춘다.
    Once,
    /// 사용자가 말한 조건이나 범위에서만 멈춘다.
    Scoped,
}

impl ChangeKind {
    const ALL: [Self; 3] = [Self::Release, Self::Once, Self::Scoped];

    fn prefix(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Once => "once",
            Self::Scoped => "scoped",
        }
    }
}

/// `constraint_change` 답을 읽은 결과.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChangeVerdict {
    /// 아무것도 하지 않는다. 질문하지 않았거나 답이 없거나 범위 밖이거나 요청 확률이 기준 미만이다.
    None,
    /// 요청과 종류가 모두 확실하다. `target`은 질문에 실은 제약의 0부터 세는 순서다.
    Apply { target: usize, kind: ChangeKind },
    /// 요청은 확실한데 종류의 확률이 기준 미만이다. `kind`는 가장 높게 본 종류다.
    UnsureKind { target: usize, kind: ChangeKind },
}

/// `scoped` 조건 문장의 글자 수 상한. 넘으면 입력 앞에서 자른다.
const CONDITION_MAX_CHARS: usize = 200;

fn change_options(count: usize) -> Vec<String> {
    let mut options = vec!["none".to_owned()];
    for number in 1..=count {
        for kind in ChangeKind::ALL {
            options.push(format!("{}_{number}", kind.prefix()));
        }
    }
    options
}

/// 해제·예외 판단을 묻는 `constraint@1.0` 질문. 선택지는 `none`과 제약마다 `release_k`, `once_k`, `scoped_k`이다.
/// 제약은 `state`에 `[k] 규칙` 줄로 싣는다.
pub fn change_question(count: usize) -> (QuestionSetId, Vec<Question>) {
    let question = choice(
        question_ids::CONSTRAINT_CHANGE,
        "Does the user's latest input ask to lift one of the listed constraints? \
         release_k lifts constraint [k] permanently, once_k lifts it only for the current task, \
         scoped_k lifts it only under a condition or scope the user states, none means no request.",
        change_options(count),
    );
    (set_id(SET_CONSTRAINT), vec![question])
}

// cost: time O(k), heap O(k), stack O(1)
// vars: k = 질문에 실은 제약 수
// basis: estimate
/// 요청 확률은 `1 − P(none)`이다. `constraint_release` 미만이면 아무것도 하지 않는다. 대상과 종류는 `none`을 뺀 가장 높은
/// 선택지에서 읽고, 그 선택지의 몫(요청 확률 중)이 `constraint_release` 미만이면 종류를 확신하지 못한 것이다.
/// 질문하지 않았거나 답의 모양이 틀렸거나 `saturn` 방식에서 확신도가 `min_confidence` 미만이면 아무것도 하지 않는다.
pub fn read_change(
    request: &RouterRequest,
    response: &RouterResponse,
    count: usize,
    thresholds: &Thresholds,
    method: Method,
) -> ChangeVerdict {
    let is_asked = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .any(|question| question.id == question_ids::CONSTRAINT_CHANGE);
    let answer = response
        .answers
        .iter()
        .find(|(id, _)| id == question_ids::CONSTRAINT_CHANGE)
        .map(|(_, answer)| answer);
    let (true, Some(answer @ Answer::Choice(probabilities))) = (is_asked, answer) else {
        return ChangeVerdict::None;
    };
    let in_range = probabilities.iter().all(|p| (0.0..=1.0).contains(p));
    if probabilities.len() != 1 + count * ChangeKind::ALL.len() || !in_range {
        return ChangeVerdict::None;
    }
    if method == Method::Saturn && answer.confidence() < thresholds.min_confidence {
        return ChangeVerdict::None;
    }
    let requested = 1.0 - probabilities[0];
    if requested < thresholds.constraint_release {
        return ChangeVerdict::None;
    }
    let Some((index, top)) = probabilities
        .iter()
        .enumerate()
        .skip(1)
        .max_by(|left, right| left.1.total_cmp(right.1))
    else {
        return ChangeVerdict::None;
    };
    let target = (index - 1) / ChangeKind::ALL.len();
    let kind = ChangeKind::ALL[(index - 1) % ChangeKind::ALL.len()];
    if top / requested >= thresholds.constraint_release {
        ChangeVerdict::Apply { target, kind }
    } else {
        ChangeVerdict::UnsureKind { target, kind }
    }
}

/// `scoped` 예외의 조건 문장. 입력 원문에서 앞뒤 공백을 뺀 연속된 글이고 요약하거나 새로 쓰지 않는다.
/// 200자(초안)를 넘으면 앞 200자까지다. 비어 있으면 `None`이다.
pub fn scoped_condition(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(CONDITION_MAX_CHARS).collect())
}
