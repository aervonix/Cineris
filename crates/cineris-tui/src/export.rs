//! `/export`: a session written to a file as Markdown or JSON (since step
//! 25; `docs/architecture/09-tui.md`, "Exporting a session";
//! `docs/security.md`, section 4).
//!
//! **What is read is masked twice.** The core masks what an export reads
//! (`session/read` with `redacted`): the harness's credentials by their
//! values, which the core alone holds, and every well-known key format,
//! by the walk the session log's writer uses, with the provider's blocks
//! dropped and what is still running left out; and its result says that it
//! masked, without which the application writes nothing (a core that does
//! not know `redacted` reads the items as they are). Then this module's
//! stricter pass ([`Export::new`]) runs every string of every item, every
//! key of every object, and the title, through [`SecretFilter`]'s formats
//! again, before anything is serialised: a key that JSON's escapes would
//! cut (a private key's line breaks) is still whole when it is looked at,
//! and a key that holds a secret is renamed to the mask. It drops the
//! provider's blocks again too (`raw` outside the JSON that is somebody
//! else's, and every provider block part).
//!
//! **The count is of the masks the export holds,** as the core counts its
//! own: each mask in its items' strings and keys and in its title, and, in
//! Markdown, each the filter adds to the text as it is shown. A pass that
//! changes nothing can add nothing to it.
//!
//! **What is written is safe to read.** In Markdown, control characters
//! are left out and what acts unseen is made visible, as `/copy` does
//! ([`readable`]); the text of each block, as shown, goes through the
//! filter once more, since leaving a control character out can join the
//! halves of a key that no string held whole; each block of code or output
//! is fenced with more backticks than any run in it; the title is escaped
//! so that it stays one heading, and a tool's name is put on one line.
//! The text of a prompt, an answer or a notice is otherwise written as it
//! is, to be read as Markdown: its headings, emphasis and links are its
//! own.
//!
//! **The file is the program's to write** ([`Writer`]), on a worker, which
//! makes the text too: this crate touches no file, and the interface's
//! thread neither masks, renders nor waits on the disk.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::Arc;

use cineris_base::SecretFilter;
use cineris_base::redact::MASK;
use cineris_protocol::item::ToolOutcome;
use cineris_protocol::{Item, Part};
use serde_json::Value;

/// The two formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Markdown, to read.
    Markdown,
    /// JSON: the items as the protocol has them, masked.
    Json,
}

impl Format {
    /// The file's ending, without its dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Json => "json",
        }
    }

    /// The format and the path `/export`'s argument asks for: nothing is
    /// Markdown to the default file, `md` or `json` that format to it, and
    /// a path takes the format from its ending.
    ///
    /// # Errors
    ///
    /// A sentence for the person, when a path ends in neither `.md` nor
    /// `.json`.
    pub fn from_argument(argument: &str) -> Result<(Self, Option<String>), String> {
        let argument = argument.trim();
        match argument {
            "" | "md" | "markdown" => return Ok((Self::Markdown, None)),
            "json" => return Ok((Self::Json, None)),
            _ => {}
        }
        let lower = argument.to_lowercase();
        if lower.ends_with(".md") {
            Ok((Self::Markdown, Some(argument.to_owned())))
        } else if lower.ends_with(".json") {
            Ok((Self::Json, Some(argument.to_owned())))
        } else {
            Err(
                "/export writes Markdown or JSON: name a file that ends in .md or .json, or \
                 say md or json for the default name."
                    .to_owned(),
            )
        }
    }
}

/// What the program's writer is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    /// The path the person gave, as typed, relative to the folder they
    /// started in. `None`: the default name there,
    /// `cineris-session-<date>-<the session's first eight characters>` with
    /// `-2`, `-3` and so on after it while that name is taken.
    pub path: Option<String>,
    /// The session's id, for the default name.
    pub session: String,
    /// The format, for the default name's ending.
    pub format: Format,
    /// What to write.
    pub text: String,
}

/// Why the file was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// Something has the name already: nothing was written over it.
    Exists(String),
    /// The write failed, and what it had written was removed.
    Failed(String),
}

/// The program's writer: writes the text whole under a name of its own
/// beside the file, private to the user, and then gives it the file's
/// name, never over one that exists; what a write that failed wrote is
/// removed. Run on a worker. Returns the path written, as the person reads
/// it.
pub type Writer = Arc<dyn Fn(ExportFile) -> Result<String, ExportError> + Send + Sync>;

/// A session read for an export, masked by the client's pass too.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    /// The session's id.
    pub session: String,
    /// Its title, masked.
    pub title: Option<String>,
    /// Its items, masked, with no provider block.
    pub items: Vec<Item>,
    /// A turn was running: what it had not finished is left out.
    pub running: bool,
    /// How many masks the items and the title hold, the core's and this
    /// pass's: each mask in a string or a key.
    pub masked: usize,
}

impl Export {
    /// The export of `items`, read with `redacted`, with the client's
    /// stricter pass over every string and key of every item and over the
    /// title, and the provider's blocks dropped.
    #[must_use]
    pub fn new(session: &str, title: Option<&str>, items: Vec<Item>, running: bool) -> Self {
        let filter = SecretFilter::new();
        let mut masked = 0;
        let items = items
            .into_iter()
            .filter_map(|item| {
                let mut value = serde_json::to_value(&item).ok()?;
                mask(&mut value, &filter, false);
                let count = masks_of(&value);
                // What does not read back after masking is left out rather
                // than written unmasked.
                let item = serde_json::from_value(value).ok()?;
                masked += count;
                Some(item)
            })
            .collect();
        let title = title.map(|title| {
            let title = filter.redact(title).into_owned();
            masked += masks_in(&title);
            title
        });
        Self {
            session: session.to_owned(),
            title,
            items,
            running,
            masked,
        }
    }

    /// The file's text in `format`, and how many values are masked in it.
    #[must_use]
    pub fn text(&self, format: Format) -> (String, usize) {
        match format {
            Format::Markdown => self.markdown_counted(),
            Format::Json => (self.json(), self.masked),
        }
    }

    /// The export as JSON: the items as the protocol has them, with the
    /// session's id, its title, the count of values masked and whether a
    /// turn was running.
    #[must_use]
    pub fn json(&self) -> String {
        let document = serde_json::json!({
            "export": "cineris session",
            "version": 1,
            "session": self.session,
            "title": self.title,
            "masked": self.masked,
            "running": self.running,
            "items": self.items,
        });
        let mut text = serde_json::to_string_pretty(&document).unwrap_or_default();
        text.push('\n');
        text
    }

    /// The export as Markdown, safe to read.
    #[must_use]
    pub fn markdown(&self) -> String {
        self.markdown_counted().0
    }

    /// The Markdown, and how many values are masked in it: the items' and
    /// the title's, and those the text as shown gave up.
    fn markdown_counted(&self) -> (String, usize) {
        let filter = SecretFilter::new();
        let mut shown = Shown {
            filter: &filter,
            added: 0,
        };
        let title = self
            .title
            .as_deref()
            .map(|title| shown.again(readable(title).replace('\n', " ")))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Session {}", readable(&self.session)));
        let mut body = String::new();
        for item in &self.items {
            shown.item(&mut body, item);
        }
        let masked = self.masked + shown.added;
        let mut out = String::new();
        let _ = writeln!(out, "# {}\n", escape_line(&title));
        let _ = write!(
            out,
            "Session `{}`, exported by Cineris. {}.",
            readable(&self.session).replace('`', ""),
            masked_sentence(masked)
        );
        if self.running {
            out.push_str(" A turn was running: what it had not finished is left out.");
        }
        out.push_str("\n\n");
        out.push_str(&body);
        (out, masked)
    }
}

/// `N values masked`, as the export's first lines say it.
#[must_use]
pub fn masked_sentence(masked: usize) -> String {
    match masked {
        0 => "No value was masked".to_owned(),
        1 => "1 value was masked".to_owned(),
        many => format!("{many} values were masked"),
    }
}

/// Text as a person reads it, and as `/copy` gives it: line breaks and
/// tabs kept, every other control character left out, and what would act
/// unseen (a direction override, a character that looks like another)
/// made visible.
pub(crate) fn readable(text: &str) -> String {
    let typed: String = text
        .chars()
        .filter(|&c| matches!(c, '\n' | '\t') || cineris_term::sanitize::visible(c).is_none())
        .collect();
    cineris_term::sanitize(&typed).into_owned()
}

/// A name inside backticks (a tool's): readable, on one line, with no
/// backtick to end its span.
fn one_line(text: &str) -> String {
    readable(text).replace('\n', " ").replace('`', "")
}

/// A heading's text: what would end it or make it something else
/// escaped.
fn escape_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A fenced block of `content`, its fence longer than any run of
/// backticks in it, so nothing in it can close it.
fn fenced(out: &mut String, info: &str, content: &str) {
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest.max(2) + 1);
    let _ = writeln!(out, "{fence}{info}");
    out.push_str(content);
    if !content.ends_with('\n') {
        out.push('\n');
    }
    let _ = writeln!(out, "{fence}\n");
}

/// The text of `parts`, readable, with an image said as one.
fn parts_text(parts: &[Part]) -> String {
    let mut text = String::new();
    for part in parts {
        match part {
            Part::Text(part) => text.push_str(&readable(&part.text)),
            Part::Image(_) => text.push_str("[an image]"),
            Part::ToolResult(result) => text.push_str(&parts_text(&result.content)),
            _ => {}
        }
    }
    text
}

/// The Markdown's text as it is shown, masked once more, and how many
/// masks that added.
struct Shown<'a> {
    filter: &'a SecretFilter,
    added: usize,
}

impl Shown<'_> {
    /// `text`, a block as it is shown, through the filter once more:
    /// leaving out a control character, or joining two parts, can make
    /// one key of two halves that no string held whole.
    fn again(&mut self, text: String) -> String {
        match self.filter.redact(&text) {
            Cow::Borrowed(_) => text,
            Cow::Owned(masked) => {
                self.added += masks_in(&masked).saturating_sub(masks_in(&text));
                masked
            }
        }
    }

    fn item(&mut self, out: &mut String, item: &Item) {
        match item {
            Item::UserMessage(message) => {
                out.push_str("## You\n\n");
                let text = self.again(parts_text(&message.parts));
                let _ = writeln!(out, "{}\n", text.trim_end());
            }
            Item::AssistantMessage(message) => {
                out.push_str("## Answer\n\n");
                for part in &message.parts {
                    self.part(out, part);
                }
            }
            Item::ToolResult(result) => {
                let outcome = match result.outcome {
                    ToolOutcome::Success => "done",
                    ToolOutcome::Error => "failed",
                    ToolOutcome::Denied => "refused",
                    ToolOutcome::Interrupted => "stopped",
                    _ => "ended",
                };
                let tool = self.again(one_line(&result.tool));
                let _ = writeln!(out, "Result of `{tool}`, {outcome}:\n");
                let text = self.again(parts_text(&result.content));
                fenced(out, "text", &text);
            }
            Item::Notice(notice) => {
                for line in self.again(readable(&notice.text)).lines() {
                    let _ = writeln!(out, "> {line}");
                }
                out.push('\n');
            }
            Item::Summary(summary) => {
                out.push_str("## Summary of the turns before\n\n");
                let text = self.again(readable(&summary.text));
                let _ = writeln!(out, "{}\n", text.trim_end());
            }
            _ => {}
        }
    }

    /// A part of an answer.
    fn part(&mut self, out: &mut String, part: &Part) {
        match part {
            Part::Reasoning(reasoning) if !reasoning.text.trim().is_empty() => {
                out.push_str("Reasoning:\n\n");
                for line in self.again(readable(&reasoning.text)).lines() {
                    let _ = writeln!(out, "> {line}");
                }
                out.push('\n');
            }
            Part::Text(text) => {
                let text = self.again(readable(&text.text));
                let _ = writeln!(out, "{}\n", text.trim_end());
            }
            Part::ToolCall(call) => {
                let name = self.again(one_line(&call.name));
                let _ = writeln!(out, "Tool call `{name}`:\n");
                let arguments = serde_json::to_string_pretty(&call.arguments).unwrap_or_default();
                let arguments = self.again(readable(&arguments));
                fenced(out, "json", &arguments);
            }
            Part::Image(_) => out.push_str("[an image]\n\n"),
            _ => {}
        }
    }
}

/// The fields of an item whose JSON is somebody else's: a tool call's
/// arguments (the model's), a tool result's payload (the tool's) and an
/// error's details. A key named `raw` inside one is theirs, not a
/// provider's block, and is kept, masked like the rest.
const FREE_FORM: [&str; 3] = ["arguments", "payload", "details"];

/// The client's pass over an item as JSON: every string and every key
/// masked with `filter`, a key that held a secret renamed to what the
/// filter makes of it (with ` 2`, ` 3` and so on after it when another key
/// has that name); and, outside JSON that is somebody else's
/// (`free_form`), every provider block dropped: a `raw` field, and a part
/// of the kind `provider_block`, which is nothing else.
fn mask(value: &mut Value, filter: &SecretFilter, free_form: bool) {
    match value {
        Value::String(text) => {
            if let Cow::Owned(masked) = filter.redact(text) {
                *text = masked;
            }
        }
        Value::Array(items) => {
            if !free_form {
                items.retain(|item| {
                    item.get("kind").and_then(Value::as_str) != Some("provider_block")
                });
            }
            for item in items {
                mask(item, filter, free_form);
            }
        }
        Value::Object(fields) => {
            if !free_form {
                fields.remove("raw");
            }
            let mut renamed = Vec::new();
            for (key, mut item) in std::mem::take(fields) {
                let inner = free_form || FREE_FORM.contains(&key.as_str());
                mask(&mut item, filter, inner);
                match filter.redact(&key) {
                    Cow::Owned(masked) if masked != key => renamed.push((masked, item)),
                    _ => {
                        fields.insert(key, item);
                    }
                }
            }
            // After every key that stays, so that none is written over.
            for (masked, item) in renamed {
                let mut name = masked.clone();
                let mut n = 1;
                while fields.contains_key(&name) {
                    n += 1;
                    name = format!("{masked} {n}");
                }
                fields.insert(name, item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// How many masks the strings and the keys of `value` hold.
fn masks_of(value: &Value) -> usize {
    match value {
        Value::String(text) => masks_in(text),
        Value::Array(items) => items.iter().map(masks_of).sum(),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, item)| masks_in(key) + masks_of(item))
            .sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

/// How many masks `text` holds.
fn masks_in(text: &str) -> usize {
    text.matches(MASK).count()
}

#[cfg(test)]
mod tests;
