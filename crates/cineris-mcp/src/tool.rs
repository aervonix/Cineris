//! One server tool as the dynamic-tool registry holds it
//! (`docs/architecture/13-mcp.md`, "How MCP tools reach the model" and
//! "Safety"; `docs/steps/30/answered.md`, points 3.5 and 6).
//!
//! - **What is admitted** ([`admit_all`], [`admit`]): a tool named with 1
//!   to 128 of ASCII letters, digits, `_`, `-` and `.`, whose input schema
//!   is the schema of an object, at most 32 KiB written compactly and 64
//!   levels deep, with no key that holds a character that acts unseen, and
//!   one the client can call (not a tool that runs only as a task); at
//!   most 512 a server, after the entry's `tools` and `deny_tools`. What is
//!   left out is said, with why ([`LeftOut`]), for `cineris mcp inspect` and
//!   a notice.
//! - **What the model is shown** ([`McpTool`]'s spec): the title, when
//!   given, as the first line, then the description, each made one line by
//!   [`cineris_base::one_line`] (what acts unseen taken out, white space
//!   folded); and the input schema, in every string of which what acts
//!   unseen is written as an escape ([`cineris_base::visible_to_the_model`]), so that a
//!   value can be given back as it is shown. The registry cuts each at
//!   2 KiB where a search shows it, and shows a held tool by its name
//!   alone ([`Hold`]). The annotations are not read: the tool asks
//!   by default, and **always runs alone** (`Exclusive`), whatever the
//!   server says of it.
//! - **A call** ([`McpTool`]'s run): the arguments checked against the
//!   server's own schema, the call made through a [`ToolCaller`] under the
//!   call's cancel token, and the result made into text under one line that
//!   names the server and the tool as the server's. The output policy is
//!   `tool_call`'s to apply (`cineris_tools::execute_produced`), once.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::{Arc, LazyLock};

use cineris_base::text::truncate_bytes;
use cineris_base::{
    CancelToken, in_code, one_line, visible, visible_keeping, visible_to_the_model,
};
use cineris_protocol::lease::RuleAction;
use cineris_tools::registry::{MAX_DESCRIPTION_BYTES, shown_schema};
use cineris_tools::{
    BoxFuture, Concurrency, ConcurrencyClass, DynamicTool, Hold, Json, Plan, PlanCx, RegistryError,
    RunCx, SourceKind, Tool, ToolError, ToolOutput, ToolRegistry, ToolResult, ToolSpec,
};

use crate::client::{CallOutput, Client, ClientError, Content, ToolDefinition};
use crate::definition::ServerDefinition;
use crate::session::MAX_TOOLS;

/// The longest name a tool may have, in characters (all ASCII).
pub const MAX_NAME_LEN: usize = 128;

/// The largest input schema admitted, written as compact JSON, in bytes.
pub const MAX_SCHEMA_BYTES: usize = 32 * 1024;

/// The deepest an input schema may be nested: the schema itself is level 1.
pub const MAX_SCHEMA_DEPTH: usize = 64;

/// The most of a server's text a result's own lines show (a link, a
/// resource's name, an error): what a search shows of a description.
const MAX_SHOWN: usize = 2048;

/// The most of a call's arguments its summary shows, in bytes.
const SUMMARY_ARGUMENTS: usize = 200;

/// What calls a server's tool for an [`McpTool`]: a [`Client`], or the
/// lifecycle in front of one, which waits for a server that is starting
/// and starts again one that stopped.
pub trait ToolCaller: Send + Sync {
    /// Calls `tool` with `arguments`. Cancelling `cancel` ends the call as
    /// [`Client::call`] says, with [`ClientError::Cancelled`].
    fn call<'a>(
        &'a self,
        tool: &'a str,
        arguments: Json,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<CallOutput, ClientError>>;
}

impl<C: Client + ?Sized> ToolCaller for C {
    fn call<'a>(
        &'a self,
        tool: &'a str,
        arguments: Json,
        cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<CallOutput, ClientError>> {
        Client::call(self, tool, arguments, cancel)
    }
}

/// Why a server's tool is not in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhyLeftOut {
    /// Its name is not 1 to [`MAX_NAME_LEN`] of ASCII letters, digits,
    /// `_`, `-` and `.`.
    Name,
    /// The server's entry names it in `deny_tools`.
    Denied,
    /// The server's entry has a list of `tools`, which does not name it.
    NotListed,
    /// Its input schema is not the schema of an object.
    SchemaNotObject,
    /// Its input schema is nested deeper than [`MAX_SCHEMA_DEPTH`].
    SchemaTooDeep,
    /// Its input schema is over [`MAX_SCHEMA_BYTES`] written compactly.
    SchemaTooLarge,
    /// A key of its input schema, a property's name among them, holds a
    /// character that acts unseen: shown as [`in_code`] shows it. A key is
    /// not cleaned, since that would change what the schema asks for.
    UnseenKey(String),
    /// The client cannot call it (`ToolDefinition::unusable`, in its
    /// words): a tool that runs only as a task.
    Unusable(&'static str),
    /// Another tool of the server has its name: the first is kept.
    Duplicate,
    /// The server offers more than [`MAX_TOOLS`]: the first are kept.
    TooMany,
}

impl std::fmt::Display for WhyLeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name => write!(
                f,
                "its name is not 1 to {MAX_NAME_LEN} of ASCII letters, digits, `_`, `-` and `.`"
            ),
            Self::Denied => f.write_str("the server's entry denies it (`deny_tools`)"),
            Self::NotListed => {
                f.write_str("the server's entry offers only the tools its `tools` names")
            }
            Self::SchemaNotObject => f.write_str("its input schema is not the schema of an object"),
            Self::SchemaTooDeep => write!(
                f,
                "its input schema is nested deeper than {MAX_SCHEMA_DEPTH} levels"
            ),
            Self::SchemaTooLarge => write!(
                f,
                "its input schema is over {} KiB written compactly",
                MAX_SCHEMA_BYTES / 1024
            ),
            Self::UnseenKey(key) => write!(
                f,
                "a key of its input schema, `{key}`, holds a character that acts unseen"
            ),
            Self::Unusable(why) => f.write_str(why),
            Self::Duplicate => {
                f.write_str("another tool of the server has its name, and the first is kept")
            }
            Self::TooMany => write!(
                f,
                "the server offers more than {MAX_TOOLS} tools, and the first {MAX_TOOLS} are kept"
            ),
        }
    }
}

/// A server's tool that is not in the registry, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// Its name, as [`in_code`] shows it, cut at 200 bytes.
    pub tool: String,
    /// Why.
    pub why: WhyLeftOut,
}

impl std::fmt::Display for LeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is left out: {}", self.tool, self.why)
    }
}

/// What [`admit_all`] made of a server's list.
#[derive(Debug, Default)]
pub struct Admitted {
    /// The tools that go in the registry, in the server's order.
    pub tools: Vec<McpTool>,
    /// The tools left out, in the server's order.
    pub left_out: Vec<LeftOut>,
}

/// Makes a server's list into the tools the registry holds: the entry's
/// `deny_tools` and `tools` first, then [`admit`]'s rules, then one tool a
/// name and [`MAX_TOOLS`] a server. None is held: which are is the
/// approvals' business ([`McpTool::held`]).
#[must_use]
pub fn admit_all(
    server: &ServerDefinition,
    definitions: &[ToolDefinition],
    caller: &Arc<dyn ToolCaller>,
) -> Admitted {
    let mut admitted = Admitted::default();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for definition in definitions {
        let name = &definition.name;
        let why = if server.deny_tools.contains(name) {
            Some(WhyLeftOut::Denied)
        } else if server
            .tools
            .as_ref()
            .is_some_and(|only| !only.contains(name))
        {
            Some(WhyLeftOut::NotListed)
        } else if names.contains(name) {
            Some(WhyLeftOut::Duplicate)
        } else if admitted.tools.len() >= MAX_TOOLS {
            Some(WhyLeftOut::TooMany)
        } else {
            match admit(&server.name, definition, Arc::clone(caller)) {
                Ok(tool) => {
                    names.insert(name.clone());
                    admitted.tools.push(tool);
                    None
                }
                Err(why) => Some(why),
            }
        };
        if let Some(why) = why {
            admitted.left_out.push(LeftOut {
                tool: shown_name(name),
                why,
            });
        }
    }
    admitted
}

/// One tool of the server `server`, if its definition may be registered:
/// its name and its input schema checked as the module says, and a tool
/// the client cannot call (`ToolDefinition::unusable`) left out.
pub fn admit(
    server: &str,
    definition: &ToolDefinition,
    caller: Arc<dyn ToolCaller>,
) -> Result<McpTool, WhyLeftOut> {
    if !is_tool_name(&definition.name) {
        return Err(WhyLeftOut::Name);
    }
    check_schema(&definition.input_schema)?;
    if let Some(why) = definition.unusable() {
        return Err(WhyLeftOut::Unusable(why));
    }
    Ok(McpTool::new(server, definition, caller))
}

/// Whether `name` may be a tool's name: 1 to [`MAX_NAME_LEN`] of ASCII
/// letters, digits, `_`, `-` and `.`.
#[must_use]
pub fn is_tool_name(name: &str) -> bool {
    (1..=MAX_NAME_LEN).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Registers `tools` as the server's source of tools in `registry`,
/// replacing what it had, each with its hold. The source's description is
/// the entry's own (`description`), never anything the server said; a
/// name an extension holds is refused ([`RegistryError::NameTaken`]).
pub fn register(
    registry: &ToolRegistry,
    server: &ServerDefinition,
    tools: Vec<McpTool>,
) -> Result<(), RegistryError> {
    let tools = tools
        .into_iter()
        .map(|tool| {
            let hold = tool.hold;
            DynamicTool {
                tool: Arc::new(tool),
                hold,
            }
        })
        .collect();
    registry.set_source_with_holds(
        &server.name,
        SourceKind::Mcp,
        server.description.as_deref().unwrap_or_default(),
        tools,
    )
}

/// Takes the server `server`'s tools out of `registry`. Whether it had a
/// source there: an extension's of that name is left alone.
pub fn unregister(registry: &ToolRegistry, server: &str) -> bool {
    registry.remove_source(server, SourceKind::Mcp)
}

/// One server's tool, as the registry holds it and `tool_call` runs it.
pub struct McpTool {
    spec: ToolSpec,
    server: String,
    /// The input schema as the server sent it: what arguments are checked
    /// against before they are sent.
    schema: Json,
    hold: Option<Hold>,
    caller: Arc<dyn ToolCaller>,
}

impl std::fmt::Debug for McpTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpTool")
            .field("server", &self.server)
            .field("name", &self.spec.name)
            .field("hold", &self.hold)
            .finish_non_exhaustive()
    }
}

impl McpTool {
    fn new(server: &str, definition: &ToolDefinition, caller: Arc<dyn ToolCaller>) -> Self {
        let spec = ToolSpec::new(
            definition.name.clone(),
            description_of(definition),
            cleaned(&definition.input_schema),
        )
        // A server's tool asks unless a rule allows it by name, and runs
        // alone whatever it says of itself: its annotations are hints for
        // the person only, and a plan that says otherwise is not taken.
        .permission(RuleAction::Ask)
        .concurrency(ConcurrencyClass::Exclusive);
        Self {
            spec,
            server: server.to_owned(),
            schema: definition.input_schema.clone(),
            hold: None,
            caller,
        }
    }

    /// The tool, held: a search shows it by its name and why it waits
    /// ([`Hold::line`]), and nothing of its definition.
    #[must_use]
    pub fn held(mut self, hold: Hold) -> Self {
        self.hold = Some(hold);
        self
    }

    /// Why it is held, when it is.
    #[must_use]
    pub fn hold(&self) -> Option<Hold> {
        self.hold
    }

    /// The server's name.
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }

    /// The first line of every result: the server's output, named as its.
    fn label(&self) -> String {
        format!(
            "Output of the tool `{}` of the MCP server `{}`, written by the server, not by the \
             harness:",
            code(&self.spec.name),
            code(&self.server)
        )
    }

    /// The text of a result.
    fn rendered(&self, output: &CallOutput) -> String {
        let mut parts: Vec<String> = Vec::new();
        for block in &output.content {
            parts.push(match block {
                Content::Text(text) => as_received(text).into_owned(),
                Content::Image { mime, bytes } => {
                    format!("[an image, {}, {}: not shown]", line(mime), size(*bytes))
                }
                Content::Audio { mime, bytes } => {
                    format!("[audio, {}, {}: not shown]", line(mime), size(*bytes))
                }
                Content::Link { uri, name } => {
                    format!("[a link to the resource {} at {}]", line(name), line(uri))
                }
                Content::Resource {
                    uri,
                    mime,
                    text: Some(text),
                    ..
                } => format!(
                    "[the resource {}{}:]\n{}",
                    line(uri),
                    mime.as_deref()
                        .map(|mime| format!(", {}", line(mime)))
                        .unwrap_or_default(),
                    as_received(text)
                ),
                Content::Resource {
                    uri,
                    mime,
                    text: None,
                    bytes,
                } => format!(
                    "[the resource {}, {}, {}: not shown]",
                    line(uri),
                    mime.as_deref().map_or(Cow::Borrowed("data"), line),
                    size(*bytes)
                ),
            });
        }
        if let Some(structured) = &output.structured {
            // A server is to give its structured result as text too; when
            // no text block carries it, it is given here, as compact JSON.
            let carried = output.content.iter().any(|block| {
                matches!(block, Content::Text(text)
                    if serde_json::from_str::<Json>(text).is_ok_and(|parsed| parsed == *structured))
            });
            if !carried {
                parts.push(as_received(&structured.to_string()).into_owned());
            }
        }
        if parts.is_empty() {
            parts.push("[no output]".to_owned());
        }
        format!("{}\n{}", self.label(), parts.join("\n"))
    }

    /// What the model is told of a call that did not get a result.
    fn failed(&self, error: &ClientError) -> ToolResult {
        let what = format!(
            "The call of the tool `{}` of the MCP server `{}`",
            code(&self.spec.name),
            code(&self.server)
        );
        match error {
            ClientError::Cancelled => ToolResult::interrupted(format!("{what} was cancelled.")),
            // The client's own words, in which a server's message may be:
            // escaped and cut. A server's standard error is never in them.
            other => ToolResult::error(format!(
                "{what} failed: {}",
                cut(&visible_to_the_model(&other.to_string()), MAX_SHOWN)
            )),
        }
    }
}

impl Tool for McpTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn plan(&self, args: &Json, _cx: &PlanCx) -> Result<Plan, ToolError> {
        let compact = args.to_string();
        let summary = format!(
            "Call the tool `{}` of the MCP server `{}` with {}",
            code(&self.spec.name),
            code(&self.server),
            cut(&visible(&compact), SUMMARY_ARGUMENTS)
        );
        // No effect of its own: `tool_call` adds the call of the tool by
        // name, which the permission engine decides.
        Ok(Plan::new(summary, args.clone()).with_concurrency(Concurrency::Exclusive))
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        _out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let arguments = match plan.into_args::<Json>() {
                Ok(arguments) => arguments,
                Err(error) => return error.into_result(),
            };
            // Against the server's own schema, before anything is sent: a
            // held tool's arguments were not checked before its question.
            if let Err(mismatch) = cineris_tools::schema::validate(&self.schema, &arguments) {
                return ToolResult::error(format!(
                    "The arguments for `{}/{}` do not fit its schema:\n{}\nIts schema is:\n{}",
                    code(&self.server),
                    code(&self.spec.name),
                    // Each line as the model is shown a schema's strings:
                    // it quotes the server's schema.
                    mismatch
                        .problems()
                        .iter()
                        .map(|problem| visible_to_the_model(problem))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    shown_schema(&self.spec.parameters)
                ));
            }
            match self
                .caller
                .call(&self.spec.name, arguments, &cx.cancel)
                .await
            {
                Ok(output) if output.is_error => ToolResult::error(self.rendered(&output)),
                Ok(output) => ToolResult::success(self.rendered(&output)),
                Err(error) => self.failed(&error),
            }
        })
    }

    fn preview(&self, _plan: &Plan, _cx: &RunCx) -> Option<cineris_tools::Preview> {
        // What a server changes is its own: the harness cannot show it.
        None
    }
}

/// The text the model is shown of a tool: the title, when given, as the
/// first line, then the description, each made one line by
/// [`one_line`]: what acts unseen taken out, white space folded, cut at
/// 2 KiB.
fn description_of(definition: &ToolDefinition) -> String {
    let one = |text: &str| one_line(text, MAX_DESCRIPTION_BYTES);
    let title = definition
        .title
        .as_deref()
        .map(one)
        .filter(|title| !title.is_empty());
    let description = definition
        .description
        .as_deref()
        .map(one)
        .filter(|text| !text.is_empty());
    match (title, description) {
        (Some(title), Some(description)) => format!("{title}\n{description}"),
        (Some(only), None) | (None, Some(only)) => only,
        (None, None) => String::new(),
    }
}

/// A string of a server's schema as the model is shown it: what acts
/// unseen written as an escape, a line break and a tab among them, so that
/// a value the model must give back can be written as it is shown.
fn clean(text: &str) -> String {
    visible_to_the_model(text).into_owned()
}

/// `schema` with every string in it [`clean`]ed. Its keys are not changed:
/// one that holds anything unseen leaves the tool out ([`admit`]).
fn cleaned(schema: &Json) -> Json {
    match schema {
        Json::String(text) => Json::String(clean(text)),
        Json::Array(items) => Json::Array(items.iter().map(cleaned).collect()),
        Json::Object(map) => Json::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), cleaned(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The control characters, which the output policy takes out of a result
/// with the terminal sequences they start (`cineris_tools::output`).
static CONTROLS: LazyLock<Vec<char>> = LazyLock::new(|| {
    (0..=0x9f_u32)
        .filter_map(char::from_u32)
        .filter(|character| character.is_control())
        .collect()
});

/// A result's text: what acts unseen and is not a control character (a
/// bidirectional control, a tag character, a character of no width, a run
/// of variation selectors) written as an escape, as `read` writes it in a
/// file's lines. The control characters are left to the output policy.
fn as_received(text: &str) -> Cow<'_, str> {
    visible_keeping(text, &CONTROLS)
}

/// A server's text on a line of a result's own: escaped, and cut.
fn line(text: &str) -> Cow<'_, str> {
    let shown = visible_to_the_model(text);
    if shown.len() <= MAX_SHOWN {
        shown
    } else {
        Cow::Owned(cut(&shown, MAX_SHOWN))
    }
}

/// `text` cut at `max` bytes, ended with `…` when it was.
fn cut(text: &str, max: usize) -> String {
    let kept = truncate_bytes(text, max);
    if kept.len() < text.len() {
        format!("{kept}…")
    } else {
        kept.to_owned()
    }
}

/// A tool's name as a message about it shows it.
fn shown_name(name: &str) -> String {
    // A name left out holds anything.
    cut(&code(name), 200)
}

/// A name shown as code, as the model is to use it again: a backtick and a
/// backslash in it escaped (`in_code`), then every variation selector too
/// ([`visible_to_the_model`]).
fn code(text: &str) -> String {
    visible_to_the_model(&in_code(text)).into_owned()
}

/// A size in bytes, in words.
fn size(bytes: usize) -> String {
    const KIB: usize = 1024;
    if bytes < KIB {
        format!("{bytes} bytes")
    } else if bytes < KIB * KIB {
        format!("{}.{} KiB", bytes / KIB, bytes % KIB * 10 / KIB)
    } else {
        format!(
            "{}.{} MiB",
            bytes / (KIB * KIB),
            bytes % (KIB * KIB) * 10 / (KIB * KIB)
        )
    }
}

/// Whether `schema` may be a tool's input schema: an object, whose `type`
/// is `"object"` when it says, nested no deeper than [`MAX_SCHEMA_DEPTH`],
/// no larger than [`MAX_SCHEMA_BYTES`] written compactly, and with no key
/// that holds a character that acts unseen.
fn check_schema(schema: &Json) -> Result<(), WhyLeftOut> {
    let is_object = schema
        .as_object()
        .is_some_and(|map| map.get("type").is_none_or(|kind| kind == "object"));
    if !is_object {
        return Err(WhyLeftOut::SchemaNotObject);
    }
    // The depth first, without recursion: what is written next walks it.
    if deeper_than(schema, MAX_SCHEMA_DEPTH) {
        return Err(WhyLeftOut::SchemaTooDeep);
    }
    if larger_than(schema, MAX_SCHEMA_BYTES) {
        return Err(WhyLeftOut::SchemaTooLarge);
    }
    if let Some(key) = unseen_key(schema) {
        return Err(WhyLeftOut::UnseenKey(cut(&code(key), 200)));
    }
    Ok(())
}

/// Whether `value` is nested deeper than `limit` levels, itself the first.
fn deeper_than(value: &Json, limit: usize) -> bool {
    let mut stack: Vec<(&Json, usize)> = vec![(value, 1)];
    while let Some((value, depth)) = stack.pop() {
        if depth > limit {
            return true;
        }
        match value {
            Json::Array(items) => stack.extend(items.iter().map(|item| (item, depth + 1))),
            Json::Object(map) => stack.extend(map.values().map(|item| (item, depth + 1))),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::String(_) => {}
        }
    }
    false
}

/// Whether `value` written as compact JSON is longer than `limit` bytes:
/// counted as it is written, and stopped as soon as it is.
fn larger_than(value: &Json, limit: usize) -> bool {
    struct Counter {
        written: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.written += bytes.len();
            if self.written > self.limit {
                Err(std::io::Error::other("over the limit"))
            } else {
                Ok(bytes.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter { written: 0, limit }, value).is_err()
}

/// The first key in `value`, at any depth, that holds a character
/// [`visible_to_the_model`] would write as an escape.
fn unseen_key(value: &Json) -> Option<&str> {
    let mut stack: Vec<&Json> = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Json::Array(items) => stack.extend(items),
            Json::Object(map) => {
                for (key, item) in map {
                    if matches!(visible_to_the_model(key), Cow::Owned(_)) {
                        return Some(key);
                    }
                    stack.push(item);
                }
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::String(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_name_is_one_to_128_of_the_safe_characters() {
        assert!(is_tool_name("find_issue"));
        assert!(is_tool_name("a.b-c_9"));
        assert!(is_tool_name(&"x".repeat(128)));
        for refused in ["", "a/b", "two words", "créer", "tab\t", "x\u{200b}"] {
            assert!(!is_tool_name(refused), "{refused:?}");
        }
        assert!(!is_tool_name(&"x".repeat(129)));
    }

    #[test]
    fn a_schema_is_measured_without_being_written_whole() {
        let small = json!({"type": "object"});
        assert!(!larger_than(&small, 17));
        assert!(larger_than(&small, 16));
        let mut deep = json!({"type": "object"});
        for _ in 0..63 {
            deep = json!({"type": "object", "properties": {"a": deep}});
        }
        // Each level is an object holding an object of properties.
        assert!(deeper_than(&deep, MAX_SCHEMA_DEPTH));
        assert!(!deeper_than(&json!({"a": {"b": 1}}), 3));
        assert!(deeper_than(&json!({"a": {"b": [1]}}), 3));
    }

    #[test]
    fn sizes_are_written_in_words() {
        assert_eq!(size(12), "12 bytes");
        assert_eq!(size(1536), "1.5 KiB");
        assert_eq!(size(3 * 1024 * 1024 + 1024 * 1024 / 10 + 1), "3.1 MiB");
    }
}
