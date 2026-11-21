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
//! and the registry's sources of tools, taken as the finding left the
//! registry. Asking takes this snapshot's lock alone, which is never held
//! while a script is evaluated or a folder read.
//!
//! **A text leaves no later than its tools.** The finding takes an
//! extension it withdraws out of the snapshot, its text and its source
//! together, before its tools leave the registry; and puts one it loads in
//! after its tools are there. The first turn's `turn/start` waits for the
//! first finding, and each later one's start looks again at what was found
//! and settles what changed (`Extensions::recheck`) before the prompt
//! reaches the loop, so that a turn's sections are those of its tools.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cineris_agent::{ExtensionPrompt, ExtensionSections, ExtensionsSource};
use cineris_tools::ToolRegistry;

/// Sections 6 and 7 of a session's system prompt, as its finding settled
/// them.
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
    /// No skill and no text settled yet; the sources are `registry`'s as
    /// it is now (an extension a caller handed in is there already).
    pub(crate) fn new(registry: Arc<ToolRegistry>) -> Self {
        let settled = ExtensionSections {
            sources: registry.sources(),
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
    pub(crate) fn set_skills(&self, skills: String) {
        self.settled().skills = skills;
    }

    /// Takes the extensions of `names` out of the snapshot, their texts and
    /// their sources together: called before their tools leave the
    /// registry, so that a text leaves no later than its tools.
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
    pub(crate) fn settle(&self, prompts: Vec<ExtensionPrompt>) {
        let mut kept: Vec<ExtensionPrompt> = Vec::new();
        for prompt in prompts {
            if !kept.iter().any(|one| one.name == prompt.name) {
                kept.push(prompt);
            }
        }
        let sources = self.registry.sources();
        let mut settled = self.settled();
        settled.prompts = kept;
        settled.sources = sources;
    }
}

impl ExtensionsSource for PromptParts {
    fn sections(&self) -> ExtensionSections {
        self.settled().clone()
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
            .set_source("tracker", "Issues", Vec::new())
            .unwrap();
        registry.set_source("notes", "Notes", Vec::new()).unwrap();
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
}
