//! Codex 입력 요청(`mcpServer/elicitation/request`의 폼과 URL, `item/tool/requestUserInput`)을 공통 입력 요청으로
//! 옮기고, 사용자 답을 Codex 응답 값으로 바꾼다. 설계: docs/design/permissions.md#입력-요청

use saturn_protocol::input::{
    InputAnswer, InputField, InputFieldKind, InputOption, InputRequest, InputValue,
};
use serde_json::{Map, Value, json};

/// 응답 모양이 다른 두 요청 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InputKind {
    /// `mcpServer/elicitation/request`
    Elicitation,
    /// `item/tool/requestUserInput`
    UserInput,
}

// cost: time O(p log p), heap O(p), stack O(1), alloc p
// vars: p = 폼 칸 수
// basis: estimate
/// 폼은 `requestedSchema.properties`를 칸으로, URL 모드는 `url`을 링크로 옮긴다. 폼 칸 순서는 `required` 목록
/// 순서가 먼저이고 나머지는 이름순이다(`properties`는 순서 없는 객체로 받는다). 모르는 `mode`면 `None`.
pub(super) fn elicitation_request(params: &Value) -> Option<InputRequest> {
    let message = params["message"].as_str().unwrap_or_default().to_owned();
    match params["mode"].as_str() {
        Some("url") => Some(InputRequest {
            message,
            fields: Vec::new(),
            url: Some(params["url"].as_str()?.to_owned()),
        }),
        Some("form") => Some(InputRequest {
            message,
            fields: form_fields(&params["requestedSchema"]),
            url: None,
        }),
        _ => None,
    }
}

// cost: time O(p log p), heap O(p), stack O(1), alloc p
// vars: p = 폼 칸 수
// basis: estimate
fn form_fields(schema: &Value) -> Vec<InputField> {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };
    let mut ids: Vec<&String> = properties.keys().collect();
    ids.sort_by_key(|id| {
        required
            .iter()
            .position(|name| name == id)
            .unwrap_or(usize::MAX)
    });
    ids.into_iter()
        .map(|id| form_field(id, &properties[id], required.contains(&id.as_str())))
        .collect()
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
fn form_field(id: &str, schema: &Value, is_required: bool) -> InputField {
    let kind = match schema["type"].as_str() {
        Some("boolean") => InputFieldKind::Boolean,
        Some("integer") => InputFieldKind::Integer,
        Some("number") => InputFieldKind::Number,
        Some("array") => {
            let options = options_of(&schema["items"]);
            InputFieldKind::MultiChoice {
                allows_other: options.is_empty(),
                options,
            }
        }
        _ => {
            let options = options_of(schema);
            if options.is_empty() {
                InputFieldKind::Text
            } else {
                InputFieldKind::Choice {
                    options,
                    allows_other: false,
                }
            }
        }
    };
    InputField {
        id: id.to_owned(),
        title: schema["title"]
            .as_str()
            .filter(|title| !title.is_empty())
            .unwrap_or(id)
            .to_owned(),
        description: schema["description"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        kind,
        is_required,
        is_secret: false,
    }
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
/// `enum`(이름은 `enumNames`)이나 `oneOf`/`anyOf`(`const`와 `title`)로 적은 선택지.
fn options_of(schema: &Value) -> Vec<InputOption> {
    if let Some(values) = schema["enum"].as_array() {
        let names = schema["enumNames"].as_array();
        return values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let text = value_text(value);
                let label = names
                    .and_then(|names| names.get(index))
                    .and_then(Value::as_str)
                    .map_or_else(|| text.clone(), str::to_owned);
                option(text, label, String::new())
            })
            .collect();
    }
    ["oneOf", "anyOf"]
        .iter()
        .find_map(|key| schema[*key].as_array())
        .map(|choices| {
            choices
                .iter()
                .filter(|choice| !choice["const"].is_null())
                .map(|choice| {
                    let text = value_text(&choice["const"]);
                    let label = choice["title"]
                        .as_str()
                        .map_or_else(|| text.clone(), str::to_owned);
                    option(text, label, String::new())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn option(value: String, label: String, description: String) -> InputOption {
    InputOption {
        value,
        label,
        description,
    }
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

// cost: time O(q), heap O(q), stack O(1), alloc q
// vars: q = 질문 수
// basis: estimate
/// `questions`의 질문마다 칸 하나. 선택지가 없으면 글 칸, 있으면 고르기 칸이고 `isOther`면 직접 쓰기도 허용한다.
pub(super) fn user_input_request(params: &Value) -> InputRequest {
    let fields = params["questions"]
        .as_array()
        .map(|questions| questions.iter().map(question_field).collect())
        .unwrap_or_default();
    InputRequest {
        message: String::new(),
        fields,
        url: None,
    }
}

// cost: time O(o), heap O(o), stack O(1), alloc o
// vars: o = 선택지 수
// basis: estimate
fn question_field(question: &Value) -> InputField {
    let options: Vec<InputOption> = question["options"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|choice| {
                    let label = choice["label"].as_str()?.to_owned();
                    let description = choice["description"].as_str().unwrap_or_default();
                    Some(option(label.clone(), label, description.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let kind = if options.is_empty() {
        InputFieldKind::Text
    } else {
        InputFieldKind::Choice {
            options,
            allows_other: question["isOther"].as_bool().unwrap_or(false),
        }
    };
    InputField {
        id: question["id"].as_str().unwrap_or_default().to_owned(),
        title: question["question"].as_str().unwrap_or_default().to_owned(),
        description: question["header"].as_str().unwrap_or_default().to_owned(),
        kind,
        is_required: true,
        is_secret: question["isSecret"].as_bool().unwrap_or(false),
    }
}

// cost: time O(v·f), heap O(v), stack O(1), alloc v
// vars: v = 답 값 수, f = 요청 칸 수
// basis: estimate
/// Elicitation은 `accept`(폼이면 `content` 포함), `decline`, `cancel`. 질문 요청은 질문 id별 `answers`이고,
/// 거절과 취소는 답 없이 끝내려고 빈 `answers`를 보낸다(요청에 거절 값이 없다).
pub(super) fn result(kind: InputKind, request: &InputRequest, answer: &InputAnswer) -> Value {
    match (kind, answer) {
        (InputKind::Elicitation, InputAnswer::Submit { values }) => {
            if request.url.is_some() {
                return json!({ "action": "accept" });
            }
            json!({ "action": "accept", "content": content(request, values) })
        }
        (InputKind::Elicitation, InputAnswer::Decline) => json!({ "action": "decline" }),
        (InputKind::Elicitation, InputAnswer::Cancel) => json!({ "action": "cancel" }),
        (InputKind::UserInput, InputAnswer::Submit { values }) => {
            let answers: Map<String, Value> = values
                .iter()
                .map(|(id, value)| (id.clone(), json!({ "answers": answer_texts(value) })))
                .collect();
            json!({ "answers": answers })
        }
        (InputKind::UserInput, InputAnswer::Decline | InputAnswer::Cancel) => {
            json!({ "answers": {} })
        }
    }
}

// cost: time O(v·f), heap O(v), stack O(1), alloc v
// vars: v = 답 값 수, f = 요청 칸 수
// basis: estimate
fn content(request: &InputRequest, values: &[(String, InputValue)]) -> Value {
    let content: Map<String, Value> = values
        .iter()
        .map(|(id, value)| {
            let is_multi = request.fields.iter().any(|field| {
                field.id == *id && matches!(field.kind, InputFieldKind::MultiChoice { .. })
            });
            (id.clone(), content_value(value, is_multi))
        })
        .collect();
    Value::Object(content)
}

fn content_value(value: &InputValue, is_multi: bool) -> Value {
    match value {
        InputValue::Text(text) => json!(text),
        InputValue::Integer(number) => json!(number),
        InputValue::Number(number) => json!(number),
        InputValue::Boolean(flag) => json!(flag),
        InputValue::Selected(picked) if is_multi => json!(picked),
        InputValue::Selected(picked) => json!(picked.first().cloned().unwrap_or_default()),
    }
}

fn answer_texts(value: &InputValue) -> Vec<String> {
    match value {
        InputValue::Text(text) => vec![text.clone()],
        InputValue::Integer(number) => vec![number.to_string()],
        InputValue::Number(number) => vec![number.to_string()],
        InputValue::Boolean(flag) => vec![flag.to_string()],
        InputValue::Selected(picked) => picked.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실측 원문(2026-10-02, codex-cli 0.158.0)의 폼 요청.
    fn measured_form() -> Value {
        json!({
            "threadId": "t", "turnId": "u", "serverName": "experiment_252", "mode": "form", "_meta": null,
            "message": "실험 입력을 작성하라",
            "requestedSchema": {
                "type": "object",
                "properties": {
                    "choice": { "type": "string", "title": "단일 선택", "enum": ["a", "b"] },
                    "count": { "type": "integer", "title": "횟수", "minimum": 0 },
                    "enabled": { "type": "boolean", "title": "사용" },
                    "tags": { "type": "array", "title": "다중 선택", "items": { "type": "string", "enum": ["x", "y"] } },
                    "title": { "type": "string", "title": "제목" }
                },
                "required": ["title", "count", "enabled", "choice", "tags"]
            }
        })
    }

    #[test]
    fn elicitation_form_fields_follow_the_required_order_and_kinds() {
        let request = elicitation_request(&measured_form()).unwrap();

        let kinds: Vec<(&str, &InputFieldKind)> = request
            .fields
            .iter()
            .map(|field| (field.id.as_str(), &field.kind))
            .collect();
        assert_eq!(request.message, "실험 입력을 작성하라");
        assert_eq!(request.url, None);
        assert_eq!(
            kinds.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            ["title", "count", "enabled", "choice", "tags"]
        );
        assert_eq!(kinds[0].1, &InputFieldKind::Text);
        assert_eq!(kinds[1].1, &InputFieldKind::Integer);
        assert_eq!(kinds[2].1, &InputFieldKind::Boolean);
        assert!(matches!(
            kinds[3].1,
            InputFieldKind::Choice { options, allows_other: false } if options.len() == 2
        ));
        assert!(matches!(
            kinds[4].1,
            InputFieldKind::MultiChoice { options, allows_other: false } if options[1].value == "y"
        ));
        assert!(request.fields.iter().all(|field| field.is_required));
        assert_eq!(request.fields[0].title, "제목");
    }

    #[test]
    fn elicitation_form_reads_titled_options() {
        let params = json!({
            "mode": "form", "message": "m",
            "requestedSchema": { "type": "object", "properties": {
                "color": { "type": "string", "oneOf": [{ "const": "r", "title": "Red" }, { "const": "g", "title": "Green" }] },
                "size": { "type": "string", "enum": ["s", "l"], "enumNames": ["Small", "Large"] },
                "free": { "type": "number" }
            } }
        });

        let request = elicitation_request(&params).unwrap();

        let ids: Vec<&str> = request.fields.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["color", "free", "size"]);
        assert!(request.fields.iter().all(|field| !field.is_required));
        assert!(matches!(
            &request.fields[0].kind,
            InputFieldKind::Choice { options, .. } if options[0].label == "Red" && options[0].value == "r"
        ));
        assert_eq!(request.fields[1].kind, InputFieldKind::Number);
        assert!(matches!(
            &request.fields[2].kind,
            InputFieldKind::Choice { options, .. } if options[1].label == "Large" && options[1].value == "l"
        ));
    }

    #[test]
    fn elicitation_url_mode_carries_the_link_only() {
        let params = json!({
            "mode": "url", "message": "URL을 열어라",
            "url": "https://example.invalid/experiment-252", "elicitationId": "experiment-252-url"
        });

        let request = elicitation_request(&params).unwrap();

        assert_eq!(
            request.url.as_deref(),
            Some("https://example.invalid/experiment-252")
        );
        assert_eq!(request.message, "URL을 열어라");
        assert!(request.fields.is_empty());
        assert_eq!(elicitation_request(&json!({ "mode": "other" })), None);
    }

    #[test]
    fn elicitation_answers_use_accept_content_decline_and_cancel() {
        let request = elicitation_request(&measured_form()).unwrap();
        let submit = InputAnswer::Submit {
            values: vec![
                ("title".to_owned(), InputValue::Text("Saturn".to_owned())),
                ("count".to_owned(), InputValue::Integer(3)),
                ("enabled".to_owned(), InputValue::Boolean(true)),
                (
                    "choice".to_owned(),
                    InputValue::Selected(vec!["a".to_owned()]),
                ),
                (
                    "tags".to_owned(),
                    InputValue::Selected(vec!["x".to_owned(), "y".to_owned()]),
                ),
            ],
        };

        assert_eq!(
            result(InputKind::Elicitation, &request, &submit),
            json!({ "action": "accept", "content": {
                "title": "Saturn", "count": 3, "enabled": true, "choice": "a", "tags": ["x", "y"]
            } })
        );
        assert_eq!(
            result(InputKind::Elicitation, &request, &InputAnswer::Decline),
            json!({ "action": "decline" })
        );
        assert_eq!(
            result(InputKind::Elicitation, &request, &InputAnswer::Cancel),
            json!({ "action": "cancel" })
        );
        let url = InputRequest {
            message: String::new(),
            fields: Vec::new(),
            url: Some("https://example.invalid".to_owned()),
        };
        assert_eq!(
            result(
                InputKind::Elicitation,
                &url,
                &InputAnswer::Submit { values: Vec::new() }
            ),
            json!({ "action": "accept" })
        );
    }

    /// 실측 원문의 질문 요청.
    fn measured_questions() -> Value {
        json!({
            "threadId": "t", "turnId": "u", "itemId": "call_1", "isBlocking": false, "autoResolutionMs": null,
            "questions": [{
                "id": "preference", "header": "선호 확인",
                "question": "지금 가장 우선해서 도와드릴 작업은 무엇인가요?",
                "isOther": true, "isSecret": false,
                "options": [
                    { "label": "코드 변경 (Recommended)", "description": "저장소의 구현이나 버그를 수정합니다." },
                    { "label": "설계 검토", "description": "문서와 구조를 검토합니다." }
                ]
            }, { "id": "token", "header": "토큰", "question": "토큰은?", "isOther": false, "isSecret": true, "options": null }]
        })
    }

    #[test]
    fn elicitation_user_input_questions_become_fields() {
        let request = user_input_request(&measured_questions());

        assert_eq!(request.fields.len(), 2);
        assert_eq!(request.fields[0].id, "preference");
        assert_eq!(request.fields[0].description, "선호 확인");
        assert!(matches!(
            &request.fields[0].kind,
            InputFieldKind::Choice { options, allows_other: true }
                if options[0].value == "코드 변경 (Recommended)" && options[1].description.starts_with("문서와")
        ));
        assert_eq!(request.fields[1].kind, InputFieldKind::Text);
        assert!(request.fields[1].is_secret);
        assert!(!request.fields[0].is_secret);
    }

    #[test]
    fn elicitation_user_input_answers_are_keyed_by_question_id() {
        let request = user_input_request(&measured_questions());
        let submit = InputAnswer::Submit {
            values: vec![(
                "preference".to_owned(),
                InputValue::Selected(vec!["설계 검토".to_owned()]),
            )],
        };

        assert_eq!(
            result(InputKind::UserInput, &request, &submit),
            json!({ "answers": { "preference": { "answers": ["설계 검토"] } } })
        );
        assert_eq!(
            result(InputKind::UserInput, &request, &InputAnswer::Cancel),
            json!({ "answers": {} })
        );
    }
}
