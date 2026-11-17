//! `/export`: a session written to a file as Markdown or JSON (since step
//! 25; `docs/architecture/09-tui.md`, "Exporting a session";
//! `docs/security.md`, section 4).
//!
//! **What is read is masked twice.** The core masks what an export reads
//! (`session/read` with `redacted`): the harness's credentials by their
//! values, which the core alone holds, and every well-known key format,
//! by the walk the session log's writer uses, with the provider's blocks
//! dropped and what is still running left out. Then this module's stricter
//! pass ([`Export::new`]) runs every string of every item, and the title,
//! through [`SecretFilter`]'s formats again, before anything is serialised:
//! a key that JSON's escapes would cut (a private key's line breaks) is
//! still whole when it is looked at.
//!
//! **What is written is safe to read.** In Markdown, control characters
//! are made visible and what acts unseen is escaped, as `/copy` does
//! ([`readable`]), and each block of code or output is fenced with more
//! backticks than any run in it.
//!
//! **The file is the program's to write** ([`Writer`]), on a worker: this
//! crate touches no file, and the interface's thread never waits on the
//! disk.

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

/// The program's writer: creates the file new and private to the user,
/// never over one that exists, and removes what it wrote when the write
/// fails part of the way. Run on a worker. Returns the path written, as
/// the person reads it.
pub type Writer = Arc<dyn Fn(ExportFile) -> Result<String, ExportError> + Send + Sync>;

/// A session read for an export, masked by the client's pass too.
#[derive(Debug, Clone, PartialEq)]
pub struct Export {
    /// The session's id.
    pub session: String,
    /// Its title, masked.
    pub title: Option<String>,
    /// Its items, masked.
    pub items: Vec<Item>,
    /// A turn was running: what it had not finished is left out.
    pub running: bool,
    /// How many values are masked, by the core and by this pass.
    pub masked: usize,
}

impl Export {
    /// The export of `items`, read with `redacted` (the core said
    /// `core_masked` values are masked in them), with the client's
    /// stricter pass over every string of every item and over the title.
    #[must_use]
    pub fn new(
        session: &str,
        title: Option<&str>,
        items: Vec<Item>,
        core_masked: u32,
        running: bool,
    ) -> Self {
        let filter = SecretFilter::new();
        let mut added = 0;
        let items = items
            .into_iter()
            .filter_map(|item| {
                let mut value = serde_json::to_value(&item).ok()?;
                added += mask(&mut value, &filter);
                // What does not read back after masking is left out rather
                // than written unmasked.
                serde_json::from_value(value).ok()
            })
            .collect();
        let title = title.map(|title| {
            let masked = filter.redact(title).into_owned();
            added += masks_in(&masked).saturating_sub(masks_in(title));
            masked
        });
        Self {
            session: session.to_owned(),
            title,
            items,
            running,
            masked: usize::try_from(core_masked).unwrap_or(usize::MAX) + added,
        }
    }

    /// The file's text in `format`.
    #[must_use]
    pub fn text(&self, format: Format) -> String {
        match format {
            Format::Markdown => self.markdown(),
            Format::Json => self.json(),
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
        let mut out = String::new();
        let title = self
            .title
            .as_deref()
            .map(|title| readable(title).replace('\n', " "))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Session {}", readable(&self.session)));
        let _ = writeln!(out, "# {}\n", escape_line(&title));
        let _ = write!(
            out,
            "Session `{}`, exported by Cineris. {}.",
            readable(&self.session).replace('`', ""),
            masked_sentence(self.masked)
        );
        if self.running {
            out.push_str(" A turn was running: what it had not finished is left out.");
        }
        out.push_str("\n\n");
        for item in &self.items {
            item_markdown(&mut out, item);
        }
        out
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

/// The text of `parts`, safe to read, with an image said as one.
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

fn item_markdown(out: &mut String, item: &Item) {
    match item {
        Item::UserMessage(message) => {
            out.push_str("## You\n\n");
            let _ = writeln!(out, "{}\n", parts_text(&message.parts).trim_end());
        }
        Item::AssistantMessage(message) => {
            out.push_str("## Answer\n\n");
            for part in &message.parts {
                match part {
                    Part::Reasoning(reasoning) if !reasoning.text.trim().is_empty() => {
                        out.push_str("Reasoning:\n\n");
                        for line in readable(&reasoning.text).lines() {
                            let _ = writeln!(out, "> {line}");
                        }
                        out.push('\n');
                    }
                    Part::Text(text) => {
                        let _ = writeln!(out, "{}\n", readable(&text.text).trim_end());
                    }
                    Part::ToolCall(call) => {
                        let _ = writeln!(
                            out,
                            "Tool call `{}`:\n",
                            readable(&call.name).replace('`', "")
                        );
                        let arguments =
                            serde_json::to_string_pretty(&call.arguments).unwrap_or_default();
                        fenced(out, "json", &readable(&arguments));
                    }
                    Part::Image(_) => out.push_str("[an image]\n\n"),
                    _ => {}
                }
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
            let _ = writeln!(
                out,
                "Result of `{}`, {outcome}:\n",
                readable(&result.tool).replace('`', "")
            );
            fenced(out, "text", &parts_text(&result.content));
        }
        Item::Notice(notice) => {
            for line in readable(&notice.text).lines() {
                let _ = writeln!(out, "> {line}");
            }
            out.push('\n');
        }
        Item::Summary(summary) => {
            out.push_str("## Summary of the turns before\n\n");
            let _ = writeln!(out, "{}\n", readable(&summary.text).trim_end());
        }
        _ => {}
    }
}

/// Masks every string in `value` with `filter`, and returns how many
/// masks it added.
fn mask(value: &mut Value, filter: &SecretFilter) -> usize {
    match value {
        Value::String(text) => {
            let masked = filter.redact(text);
            if let std::borrow::Cow::Owned(masked) = masked {
                let added = masks_in(&masked).saturating_sub(masks_in(text));
                *text = masked;
                added.max(1)
            } else {
                0
            }
        }
        Value::Array(items) => items.iter_mut().map(|item| mask(item, filter)).sum(),
        Value::Object(fields) => fields.values_mut().map(|item| mask(item, filter)).sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

/// How many masks `text` holds.
fn masks_in(text: &str) -> usize {
    text.matches(MASK).count()
}

#[cfg(test)]
mod tests;
