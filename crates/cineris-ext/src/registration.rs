//! What an extension's script registers when its main module is first
//! evaluated: its tools, its commands and its hooks.
//!
//! It is what the registration cache keeps for an approved extension, so
//! that a session knows an extension's contributions without starting the
//! engine. A live runtime registers again when it starts, and must register
//! exactly this, or the extension is disabled (`docs/steps/28/answered.md`,
//! "The manifest and the registration cache").
//!
//! **The checks are here, once.** A live runtime meets them as a script
//! registers each definition ([`check_name`], [`check_description`],
//! [`check_schema`], [`check_filter`], [`check_timeout`], and the caps), and
//! the registration cache meets them again on every entry it reads
//! ([`Registration::check`]), so that what a session takes from the cache,
//! before any runtime starts, is what a live runtime could have registered.
//! Whether its hooks are ones the manifest allows is the manifest's question
//! ([`Registration::check_hooks`]): a live runtime asks it as each hook is
//! registered, and the core asks it of a cached entry as it takes the entry
//! for an extension (`cache::Entry::fits`).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::event::Event;

/// The most tools one extension registers.
pub const MAX_TOOLS: usize = 64;
/// The most commands one extension registers.
pub const MAX_COMMANDS: usize = 32;
/// The most hooks one extension registers.
pub const MAX_HOOKS: usize = 32;
/// The longest description of a tool or a command.
pub const MAX_DESCRIPTION: usize = 2048;
/// The largest schema of a tool's arguments, as JSON.
pub const MAX_SCHEMA_BYTES: usize = 16 * 1024;
/// The most arguments a command names.
pub const MAX_ARGUMENTS: usize = 16;
/// The most tool names a hook's filter lists.
pub const MAX_HOOK_TOOLS: usize = 64;
/// The longest tool name a hook's filter lists: a registry name, or a
/// source's names (`source/*`).
pub const MAX_FILTER_ENTRY: usize = 129;
/// The longest name of a tool, a command or a command's argument.
pub const MAX_NAME: usize = 64;
/// The longest a hook's time may be set, in milliseconds: an observer's.
pub const MAX_HOOK_TIME_MS: u64 = 600_000;

/// A name of a tool or a command, or of a command's argument: what the
/// registry takes, 1 to [`MAX_NAME`] of ASCII letters, digits, `_`, `-` and
/// `.`.
///
/// # Errors
///
/// Why it is not one, quoting it cut and escaped.
pub fn check_name(name: &str) -> Result<(), String> {
    let fine = !name.is_empty()
        && name.len() <= MAX_NAME
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        });
    if fine {
        Ok(())
    } else {
        Err(format!(
            "`{}` cannot be a name: 1 to 64 of ASCII letters, digits, `_`, `-` and `.`",
            shown(name)
        ))
    }
}

/// A description of a tool or a command: at most [`MAX_DESCRIPTION`], and
/// shown as written, so with nothing in it that acts unseen but a line
/// break and a tab.
///
/// # Errors
///
/// Why it is not one.
pub fn check_description(description: &str) -> Result<(), String> {
    if description.len() > MAX_DESCRIPTION {
        Err("a description is at most 2 KiB".to_owned())
    } else if hides_something(description) {
        Err(
            "a description holds a character that acts unseen (a control or a format character)"
                .to_owned(),
        )
    } else {
        Ok(())
    }
}

/// A tool's `input`: a JSON Schema object of at most [`MAX_SCHEMA_BYTES`]
/// as JSON, every text of it, its keys included, shown as written.
///
/// # Errors
///
/// Why it is not one.
pub fn check_schema(schema: &Json) -> Result<(), String> {
    if !schema.is_object() || schema.to_string().len() > MAX_SCHEMA_BYTES {
        return Err("`input` must be a JSON Schema object of at most 16 KiB".to_owned());
    }
    check_schema_text(schema)
}

/// Every text of a schema, its keys included, shown as written.
fn check_schema_text(schema: &Json) -> Result<(), String> {
    let found = match schema {
        Json::String(text) => hides_something(text),
        Json::Array(items) => return items.iter().try_for_each(check_schema_text),
        Json::Object(fields) => {
            for (key, value) in fields {
                if hides_something(key) {
                    return Err(SCHEMA_HIDES.to_owned());
                }
                check_schema_text(value)?;
            }
            false
        }
        _ => false,
    };
    if found {
        Err(SCHEMA_HIDES.to_owned())
    } else {
        Ok(())
    }
}

const SCHEMA_HIDES: &str =
    "`input` holds a character that acts unseen (a control or a format character)";

/// One entry of a hook's filter: a tool's registry name, or a source's
/// tools (`source/*`), 1 to [`MAX_FILTER_ENTRY`] of ASCII letters, digits,
/// `_`, `-`, `.`, `/` and `*`.
///
/// # Errors
///
/// Why it is not one, quoting it cut and escaped.
pub fn check_filter(entry: &str) -> Result<(), String> {
    let fine = !entry.is_empty()
        && entry.len() <= MAX_FILTER_ENTRY
        && entry.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | '*')
        });
    if fine {
        Ok(())
    } else {
        Err(format!(
            "`{}` cannot be a tool in a hook's filter: 1 to {MAX_FILTER_ENTRY} of ASCII letters, \
             digits, `_`, `-`, `.`, `/` and `*`",
            shown(entry)
        ))
    }
}

/// A hook's time, when it sets one: from 1 to [`MAX_HOOK_TIME_MS`].
///
/// # Errors
///
/// When it is outside that.
pub fn check_timeout(timeout_ms: Option<u64>) -> Result<(), String> {
    if timeout_ms.is_some_and(|time| time == 0 || time > MAX_HOOK_TIME_MS) {
        Err("a hook's `timeout` is from 1 to 600000 (milliseconds)".to_owned())
    } else {
        Ok(())
    }
}

/// A text of the script's as an error shows it: cut, and nothing in it
/// acting unseen.
fn shown(text: &str) -> String {
    cineris_base::visible(cineris_base::text::truncate_bytes(text, 80)).into_owned()
}

/// Whether `text` holds a character that acts unseen (a control, a format
/// character such as a direction override), a line break and a tab apart:
/// text a script registers is shown to the model and the user as written.
fn hides_something(text: &str) -> bool {
    // Shown as written only if nothing would be escaped: a control, a
    // format character, and a run of variation selectors, which hide data
    // past the first (the review of step 28's fixes).
    cineris_base::visible_keeping(text, &['\n', '\t']) != text
}

/// One tool a script registered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDef {
    /// Its name within the extension: the part after the `/`.
    pub name: String,
    /// What it does, for the model.
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input: Json,
    /// What its author says: it changes nothing. Shown to the user; it
    /// decides nothing, since an extension's tool always runs alone.
    #[serde(default)]
    pub read_only: bool,
}

/// One command a script registered: a slash command its code handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandDef {
    /// The name the user types after `/`.
    pub name: String,
    /// What it does, for the menu.
    pub description: String,
    /// Its named arguments.
    #[serde(default)]
    pub arguments: Vec<String>,
}

/// One hook a script registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookDef {
    /// The event, as its name is written (`tool.before`).
    pub event: String,
    /// For a tool event: the tools it runs on, by name or as `source/*`.
    /// Empty: every tool.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether it only observes: never waited for, its answer not read.
    #[serde(default)]
    pub observe: bool,
    /// Its time, in milliseconds, when not the default.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// Everything one script registered, in the order it registered it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    /// Its tools.
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    /// Its commands.
    #[serde(default)]
    pub commands: Vec<CommandDef>,
    /// Its hooks.
    #[serde(default)]
    pub hooks: Vec<HookDef>,
}

impl ToolDef {
    /// Whether a live runtime would have registered this tool: its name,
    /// its description and its schema.
    ///
    /// # Errors
    ///
    /// Why it would not have.
    pub fn check(&self) -> Result<(), String> {
        check_name(&self.name)?;
        check_description(&self.description)?;
        check_schema(&self.input)
    }
}

impl CommandDef {
    /// Whether a live runtime would have registered this command: its name,
    /// its description, and at most [`MAX_ARGUMENTS`] arguments, each a
    /// name.
    ///
    /// # Errors
    ///
    /// Why it would not have.
    pub fn check(&self) -> Result<(), String> {
        check_name(&self.name)?;
        check_description(&self.description)?;
        if self.arguments.len() > MAX_ARGUMENTS {
            return Err(format!(
                "`arguments` lists at most {MAX_ARGUMENTS}, and this lists {}",
                self.arguments.len()
            ));
        }
        self.arguments.iter().try_for_each(|name| check_name(name))
    }
}

impl HookDef {
    /// Whether a live runtime would have registered this hook, whatever the
    /// manifest allows: its event, at most [`MAX_HOOK_TOOLS`] entries in its
    /// filter, each one, and its time. Gives the event.
    ///
    /// # Errors
    ///
    /// Why it would not have.
    pub fn check(&self) -> Result<Event, String> {
        let event = Event::parse(&self.event).ok_or_else(|| {
            format!(
                "`{}` is not an event: {}",
                shown(&self.event),
                Event::ALL.map(Event::as_str).join(", ")
            )
        })?;
        if self.tools.len() > MAX_HOOK_TOOLS {
            return Err(format!(
                "`tools` lists at most {MAX_HOOK_TOOLS}, and this lists {}",
                self.tools.len()
            ));
        }
        self.tools
            .iter()
            .try_for_each(|entry| check_filter(entry))?;
        check_timeout(self.timeout_ms)?;
        Ok(event)
    }
}

impl Registration {
    /// Whether a live runtime could have registered all of this, whatever
    /// the manifest allows: every definition's own checks, at most
    /// [`MAX_TOOLS`] tools, [`MAX_COMMANDS`] commands and [`MAX_HOOKS`]
    /// hooks, and no tool or command name given twice. What the registration
    /// cache asks of each entry it reads.
    ///
    /// # Errors
    ///
    /// The first thing a live runtime would have refused, and why.
    pub fn check(&self) -> Result<(), String> {
        for (what, count, most) in [
            ("tools", self.tools.len(), MAX_TOOLS),
            ("commands", self.commands.len(), MAX_COMMANDS),
            ("hooks", self.hooks.len(), MAX_HOOKS),
        ] {
            if count > most {
                return Err(format!(
                    "it has {count} {what}, and an extension registers at most {most}"
                ));
            }
        }
        for (index, tool) in self.tools.iter().enumerate() {
            tool.check()
                .map_err(|why| format!("its tool `{}`: {why}", shown(&tool.name)))?;
            if self.tools[..index]
                .iter()
                .any(|earlier| earlier.name == tool.name)
            {
                return Err(format!(
                    "a tool `{}` is registered twice",
                    shown(&tool.name)
                ));
            }
        }
        for (index, command) in self.commands.iter().enumerate() {
            command
                .check()
                .map_err(|why| format!("its command `{}`: {why}", shown(&command.name)))?;
            if self.commands[..index]
                .iter()
                .any(|earlier| earlier.name == command.name)
            {
                return Err(format!(
                    "a command `{}` is registered twice",
                    shown(&command.name)
                ));
            }
        }
        for hook in &self.hooks {
            hook.check()
                .map_err(|why| format!("its hook on `{}`: {why}", shown(&hook.event)))?;
        }
        Ok(())
    }

    /// Whether every hook is on an event in `allowed`: the manifest's
    /// `capabilities.hooks`, which a live runtime asks as each hook is
    /// registered.
    ///
    /// # Errors
    ///
    /// The first hook on an event not allowed, or not an event.
    pub fn check_hooks(&self, allowed: &[Event]) -> Result<(), String> {
        for hook in &self.hooks {
            check_allowed(&hook.event, hook.check()?, allowed)?;
        }
        Ok(())
    }
}

/// Whether a hook on `event`, written `name`, is one `allowed` (the
/// manifest's `capabilities.hooks`) lets an extension register.
///
/// # Errors
///
/// When it is not, saying what the event would let the extension see.
pub fn check_allowed(name: &str, event: Event, allowed: &[Event]) -> Result<(), String> {
    if allowed.contains(&event) {
        Ok(())
    } else {
        Err(format!(
            "the extension's manifest does not allow a hook on `{}`, which lets it see {} \
             (`capabilities.hooks`)",
            name,
            event.sees()
        ))
    }
}
