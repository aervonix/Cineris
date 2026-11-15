//! The `outline` tool (`docs/architecture/05-tools.md`, "`outline`"): the
//! structure of a file or a directory without its content.
//!
//! - **A file:** its definitions with their lines, from the repository
//!   index through the session's [`Outliner`]: the cached outline when the
//!   file is as the index last saw it, one made now otherwise. A file over
//!   the cap on one file is outlined from its first bytes, and the result
//!   says how much it covers. A file in no language with an extractor is
//!   outlined from its indentation, and the result says so. A binary file
//!   has none.
//! - **A directory:** the outlines of its files, down to `depth` (1, its
//!   own files, unless given), in path order, as many as the output limit
//!   holds, each file's to its first 30 definitions; the rest are counted. The walk is `list`'s: the same ignore
//!   rules, and every entry put to the read gate, so a file a rule protects
//!   is neither read nor named.
//! - **Only inside the workspace.** The index is the workspace's; a path
//!   outside it is refused when the call is planned, and nothing is read.

#![allow(
    clippy::disallowed_methods,
    reason = "looking at a path blocks; the tool runs its work on the blocking pool"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use cortex_base::{AbsPath, CancelToken, ResolvedPath};
use serde::Deserialize;
use serde_json::json;

use crate::args;
use crate::blocking::run_blocking;
use crate::context::{Outliner, PlanCx, ReadGate, RunCx};
use crate::contract::{BoxFuture, Effect, Json, Plan, Tool, ToolOutput, ToolSpec};
use crate::output::{OutputLimits, count};
use crate::result::{ToolError, ToolResult};
use crate::walk::{self, EntryKind, WalkSpec};

/// How deep a directory's outline may go.
const MAX_DEPTH: u64 = 8;

/// How many of a directory's files are kept in path order while the walk
/// runs: more than any result can show.
const KEPT_FILES: usize = 2_000;

/// The most lines one file of a directory's outline takes, so that a large
/// file leaves room for the others: the rest is the file's own outline.
const FILE_LINES: usize = 30;

/// A path of a directory's listing, ordered name by name, so that a
/// directory's files sort together whatever their neighbours' names hold
/// (`a/x.rs` before `a-b.rs`): the order the repository map uses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct InOrder(String);

impl Ord for InOrder {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.split('/').cmp(other.0.split('/'))
    }
}

impl PartialOrd for InOrder {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `text` with every character that would act unseen (a line break, an
/// escape, a bidirectional control) written as an escape: a file can be
/// named so that, written as it is, it starts lines of its own.
fn visible(text: &str) -> String {
    let hidden = |character: char| {
        character.is_control()
            || matches!(
                character,
                '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
            )
    };
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if hidden(other) => out.push_str(&format!("\\u{{{:x}}}", u32::from(other))),
            other => out.push(other),
        }
    }
    out
}

/// What the header and the closing notes may take of the output limits.
const RESERVED_LINES: usize = 8;
const RESERVED_BYTES: usize = 1536;

/// The `outline` tool.
#[derive(Debug)]
pub struct OutlineTool {
    spec: ToolSpec,
}

impl OutlineTool {
    /// The tool.
    #[must_use]
    pub fn new() -> Self {
        Self {
            spec: ToolSpec::new(
                "outline",
                "Show the structure of a file or a directory without its content: each \
                 definition (function, type, class, method, constant, heading) with its line \
                 number, nested ones indented. For a directory, the outlines of its files, \
                 down to `depth`. Outlines are approximate: they are found by reading lines, \
                 not by parsing. Use it to find where something is, then `read` with `offset` \
                 for the lines you need.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "minLength": 1,
                            "description": "A file or a directory in the workspace."
                        },
                        "depth": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_DEPTH,
                            "description": "For a directory: how many levels down to go. 1, its own files, unless given."
                        }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }),
            ),
        }
    }
}

impl Default for OutlineTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    path: String,
    depth: Option<u64>,
}

/// A validated call.
#[derive(Debug, Clone)]
struct Call {
    target: ResolvedPath,
    directory: bool,
    depth: usize,
}

impl Tool for OutlineTool {
    fn preview(&self, _plan: &Plan, _cx: &RunCx) -> Option<crate::Preview> {
        // It changes no file.
        None
    }

    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn plan(&self, args: &Json, cx: &PlanCx) -> Result<Plan, ToolError> {
        let arguments: Arguments = args::parse(&self.spec, args)?;
        let target = cx.paths.resolve(&arguments.path)?;
        let shown = target.display();
        if !target.is_inside_workspace() {
            return Err(ToolError::invalid(format!(
                "`{shown}` is outside the workspace, and outlines come from the workspace's \
                 index. Use `read` for a file elsewhere."
            )));
        }
        let directory =
            std::fs::symlink_metadata(target.to_fs_path()).is_ok_and(|metadata| metadata.is_dir());
        let depth = usize::try_from(arguments.depth.unwrap_or(1)).unwrap_or(usize::MAX);
        // A directory's outline reads the files under it.
        let (summary, effect) = if directory {
            (
                format!("Outline the files in {shown}"),
                Effect::ReadTree(target.clone()),
            )
        } else {
            (format!("Outline {shown}"), Effect::Read(target.clone()))
        };
        Ok(Plan::new(
            summary,
            Call {
                target,
                directory,
                depth,
            },
        )
        .with_effect(effect))
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        _out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let call = match plan.into_args::<Call>() {
                Ok(call) => call,
                Err(error) => return error.into_result(),
            };
            let Some(outliner) = cx.outliner.clone() else {
                return ToolResult::error(
                    "Outlines are not available in this session: there is no repository \
                     index. Use `search` or `read`.",
                );
            };
            let gate = Arc::clone(&cx.gate);
            let limits = cx.limits;
            run_blocking(&cx.cancel, move |cancel| {
                if call.directory {
                    outline_directory(&call, outliner.as_ref(), gate.as_ref(), &limits, cancel)
                } else {
                    outline_file(&call.target, outliner.as_ref(), &limits)
                }
            })
            .await
            .unwrap_or_else(|error| args::unfinished(error, "outline"))
        })
    }
}

/// What a file's outline is, in a few words.
fn describing(language: Option<&str>, definitions: usize) -> String {
    let what = if definitions == 1 {
        "1 definition".to_owned()
    } else {
        format!("{} definitions", count(definitions))
    };
    match language {
        Some(language) => format!("{language}, {what}"),
        None => format!("{what}, from its indentation"),
    }
}

fn outline_file(
    target: &ResolvedPath,
    outliner: &dyn Outliner,
    limits: &OutputLimits,
) -> ToolResult {
    let shown = target.display();
    let outlined = match outliner.describe(target.abs()) {
        Ok(outlined) => outlined,
        Err(why) => {
            return ToolResult::error(format!("`{}` has no outline: {why}.", visible(&shown)));
        }
    };
    let mut text = format!(
        "{} ({}; approximate):",
        visible(&shown),
        describing(outlined.language.as_deref(), outlined.definitions)
    );
    if outlined.definitions == 0 {
        text.push_str("\n[No definitions found. `read` shows the file.]");
    }
    let mut budget = limits.budget(RESERVED_LINES, RESERVED_BYTES);
    let lines: Vec<&str> = outlined.text.lines().collect();
    let mut shown_lines = 0;
    for line in &lines {
        let line = limits.clip(line);
        if !budget.take(&line) {
            break;
        }
        text.push('\n');
        text.push_str(&line);
        shown_lines += 1;
    }
    if shown_lines < lines.len() {
        text.push_str(&format!(
            "\n[{} more definitions not shown: the output limit was reached. `search` finds \
             a definition by its name.]",
            count(lines.len() - shown_lines)
        ));
    }
    if outlined.language.is_none() && outlined.definitions > 0 {
        text.push_str(
            "\n[No extractor reads this kind of file: the outline is the lines that have \
             more indented lines under them.]",
        );
    }
    if let (Some(covered), Some(size)) = (outlined.covered, outlined.size) {
        text.push_str(&format!(
            "\n[The file is {size} bytes, more than the index reads of one file: this outline \
             covers its first {covered} bytes.]"
        ));
    }
    ToolResult::success(text).with_payload(json!({
        "path": shown,
        "kind": "file",
        "language": outlined.language,
        "definitions": outlined.definitions,
        "shown": shown_lines,
        "covered": outlined.covered,
    }))
}

fn outline_directory(
    call: &Call,
    outliner: &dyn Outliner,
    gate: &dyn ReadGate,
    limits: &OutputLimits,
    cancel: &CancelToken,
) -> ToolResult {
    let shown = call.target.display();
    // The first files in path order, kept as the walk goes; the rest are
    // counted.
    let found = Mutex::new((BTreeSet::<InOrder>::new(), 0usize));
    let skipped = walk::walk(
        &WalkSpec {
            root: &call.target,
            max_depth: Some(call.depth),
            include_ignored: false,
            glob: None,
            threads: 0,
        },
        gate,
        cancel,
        &mut || {
            let found = &found;
            Box::new(move |entry: &walk::Found<'_>| {
                if entry.kind != EntryKind::File {
                    return;
                }
                let mut found = found.lock().unwrap_or_else(PoisonError::into_inner);
                found.1 += 1;
                found.0.insert(InOrder(entry.relative.to_owned()));
                if found.0.len() > KEPT_FILES {
                    found.0.pop_last();
                }
            })
        },
    );
    if cancel.is_cancelled() {
        return ToolResult::interrupted(format!("Outlining `{shown}` was interrupted."));
    }
    let (files, total) = found.into_inner().unwrap_or_else(PoisonError::into_inner);

    let mut budget = limits.budget(RESERVED_LINES, RESERVED_BYTES);
    let mut body = String::new();
    let mut outlined = 0usize;
    let mut empty = 0usize;
    let mut out_of_room = false;
    let mut looked = 0usize;
    for InOrder(relative) in &files {
        if cancel.is_cancelled() {
            return ToolResult::interrupted(format!("Outlining `{shown}` was interrupted."));
        }
        looked += 1;
        let Ok(path) = call.target.abs().join(relative) else {
            continue;
        };
        let file_shown = visible(&walk::shown_path(&shown, relative));
        let Some(text) = file_outline(&path, outliner) else {
            empty += 1;
            continue;
        };
        let header = format!("{file_shown}:");
        if !budget.take(&header) {
            out_of_room = true;
            looked -= 1;
            break;
        }
        let mut block = header;
        let lines: Vec<&str> = text.lines().collect();
        let mut taken = 0;
        for line in lines.iter().take(FILE_LINES) {
            let line = format!("  {}", limits.clip(line));
            if !budget.take(&line) {
                out_of_room = true;
                break;
            }
            block.push('\n');
            block.push_str(&line);
            taken += 1;
        }
        if taken < lines.len() {
            block.push_str(&format!(
                "\n  [{} more definitions: outline the file itself.]",
                count(lines.len() - taken)
            ));
        }
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&block);
        outlined += 1;
        if out_of_room {
            break;
        }
    }

    let level = if call.depth == 1 {
        "its own files".to_owned()
    } else {
        format!("{} levels down", call.depth)
    };
    let mut text = if total == 0 {
        format!(
            "`{shown}` has no files to outline ({level}). Hidden and ignored files are left out."
        )
    } else {
        format!(
            "Outlines of the files in `{shown}` ({level}; approximate): {} of {} {}.",
            count(outlined),
            count(total),
            if total == 1 { "file" } else { "files" }
        )
    };
    if !body.is_empty() {
        text.push('\n');
        text.push_str(&body);
    }
    let left = total - looked.min(total);
    if left > 0 {
        let reason = if out_of_room {
            "the output limit was reached"
        } else {
            "too many files"
        };
        text.push_str(&format!(
            "\n[{} more files not shown: {reason}. Outline a narrower `path`.]",
            count(left)
        ));
    }
    if empty > 0 {
        text.push_str(&format!(
            "\n[{} {} with no definitions, or binary, not shown.]",
            count(empty),
            if empty == 1 { "file" } else { "files" }
        ));
    }
    for note in skipped.notes() {
        text.push('\n');
        text.push_str(&note);
    }
    ToolResult::success(text).with_payload(json!({
        "path": shown,
        "kind": "directory",
        "files": total,
        "shown": outlined,
    }))
}

/// A file's outline for a directory's listing: `None` when it has no
/// definitions or none at all.
fn file_outline(path: &AbsPath, outliner: &dyn Outliner) -> Option<String> {
    let outlined = outliner.describe(path).ok()?;
    (outlined.definitions > 0).then_some(outlined.text)
}
