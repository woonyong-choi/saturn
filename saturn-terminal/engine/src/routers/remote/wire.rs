use saturn_core::routers::{
    Answer, AnswerKind, Question, RouterError, RouterRequest, RouterResponse,
};
use serde_json::{Map, Value, json};

use super::{MAX_CHOICES, OTHER_CHUNK_OPTION, REQUEST_SPLIT_LIMIT, STATE_SPLIT_LIMIT};

/// 조각마다 `state`는 그대로 싣고, 질문 하나와 `state`만으로 한도를 넘으면 그 질문만 담아 보낸다.
pub(crate) fn split_request(request: RouterRequest) -> Vec<RouterRequest> {
    let base = request.state.len() + request.model.len();
    let mut parts: Vec<RouterRequest> = Vec::new();
    let mut size = base;
    let mut longest = 0;
    for (set, questions) in request.sets {
        for question in questions {
            let question_size = question_body(&question).to_string().len();
            let fits = !parts.is_empty()
                && size + question_size <= REQUEST_SPLIT_LIMIT
                && request.state.len() + longest.max(question_size) <= STATE_SPLIT_LIMIT;
            if !fits {
                parts.push(RouterRequest {
                    model: request.model.clone(),
                    state: request.state.clone(),
                    sets: Vec::new(),
                });
                size = base;
                longest = 0;
            }
            let part = parts.last_mut().expect("a part was pushed above");
            match part.sets.last_mut() {
                Some((last, questions)) if *last == set => questions.push(question),
                _ => part.sets.push((set.clone(), vec![question])),
            }
            size += question_size;
            longest = longest.max(question_size);
        }
    }
    if parts.is_empty() {
        parts.push(RouterRequest {
            model: request.model,
            state: request.state,
            sets: Vec::new(),
        });
    }
    parts
}

/// 조각 사이 무게는 조각의 `none of these`가 아닌 확률 질량으로 정한다. 초안 방식.
/// TODO(#68): 계층 질문의 묶음 나누기와 2차 질문 방식 미정
pub(crate) fn split_choices(question: &Question) -> Vec<Question> {
    let AnswerKind::Choice { options } = &question.kind else {
        return vec![question.clone()];
    };
    if options.len() <= MAX_CHOICES {
        return vec![question.clone()];
    }
    options
        .chunks(MAX_CHOICES - 1)
        .enumerate()
        .map(|(index, chunk)| {
            let mut options = chunk.to_vec();
            options.push(OTHER_CHUNK_OPTION.to_owned());
            Question {
                id: format!("{}#{index}", question.id),
                text: question.text.clone(),
                kind: AnswerKind::Choice { options },
            }
        })
        .collect()
}

/// 계층 선택은 원래 선택지 확률로 되돌린다.
pub(crate) fn merge_responses(
    request: &RouterRequest,
    parts: Vec<RouterResponse>,
) -> RouterResponse {
    let model = parts
        .first()
        .map_or_else(|| request.model.clone(), |part| part.model.clone());
    let tokens = parts.iter().fold((0, 0), |(input, output), part| {
        (input + part.tokens.0, output + part.tokens.1)
    });
    let all: Vec<(String, Answer)> = parts.into_iter().flat_map(|part| part.answers).collect();
    let mut answers = Vec::new();
    for question in request.sets.iter().flat_map(|(_, questions)| questions) {
        let chunks = split_choices(question);
        if chunks.len() == 1 {
            if let Some((_, answer)) = all.iter().find(|(id, _)| *id == question.id) {
                answers.push((question.id.clone(), answer.clone()));
            }
            continue;
        }
        let pieces: Option<Vec<Vec<f64>>> = chunks
            .iter()
            .map(|chunk| match all.iter().find(|(id, _)| *id == chunk.id) {
                Some((_, Answer::Choice(probabilities))) => Some(probabilities.clone()),
                _ => None,
            })
            .collect();
        if let Some(pieces) = pieces {
            answers.push((question.id.clone(), Answer::Choice(combine_chunks(&pieces))));
        }
    }
    RouterResponse {
        model,
        answers,
        tokens,
    }
}

/// 조각 무게는 `1 - P(none)`에 비례한다.
fn combine_chunks(pieces: &[Vec<f64>]) -> Vec<f64> {
    let weights: Vec<f64> = pieces
        .iter()
        .map(|piece| 1.0 - piece.last().copied().unwrap_or(0.0))
        .collect();
    let total: f64 = weights.iter().sum();
    let mut combined = Vec::new();
    for (piece, weight) in pieces.iter().zip(&weights) {
        let inside: f64 = piece[..piece.len() - 1].iter().sum();
        for probability in &piece[..piece.len() - 1] {
            let share = if inside > 0.0 {
                probability / inside
            } else {
                0.0
            };
            let scale = if total > 0.0 {
                weight / total
            } else {
                1.0 / pieces.len() as f64
            };
            combined.push(share * scale);
        }
    }
    combined
}

pub(super) fn expand_choices(request: &RouterRequest) -> RouterRequest {
    RouterRequest {
        model: request.model.clone(),
        state: request.state.clone(),
        sets: request
            .sets
            .iter()
            .map(|(set, questions)| {
                (
                    set.clone(),
                    questions.iter().flat_map(split_choices).collect(),
                )
            })
            .collect(),
    }
}

/// 로컬 서버도 같은 본문을 쓴다.
pub(crate) fn router_body(request: &RouterRequest) -> Value {
    let questions: Map<String, Value> = request
        .sets
        .iter()
        .flat_map(|(_, questions)| questions)
        .map(|question| (question.id.clone(), question_body(question)))
        .collect();
    json!({ "model": request.model, "state": request.state, "questions": questions })
}

/// `score` 단계 설명은 질문에 없어 `level 1`..`level N`을 쓴다. 초안.
fn question_body(question: &Question) -> Value {
    match &question.kind {
        AnswerKind::Noul => json!({ "type": "noul", "instructions": question.text }),
        AnswerKind::Choice { options } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|option| (option.clone(), Value::Null))
                .collect();
            json!({ "type": "choice", "instructions": question.text, "criteria": criteria })
        }
        AnswerKind::Score { levels } => {
            let criteria: Vec<String> = (1..=*levels)
                .map(|level| format!("level {level}"))
                .collect();
            json!({ "type": "score", "instructions": question.text, "criteria": criteria })
        }
    }
}

/// 확률이 없거나 0~1 밖이면 `Invalid`. 답 누락 검사는 `core::validate`가 한다.
pub(crate) fn parse_router_reply(
    request: &RouterRequest,
    body: &str,
) -> Result<RouterResponse, RouterError> {
    let invalid = |reason: &str| RouterError::Invalid {
        reason: reason.to_owned(),
    };
    let parsed: Value = serde_json::from_str(body).map_err(|_| invalid("response is not json"))?;
    let mut answers = Vec::new();
    for question in request.sets.iter().flat_map(|(_, questions)| questions) {
        let Some(answer) = parsed["answers"].get(&question.id) else {
            continue;
        };
        answers.push((question.id.clone(), parse_answer(question, answer)?));
    }
    Ok(RouterResponse {
        model: parsed["model"]
            .as_str()
            .unwrap_or(&request.model)
            .to_owned(),
        answers,
        tokens: (
            parsed["usage"]["input_tokens"].as_u64().unwrap_or(0),
            parsed["usage"]["output_tokens"].as_u64().unwrap_or(0),
        ),
    })
}

fn parse_answer(question: &Question, answer: &Value) -> Result<Answer, RouterError> {
    let invalid = |reason: String| RouterError::Invalid { reason };
    let probability = |value: &Value, what: &str| {
        value
            .as_f64()
            .filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
            .ok_or_else(|| invalid(format!("{}: missing or out of range {what}", question.id)))
    };
    match &question.kind {
        AnswerKind::Noul => Ok(Answer::Noul(probability(&answer["noul"], "noul")?)),
        AnswerKind::Choice { options } => {
            let probabilities = options
                .iter()
                .map(|option| probability(&answer["probabilities"][option], option))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Answer::Choice(probabilities))
        }
        AnswerKind::Score { levels } => {
            let probabilities = (0..*levels)
                .map(|level| {
                    let key = level.to_string();
                    probability(&answer["probabilities"][key.as_str()], &key)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Answer::Score(probabilities))
        }
    }
}
