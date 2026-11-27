//! What a session's skills and extensions give its system prompt: sections
//! 6 and 7 (`docs/architecture/07-sessions-and-context.md`, "What goes
//! into a request"; `docs/steps/28/answered.md`, point 7), as the loop asks
//! for them (`cineris_agent::ExtensionsSource`).
//!
//! **It answers from memory, and reads nothing.** The loop asks at every
//! turn's start and before each request. What it answers is one snapshot,
//! which the session's finding (`super::found`) settles and nothing else
//! writes: the skills' part of section 6, rendered when the skills were
//! found; each loaded extension's prompt text, from the registration cache;
//! and the registry's extensions' sources of tools, taken as the finding
//! left the registry. Asking takes this snapshot's lock alone, which is
//! never held while a script is evaluated or a folder read.
//!
//! **A text leaves no later than its tools.** The finding takes an
//! extension it withdraws out of the snapshot, its text and its source
//! together, before its tools leave the registry; and puts one it loads in
//! after its tools are there. The first turn's `turn/start` waits for the
//! first finding, and each later one's start looks again at what was found
//! and settles what changed (`Extensions::recheck`) before the prompt
//! reaches the loop, so that a turn's sections are those of its tools.
//!
//! **The MCP servers' sources are read from the registry as it is** when
//! the loop asks (step 30), behind the registry's own lock: a server has no
//! text for section 7, so nothing of it has to leave with its tools, and its
//! lifecycle changes its source whenever its tools change. So section 6
//! lists the servers in every build, the one without the script engine
//! included, where no finding runs. Each is listed by the description its
//! entry gives the registry, never by the server's own text
//! (`cineris_mcp::register`).

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cineris_agent::{ExtensionPrompt, ExtensionSections, ExtensionsSource};
use cineris_tools::{SourceKind, ToolRegistry};

/// Sections 6 and 7 of a session's system prompt: the skills, the
/// extensions' texts and their sources as its finding settled them, and
/// the MCP servers' sources as the registry holds them.
pub(crate) struct PromptParts {
    registry: Arc<ToolRegistry>,
    settled: Mutex<ExtensionSections>,
}

impl std::fmt::Debug for PromptParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptParts").finish_non_exhaustive()
    }
}

impl PromptParts {
    /// No skill and no text settled yet; the extensions' sources are
    /// `registry`'s as it is now (an extension a caller handed in is there
    /// already).
    pub(crate) fn new(registry: Arc<ToolRegistry>) -> Self {
        let settled = ExtensionSections {
            sources: registry.sources_of(SourceKind::Extension),
            ..ExtensionSections::default()
        };
        Self {
            registry,
            settled: Mutex::new(settled),
        }
    }

    fn settled(&self) -> MutexGuard<'_, ExtensionSections> {
        self.settled.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The skills' part of section 6, as it was rendered.
    #[cfg_attr(
        not(feature = "scripts"),
        allow(
            dead_code,
            reason = "only the finding of the scripts' build settles it"
        )
    )]
    pub(crate) fn set_skills(&self, skills: String) {
        self.settled().skills = skills;
    }

    /// Takes the extensions of `names` out of the snapshot, their texts and
    /// their sources together: called before their tools leave the
    /// registry, so that a text leaves no later than its tools.
    #[cfg_attr(
        not(feature = "scripts"),
        allow(
            dead_code,
            reason = "only the finding of the scripts' build withdraws one"
        )
    )]
    pub(crate) fn withdraw(&self, names: &[String]) {
        let mut settled = self.settled();
        settled
            .prompts
            .retain(|prompt| !names.contains(&prompt.name));
        settled
            .sources
            .retain(|source| !names.contains(&source.name));
    }

    /// The loaded extensions' texts, and the registry's sources as they are
    /// now, together: called once the registry holds what was loaded. One
    /// text a name, the first given.
    #[cfg_attr(
        not(feature = "scripts"),
        allow(
            dead_code,
            reason = "only the finding of the scripts' build settles them"
        )
    )]
    pub(crate) fn settle(&self, prompts: Vec<ExtensionPrompt>) {
        let mut kept: Vec<ExtensionPrompt> = Vec::new();
        for prompt in prompts {
            if !kept.iter().any(|one| one.name == prompt.name) {
                kept.push(prompt);
            }
        }
        let sources = self.registry.sources_of(SourceKind::Extension);
        let mut settled = self.settled();
        settled.prompts = kept;
        settled.sources = sources;
    }
}

impl ExtensionsSource for PromptParts {
    fn sections(&self) -> ExtensionSections {
        let mut sections = self.settled().clone();
        // A name is one source's, of one kind (`ToolRegistry::set_source`):
        // a server and an extension are never both listed under it.
        sections
            .sources
            .extend(self.registry.sources_of(SourceKind::Mcp));
        sections
            .sources
            .sort_by(|one, other| one.name.cmp(&other.name));
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(name: &str, text: &str) -> ExtensionPrompt {
        ExtensionPrompt {
            name: name.to_owned(),
            text: text.to_owned(),
        }
    }

    fn shown(sections: &ExtensionSections) -> (Vec<String>, Vec<String>) {
        (
            sections
                .prompts
                .iter()
                .map(|prompt| prompt.text.clone())
                .collect(),
            sections
                .sources
                .iter()
                .map(|source| source.name.clone())
                .collect(),
        )
    }

    /// A text and its source leave the snapshot together, while the
    /// registry still has the extension's tools; and a name has one text.
    #[test]
    fn a_text_and_its_source_leave_together_and_a_name_has_one_text() {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .set_source("tracker", SourceKind::Extension, "Issues", Vec::new())
            .unwrap();
        registry
            .set_source("notes", SourceKind::Extension, "Notes", Vec::new())
            .unwrap();
        let parts = PromptParts::new(Arc::clone(&registry));
        parts.settle(vec![
            prompt("tracker", "Use the tracker."),
            prompt("notes", "Write notes."),
            prompt("tracker", "Another."),
        ]);
        assert_eq!(
            shown(&parts.sections()),
            (
                vec!["Use the tracker.".to_owned(), "Write notes.".to_owned()],
                vec!["notes".to_owned(), "tracker".to_owned()]
            )
        );
        parts.withdraw(&["tracker".to_owned()]);
        assert_eq!(registry.sources().len(), 2, "the registry is not told");
        assert_eq!(
            shown(&parts.sections()),
            (vec!["Write notes.".to_owned()], vec!["notes".to_owned()])
        );
    }

    /// An MCP server's source is listed as the registry holds it when the
    /// loop asks, beside the extensions' as the finding settled them, in
    /// name order: no finding has to run for it, and none drops it.
    #[test]
    fn a_servers_source_is_listed_as_the_registry_holds_it() {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .set_source("notes", SourceKind::Extension, "Notes", Vec::new())
            .unwrap();
        // A server's source there before the parts are made is listed
        // once: the snapshot takes the extensions' alone.
        registry
            .set_source("early", SourceKind::Mcp, "Early", Vec::new())
            .unwrap();
        let parts = PromptParts::new(Arc::clone(&registry));
        assert_eq!(shown(&parts.sections()).1, ["early", "notes"]);
        assert!(registry.remove_source("early", SourceKind::Mcp));
        assert_eq!(shown(&parts.sections()).1, ["notes"]);
        registry
            .set_source("docs", SourceKind::Mcp, "Product docs", Vec::new())
            .unwrap();
        registry
            .set_source("zed", SourceKind::Mcp, "Editor", Vec::new())
            .unwrap();
        assert_eq!(shown(&parts.sections()).1, ["docs", "notes", "zed"]);
        // A finding settles the extensions' alone.
        parts.settle(Vec::new());
        parts.withdraw(&["notes".to_owned(), "docs".to_owned()]);
        assert_eq!(shown(&parts.sections()).1, ["docs", "zed"]);
        assert_eq!(
            parts.sections().sources[0].description,
            "Product docs",
            "the entry's text, as the registry was given it"
        );
        assert!(registry.remove_source("docs", SourceKind::Mcp));
        assert_eq!(shown(&parts.sections()).1, ["zed"]);
    }
}
