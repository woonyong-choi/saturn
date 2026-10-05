//! 모델 판단 그림자: 기존 입력 처리 요청에 후보별 독립 질문을 묶어 모델 선택을 미리 재 보되 실제 선택에는 쓰지 않는다.
//! 설계: docs/design/router.md#모델-판단-그림자

use super::split::{MAX_REQUEST_BYTES, MAX_STATE_AND_QUESTION_BYTES, question_bytes};
use super::{
    Answer, Question, QuestionSetId, RouterRequest, RouterResponse, noul, set_id_at, validate,
};

/// 질문 세트 이름. 뜻이 바뀌면 `major`를 올린다.
pub const SET_MODEL_SHADOW: &str = "model-shadow";

const QUESTION_PREFIX: &str = "sufficient:";

/// 후보마다 독립 질문 하나. 질문끼리 서로의 답을 참조하지 않고, 모델과 추론 깊이를 한 질문에 섞지 않아 서로 모순되는 답이 나올
/// 자리가 없다. 질문 id는 `sufficient:<후보 글>`이고 같은 후보는 한 번만 묻는다.
// cost: time O(m), heap O(m), stack O(1)
// vars: m = 후보 수
// basis: estimate
#[must_use]
pub fn shadow_questions(candidates: &[String]) -> (QuestionSetId, Vec<Question>) {
    let mut unique: Vec<&String> = Vec::new();
    for candidate in candidates {
        if !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    let questions = unique
        .into_iter()
        .map(|candidate| {
            noul(
                &format!("{QUESTION_PREFIX}{candidate}"),
                &format!(
                    "Judged on its own, would the model `{candidate}` handle this input well enough? \
                     Do not assume any other question was answered."
                ),
            )
        })
        .collect();
    (set_id_at(SET_MODEL_SHADOW, 1, 0), questions)
}

/// 그림자 질문을 더한 요청이 크기 한도로 나뉘지 않는지. 나뉘면 조각 하나의 실패가 실제 판단까지 막을 수 있어 묻지 않는다.
// cost: time O(q), heap O(1), stack O(1)
// vars: q = 요청 질문 수
// basis: estimate
#[must_use]
pub fn shadow_fits(base: &RouterRequest, shadow: &[Question]) -> bool {
    let fixed = base.model.len() + base.state.len();
    let total: usize = base
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .chain(shadow)
        .map(question_bytes)
        .sum();
    let largest = shadow.iter().map(question_bytes).max().unwrap_or(0);
    fixed + total <= MAX_REQUEST_BYTES && fixed + largest <= MAX_STATE_AND_QUESTION_BYTES
}

/// 그림자 질문을 요청과 답에서 떼어 낸 값. 실제 판단은 `request`와 `response`만 읽는다.
#[derive(Debug, Clone)]
pub struct Split {
    pub request: RouterRequest,
    pub response: Option<RouterResponse>,
    pub shadow: ShadowRead,
}

/// 그림자 질문의 답을 읽은 결과.
#[derive(Debug, Clone, PartialEq)]
pub enum ShadowRead {
    /// 묻지 않았다.
    NotAsked,
    /// 후보 글과 충분할 확률. 물은 후보 순서다.
    Answered(Vec<(String, f64)>),
    /// 답이 형식에 맞지 않았다. 실제 판단에는 영향이 없다.
    Invalid,
    /// 호출이 실패해 답이 없다.
    NoAnswer,
}

/// `request`에서 그림자 세트를 떼고, 있으면 `response`에서 그림자 답을 떼어 읽는다. `response`가 `None`이면 호출이 실패한 것이다.
// cost: time O(q + a), heap O(q + a), stack O(1)
// vars: q = 요청 질문 수, a = 답 수
// basis: estimate
#[must_use]
pub fn split_shadow(request: &RouterRequest, response: Option<&RouterResponse>) -> Split {
    let (shadow_sets, base_sets): (Vec<_>, Vec<_>) = request
        .sets
        .iter()
        .cloned()
        .partition(|(id, _)| id.name == SET_MODEL_SHADOW);
    let base = RouterRequest {
        model: request.model.clone(),
        state: request.state.clone(),
        sets: base_sets,
    };
    let Some((_, shadow_questions)) = shadow_sets.into_iter().next() else {
        return Split {
            request: base,
            response: response.cloned(),
            shadow: ShadowRead::NotAsked,
        };
    };
    let Some(response) = response else {
        return Split {
            request: base,
            response: None,
            shadow: ShadowRead::NoAnswer,
        };
    };
    let is_shadow = |id: &str| shadow_questions.iter().any(|question| question.id == id);
    let (shadow_answers, base_answers): (Vec<_>, Vec<_>) = response
        .answers
        .iter()
        .cloned()
        .partition(|(id, _)| is_shadow(id));
    let shadow_response = RouterResponse {
        model: response.model.clone(),
        answers: shadow_answers,
        tokens: response.tokens,
    };
    let shadow_request = RouterRequest {
        model: request.model.clone(),
        state: String::new(),
        sets: vec![(set_id_at(SET_MODEL_SHADOW, 1, 0), shadow_questions.clone())],
    };
    let read = if validate(&shadow_request, &shadow_response).is_ok() {
        let probabilities = shadow_questions
            .iter()
            .map(|question| {
                let yes = shadow_response
                    .answers
                    .iter()
                    .find_map(|(id, answer)| match answer {
                        Answer::Noul(yes) if *id == question.id => Some(*yes),
                        _ => None,
                    })
                    .unwrap_or(f64::NAN);
                let candidate = question
                    .id
                    .strip_prefix(QUESTION_PREFIX)
                    .unwrap_or(&question.id);
                (candidate.to_owned(), yes)
            })
            .collect();
        ShadowRead::Answered(probabilities)
    } else {
        ShadowRead::Invalid
    };
    Split {
        request: base,
        response: Some(RouterResponse {
            model: response.model.clone(),
            answers: base_answers,
            tokens: response.tokens,
        }),
        shadow: read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::{ConstraintQuestion, questions_for_input};

    fn request(candidates: &[String], state_bytes: usize) -> RouterRequest {
        let mut sets = questions_for_input(false, false, false, &[], ConstraintQuestion::Without);
        if !candidates.is_empty() {
            sets.push(shadow_questions(candidates));
        }
        RouterRequest {
            model: "jev-test".to_owned(),
            state: "s".repeat(state_bytes),
            sets,
        }
    }

    fn response(
        request: &RouterRequest,
        yes: f64,
        skip_shadow_at: Option<usize>,
    ) -> RouterResponse {
        let mut answers = Vec::new();
        for (id, questions) in &request.sets {
            for (index, question) in questions.iter().enumerate() {
                if id.name == SET_MODEL_SHADOW && skip_shadow_at == Some(index) {
                    continue;
                }
                answers.push((question.id.clone(), Answer::Noul(yes)));
            }
        }
        RouterResponse {
            model: "jev-test".to_owned(),
            answers,
            tokens: (1, 1),
        }
    }

    // 그림자 질문은 실제 요청과 답에서 갈라지고, 그림자 답이 틀려도 실제 판단 입력은 그대로이며, 크기를 넘으면 묻지 않는다
    #[test]
    fn shadow_answers_split_off_and_never_disturb_the_real_judgment() {
        let candidates = vec!["claude/opus".to_owned(), "codex/gpt-x".to_owned()];
        let full = request(&candidates, 10);
        let plain = request(&[], 10);

        let good = split_shadow(&full, Some(&response(&full, 0.7, None)));
        assert_eq!(
            good.shadow,
            ShadowRead::Answered(vec![
                ("claude/opus".to_owned(), 0.7),
                ("codex/gpt-x".to_owned(), 0.7)
            ])
        );
        assert!(validate(&good.request, good.response.as_ref().unwrap()).is_ok());
        assert_eq!(good.request.sets.len(), plain.sets.len());

        // 그림자 답 하나가 빠져도 실제 판단은 그대로 유효하다
        let partial = split_shadow(&full, Some(&response(&full, 0.7, Some(1))));
        assert_eq!(partial.shadow, ShadowRead::Invalid);
        assert!(validate(&partial.request, partial.response.as_ref().unwrap()).is_ok());

        let failed = split_shadow(&full, None);
        assert_eq!(failed.shadow, ShadowRead::NoAnswer);
        assert!(failed.response.is_none());

        let not_asked = split_shadow(&plain, Some(&response(&plain, 0.5, None)));
        assert_eq!(not_asked.shadow, ShadowRead::NotAsked);

        let (_, questions) = shadow_questions(&candidates);
        assert!(shadow_fits(&plain, &questions));
        assert!(!shadow_fits(&request(&[], MAX_REQUEST_BYTES), &questions));
    }
}
