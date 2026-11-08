//! JSON-RPC 2.0 envelopes and errors with a stable `data.kind`.
//!
//! The protocol takes params by name only and has no batches.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request id. A request without an id is a notification and gets no reply.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
}

/// The `"jsonrpc": "2.0"` member. Any other value fails to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct V2;

impl Serialize for V2 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for V2 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<V2, D::Error> {
        match String::deserialize(d)?.as_str() {
            "2.0" => Ok(V2),
            other => Err(serde::de::Error::custom(format!(
                "jsonrpc must be \"2.0\", not {other:?}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: V2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Result(Value),
    Error(RpcError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: V2,
    /// `null` when the core could not read the id of the request.
    pub id: Option<Id>,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// A message from the core with no id: an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: V2,
    pub method: String,
    pub params: Value,
}

impl Notification {
    /// The notification of event `E`.
    pub fn of<E: crate::Event>(params: &E::Params) -> Notification {
        Notification {
            jsonrpc: V2,
            method: E::NAME.into(),
            params: serde_json::to_value(params).unwrap_or_default(),
        }
    }

    /// The params as the type of event `E`, or `None` for another event.
    pub fn parse<E: crate::Event>(&self) -> Option<E::Params> {
        if self.method != E::NAME {
            return None;
        }
        serde_json::from_value(self.params.clone()).ok()
    }
}

/// Each message that a client can get from the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Response(Response),
    Notification(Notification),
}

/// The stable kind of an error, for code. `message` and `hint` are for a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    NotFound,
    Invalid,
    Conflict,
    Busy,
    Unavailable,
    Internal,
    /// A `data.clear` whose plan is old or no longer matches the data.
    PlanChanged,
}

/// The JSON-RPC 2.0 error codes that the protocol uses.
pub mod code {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
    /// An error of the application.
    pub const APP_ERROR: i64 = -32000;
}

impl ErrorKind {
    /// The code of an error of this kind. Only a parse error, a bad request
    /// and an unknown method use another code.
    pub fn code(self) -> i64 {
        match self {
            ErrorKind::Invalid => code::INVALID_PARAMS,
            ErrorKind::Internal => code::INTERNAL_ERROR,
            _ => code::APP_ERROR,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorData {
    pub kind: ErrorKind,
    /// One sentence for a person: what to do next.
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: ErrorData,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({:?}): {}",
            self.message, self.data.kind, self.data.hint
        )
    }
}

impl std::error::Error for RpcError {}

impl RpcError {
    pub fn new(kind: ErrorKind, message: impl Into<String>, hint: impl Into<String>) -> RpcError {
        RpcError {
            code: kind.code(),
            message: message.into(),
            data: ErrorData {
                kind,
                hint: hint.into(),
            },
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.data.kind
    }

    fn with_code(mut self, code: i64) -> RpcError {
        self.code = code;
        self
    }

    pub fn parse_error(message: impl Into<String>) -> RpcError {
        RpcError::new(
            ErrorKind::Invalid,
            message,
            "Send one JSON-RPC 2.0 object on each line.",
        )
        .with_code(code::PARSE_ERROR)
    }

    pub fn invalid_request(message: impl Into<String>) -> RpcError {
        RpcError::new(
            ErrorKind::Invalid,
            message,
            "Send jsonrpc \"2.0\", a method and params by name.",
        )
        .with_code(code::INVALID_REQUEST)
    }

    pub fn unknown_method(method: &str) -> RpcError {
        RpcError::new(
            ErrorKind::NotFound,
            format!("unknown method {method:?}"),
            "Call rpc.discover for the list of methods.",
        )
        .with_code(code::METHOD_NOT_FOUND)
    }

    pub fn invalid_params(message: impl Into<String>) -> RpcError {
        RpcError::new(
            ErrorKind::Invalid,
            message,
            "Call rpc.discover for the params of this method.",
        )
        .with_code(code::INVALID_PARAMS)
    }

    pub fn internal(message: impl Into<String>) -> RpcError {
        RpcError::new(
            ErrorKind::Internal,
            message,
            "Read the core log next to the socket.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_response_has_either_a_result_or_an_error() {
        let ok = Response {
            jsonrpc: V2,
            id: Some(Id::Number(1)),
            outcome: Outcome::Result(json!({"a": 1})),
        };
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"jsonrpc": "2.0", "id": 1, "result": {"a": 1}})
        );
        let err = Response {
            jsonrpc: V2,
            id: None,
            outcome: Outcome::Error(RpcError::unknown_method("x.y")),
        };
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["id"], Value::Null);
        assert_eq!(v["error"]["code"], -32601);
        assert_eq!(v["error"]["data"]["kind"], "not_found");
        let back: Incoming = serde_json::from_value(v).unwrap();
        assert!(matches!(
            back,
            Incoming::Response(Response {
                outcome: Outcome::Error(_),
                ..
            })
        ));
    }

    #[test]
    fn a_notification_is_told_apart_from_a_response() {
        let line = r#"{"jsonrpc":"2.0","method":"terminal.exit","params":{"id":"t1","code":0}}"#;
        assert!(matches!(
            serde_json::from_str::<Incoming>(line).unwrap(),
            Incoming::Notification(_)
        ));
    }

    #[test]
    fn only_version_2_0_parses() {
        assert!(serde_json::from_str::<Request>(r#"{"jsonrpc":"1.0","method":"a"}"#).is_err());
        let r: Request = serde_json::from_str(r#"{"jsonrpc":"2.0","method":"a"}"#).unwrap();
        assert_eq!((r.id, r.params), (None, None));
    }
}
