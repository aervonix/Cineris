//! The protocol's types as lines of JSON-RPC, both ways
//! (`docs/architecture/11-rpc-protocol.md`, "Messages").
//!
//! Reading: a line is a request, a response to a request the server put, a
//! notification (none is defined from a client, and each is ignored), or a
//! line that is none of these, answered with an error that names what is
//! wrong. Writing: an event is a `session/event` notification, a request to
//! the client is a request under the core's id, and a result is a response.
//!
//! The method tables are `cortex-protocol`'s (`Op::from_wire`,
//! `ServerResponse::from_wire`), so the server adds no mapping of its own.

use cortex_base::framing::{self, FrameError};
use cortex_protocol::jsonrpc::{self, ErrorObject, Message, Request, RequestId, Response};
use cortex_protocol::methods::{OpResult, SESSION_EVENT, ServerRequest};
use cortex_protocol::{ErrorData, ErrorKind, SessionEvent};
use serde::Serialize;
use serde_json::Value;

/// What a line from the client is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    /// A request: an operation, by its method name and parameters, read
    /// later against the method table.
    Request(Request),
    /// The client's answer to a request the server put to it.
    Response(Response),
    /// A notification. None is defined from a client: it is ignored, as an
    /// unknown notification is.
    Notification,
    /// Not a message: it is answered with this error, under the line's id
    /// when one could be read.
    Bad {
        /// The line's id, if it had one that can be read.
        id: Option<RequestId>,
        /// What is wrong.
        error: ErrorObject,
    },
}

/// Reads one line, without its line ending.
pub(crate) fn read(line: &[u8]) -> Incoming {
    match framing::decode::<Message>(line) {
        Ok(Message::Request(request)) => Incoming::Request(request),
        Ok(Message::Response(response)) => Incoming::Response(response),
        Ok(Message::Notification(_)) => Incoming::Notification,
        Err(error) => {
            // JSON, but not a message: its id is still the client's way to
            // match the error to what it sent.
            let id = matches!(error, FrameError::InvalidMessage(_))
                .then(|| serde_json::from_slice::<Value>(line).ok())
                .flatten()
                .and_then(|value| value.get("id").cloned())
                .and_then(|id| serde_json::from_value::<RequestId>(id).ok());
            Incoming::Bad {
                id,
                error: error.to_error_object(),
            }
        }
    }
}

/// The `session/event` notification, written straight from the event: the
/// line every delta becomes, so it is not built as a value first.
#[derive(Serialize)]
struct EventLine<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: &'a SessionEvent,
}

/// An event as its line.
pub(crate) fn event(event: &SessionEvent, out: &mut Vec<u8>) -> Result<(), FrameError> {
    framing::encode(
        &EventLine {
            jsonrpc: jsonrpc::JSONRPC_VERSION,
            method: SESSION_EVENT,
            params: event,
        },
        out,
    )
}

/// A request to the client as its line, under the core's id.
pub(crate) fn request(
    id: RequestId,
    request: &ServerRequest,
    out: &mut Vec<u8>,
) -> Result<(), FrameError> {
    let request = request
        .to_request(id)
        .map_err(|error| FrameError::Encode(error.to_string()))?;
    framing::encode(&Message::Request(request), out)
}

/// The response to a client's request.
pub(crate) fn result(
    id: RequestId,
    result: Result<OpResult, ErrorData>,
    out: &mut Vec<u8>,
) -> Result<(), FrameError> {
    let result = match result {
        Ok(result) => match result.result() {
            Ok(value) => Ok(value),
            // The protocol's own types always write; said as the server's
            // failure if one ever does not.
            Err(error) => Err(ErrorObject {
                code: jsonrpc::code::INTERNAL_ERROR,
                message: format!("the result could not be written: {error}"),
                data: None,
            }),
        },
        Err(error) => Err(ErrorObject::application(&error)),
    };
    framing::encode(
        &Message::Response(Response {
            id: Some(id),
            result,
        }),
        out,
    )
}

/// An error response, under `id` or under none.
pub(crate) fn error(
    id: Option<RequestId>,
    error: ErrorObject,
    out: &mut Vec<u8>,
) -> Result<(), FrameError> {
    framing::encode(
        &Message::Response(Response {
            id,
            result: Err(error),
        }),
        out,
    )
}

/// An application error (`-32000`) with the protocol's error object.
pub(crate) fn application(kind: ErrorKind, code: &str, message: impl Into<String>) -> ErrorObject {
    ErrorObject::application(&ErrorData::new(kind, code, message))
}

#[cfg(test)]
mod tests {
    use cortex_protocol::SessionId;
    use cortex_protocol::event::{Event, ItemRef};
    use cortex_protocol::methods::{Empty, QuestionRequest};

    use super::*;

    fn line(text: &str) -> Incoming {
        read(text.as_bytes())
    }

    #[test]
    fn each_kind_of_line_is_told_apart() {
        assert!(matches!(
            line(r#"{"jsonrpc":"2.0","id":1,"method":"models/list"}"#),
            Incoming::Request(Request {
                id: RequestId::Number(1),
                ..
            })
        ));
        assert!(matches!(
            line(r#"{"jsonrpc":"2.0","id":"a","result":{}}"#),
            Incoming::Response(_)
        ));
        assert_eq!(
            line(r#"{"jsonrpc":"2.0","method":"x/whatever"}"#),
            Incoming::Notification
        );
    }

    #[test]
    fn a_line_that_is_not_json_is_a_parse_error_with_no_id() {
        let Incoming::Bad { id, error } = line("{not json") else {
            panic!("read as a message");
        };
        assert_eq!(id, None);
        assert_eq!(error.code, jsonrpc::code::PARSE_ERROR);
    }

    /// JSON that is not a message keeps its id, so the client can match
    /// the error to what it sent.
    #[test]
    fn json_that_is_not_a_message_is_answered_under_its_id() {
        let Incoming::Bad { id, error } = line(r#"{"jsonrpc":"1.0","id":7,"method":"shutdown"}"#)
        else {
            panic!("read as a message");
        };
        assert_eq!(id, Some(RequestId::Number(7)));
        assert_eq!(error.code, jsonrpc::code::INVALID_REQUEST);
        let Incoming::Bad { id, .. } = line(r#"[1,2]"#) else {
            panic!("read as a message");
        };
        assert_eq!(id, None);
    }

    #[test]
    fn an_event_is_one_line_that_reads_back_as_the_same_notification() {
        let event = SessionEvent {
            session_id: SessionId::new("s_1"),
            seq: 4,
            turn_id: None,
            event: Event::ItemResync(ItemRef {
                item_id: cortex_protocol::ItemId::new("i_1"),
            }),
        };
        let mut out = Vec::new();
        super::event(&event, &mut out).unwrap();
        assert_eq!(out.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert!(out.ends_with(b"\n"));
        let Ok(Message::Notification(notification)) =
            framing::decode::<Message>(&out[..out.len() - 1])
        else {
            panic!("not a notification");
        };
        let read = cortex_protocol::methods::ServerNotification::from_wire(notification).unwrap();
        assert_eq!(
            read,
            cortex_protocol::methods::ServerNotification::Event(Box::new(event))
        );
    }

    #[test]
    fn a_request_and_a_result_carry_their_ids() {
        let mut out = Vec::new();
        let asked = ServerRequest::Question(QuestionRequest {
            session_id: SessionId::new("s_1"),
            turn_id: None,
            question: "Which?".to_owned(),
            choices: Vec::new(),
            extension: None,
        });
        request(RequestId::Number(9), &asked, &mut out).unwrap();
        let Ok(Message::Request(sent)) = framing::decode::<Message>(&out[..out.len() - 1]) else {
            panic!("not a request");
        };
        assert_eq!(sent.id, RequestId::Number(9));
        assert_eq!(sent.method, "question/ask");

        out.clear();
        result(
            RequestId::String("x".to_owned()),
            Ok(OpResult::Shutdown(Empty {})),
            &mut out,
        )
        .unwrap();
        assert_eq!(
            std::str::from_utf8(&out).unwrap(),
            "{\"jsonrpc\":\"2.0\",\"id\":\"x\",\"result\":{}}\n"
        );

        out.clear();
        result(
            RequestId::Number(2),
            Err(ErrorData::new(
                ErrorKind::Usage,
                "turn_running",
                "A turn runs.",
            )),
            &mut out,
        )
        .unwrap();
        let Ok(Message::Response(response)) = framing::decode::<Message>(&out[..out.len() - 1])
        else {
            panic!("not a response");
        };
        let error = response.result.unwrap_err();
        assert_eq!(error.code, jsonrpc::code::APPLICATION_ERROR);
        assert_eq!(error.error_data().unwrap().code, "turn_running");
    }
}
