//! JSON-RPC 2.0 봉투와 한 줄 코덱. 소켓 한 줄에 메시지 하나를 쓴다.
//!
//! 설계: docs/architecture.md(구성 요소 연결), docs/design/engine-lifecycle.md(TUI 접속).
//!
//! - 클라이언트 → engine: `ClientMessage`. `id`가 있는 요청만 쓴다.
//! - engine → 클라이언트: `ServerMessage`. 요청마다 `Response` 하나, 그 밖의 화면 갱신은 `id` 없는 알림이다.
//! - 해석 오류에는 입력 원문을 담지 않는다. `SubmitJudgeKey` 줄에는 judge 키가 들어 있다.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use crate::rpc::{Notification, Request};

/// 봉투의 `jsonrpc` 값.
const VERSION: &str = "2.0";

/// 표준 JSON-RPC 오류 코드: 줄이 JSON이 아니다.
pub const PARSE_ERROR: i32 = -32700;
/// 표준 JSON-RPC 오류 코드: JSON이지만 요청 봉투가 아니다.
pub const INVALID_REQUEST: i32 = -32600;
/// 표준 JSON-RPC 오류 코드: 모르는 메서드.
pub const METHOD_NOT_FOUND: i32 = -32601;
/// 표준 JSON-RPC 오류 코드: 메서드는 알지만 `params`가 맞지 않는다.
pub const INVALID_PARAMS: i32 = -32602;
/// 표준 JSON-RPC 오류 코드: engine 내부 오류.
pub const INTERNAL_ERROR: i32 = -32603;

/// 봉투 인코딩과 해석 오류. 메시지에 입력 원문을 담지 않는다.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// 메시지를 JSON으로 쓰지 못했다.
    #[error("failed to encode message")]
    Encode(#[source] serde_json::Error),
    /// 줄을 해석하지 못했다.
    #[error("failed to decode message at column {column}: {kind}")]
    Decode {
        /// 요청 봉투에서 읽어 낸 `id`. 오류 응답에 그대로 돌려준다. 읽지 못하면 `None`.
        id: Option<RequestId>,
        /// 실패 종류.
        kind: DecodeFailure,
        /// 실패한 칸(1부터).
        column: usize,
    },
}

impl CodecError {
    /// 해석 오류를 클라이언트에 돌려줄 오류 응답으로 바꾼다. 인코딩 오류는 내부 오류다.
    pub fn to_response(&self) -> Response {
        match self {
            Self::Encode(_) => Response::error(None, INTERNAL_ERROR, "failed to encode message"),
            Self::Decode { id, kind, .. } => Response::error(*id, kind.code(), kind.message()),
        }
    }

    /// `serde_json` 오류에서 원문 없이 종류와 위치만 옮긴다.
    fn decode(id: Option<RequestId>, error: &serde_json::Error) -> Self {
        let kind = match error.classify() {
            serde_json::error::Category::Io | serde_json::error::Category::Syntax => {
                DecodeFailure::Syntax
            }
            serde_json::error::Category::Eof => DecodeFailure::Truncated,
            serde_json::error::Category::Data => DecodeFailure::Shape,
        };
        Self::Decode {
            id,
            kind,
            column: error.column(),
        }
    }
}

/// 줄 해석 실패 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailure {
    /// JSON 문법이 틀렸다.
    Syntax,
    /// JSON이 중간에 끊겼다.
    Truncated,
    /// JSON이지만 메시지 형식이 아니다(모르는 메서드, 빠진 필드, 틀린 타입).
    Shape,
}

impl DecodeFailure {
    /// 오류 응답 코드. 메서드와 `params`를 구분하지 않고 형식 오류는 `INVALID_REQUEST`로 묶는다.
    pub fn code(self) -> i32 {
        match self {
            Self::Syntax | Self::Truncated => PARSE_ERROR,
            Self::Shape => INVALID_REQUEST,
        }
    }

    /// 오류 응답 문구.
    pub fn message(self) -> &'static str {
        match self {
            Self::Syntax => "parse error",
            Self::Truncated => "truncated message",
            Self::Shape => "invalid request",
        }
    }
}

impl std::fmt::Display for DecodeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

/// `jsonrpc` 필드. 항상 `"2.0"`이고 다른 값은 해석 오류다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, TS)]
#[ts(type = "\"2.0\"")]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(VERSION)
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == VERSION {
            Ok(Self)
        } else {
            Err(de::Error::invalid_value(
                de::Unexpected::Str(&value),
                &"\"2.0\"",
            ))
        }
    }
}

impl JsonSchema for JsonRpcVersion {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "JsonRpcVersion".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "const": VERSION })
    }
}

/// 요청 번호. 클라이언트가 연결마다 매기고 engine은 `Response`에 그대로 돌려준다.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct RequestId(pub u64);

/// 클라이언트 → engine 한 줄.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ClientMessage {
    /// 항상 `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// 요청 번호.
    pub id: RequestId,
    /// 메서드와 `params`.
    #[serde(flatten)]
    pub request: Request,
}

impl ClientMessage {
    /// 요청을 봉투에 담는다.
    pub fn new(id: RequestId, request: Request) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id,
            request,
        }
    }
}

/// engine → 클라이언트 한 줄. `method`가 있으면 알림, 없으면 응답이다.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ServerMessage {
    /// 요청 하나의 답.
    Response(Response),
    /// 화면 갱신 알림.
    Notification(NotificationMessage),
}

impl<'de> Deserialize<'de> for ServerMessage {
    /// `untagged` 자동 해석은 실패 원인을 잃는다. `method` 유무로 먼저 갈라 원인을 남긴다.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let is_notification = value.get("method").is_some();
        let message = if is_notification {
            NotificationMessage::deserialize(value).map(Self::Notification)
        } else {
            Response::deserialize(value).map(Self::Response)
        };
        message.map_err(de::Error::custom)
    }
}

/// 요청 하나의 답. 요청의 결과 화면 갱신은 따로 알림으로 온다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct Response {
    /// 항상 `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// 답하는 요청 번호. 줄을 해석하지 못해 번호를 모르면 `null`.
    pub id: Option<RequestId>,
    /// 성공이면 `result`, 실패면 `error`.
    #[serde(flatten)]
    pub outcome: Outcome,
}

impl Response {
    /// 성공 응답.
    pub fn ok(id: RequestId) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id: Some(id),
            outcome: Outcome::Ok(()),
        }
    }

    /// 오류 응답.
    pub fn error(id: Option<RequestId>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id,
            outcome: Outcome::Err(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// 응답 결과. 지금 요청은 모두 결과 값 없이 접수 여부만 돌려준다.
/// TODO(#46): 메서드 목록을 확정하면 조회 요청의 결과를 알림 대신 `result`로 돌려줄지 정한다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Outcome {
    /// 접수했다. `"result": null`.
    #[serde(rename = "result")]
    Ok(()),
    /// 거절했다.
    #[serde(rename = "error")]
    Err(RpcError),
}

/// JSON-RPC 오류 객체. `message`에 사용자 입력과 비밀값을 넣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct RpcError {
    /// 오류 코드. 표준 코드는 이 모듈의 상수.
    pub code: i32,
    /// 오류 문구.
    pub message: String,
}

/// engine이 보내는 알림 한 줄.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct NotificationMessage {
    /// 항상 `"2.0"`.
    pub jsonrpc: JsonRpcVersion,
    /// 메서드와 `params`.
    #[serde(flatten)]
    pub notification: Notification,
}

impl NotificationMessage {
    /// 알림을 봉투에 담는다.
    pub fn new(notification: Notification) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            notification,
        }
    }
}

impl From<Notification> for ServerMessage {
    fn from(notification: Notification) -> Self {
        Self::Notification(NotificationMessage::new(notification))
    }
}

impl From<Response> for ServerMessage {
    fn from(response: Response) -> Self {
        Self::Response(response)
    }
}

/// 메시지를 줄바꿈으로 끝나는 한 줄로 쓴다. JSON 문자열 안 줄바꿈은 이스케이프되어 줄이 나뉘지 않는다.
///
/// # Errors
/// 직렬화에 실패하면 `Encode`.
pub fn encode_line<T: Serialize>(message: &T) -> Result<String, CodecError> {
    let mut line = serde_json::to_string(message).map_err(CodecError::Encode)?;
    line.push('\n');
    Ok(line)
}

/// 클라이언트 한 줄을 해석한다. 실패해도 요청 `id`를 읽을 수 있으면 오류에 담는다.
///
/// # Errors
/// JSON이 아니거나 요청 봉투가 아니면 `Decode`.
pub fn decode_client_line(line: &str) -> Result<ClientMessage, CodecError> {
    serde_json::from_str(line).map_err(|error| CodecError::decode(request_id_of(line), &error))
}

/// engine 한 줄을 해석한다.
///
/// # Errors
/// JSON이 아니거나 응답, 알림 형식이 아니면 `Decode`.
pub fn decode_server_line(line: &str) -> Result<ServerMessage, CodecError> {
    decode_line(line)
}

/// 아무 메시지 한 줄을 해석한다. 앞뒤 공백과 줄바꿈은 무시한다.
///
/// # Errors
/// JSON이 아니거나 `T` 형식이 아니면 `Decode`.
pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, CodecError> {
    serde_json::from_str(line).map_err(|error| CodecError::decode(None, &error))
}

/// 해석에 실패한 요청 줄에서 `id`만 다시 읽는다.
fn request_id_of(line: &str) -> Option<RequestId> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.get("id")?.as_u64().map(RequestId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ChatId, InputId, TaskId, TaskLabel};
    use crate::state::TaskState;

    #[test]
    fn client_message_uses_method_and_params() {
        let message = ClientMessage::new(
            RequestId(7),
            Request::RenameChat {
                chat: ChatId(3),
                name: "로그인".into(),
            },
        );

        let line = encode_line(&message).unwrap();

        assert_eq!(
            line,
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"RenameChat\",\"params\":{\"chat\":3,\"name\":\"로그인\"}}\n"
        );
        assert_eq!(decode_client_line(&line).unwrap(), message);
    }

    #[test]
    fn unit_request_has_no_params() {
        let message = ClientMessage::new(RequestId(1), Request::Detach);

        let line = encode_line(&message).unwrap();

        assert_eq!(
            line,
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"Detach\"}\n"
        );
        assert_eq!(decode_client_line(&line).unwrap(), message);
    }

    #[test]
    fn text_with_newline_stays_on_one_line() {
        let message = ClientMessage::new(
            RequestId(2),
            Request::SubmitInput {
                chat: ChatId(1),
                client_ref: 4,
                text: "첫 줄\n둘째 줄".into(),
                pinned_model: None,
                skip_relation: false,
            },
        );

        let line = encode_line(&message).unwrap();

        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(decode_client_line(&line).unwrap(), message);
    }

    #[test]
    fn notification_round_trips_without_id() {
        let message = ServerMessage::from(Notification::TaskChanged {
            task: TaskId(5),
            label: TaskLabel('A'),
            state: TaskState::Running,
            provider: None,
            elapsed_ms: 1200,
            failure: None,
        });

        let line = encode_line(&message).unwrap();

        assert!(!line.contains("\"id\""));
        assert_eq!(decode_server_line(&line).unwrap(), message);
    }

    #[test]
    fn history_chunk_nests_notifications() {
        let message = ServerMessage::from(Notification::HistoryChunk {
            chat: ChatId(1),
            entries: vec![Notification::InputAccepted {
                client_ref: 1,
                input: InputId(9),
            }],
            has_more: false,
        });

        let line = encode_line(&message).unwrap();

        assert_eq!(decode_server_line(&line).unwrap(), message);
    }

    #[test]
    fn responses_round_trip() {
        let ok = ServerMessage::from(Response::ok(RequestId(3)));
        let error = ServerMessage::from(Response::error(None, PARSE_ERROR, "parse error"));

        let ok_line = encode_line(&ok).unwrap();
        let error_line = encode_line(&error).unwrap();

        assert_eq!(ok_line, "{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":null}\n");
        assert_eq!(
            error_line,
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32700,\"message\":\"parse error\"}}\n"
        );
        assert_eq!(decode_server_line(&ok_line).unwrap(), ok);
        assert_eq!(decode_server_line(&error_line).unwrap(), error);
    }

    #[test]
    fn wrong_version_is_rejected() {
        let line = "{\"jsonrpc\":\"1.0\",\"id\":1,\"method\":\"Detach\"}";

        let error = decode_client_line(line).unwrap_err();

        assert!(matches!(
            error,
            CodecError::Decode {
                id: Some(RequestId(1)),
                kind: DecodeFailure::Shape,
                ..
            }
        ));
    }

    #[test]
    fn unknown_method_keeps_request_id() {
        let line = "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"Nope\"}";

        let response = decode_client_line(line).unwrap_err().to_response();

        assert_eq!(response.id, Some(RequestId(11)));
        assert_eq!(
            response.outcome,
            Outcome::Err(RpcError {
                code: INVALID_REQUEST,
                message: "invalid request".into(),
            })
        );
    }

    #[test]
    fn broken_json_is_parse_error() {
        let response = decode_client_line("{\"jsonrpc\":")
            .unwrap_err()
            .to_response();

        assert_eq!(response.id, None);
        assert!(matches!(
            response.outcome,
            Outcome::Err(RpcError {
                code: PARSE_ERROR,
                ..
            })
        ));
    }

    #[test]
    fn decode_error_hides_judge_key() {
        let line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"SubmitJudgeKey\",\"params\":{\"key\":\"sk-secret\",\"key\":1}}";
        let mistyped = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"Usage\",\"params\":{\"scope\":\"sk-secret\"}}";

        for input in [line, mistyped] {
            let error = decode_client_line(input).unwrap_err();
            let shown = format!("{error} {error:?} {:?}", error.to_response());

            assert!(!shown.contains("sk-secret"), "{shown}");
        }
    }
}
