# Step 30, its parts: the registry and the tool

The second pull request of step 30 ([answered.md](../answered.md), "The design, before building", points 2, 3.5, 5 and 6, and the table's answers): the registry's kinds of source and held tools, one MCP server's tool as the registry holds it (`McpTool`), section 6 of the system prompt without the script engine and the prefix's tests, the build without the MCP client in `cargo xtask check`, and the two open items the step owns. Nothing registers a real server's tools in a session yet: the lifecycle's part does, with what this part gives it.

**The independent review of the design changed this part before it was built,** and its changes are in what follows: a held tool is shown in a search by its name and one line, never its description or schema; the cap on a schema is 32 KiB, every string of a shown schema is cut at 2 KiB, and a tool whose schema has a key holding a character that acts unseen is left out, arguments still checked against the server's own schema; section 6's notice is sent at most once a turn, and only when the section's text changed since the last one sent; `cargo xtask check` has a `no-mcp` step; and the registry keeps refusing a name the other kind holds, which of the two a configuration's name goes to being the lifecycle's to decide.

## What was built

- **The registry knows each source's kind** (`cineris-tools`, `registry.rs`). `SourceKind { Extension, Mcp }`; `set_source` takes the kind, and a name a source of the other kind holds is refused with `RegistryError::NameTaken { source_name, holder, wanted }`, which names both kinds; a source of the same kind is replaced as before; `remove_source` takes the kind and removes only its own kind's. `sources_of(kind)` lists one kind's sources. The core's extensions pass `SourceKind::Extension` (`ext::tools::register`, the finding's withdrawal). Review 28's note, an MCP server and an extension of one name replacing each other's tools, is closed.
- **A held tool** (`Hold { New, Changed }`, in `cineris-tools` beside the registry, re-exported as `cineris_mcp::Hold`). A source registers its tools with their holds (`set_source_with_holds`, `DynamicTool`). A search shows a held tool by its full name and `Hold::line()`, one line saying it is new, or changed since the person approved it, and needs their approval before it can be called; nothing of its description or schema, and its description is not searched (a search must not tell what it says). `tool_call` does not check a held tool's arguments against the schema the model was not shown, so that a call reaches the question about it; the tool checks them against the server's own schema before it sends anything. What a call of a held tool does, and which tools are held, are the approvals' part's.
- **What a search shows of a schema:** every string in it, a key or a value, cut at 2 KiB and ended with `…`, in a search result and in the error that quotes it (`registry::shown_schema`), as a description is cut.
- **`McpTool`** (`cineris-mcp`, `tool.rs`): one server's tool, implementing `cineris_tools::Tool`, made from a `ToolDefinition`, the server's name and a `ToolCaller`.
  - **Admission** (`admit`, `admit_all`): the entry's `deny_tools` and `tools` first; then a name of 1 to 128 of `A-Z a-z 0-9 _ - .`; an input schema that is a JSON object whose `type`, when it says, is `"object"`, nested no deeper than 64 levels (the schema itself the first), at most 32 KiB written compactly (counted as it is written, and stopped as soon as it is over), with no key anywhere in it that holds a character `visible_to_the_model` would escape; a tool the client can call (`ToolDefinition::unusable`, the client's: not one that runs only as a task); then one tool a name (the first) and 512 a server (`cineris_mcp::MAX_TOOLS`, the client's own cap on a listing). What is left out is a `LeftOut` with its `WhyLeftOut`, in words, for `cineris mcp inspect` and a notice.
  - **The spec:** the tool's name; its description, the title, when given, as the first line, then the description, each made one line by `cineris_base::one_line` (what acts unseen taken out, white space folded, cut at 2 KiB), as the maintainers asked once step 28's fixes A were on `main`; the input schema with what acts unseen in every string of it written as an escape (`cineris_base::visible`, a line break and a tab among them), so that a value can be given back as it is shown; `RuleAction::Ask`; `ConcurrencyClass::Exclusive` always, the annotations not read.
  - **The plan:** "Call the tool `find` of the MCP server `docs` with {…}", the arguments as compact JSON, escaped and cut at 200 bytes; `Concurrency::Exclusive`; no effect of its own (`tool_call` adds `Effect::Ext`).
  - **The run:** the arguments checked against the server's own schema (a mismatch is an error result that quotes the schema as shown, and nothing is sent); the call through the `ToolCaller` under the call's own cancel token; the result mapped as point 3.5 says. Its first line is ``Output of the tool `find` of the MCP server `docs`, written by the server, not by the harness:``; text blocks as text; the structured result as compact JSON when no text block parses to it; an image, audio, or a resource's data as one line naming its type and size (`[an image, image/png, 2.0 KiB: not shown]`); a link as `[a link to the resource NAME at URI]`; an embedded text resource under `[the resource URI, MIME:]`; `[no output]` for none. `is_error` is an error result with the same text. A `ClientError` is an error result in words, `The call of the tool … failed: …`, the client's words escaped and cut at 2 KiB (never a server's standard error, which the client never puts in them); `Cancelled` is an interrupted result. The output policy is not applied: `tool_call` applies it (`execute_produced`), once.
  - **Registering:** `register(registry, server, tools)` registers the tools as the server's source of kind `Mcp`, each with its hold, the source's description the entry's own `description` (never the server's `instructions` or `serverInfo`); `unregister(registry, name)` removes only the server's own.
- **A fake client for tests** (`cineris_mcp::fake::client`, beside the client's fake servers in `cineris_mcp::fake`, behind the `test-kit` feature, turned on by dev-dependencies alone: `cineris-mcp`'s own tests and `cineris-core`'s): `FakeClient`, a `Client` and so a `ToolCaller`, with a list of tools (`set_tools` bumps `changes()`), answers scripted a tool at a time (`answer`; with none left, its arguments given back), the calls made (`calls`), a call that waits at a `CallGate` until the test opens it or its cancel token fires (`hold`, `CallGate::reached`, `arrivals`, `open`), and `close`. `definition(name, description, schema)` and `text_output(blocks)` make a definition and a result.
- **Section 6 without the script engine** (`cineris-core`): `ext::PromptParts` is built in every build and is the loop's `ExtensionsSource` always. The extensions' sources stay in the snapshot the finding settles (`settle` and `PromptParts::new` take `sources_of(Extension)`); the MCP servers' sources are read from the registry as it is when the loop asks, merged by name. The script build is as it was; without it section 6 lists the servers alone and section 7 is empty. `CoreOptions::on_registry` (a `RegistryProbe`, a test's) is given each session's registry before anything is in it, so that a test registers a source as the lifecycle will.
- **Section 7 and a name a server holds** (found when the part was rebased onto step 28's fixes, whose finding keeps an extension's text only when the registry has a source of its name): the finding now asks for the extensions' sources alone (`sources_of(SourceKind::Extension)`), so an extension refused a name an MCP server holds gives no text, though the registry has a source of its name. Tested by `cineris-core::it extensions_prompt::an_extension_refused_a_name_a_server_holds_gives_no_text`, which fails with `sources()`.
- **The notice of a change, once a turn:** the loop already told section 6's change at a turn's start alone, and only when the section's text was not the one the model had (`turn::epochs::extension_changes`, against `given_index`); nothing in the loop changed for it, and the prefix test holds it (a server that goes and comes back as it was between two turns costs no notice).
- **`cargo xtask check`'s `no-mcp` step** (`xtask/src/checks.rs`): `cineris-cli` passes the core's `mcp` through as its own feature, on by default (`mcp = ["cineris-core/mcp"]`; the core's dependency names `sandbox` alone). The step mirrors `no-scripts`, which is now the same code with another `Without` (`NO_SCRIPTS`, `NO_MCP`): the graph of the binary with `mcp` off and its other defaults on, for every target, must not reach `cineris-mcp`, and that build is linted with warnings denied. `no-scripts` now selects `--features mcp`, since `mcp` is a default of the binary's. Step 28's breaks of that code (`mutations/step-28-close.breaks`) are moved to its new names.
- **The two open items** (point 6):
  - `a-tool-that-makes-links-beside-other-calls` is settled: an MCP tool is `Exclusive` in its spec, so `tool_call` schedules every call of one alone whatever its plan says, as an extension's since step 28. Deleted.
  - `a-kept-run-adopted-after-its-call-moved` is fixed (`cineris-agent`, `Round::start`): when a call that runs alone and may change the file system starts (a command, a tool from an extension or an MCP server), every run kept from the response sent again is discarded, one still running included, as a run not asked for again is (its reads forgotten by the table of seen files), and the calls they were kept for run again. Deleted. Step 28's record names both as settled. **It differs from the item's words,** which said to hold the call's plan against its first (`Call::moved`). The first build held the plan against the kept run's own (the call's first plan can itself come after a link, in some orders of the blocking work); the full review then found a tool from outside that writes the file without moving any path, whose following read is planned onto the same path and was answered with the kept run. Discarding every kept run when such a call starts covers both, and replaced the comparison.

## Interfaces added

In `cineris-tools` (`registry`, re-exported at the crate's root but `shown_schema` and `MAX_DESCRIPTION_BYTES`):

```rust
pub enum SourceKind { Extension, Mcp }                      // Display: "an extension", "an MCP server"
pub enum Hold { New, Changed }
impl Hold { pub fn line(self) -> &'static str }
pub struct DynamicTool { pub tool: Arc<dyn Tool>, pub hold: Option<Hold> }
impl DynamicTool { pub fn new(tool: Arc<dyn Tool>) -> Self }
pub struct FoundTool { pub name: String, pub description: String, pub parameters: Json, pub hold: Option<Hold> } // `hold` added
RegistryError::NameTaken { source_name: String, holder: SourceKind, wanted: SourceKind }
impl ToolRegistry {
    pub fn set_source(&self, source: &str, kind: SourceKind, description: &str, tools: Vec<Arc<dyn Tool>>) -> Result<(), RegistryError>; // `kind` added
    pub fn set_source_with_holds(&self, source: &str, kind: SourceKind, description: &str, tools: Vec<DynamicTool>) -> Result<(), RegistryError>;
    pub fn remove_source(&self, source: &str, kind: SourceKind) -> bool; // `kind` added
    pub fn entry(&self, full_name: &str) -> Option<DynamicTool>;
    pub fn held(&self, full_name: &str) -> Option<Hold>;
    pub fn sources_of(&self, kind: SourceKind) -> Vec<SourceSummary>;
}
pub fn shown_schema(schema: &Json) -> Cow<'_, Json>;
pub const MAX_DESCRIPTION_BYTES: usize = 2048;
```

In `cineris-mcp` (`tool`, re-exported at the crate's root):

```rust
pub trait ToolCaller: Send + Sync {
    fn call<'a>(&'a self, tool: &'a str, arguments: Json, cancel: &'a CancelToken)
        -> BoxFuture<'a, Result<CallOutput, ClientError>>;
}
impl<C: Client + ?Sized> ToolCaller for C { … }
pub struct McpTool;                                     // implements cineris_tools::Tool
impl McpTool {
    pub fn held(self, hold: Hold) -> Self;
    pub fn hold(&self) -> Option<Hold>;
    pub fn server(&self) -> &str;
}
pub fn admit(server: &str, definition: &ToolDefinition, caller: Arc<dyn ToolCaller>) -> Result<McpTool, WhyLeftOut>;
pub fn admit_all(server: &ServerDefinition, definitions: &[ToolDefinition], caller: &Arc<dyn ToolCaller>) -> Admitted;
pub struct Admitted { pub tools: Vec<McpTool>, pub left_out: Vec<LeftOut> }
pub struct LeftOut { pub tool: String, pub why: WhyLeftOut }   // Display: "`name` is left out: …"
pub enum WhyLeftOut { Name, Denied, NotListed, SchemaNotObject, SchemaTooDeep, SchemaTooLarge, UnseenKey(String), Unusable(&'static str), Duplicate, TooMany }
pub fn is_tool_name(name: &str) -> bool;
pub fn register(registry: &ToolRegistry, server: &ServerDefinition, tools: Vec<McpTool>) -> Result<(), RegistryError>;
pub fn unregister(registry: &ToolRegistry, server: &str) -> bool;
pub const MAX_NAME_LEN: usize = 128;
pub const MAX_SCHEMA_BYTES: usize = 32 * 1024;
pub const MAX_SCHEMA_DEPTH: usize = 64;
// the cap of 512 tools a server is the client's own, `cineris_mcp::MAX_TOOLS`
pub use cineris_tools::Hold;
// with `test-kit`: cineris_mcp::fake::client::{FakeClient, CallGate, definition, text_output}, beside the client's fake servers in `cineris_mcp::fake`
```

In `cineris-core`: `CoreOptions::on_registry: Option<RegistryProbe>`, `pub type RegistryProbe = Arc<dyn Fn(&Arc<ToolRegistry>) + Send + Sync>`.

No signature of the shared interface (`client.rs`, `definition.rs`, `process.rs`) changed.

## Tests, by rule

| Rule | Tests |
|---|---|
| A name is one kind's; a removal reaches only its own kind | `cineris-tools::it registry::a_source_of_the_other_kind_never_takes_a_name_or_removes_its_tools` |
| A held tool is shown by its name and one line, its description not searched; a call of it is refused before the policy is asked, with how to approve it | `registry::a_held_tool_shows_only_its_name_and_why_it_waits`; `cineris-mcp::it tool::a_held_tools_call_is_refused_and_never_reaches_the_server`; `cineris-core::it mcp_sources::a_held_tools_call_never_reaches_its_server_whatever_would_allow_it` |
| Every string of a shown schema is cut at 2 KiB, in a search and in a mismatch | `registry::every_string_of_a_shown_schema_is_cut_at_two_kib` |
| A tool's spec says exclusive: `tool_call` schedules it alone whatever its plan says | `registry::the_call_is_planned_with_the_tools_own_effects_and_scheduling` (a plan left at read-only); `cineris-mcp::it tool::a_call_is_planned_alone_with_the_tools_name_as_its_effect` (an MCP tool whose annotations say read-only) |
| Admission: the entry's lists, the name, the schema's type, depth, size and keys, one a name, 512 a server | `cineris-mcp::it tool::a_servers_list_is_admitted_by_the_entrys_lists_and_the_rules`, `tool::the_limits_are_where_they_say`; unit tests in `tool.rs` |
| The spec cleaned, the title first, `Ask`, `Exclusive` whatever the annotations | `tool::the_spec_is_cleaned_asks_and_runs_alone_whatever_the_server_says` |
| The plan: summary, no effect, exclusive; through `tool_call` an `ext` effect | `tool::a_call_is_planned_alone_with_the_tools_name_as_its_effect` |
| A result labelled, each block in its form, unseen text escaped | `tool::a_result_is_labelled_as_the_servers_and_each_block_has_its_form` |
| Structured content once | `tool::a_structured_result_is_given_once` |
| `is_error`, a client's error in words, a cancelled call interrupted | `tool::errors_are_error_results_in_words` |
| The call under its own cancel token | `tool::a_call_is_cancelled_with_its_token` |
| Arguments checked against the server's schema before sending | `tool::arguments_that_do_not_fit_the_servers_schema_are_never_sent` |
| The output policy applied once, by `tool_call` | `tool::the_output_policy_is_applied_once_by_tool_call` |
| Registered under the entry, its description, its holds; the other kind's name refused | `tool::a_server_registers_under_its_entry_with_its_holds` |
| Section 6 lists the servers as the registry holds them, the finding settling the extensions' alone | `cineris-core ext::prompt::tests::a_servers_source_is_listed_as_the_registry_holds_it` |
| **Invariant 3, the request prefix:** a server added, its tools changed, flapping and removed mid-epoch leave the declared tools, the prompt and the history before each request as they were, byte for byte; each change is one notice at the next turn's start, before its prompt, none where nothing changed; the entry's description, never the server's words; a new epoch's prompt holds the section | `cineris-core::it mcp_sources::a_servers_tools_change_the_index_by_notice_and_never_the_prefix` |
| Every kept run goes when a call that runs alone and may change the file system starts | `cineris-agent::it redecide::a_kept_read_is_not_used_for_a_call_planned_again_onto_another_path` (a link), `early::a_kept_read_is_not_used_after_a_tool_from_outside_ran_before_it` (a write), `early::a_kept_read_still_running_is_stopped_when_a_tool_from_outside_starts`; each fails without the fix |
| `check` holds a build without the MCP client | `xtask checks::tests::check_holds_a_build_of_the_binary_without_the_mcp_client` |

The breaks are in `mutations/step-30-pr2-registry-tool.breaks`, one a rule.

## Decided here, and open

- **A result's text has what acts unseen written as an escape** (the bidirectional controls, the tag characters, the characters of no width), the control characters left to the output policy, which takes them out with the terminal sequences they start. Point 3.5 says "text blocks as text"; `read` escapes a file's lines the same way, and a result is the one place a server's text reaches the model unasked. The maintainers may want it otherwise. Like `read`'s, it keeps one variation selector after each character shown as it is (`visible_keeping`), which the full review accepted as within point 3.5 (L5).
- **A description loses its lines:** `one_line` folds the server's line breaks into spaces, title and description each one line, the title first.
- **A schema with no `type`** (`{}`) is admitted as an object's; one whose `type` is anything but `"object"`, a list of types included, is left out.
- **Two checks of the arguments:** `tool_call` checks them against the tool's spec, the schema as shown (cleaned), before the call is decided; the tool checks them against the server's own schema before it sends them, first as given and then, if they fit only so, with the escapes the model was shown read back (`cineris_base::unescape`): a value shown as `\u{fe0f}` and given back as shown is sent as the server wrote it, and a value whose backslashes are its own (a Windows path) is sent as given whenever it fits as given.
- **A held tool's call is refused by `tool_call`** before the permission engine is asked, at every level and whatever the rules, with a denial that says how the person approves it (`cineris mcp approve <server>`) and nothing of its definition (`ToolError::NeedsApproval`, code `needs_approval`, a new variant of the tools' errors, counted as a denial). The approvals' part puts its question in that place. A held `McpTool` that is run all the same (by a caller that put the question) quotes nothing of its definition in a mismatch.
- **Section 6's notice keeps its words** ("The index of skills and of tools from extensions changed"), though the index now lists servers too: a notice read back from a log is matched by its words (`context::extensions::told`), and new words would read an older log's notices as withdrawals. A change of words, with the old ones still read, is a small change for a later part.
- **Which source gets a name a configuration gives both an extension and a server** is the lifecycle's to decide (the review's point 5): the registry refuses the second, whichever it is. No helper for it was needed here.
- **`one_line` and `visible_to_the_model`, as on `main` since step 28's fixes A (`0ce319cc`):** the part was written before they were on its base, and switched to them when it was rebased onto the client and part 1. A tool's title and description go through `one_line`, which drops every variation selector. `visible_to_the_model`, which writes every selector as its escape too, is used for every string of a schema (`clean`), for a key's test (a key it would escape leaves the tool out), for the names in a result's first line, a call's summary and a failed call's words (`code`: `in_code`'s escaping of a backtick, then `visible_to_the_model`), for a mismatch's lines (they quote the server's schema), for a result's own lines (`line`: a link, a resource's address and type), for a failed call's words, and for a name or key left out. Section 6's line of a source goes through the agent's `one_line`.

## The full review

No high finding, three medium and four low, all taken; the reviewer's failing tests are folded in (`cineris-mcp::it tool::a_held_tools_call_is_refused_and_never_reaches_the_server`, `a_mismatch_on_a_held_tool_shows_nothing_of_its_schema`, `a_search_that_finds_nothing_shows_each_sources_description_cleaned`, `a_value_shown_with_escapes_is_given_back_as_it_was_shown`; `cineris-agent::it early::a_kept_read_is_not_used_after_a_tool_from_outside_ran_before_it`).

- **M1, a held tool could be called without its question, and a mismatch showed its held schema.** `ToolCall::plan` refuses a held entry (above); `McpTool::run` quotes nothing of a held tool's definition. Tested in `cineris-tools` (`registry::a_held_tool_shows_only_its_name_and_why_it_waits`), in `cineris-mcp`, and end to end at `unrestricted` and under a rule that allows `docs/*` (`cineris-core::it mcp_sources::a_held_tools_call_never_reaches_its_server_whatever_would_allow_it`, with the same tool not held reaching its server under each).
- **M2, a search that found nothing showed each source's description raw.** A source's description is made one line, what acts unseen taken out, when it is registered (`one_line`, 200 bytes), so every view of it is clean. The search of the rest found an extension's tool text raw in a search and in `tool_call`'s mismatch: a tool's description lines, every string of a shown schema and the problems a mismatch quotes now go through `visible_to_the_model` for every tool (an MCP server's are clean already, so nothing changes for them). Tests: `registry::a_sources_description_is_cleaned_where_it_is_registered`, `registry::what_acts_unseen_in_a_tools_text_is_escaped_where_it_is_shown`.
- **M3, a kept read was used though a tool from outside ran before it.** Fixed as the item above says; tests `early::a_kept_read_is_not_used_after_a_tool_from_outside_ran_before_it` and `early::a_kept_read_still_running_is_stopped_when_a_tool_from_outside_starts`. 03-agent-loop's "The cache of early results" says so.
- **L4, an escaped value could never be given back:** the escapes are read back when the arguments fit only so (above).
- **L5, one selector kept after a character in results:** left, as `read` does (above).
- **L6, the cap was on the schema as sent:** it holds of the schema as shown too (`a_schema_over_its_cap_once_escaped_is_left_out`).
- **L7:** `testing-and-ci.md`'s hand-run Windows command for `no-scripts` names `--features mcp` (and `no-mcp` has its own); a prefix test where the sources change between two requests of one turn (`mcp_sources::a_change_within_a_turn_is_told_at_the_next_turns_start`).
- **The breaks no test caught** now have tests and breaks: the run checked against the shown schema in place of the server's (`a_value_shown_with_escapes_is_given_back_as_it_was_shown`); an embedded resource's text, the structured JSON, a link's address and each block's type passed on raw (unseen characters in every server text of `a_result_is_labelled_as_the_servers_and_each_block_has_its_form` and `a_structured_result_is_given_once`); the cuts in `failed` and `line` (`a_servers_long_texts_are_cut_at_two_kib`); `shown_schema` not cutting keys (`registry::a_shown_schemas_keys_are_cut_too`); a mismatch without `shown_schema` or `visible_to_the_model` (`a_mismatch_quotes_the_schema_as_shown`); `PromptParts::new` taking every source (`ext::prompt::tests::a_servers_source_is_listed_as_the_registry_holds_it`, made with a server's source present).

## Windows

Nothing here is written for one system. The kept-read test makes a directory link, as the other re-decide tests do, which the hosted Windows runners may make. Not run on Windows by this part: the step's session runs the runners.

## Measured

- `check`'s `no-mcp` step took 5.1 s in a whole `check` on the reference machine while other builds ran on it (`no-scripts` 9.0 s, `clippy` 46.3 s in the same run), the workspace's crates already linted once. `cargo xtask check no-mcp` repeats it.

## Records of other steps touched

- `docs/steps/28/answered.md` and `docs/steps/step-12-redecide/answered.md` named the two items by links to their files; they name them as settled in this step now, unlinked, as earlier settled items are named.
- `mutations/step-28-close.breaks`: the six breaks of the `no-scripts` check follow its code to its new names (`without_selection`, `without_tree`, `without_lint`, `reaches`). `mutations/step-28.breaks`, `pinned-withdrawn-kept`: its `find` is the removal with its kind.
