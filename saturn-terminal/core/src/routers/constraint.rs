//! 제약 등록 질문을 만들고 답을 읽는 순수 규칙.
//! 설계: docs/design/constraints.md, docs/design/router.md

use super::{
    Answer, Method, Question, QuestionSetId, RouterRequest, RouterResponse, Thresholds,
    question_ids, set_id,
};
use super::{SET_CONSTRAINT, noul};

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
