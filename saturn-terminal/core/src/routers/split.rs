//! 크기 한도를 넘는 router 요청을 질문 단위로 나누는 순수 규칙(전송은 `saturn-engine`).
//! 설계: docs/design/router.md

use super::{AnswerKind, Question, QuestionSetId, RouterRequest};

/// 요청 한 건의 한도. 토큰을 셀 수 없어 본문 바이트로 잰다(초안).
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

/// `state`와 가장 긴 질문 한 개의 합 한도(바이트, 초안).
pub const MAX_STATE_AND_QUESTION_BYTES: usize = 32 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    /// 나눠도 줄지 않는 크기라 `state`를 줄여야 한다.
    #[error("state and question {question_id} exceed the size limit")]
    QuestionTooLarge { question_id: String },
}

// cost: time O(q), heap O(q), stack O(1), alloc 1
// vars: q = 질문 수
// basis: estimate
/// 질문을 순서대로 채워 조각마다 `model`과 같은 `state`를 싣는다. 질문이 없으면 빈 목록이다.
///
/// # Errors
/// `state`와 질문 하나만으로 [`MAX_STATE_AND_QUESTION_BYTES`]를 넘으면 `SplitError::QuestionTooLarge`.
pub fn split_request(request: RouterRequest) -> Result<Vec<RouterRequest>, SplitError> {
    let RouterRequest { model, state, sets } = request;
    let fixed_bytes = model.len() + state.len();
    let mut pieces = Vec::new();
    let mut current: Vec<(QuestionSetId, Vec<Question>)> = Vec::new();
    let mut used_bytes = fixed_bytes;
    for (set_id, questions) in sets {
        for question in questions {
            let bytes = question_bytes(&question);
            if fixed_bytes + bytes > MAX_STATE_AND_QUESTION_BYTES {
                return Err(SplitError::QuestionTooLarge {
                    question_id: question.id,
                });
            }
            if used_bytes + bytes > MAX_REQUEST_BYTES && !current.is_empty() {
                pieces.push(piece(&model, &state, std::mem::take(&mut current)));
                used_bytes = fixed_bytes;
            }
            used_bytes += bytes;
            push_question(&mut current, &set_id, question);
        }
    }
    if !current.is_empty() {
        pieces.push(piece(&model, &state, current));
    }
    Ok(pieces)
}

// cost: time O(o), heap O(1), stack O(1)
// vars: o = 선택지 수
// basis: estimate
fn question_bytes(question: &Question) -> usize {
    let options_bytes = match &question.kind {
        AnswerKind::Choice { options } => options.iter().map(String::len).sum(),
        AnswerKind::Noul | AnswerKind::Score { .. } => 0,
    };
    question.id.len() + question.text.len() + options_bytes
}

// cost: time O(1), heap O(1), stack O(1)
// basis: estimate
/// 이어지는 질문은 앞 조각의 같은 질문 세트에 붙는다.
fn push_question(
    current: &mut Vec<(QuestionSetId, Vec<Question>)>,
    set_id: &QuestionSetId,
    question: Question,
) {
    match current.last_mut() {
        Some((last_id, questions)) if last_id == set_id => questions.push(question),
        _ => current.push((set_id.clone(), vec![question])),
    }
}

// cost: time O(s), heap O(s), stack O(1), alloc 1
// vars: s = state 글자 수
// basis: estimate
fn piece(model: &str, state: &str, sets: Vec<(QuestionSetId, Vec<Question>)>) -> RouterRequest {
    RouterRequest {
        model: model.to_string(),
        state: state.to_string(),
        sets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routers::{SET_COMPACT, compact_questions};
    use saturn_protocol::ids::LedgerSeq;

    // cost: time O(q), heap O(q), stack O(1)
    // vars: q = 질문 수
    // basis: estimate
    fn compact_request(state_bytes: usize, candidates: u64) -> RouterRequest {
        let seqs: Vec<LedgerSeq> = (0..candidates).map(LedgerSeq).collect();
        RouterRequest {
            model: "jev-test".into(),
            state: "s".repeat(state_bytes),
            sets: vec![compact_questions(&seqs)],
        }
    }

    // cost: time O(p), heap O(1), stack O(1)
    // vars: p = 조각 수
    // basis: estimate
    fn question_count(pieces: &[RouterRequest]) -> usize {
        pieces
            .iter()
            .flat_map(|piece| &piece.sets)
            .map(|(_, questions)| questions.len())
            .sum()
    }

    #[test]
    fn split_request_within_limit_stays_one_piece() {
        let pieces = split_request(compact_request(1_000, 150)).unwrap();

        assert_eq!(pieces.len(), 1);
        assert_eq!(question_count(&pieces), 300);
    }

    // cost: time O(c), heap O(c), stack O(1)
    // vars: c = 질문 수
    // basis: estimate
    #[test]
    fn split_request_over_limit_splits_by_question_with_same_state() {
        let request = compact_request(20_000, 1_000);

        let pieces = split_request(request).unwrap();

        assert!(pieces.len() > 1);
        assert_eq!(question_count(&pieces), 2_000);
        for piece in &pieces {
            let bytes: usize = piece.state.len()
                + piece.model.len()
                + piece
                    .sets
                    .iter()
                    .flat_map(|(_, questions)| questions)
                    .map(question_bytes)
                    .sum::<usize>();
            assert!(bytes <= MAX_REQUEST_BYTES);
            assert_eq!(piece.state, "s".repeat(20_000));
            assert!(piece.sets.iter().all(|(id, _)| id.name == SET_COMPACT));
        }
    }

    // cost: time O(c), heap O(c), stack O(1)
    // vars: c = 질문 수
    // basis: estimate
    #[test]
    fn split_request_keeps_question_order_across_pieces() {
        let pieces = split_request(compact_request(20_000, 1_000)).unwrap();

        let ids: Vec<&str> = pieces
            .iter()
            .flat_map(|piece| &piece.sets)
            .flat_map(|(_, questions)| questions)
            .map(|question| question.id.as_str())
            .collect();

        assert_eq!(ids[0], "call_0_keep");
        assert_eq!(ids[1], "result_0_keep");
        assert_eq!(ids[1_998], "call_999_keep");
    }

    #[test]
    fn split_request_state_too_large_for_one_question_is_error() {
        let request = compact_request(MAX_STATE_AND_QUESTION_BYTES, 1);

        assert!(matches!(
            split_request(request),
            Err(SplitError::QuestionTooLarge { .. })
        ));
    }

    #[test]
    fn split_request_no_questions_is_empty() {
        assert!(split_request(compact_request(100, 0)).unwrap().is_empty());
    }
}
