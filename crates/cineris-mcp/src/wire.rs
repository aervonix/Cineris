//! The tools part of MCP's types, read from and written to JSON by hand
//! (`docs/steps/30/answered.md`, points 0 and 3.5; the research notes'
//! sections 2.4, 2.5 and 3.6).
//!
//! What the client keeps of a server's text is bounded here: a tool's
//! definition is kept as the server sent it (its hash is of that), an
//! error's message is cut, and an image's or a blob's data is counted, not
//! kept.

use cineris_tools::Json;
use serde_json::{Map, json};

use crate::client::{CallOutput, ClientError, Content, Era, ToolDefinition};
use crate::jsonrpc::RpcError;

/// The modern revisions the client speaks, the one it prefers first.
pub(crate) const MODERN_VERSIONS: &[&str] = &["2026-07-28"];

/// The legacy revisions the client speaks, the newest first: the one it
/// asks for in `initialize`, and those it accepts in the answer.
pub(crate) const LEGACY_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The modern error codes (`-32020` to `-32022`): a server that answers
/// with one speaks the modern era.
pub(crate) const HEADER_MISMATCH: i64 = -32020;
/// A client capability the server needs and the client did not declare.
pub(crate) const MISSING_CLIENT_CAPABILITY: i64 = -32021;
/// A revision the server does not speak, with those it does.
pub(crate) const UNSUPPORTED_VERSION: i64 = -32022;

/// How much of a server's error message is kept.
const MESSAGE_CAP: usize = 2048;

/// How much of the name a server gives itself is kept.
const NAME_CAP: usize = 256;

/// Who the client says it is.
pub(crate) fn client_info() -> Json {
    json!({"name": "cineris", "version": env!("CARGO_PKG_VERSION")})
}

/// The `_meta` every modern request carries: the revision, the client's
/// capabilities (none), and who it is.
pub(crate) fn request_meta(version: &str) -> Json {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": client_info(),
    })
}

/// The `_meta` key that names the subscription a notification belongs to.
pub(crate) const SUBSCRIPTION_ID: &str = "io.modelcontextprotocol/subscriptionId";

/// The key of the server's own description of itself in a modern result.
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// A JSON-RPC error a server answered with, as the client reports it.
pub(crate) fn server_error(error: RpcError) -> ClientError {
    ClientError::Server {
        code: error.code,
        message: cut(&error.message, MESSAGE_CAP),
    }
}

/// `text` cut to at most `cap` bytes, at a character's boundary.
fn cut(text: &str, cap: usize) -> String {
    cineris_base::text::truncate_bytes(text, cap).to_owned()
}

/// What a server said about itself, from `server/discover` or
/// `initialize`: the name it gave, and whether it says it tells the client
/// when its tools change.
pub(crate) fn about(result: &Json, era: Era) -> (Option<String>, bool) {
    let implementation = match era {
        Era::Modern => result
            .pointer("/_meta")
            .and_then(|meta| meta.get(SERVER_INFO)),
        Era::Legacy => result.get("serverInfo"),
    };
    let name = implementation
        .and_then(|info| info.get("name"))
        .and_then(Json::as_str)
        .map(|name| cut(name, NAME_CAP));
    let list_changed = result
        .pointer("/capabilities/tools/listChanged")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    (name, list_changed)
}

/// The strings of a list, leaving out what is not one.
pub(crate) fn strings(value: Option<&Json>) -> Vec<String> {
    value
        .and_then(Json::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// What a modern result is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultType {
    /// The final result (`complete`, or no `resultType`).
    Complete,
    /// The server needs something before it can answer: send the request
    /// again (`input_required`).
    InputRequired,
}

/// The type of a modern result. Any type the client does not know is the
/// server breaking the protocol.
pub(crate) fn result_type(result: &Json) -> Result<ResultType, ClientError> {
    match result.get("resultType") {
        None | Some(Json::Null) => Ok(ResultType::Complete),
        Some(Json::String(kind)) if kind == "complete" => Ok(ResultType::Complete),
        Some(Json::String(kind)) if kind == "input_required" => Ok(ResultType::InputRequired),
        Some(_) => Err(ClientError::Protocol(
            "it answered with a result of a type the client does not know".to_owned(),
        )),
    }
}

/// One page of `tools/list`.
#[derive(Debug)]
pub(crate) struct Page {
    pub(crate) tools: Vec<ToolDefinition>,
    /// The cursor of the next page; none on the last. An empty string is a
    /// cursor.
    pub(crate) next: Option<String>,
    /// How long the page may be kept, in milliseconds: absent or negative
    /// is 0.
    pub(crate) ttl_ms: u64,
    /// What its tools' definitions take in memory, every copy counted.
    pub(crate) bytes: usize,
}

/// Reads one page of `tools/list`.
pub(crate) fn tools_page(mut result: Json) -> Result<Page, ClientError> {
    let broken = |words: &str| ClientError::Protocol(format!("its list of tools {words}"));
    let next = match result.get("nextCursor") {
        None | Some(Json::Null) => None,
        Some(Json::String(cursor)) => Some(cursor.clone()),
        Some(_) => return Err(broken("has a cursor that is not a string")),
    };
    let ttl_ms = result
        .get("ttlMs")
        .and_then(|ttl| ttl.as_i64().or_else(|| ttl.as_u64().map(|_| i64::MAX)))
        .map_or(0, |ttl| u64::try_from(ttl).unwrap_or(0));
    let Some(Json::Array(entries)) = result.get_mut("tools").map(Json::take) else {
        return Err(broken("has no `tools` array"));
    };
    let mut tools = Vec::with_capacity(entries.len());
    let mut bytes = 0usize;
    for (index, raw) in entries.into_iter().enumerate() {
        let Some(name) = raw.get("name").and_then(Json::as_str) else {
            return Err(broken(&format!(
                "has an entry ({}) with no name",
                index + 1
            )));
        };
        let text = |key: &str| raw.get(key).and_then(Json::as_str).map(str::to_owned);
        let present = |key: &str| raw.get(key).filter(|value| !value.is_null()).cloned();
        tools.push(ToolDefinition {
            name: name.to_owned(),
            title: text("title"),
            description: text("description"),
            input_schema: raw.get("inputSchema").cloned().unwrap_or(Json::Null),
            output_schema: present("outputSchema"),
            annotations: raw
                .get("annotations")
                .filter(|value| value.is_object())
                .cloned(),
            raw,
        });
        if let Some(tool) = tools.last() {
            bytes = bytes.saturating_add(held_tool(tool));
        }
    }
    Ok(Page {
        tools,
        next,
        ttl_ms,
        bytes,
    })
}

/// What `value` takes in memory: each value at its own size, and what it
/// holds on the heap. A JSON value is far larger in memory than written:
/// `0` is one byte of a line and 32 in memory.
fn held_size(value: &Json) -> usize {
    const VALUE: usize = std::mem::size_of::<Json>();
    // A map's entry: its key's `String`, and the map's own bookkeeping.
    const ENTRY: usize = std::mem::size_of::<String>() + 16;
    let held = match value {
        Json::Null | Json::Bool(_) | Json::Number(_) => 0,
        Json::String(text) => text.len(),
        Json::Array(items) => items.iter().map(held_size).fold(0, usize::saturating_add),
        Json::Object(map) => map
            .iter()
            .map(|(key, item)| {
                ENTRY
                    .saturating_add(key.len())
                    .saturating_add(held_size(item))
            })
            .fold(0, usize::saturating_add),
    };
    VALUE.saturating_add(held)
}

/// What a tool's definition takes in memory: `raw`, and the copies of its
/// parts read from it.
fn held_tool(tool: &ToolDefinition) -> usize {
    let text = |text: &Option<String>| text.as_ref().map_or(0, String::len);
    let json = |value: &Option<Json>| value.as_ref().map_or(0, held_size);
    [
        std::mem::size_of::<ToolDefinition>(),
        tool.name.len(),
        text(&tool.title),
        text(&tool.description),
        held_size(&tool.input_schema),
        json(&tool.output_schema),
        json(&tool.annotations),
        held_size(&tool.raw),
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
}

impl ToolDefinition {
    /// Why the client cannot call this tool, if it cannot: a tool that runs
    /// only as a task (`2025-11-25`'s `execution.taskSupport: "required"`),
    /// which this client does not do, or one whose input schema is not an
    /// object. Admission leaves such a tool out and names it.
    #[must_use]
    pub fn unusable(&self) -> Option<&'static str> {
        if self
            .raw
            .pointer("/execution/taskSupport")
            .and_then(Json::as_str)
            == Some("required")
        {
            return Some("it runs only as a task, which this client does not do");
        }
        if !self.input_schema.is_object() {
            return Some("its input schema is not a JSON object");
        }
        None
    }
}

/// The parameters of `tools/call`: the tool, its arguments when there are
/// any, and the state a server asked to be given back.
pub(crate) fn call_params(tool: &str, arguments: &Json, state: Option<&str>) -> Map<String, Json> {
    let mut params = Map::new();
    params.insert("name".to_owned(), Json::from(tool));
    if !arguments.is_null() {
        params.insert("arguments".to_owned(), arguments.clone());
    }
    if let Some(state) = state {
        params.insert("requestState".to_owned(), Json::from(state));
    }
    params
}

/// Reads the result of `tools/call`. A legacy server's structured result
/// is kept only when it is an object, as its revisions say it is; a modern
/// one's may be any value.
pub(crate) fn call_output(mut result: Json, era: Era) -> CallOutput {
    let structured =
        result
            .get_mut("structuredContent")
            .map(Json::take)
            .filter(|value| match era {
                Era::Modern => !value.is_null(),
                Era::Legacy => value.is_object(),
            });
    let content = match result.get("content") {
        Some(Json::Array(blocks)) => blocks.iter().filter_map(content).collect(),
        _ => Vec::new(),
    };
    CallOutput {
        content,
        structured,
        is_error: result.get("isError").and_then(Json::as_bool) == Some(true),
    }
}

/// One content block, or nothing for a block of a type the client does not
/// know.
fn content(block: &Json) -> Option<Content> {
    let text = |value: &Json, key: &str| value.get(key).and_then(Json::as_str).map(str::to_owned);
    let mime = |value: &Json| text(value, "mimeType").unwrap_or_default();
    let size = |value: &Json, key: &str| {
        value
            .get(key)
            .and_then(Json::as_str)
            .map_or(0, decoded_size)
    };
    match block.get("type").and_then(Json::as_str)? {
        "text" => Some(Content::Text(text(block, "text").unwrap_or_default())),
        "image" => Some(Content::Image {
            mime: mime(block),
            bytes: size(block, "data"),
        }),
        "audio" => Some(Content::Audio {
            mime: mime(block),
            bytes: size(block, "data"),
        }),
        "resource_link" => Some(Content::Link {
            uri: text(block, "uri").unwrap_or_default(),
            name: text(block, "name").unwrap_or_default(),
        }),
        "resource" => {
            let resource = block.get("resource")?;
            Some(Content::Resource {
                uri: text(resource, "uri").unwrap_or_default(),
                mime: text(resource, "mimeType"),
                text: text(resource, "text"),
                bytes: size(resource, "blob"),
            })
        }
        _ => None,
    }
}

/// How many bytes base64 `data` decodes to, counted without decoding it.
fn decoded_size(data: &str) -> usize {
    // Four characters carry three bytes; padding and line breaks carry
    // none.
    let characters = data
        .bytes()
        .filter(|byte| *byte != b'=' && !byte.is_ascii_whitespace())
        .count();
    characters.saturating_mul(3) / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_keeps_each_tool_as_it_was_sent_and_its_cursor_and_lifetime() {
        let raw = json!({
            "name": "find",
            "title": "Find",
            "description": "Finds",
            "inputSchema": {"type": "object"},
            "outputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true},
            "icons": [{"src": "x"}],
            "x-unknown": 1,
        });
        let page =
            tools_page(json!({"tools": [raw.clone()], "nextCursor": "", "ttlMs": 5})).unwrap();
        assert_eq!(
            page.next.as_deref(),
            Some(""),
            "an empty string is a cursor"
        );
        assert_eq!(page.ttl_ms, 5);
        let tool = &page.tools[0];
        assert_eq!(tool.raw, raw);
        assert_eq!(tool.title.as_deref(), Some("Find"));
        assert_eq!(tool.annotations, Some(json!({"readOnlyHint": true})));
        assert_eq!(tool.unusable(), None);

        for (ttl, kept) in [(json!(-4), 0), (json!(null), 0), (json!("9"), 0)] {
            let page = tools_page(json!({"tools": [], "ttlMs": ttl})).unwrap();
            assert_eq!(page.ttl_ms, kept);
            assert_eq!(page.next, None);
        }
        assert!(tools_page(json!({"tools": [{"title": "x"}]})).is_err());
        assert!(tools_page(json!({"tools": [], "nextCursor": 3})).is_err());
        assert!(tools_page(json!({})).is_err());
    }

    #[test]
    fn a_page_is_charged_what_its_definitions_take_in_memory_every_copy_counted() {
        let zeros = vec![0; 10_000];
        let page = tools_page(json!({"tools": [
            {"name": "a", "inputSchema": {"type": "object", "x": zeros}},
        ]}))
        .unwrap();
        let value = std::mem::size_of::<Json>();
        // Two bytes of a line each, and at least a value's size in memory,
        // in `raw` and again in `input_schema`.
        assert!(page.bytes >= 2 * 10_000 * value, "{}", page.bytes);
        assert!(page.bytes < 3 * 10_000 * value, "{}", page.bytes);
        assert_eq!(held_size(&json!(0)), value);
        assert_eq!(held_size(&json!("abc")), value + 3);
    }

    #[test]
    fn a_tool_that_runs_only_as_a_task_or_has_no_object_schema_is_unusable() {
        let page = tools_page(json!({"tools": [
            {"name": "a", "inputSchema": {"type": "object"}, "execution": {"taskSupport": "required"}},
            {"name": "b", "inputSchema": {"type": "object"}, "execution": {"taskSupport": "optional"}},
            {"name": "c"},
        ]}))
        .unwrap();
        assert!(page.tools[0].unusable().unwrap().contains("task"));
        assert_eq!(page.tools[1].unusable(), None);
        assert!(page.tools[2].unusable().unwrap().contains("schema"));
    }

    #[test]
    fn content_is_read_with_data_counted_and_not_kept() {
        let output = call_output(
            json!({
                "content": [
                    {"type": "text", "text": "hi"},
                    {"type": "image", "data": "QUJD\nREVG", "mimeType": "image/png"},
                    {"type": "audio", "data": "QQ==", "mimeType": "audio/wav"},
                    {"type": "resource_link", "uri": "file:///a", "name": "a"},
                    {"type": "resource", "resource": {"uri": "u", "text": "body"}},
                    {"type": "resource", "resource": {"uri": "v", "mimeType": "x/y", "blob": "QUI="}},
                    {"type": "hologram"},
                ],
                "isError": true,
            }),
            Era::Legacy,
        );
        assert_eq!(
            output.content,
            [
                Content::Text("hi".to_owned()),
                Content::Image {
                    mime: "image/png".to_owned(),
                    bytes: 6
                },
                Content::Audio {
                    mime: "audio/wav".to_owned(),
                    bytes: 1
                },
                Content::Link {
                    uri: "file:///a".to_owned(),
                    name: "a".to_owned()
                },
                Content::Resource {
                    uri: "u".to_owned(),
                    mime: None,
                    text: Some("body".to_owned()),
                    bytes: 0
                },
                Content::Resource {
                    uri: "v".to_owned(),
                    mime: Some("x/y".to_owned()),
                    text: None,
                    bytes: 2
                },
            ]
        );
        assert!(output.is_error);

        let sent_false = call_output(json!({"content": [], "isError": false}), Era::Modern);
        assert!(!sent_false.is_error, "`isError: false` sent explicitly");
    }

    #[test]
    fn a_legacy_structured_result_is_kept_only_when_it_is_an_object() {
        let legacy = call_output(
            json!({"content": [], "structuredContent": [1, 2]}),
            Era::Legacy,
        );
        assert_eq!(legacy.structured, None);
        assert!(!legacy.is_error);
        let modern = call_output(
            json!({"content": [], "structuredContent": [1, 2]}),
            Era::Modern,
        );
        assert_eq!(modern.structured, Some(json!([1, 2])));
        let object = call_output(json!({"structuredContent": {"a": 1}}), Era::Legacy);
        assert_eq!(object.structured, Some(json!({"a": 1})));
        assert!(object.content.is_empty());
    }

    #[test]
    fn a_result_of_an_unknown_type_is_a_fault_and_an_absent_one_is_complete() {
        assert_eq!(result_type(&json!({})).unwrap(), ResultType::Complete);
        assert_eq!(
            result_type(&json!({"resultType": "input_required"})).unwrap(),
            ResultType::InputRequired
        );
        assert!(result_type(&json!({"resultType": "task"})).is_err());
    }

    #[test]
    fn a_servers_error_message_is_cut() {
        let ClientError::Server { code, message } = server_error(RpcError {
            code: 5,
            message: "é".repeat(2000),
            data: None,
        }) else {
            panic!("not a server error");
        };
        assert_eq!(code, 5);
        assert_eq!(message.len(), 2048);
    }
}
