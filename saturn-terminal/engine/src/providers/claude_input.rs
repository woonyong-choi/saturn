//! Claude `AskUserQuestion`(stream-json `can_use_tool` 요청)을 공통 입력 요청으로 옮기고, 사용자 답을 응답 값으로
//! 바꾼다. 설계: docs/design/permissions.md#입력-요청

use saturn_protocol::input::{
    InputAnswer, InputField, InputFieldKind, InputOption, InputRequest, InputValue,
};
use serde_json::{Map, Value, json};

/// 이 도구 이름의 `can_use_tool` 요청이 사용자 질문이다.
pub(super) const ASK_TOOL: &str = "AskUserQuestion";

/// 거절과 취소에 붙는 문구. 모델에 전달된다. 초안.
const DECLINE_MESSAGE: &str = "The user declined to answer in Saturn.";

/// 여러 개를 고른 답을 한 글로 합칠 때 쓰는 구분.
const MULTI_SEPARATOR: &str = ", ";

// cost: time O(q·o), heap O(q·o), stack O(1), alloc q·o
// vars: q = 질문 수, o = 선택지 수
// basis: estimate
/// `input.questions`의 질문마다 칸 하나. Claude는 질문 글이 답의 키라서 칸 `id`도 질문 글이고, 어느 질문에서나
/// 목록에 없는 글을 쓸 수 있다(`Other`). 질문이 없으면 `None`.
pub(super) fn request(input: &Value) -> Option<InputRequest> {
    let questions = input["questions"]
        .as_array()
        .filter(|list| !list.is_empty())?;
    Some(InputRequest {
        message: String::new(),
        fields: questions.iter().map(field).collect(),
        url: None,
    })
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
fn field(question: &Value) -> InputField {
    let text = question["question"].as_str().unwrap_or_default().to_owned();
    let options: Vec<InputOption> = question["options"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|choice| {
                    let label = choice["label"].as_str()?.to_owned();
                    Some(InputOption {
                        value: label.clone(),
                        label,
                        description: choice["description"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let kind = match (options.is_empty(), question["multiSelect"].as_bool()) {
        (true, _) => InputFieldKind::Text,
        (false, Some(true)) => InputFieldKind::MultiChoice {
            options,
            allows_other: true,
        },
        (false, _) => InputFieldKind::Choice {
            options,
            allows_other: true,
        },
    };
    InputField {
        id: text.clone(),
        title: text,
        description: question["header"].as_str().unwrap_or_default().to_owned(),
        kind,
        is_required: true,
        is_secret: false,
    }
}

// cost: time O(v), heap O(v), stack O(1), alloc v
// vars: v = 답 값 수
// basis: estimate
/// 답은 요청 `input`에 `answers`(질문 글 → 답 글)를 더해 돌려준다. 여럿을 고른 답은 `, `로 합친다. 거절과
/// 취소는 호출을 거부해 모델이 답이 없음을 안다.
pub(super) fn response(input: &Value, answer: &InputAnswer) -> Value {
    let InputAnswer::Submit { values } = answer else {
        return json!({ "behavior": "deny", "message": DECLINE_MESSAGE });
    };
    let answers: Map<String, Value> = values
        .iter()
        .map(|(question, value)| (question.clone(), json!(answer_text(value))))
        .collect();
    let mut updated = input.clone();
    updated["answers"] = Value::Object(answers);
    json!({ "behavior": "allow", "updatedInput": updated })
}

// cost: time O(s), heap O(s), stack O(1), alloc 1
// vars: s = 고른 값 글자 수 합
// basis: estimate
fn answer_text(value: &InputValue) -> String {
    match value {
        InputValue::Text(text) => text.clone(),
        InputValue::Integer(number) => number.to_string(),
        InputValue::Number(number) => number.to_string(),
        InputValue::Boolean(flag) => flag.to_string(),
        InputValue::Selected(picked) => picked.join(MULTI_SEPARATOR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실측 원문(2026-10-02, claude 2.1.285)의 `input`.
    fn measured_input() -> Value {
        json!({ "questions": [{
            "question": "What is the name of your project?", "header": "Project name",
            "options": [
                { "label": "Saturn WT", "description": "The Saturn Waterfall Testing project" },
                { "label": "Other project", "description": "A different project name" }
            ],
            "multiSelect": false
        }] })
    }

    #[test]
    fn ask_user_question_becomes_one_choice_field_per_question() {
        let request = request(&measured_input()).unwrap();

        assert_eq!(request.fields.len(), 1);
        let field = &request.fields[0];
        assert_eq!(field.id, "What is the name of your project?");
        assert_eq!(field.description, "Project name");
        assert!(field.is_required);
        assert!(matches!(
            &field.kind,
            InputFieldKind::Choice { options, allows_other: true }
                if options[0].value == "Saturn WT" && options[1].description == "A different project name"
        ));
        assert_eq!(super::request(&json!({ "questions": [] })), None);
    }

    #[test]
    fn ask_user_question_multi_select_is_a_multi_choice_field() {
        let input = json!({ "questions": [{
            "question": "Which?", "header": "h", "multiSelect": true,
            "options": [{ "label": "A", "description": "" }, { "label": "B", "description": "" }]
        }] });

        let request = request(&input).unwrap();

        assert!(matches!(
            &request.fields[0].kind,
            InputFieldKind::MultiChoice { options, .. } if options.len() == 2
        ));
    }

    #[test]
    fn ask_user_question_answers_are_keyed_by_the_question_text() {
        let input = measured_input();
        let submit = InputAnswer::Submit {
            values: vec![
                (
                    "What is the name of your project?".to_owned(),
                    InputValue::Selected(vec!["Saturn WT".to_owned()]),
                ),
                (
                    "Which?".to_owned(),
                    InputValue::Selected(vec!["A".to_owned(), "B".to_owned()]),
                ),
            ],
        };

        let response = response(&input, &submit);

        assert_eq!(response["behavior"], "allow");
        assert_eq!(response["updatedInput"]["questions"], input["questions"]);
        assert_eq!(
            response["updatedInput"]["answers"],
            json!({ "What is the name of your project?": "Saturn WT", "Which?": "A, B" })
        );
    }

    #[test]
    fn ask_user_question_decline_and_cancel_deny_the_call() {
        let input = measured_input();

        for answer in [InputAnswer::Decline, InputAnswer::Cancel] {
            assert_eq!(
                response(&input, &answer),
                json!({ "behavior": "deny", "message": DECLINE_MESSAGE })
            );
        }
    }
}
