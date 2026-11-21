//! Sections 6 and 7 of the system prompt: what extensions give the model
//! (`docs/architecture/07-sessions-and-context.md`, "What goes into a
//! request"; `docs/steps/28/answered.md`, point 7).
//!
//! What a session's extensions are, and its skills, are another crate's,
//! which this one does not see: the loop asks an [`ExtensionsSource`] for
//! the parts of both sections, and the core fills it. The sections are
//! written here ([`super::prompt`]), with their caps.
//!
//! - **Section 6, the index:** the skills' part, which the core renders
//!   (`cineris_ext::skills::render`, within [`SKILLS_CAP`]), then each
//!   source of dynamic tools with its tool count and the first line of its
//!   description, then one line on `tool_search`. [`INDEX_CAP`] in all: the
//!   sources that do not fit are counted. Custom commands are not listed:
//!   they are for the user to type, and the model cannot run one. **It is
//!   outside the epoch's `base` hash,** as sections 4 and 5 are, so a skill
//!   added between two processes does not make a resume a new epoch.
//! - **Section 7:** the `prompt` text of each enabled extension, from the
//!   registration cache (so it is the approved text), ordered by name, each
//!   under a line that names the extension and says the text is its
//!   author's. [`EXTENSION_TEXT_CAP`] each and [`EXTENSIONS_CAP`] in all.
//!   **It is inside `base`:** it is text with the system prompt's authority,
//!   which the user may take back.
//!
//! Text from a repository has what acts unseen taken out: a source's
//! description is one line, and an extension's text keeps its line breaks
//! and tabs and nothing else that acts unseen. A name shown as code is
//! written with [`cineris_base::in_code`].

use cineris_tools::SourceSummary;

/// The most section 6 holds, in bytes.
pub const INDEX_CAP: usize = 8 * 1024;

/// The most the skills' part of section 6 may be, in bytes: the section's
/// cap less the room kept for the sources of tools, so that their heading,
/// the line that counts those not listed and the line on `tool_search`
/// always fit, with a few sources. The core renders the skills within it.
pub const SKILLS_CAP: usize = INDEX_CAP - 1024;

/// The most of one extension's text section 7 holds, in bytes: what a
/// manifest's prompt file may hold.
pub const EXTENSION_TEXT_CAP: usize = 4 * 1024;

/// The most section 7 holds in all, in bytes.
pub const EXTENSIONS_CAP: usize = 16 * 1024;

/// Where sections 6 and 7 come from.
pub trait ExtensionsSource: Send + Sync {
    /// What the two sections are to hold now.
    ///
    /// It may block, and is called on the blocking pool: when a prompt is
    /// built.
    fn sections(&self) -> ExtensionSections;
}

/// What sections 6 and 7 are made from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionSections {
    /// Section 6's list of skills, as the core renders it within
    /// [`SKILLS_CAP`]: a heading, one line a skill, and a line that counts
    /// the skills not listed. Empty when there is none. A longer text is cut
    /// at a character's edge.
    pub skills: String,
    /// The sources of dynamic tools (`ToolRegistry::sources`). Section 6
    /// lists them by name, and leaves out a source with no tool.
    pub sources: Vec<SourceSummary>,
    /// The `prompt` text of each enabled extension that has one, as the
    /// registration cache holds it. Section 7 lists them by name.
    pub prompts: Vec<ExtensionPrompt>,
}

/// One extension's text for section 7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPrompt {
    /// The extension's name, from its checked manifest. One extension a
    /// name: a project's of a name the user's has is not loaded.
    pub name: String,
    /// The text of its prompt file, as it was approved.
    pub text: String,
}
