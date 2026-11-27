//! JSON-RPC 2.0 as MCP uses it (`docs/steps/30/answered.md`, point 0; the
//! specification's base protocol).
//!
//! A message is one JSON object: a request (a method and an id), a
//! notification (a method and no id), or a response (an id and a result or
//! an error). An id is a string or an integer, never null. An error with no
//! id is a server's answer to something it could not read, and is accepted
//! as a message of its own. A `2025-03-26` server may also send a batch, an
//! array of messages; the transport takes one only from such a server.
//!
//! Messages are read as [`Json`] values by hand, with no derived types, so
//! that unknown fields are kept where they matter (a tool's definition) and
//! cost nothing where they do not.

use cineris_tools::Json;
use serde_json::Map;

/// The id of a request the server sent, as it sent it, so that the answer
/// carries it back exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RequestId {
    /// An integer.
    Number(i64),
    /// A string.
    Text(String),
}

impl RequestId {
    /// The id a value is, if it is one: a string, or an integer. Null, a
    /// fraction and anything else are not ids.
    #[must_use]
    pub(crate) fn from_json(value: &Json) -> Option<Self> {
        match value {
            Json::String(text) => Some(Self::Text(text.clone())),
            Json::Number(number) => number.as_i64().map(Self::Number),
            _ => None,
        }
    }

    /// The id as JSON.
    #[must_use]
    pub(crate) fn to_json(&self) -> Json {
        match self {
            Self::Number(number) => Json::from(*number),
            Self::Text(text) => Json::from(text.as_str()),
        }
    }

    /// The id of one of our requests this is, if it can be: ours are
    /// integers from 1, and a server that echoes one as a string of its
    /// digits is understood.
    #[must_use]
    pub(crate) fn ours(&self) -> Option<u64> {
        match self {
            Self::Number(number) => u64::try_from(*number).ok(),
            Self::Text(text) => {
                let number: u64 = text.parse().ok()?;
                // Only its plain spelling: "007" and "+7" are other ids.
                (number.to_string() == *text).then_some(number)
            }
        }
    }
}

/// An error a peer answered with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RpcError {
    /// Its code.
    pub(crate) code: i64,
    /// Its message: the peer's text.
    pub(crate) message: String,
    /// What else it gave.
    pub(crate) data: Option<Json>,
}

/// The answer to a request: its result, or the error it was answered with.
pub(crate) type Answer = Result<Json, RpcError>;

/// One message read from a server.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Message {
    /// A request the server sent to the client.
    Request {
        /// Its id, to answer with.
        id: RequestId,
        /// Its method.
        method: String,
        /// Its parameters, if it gave any.
        params: Option<Json>,
    },
    /// A notification.
    Notification {
        /// Its method.
        method: String,
        /// Its parameters, if it gave any.
        params: Option<Json>,
    },
    /// The answer to a request, ours or not.
    Response {
        /// The id it answers.
        id: RequestId,
        /// What it answered.
        answer: Answer,
    },
    /// An error with no id: the server could not read something it was
    /// sent, and did not say what.
    Unaddressed(RpcError),
}

/// What one line of a server's output held.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decoded {
    /// One message.
    One(Message),
    /// An array of values: a batch, each one a message or not.
    Batch(Vec<Option<Message>>),
    /// Not JSON, or JSON that is not a message.
    Malformed,
}

/// Reads one line of a server's output.
#[must_use]
pub(crate) fn decode(line: &[u8]) -> Decoded {
    let Ok(value) = serde_json::from_slice::<Json>(line) else {
        return Decoded::Malformed;
    };
    match value {
        Json::Array(values) => Decoded::Batch(values.into_iter().map(message).collect()),
        value => message(value).map_or(Decoded::Malformed, Decoded::One),
    }
}

/// The message a value is, if it is one.
#[must_use]
pub(crate) fn message(value: Json) -> Option<Message> {
    let Json::Object(mut object) = value else {
        return None;
    };
    let params = object.remove("params");
    if let Some(method) = object.get("method") {
        let method = method.as_str()?.to_owned();
        return match object.get("id") {
            None | Some(Json::Null) => Some(Message::Notification { method, params }),
            Some(id) => Some(Message::Request {
                id: RequestId::from_json(id)?,
                method,
                params,
            }),
        };
    }
    let answer = match (object.remove("result"), object.remove("error")) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(rpc_error(error)?),
        _ => return None,
    };
    match object.get("id") {
        None | Some(Json::Null) => match answer {
            Err(error) => Some(Message::Unaddressed(error)),
            Ok(_) => None,
        },
        Some(id) => Some(Message::Response {
            id: RequestId::from_json(id)?,
            answer,
        }),
    }
}

/// An error object, if the value is one. Its code must be an integer; a
/// missing message is read as empty.
fn rpc_error(value: Json) -> Option<RpcError> {
    let Json::Object(mut object) = value else {
        return None;
    };
    let code = object.get("code")?.as_i64()?;
    let message = match object.remove("message") {
        Some(Json::String(message)) => message,
        _ => String::new(),
    };
    Some(RpcError {
        code,
        message,
        data: object.remove("data"),
    })
}

/// A request of ours.
#[must_use]
pub(crate) fn request(id: u64, method: &str, params: Json) -> Json {
    let mut object = envelope();
    object.insert("id".to_owned(), Json::from(id));
    object.insert("method".to_owned(), Json::from(method));
    object.insert("params".to_owned(), params);
    Json::Object(object)
}

/// A notification of ours.
#[must_use]
pub(crate) fn notification(method: &str, params: Option<Json>) -> Json {
    let mut object = envelope();
    object.insert("method".to_owned(), Json::from(method));
    if let Some(params) = params {
        object.insert("params".to_owned(), params);
    }
    Json::Object(object)
}

/// The result we answer a server's request with.
#[must_use]
pub(crate) fn result(id: &RequestId, result: Json) -> Json {
    let mut object = envelope();
    object.insert("id".to_owned(), id.to_json());
    object.insert("result".to_owned(), result);
    Json::Object(object)
}

/// The error we answer a server's request with.
#[must_use]
pub(crate) fn error(id: &RequestId, code: i64, message: &str) -> Json {
    let mut object = envelope();
    object.insert("id".to_owned(), id.to_json());
    let mut error = Map::new();
    error.insert("code".to_owned(), Json::from(code));
    error.insert("message".to_owned(), Json::from(message));
    object.insert("error".to_owned(), Json::Object(error));
    Json::Object(object)
}

fn envelope() -> Map<String, Json> {
    let mut object = Map::new();
    object.insert("jsonrpc".to_owned(), Json::from("2.0"));
    object
}

/// The method a server does not have.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn one(text: &str) -> Message {
        match decode(text.as_bytes()) {
            Decoded::One(message) => message,
            other => panic!("not one message: {other:?}"),
        }
    }

    #[test]
    fn every_kind_of_message_is_read() {
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","id":"a","method":"ping"}"#),
            Message::Request {
                id: RequestId::Text("a".to_owned()),
                method: "ping".to_owned(),
                params: None,
            }
        );
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","method":"notifications/x","params":{"k":1}}"#),
            Message::Notification {
                method: "notifications/x".to_owned(),
                params: Some(json!({"k": 1})),
            }
        );
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#),
            Message::Response {
                id: RequestId::Number(7),
                answer: Ok(json!({"ok": true})),
            }
        );
        assert_eq!(
            one(r#"{"jsonrpc":"2.0","id":7,"error":{"code":-32601,"message":"no","data":[1]}}"#),
            Message::Response {
                id: RequestId::Number(7),
                answer: Err(RpcError {
                    code: -32601,
                    message: "no".to_owned(),
                    data: Some(json!([1])),
                }),
            }
        );
    }

    #[test]
    fn an_error_with_no_id_is_a_message_of_its_own() {
        for text in [
            r#"{"jsonrpc":"2.0","error":{"code":-32700,"message":"parse error"}}"#,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#,
        ] {
            assert_eq!(
                one(text),
                Message::Unaddressed(RpcError {
                    code: -32700,
                    message: "parse error".to_owned(),
                    data: None,
                }),
                "{text}"
            );
        }
    }

    #[test]
    fn a_null_or_fractional_id_and_a_result_with_no_id_are_not_messages() {
        for text in [
            r#"{"jsonrpc":"2.0","result":{}}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"result":{}}"#,
            r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":""}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":"x","message":""}}"#,
            r#"{"jsonrpc":"2.0","method":7}"#,
            r#"[1,2"#,
            "not json",
            "42",
        ] {
            assert_eq!(decode(text.as_bytes()), Decoded::Malformed, "{text}");
        }
    }

    #[test]
    fn an_array_is_a_batch_of_what_it_holds() {
        let Decoded::Batch(messages) =
            decode(br#"[{"jsonrpc":"2.0","id":1,"result":{}},5,{"jsonrpc":"2.0","method":"m"}]"#)
        else {
            panic!("not a batch");
        };
        assert_eq!(messages.len(), 3);
        assert!(matches!(messages[0], Some(Message::Response { .. })));
        assert_eq!(messages[1], None);
        assert!(matches!(messages[2], Some(Message::Notification { .. })));
    }

    #[test]
    fn our_ids_are_matched_as_numbers_or_their_digits() {
        assert_eq!(RequestId::Number(4).ours(), Some(4));
        assert_eq!(RequestId::Text("4".to_owned()).ours(), Some(4));
        assert_eq!(RequestId::Text("04".to_owned()).ours(), None);
        assert_eq!(RequestId::Text("x".to_owned()).ours(), None);
        assert_eq!(RequestId::Number(-1).ours(), None);
    }

    #[test]
    fn an_answer_carries_the_id_back_as_it_came() {
        let id = RequestId::Text("ping-1".to_owned());
        assert_eq!(
            result(&id, json!({})),
            json!({"jsonrpc": "2.0", "id": "ping-1", "result": {}})
        );
        assert_eq!(
            error(&RequestId::Number(3), METHOD_NOT_FOUND, "no"),
            json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32601, "message": "no"}})
        );
        assert_eq!(
            notification("notifications/initialized", None),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        );
    }
}
