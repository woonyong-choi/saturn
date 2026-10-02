//! provider가 사용자에게 묻는 입력 요청과 답. provider 고유 형식은 engine이 이 모양으로 옮긴다.
//! 설계: docs/design/permissions.md#입력-요청

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 질문 목록이나 폼, 또는 링크 하나. `url`이 있으면 `fields`는 비어 있다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct InputRequest {
    /// 요청 전체를 설명하는 글. 없으면 비어 있다.
    pub message: String,
    pub fields: Vec<InputField>,
    /// 링크만 보이고 Saturn은 열지 않는다. 사용자가 직접 연다.
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct InputField {
    /// 답에서 이 칸을 가리키는 값. provider가 정한다.
    pub id: String,
    /// 칸 이름이나 질문 글.
    pub title: String,
    /// 보충 설명. 없으면 비어 있다.
    pub description: String,
    pub kind: InputFieldKind,
    /// 참이면 비운 채 보낼 수 없다.
    pub is_required: bool,
    /// 참이면 화면에 입력 글자를 가린다.
    pub is_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum InputFieldKind {
    Text,
    Integer,
    Number,
    Boolean,
    /// 하나만 고른다. `allows_other`가 참이면 목록에 없는 글도 쓸 수 있다.
    Choice {
        options: Vec<InputOption>,
        allows_other: bool,
    },
    /// 여럿을 고른다.
    MultiChoice {
        options: Vec<InputOption>,
        allows_other: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct InputOption {
    /// 답에 실리는 값.
    pub value: String,
    /// 화면에 보이는 이름.
    pub label: String,
    /// 보충 설명. 없으면 비어 있다.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum InputValue {
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    /// 고른 값. `Choice`는 하나, `MultiChoice`는 여럿.
    Selected(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum InputAnswer {
    /// 칸 `id`별 값. 비워 둔 선택 칸은 싣지 않는다. URL 요청은 `values`가 비어 있다.
    Submit { values: Vec<(String, InputValue)> },
    /// 묻는 내용에 답하지 않겠다.
    Decline,
    /// 요청을 없던 일로 한다.
    Cancel,
}
