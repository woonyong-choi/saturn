//! JSON-RPC 2.0 봉투와 한 줄 코덱. 설계: docs/design/engine-lifecycle.md
//! 해석 오류에는 입력 원문을 담지 않는다. `SubmitRouterKey` 줄에 router 키가 있다.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

use crate::rpc::{Notification, Request};

const VERSION: &str = "2.0";

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("failed to encode message")]
    Encode(#[source] serde_json::Error),
    #[error("failed to decode message at column {column}: {kind}")]
    Decode {
        /// 읽지 못하면 `None`.
        id: Option<RequestId>,
        kind: DecodeFailure,
        /// 1부터.
        column: usize,
    },
}

impl CodecError {
    pub fn to_response(&self) -> Response {
        match self {
            Self::Encode(_) => Response::error(None, INTERNAL_ERROR, "failed to encode message"),
            Self::Decode { id, kind, .. } => Response::error(*id, kind.code(), kind.message()),
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeFailure {
    Syntax,
    Truncated,
    /// 모르는 메서드, 빠진 필드, 틀린 타입.
    Shape,
}

impl DecodeFailure {
    /// 메서드와 `params` 오류를 구분하지 않는다.
    pub fn code(self) -> i32 {
        match self {
            Self::Syntax | Self::Truncated => PARSE_ERROR,
            Self::Shape => INVALID_REQUEST,
        }
    }

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

/// 항상 `"2.0"`.
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

/// 클라이언트가 연결마다 매긴다.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, TS,
)]
pub struct RequestId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct ClientMessage {
    pub jsonrpc: JsonRpcVersion,
    pub id: RequestId,
    #[serde(flatten)]
    pub request: Request,
}

impl ClientMessage {
    pub fn new(id: RequestId, request: Request) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id,
            request,
        }
    }
}

/// `method`가 있으면 알림, 없으면 응답.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum ServerMessage {
    Response(Response),
    Notification(NotificationMessage),
}

impl<'de> Deserialize<'de> for ServerMessage {
    /// `untagged` 자동 해석은 실패 원인을 잃어 `method` 유무로 먼저 가른다.
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

/// 요청 결과의 화면 갱신은 알림으로 따로 온다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    /// 줄을 해석하지 못해 번호를 모르면 `null`.
    pub id: Option<RequestId>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

impl Response {
    pub fn ok(id: RequestId) -> Self {
        Self {
            jsonrpc: JsonRpcVersion,
            id: Some(id),
            outcome: Outcome::Ok(()),
        }
    }

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

/// TODO(#177): 조회 결과를 알림 대신 `result`로 돌려줄지
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum Outcome {
    #[serde(rename = "result")]
    Ok(()),
    #[serde(rename = "error")]
    Err(RpcError),
}

/// `message`에 사용자 입력과 비밀값을 넣지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub struct NotificationMessage {
    pub jsonrpc: JsonRpcVersion,
    #[serde(flatten)]
    pub notification: Notification,
}

impl NotificationMessage {
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

/// JSON 문자열 안 줄바꿈은 이스케이프되어 한 줄을 유지한다.
pub fn encode_line<T: Serialize>(message: &T) -> Result<String, CodecError> {
    let mut line = serde_json::to_string(message).map_err(CodecError::Encode)?;
    line.push('\n');
    Ok(line)
}

/// 실패해도 요청 `id`를 읽을 수 있으면 오류에 담는다.
pub fn decode_client_line(line: &str) -> Result<ClientMessage, CodecError> {
    serde_json::from_str(line).map_err(|error| CodecError::decode(request_id_of(line), &error))
}

pub fn decode_server_line(line: &str) -> Result<ServerMessage, CodecError> {
    decode_line(line)
}

pub fn decode_line<T: DeserializeOwned>(line: &str) -> Result<T, CodecError> {
    serde_json::from_str(line).map_err(|error| CodecError::decode(None, &error))
}

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
    fn decode_error_hides_router_key() {
        let line = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"SubmitRouterKey\",\"params\":{\"key\":\"sk-secret\",\"key\":1}}";
        let mistyped = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"Usage\",\"params\":{\"scope\":\"sk-secret\"}}";

        for input in [line, mistyped] {
            let error = decode_client_line(input).unwrap_err();
            let shown = format!("{error} {error:?} {:?}", error.to_response());

            assert!(!shown.contains("sk-secret"), "{shown}");
        }
    }

    #[test]
    fn request_debug_hides_router_key() {
        let message = ClientMessage::new(
            RequestId(1),
            Request::SubmitRouterKey {
                key: "sk-secret".into(),
            },
        );

        assert!(!format!("{message:?} {:?}", message.request).contains("sk-secret"));
    }
}
