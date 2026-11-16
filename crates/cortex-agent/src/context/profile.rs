//! The project profile in a context epoch
//! (`docs/architecture/07-sessions-and-context.md`, "What goes into a
//! request"; `docs/architecture/14-adaptation.md`, "How entries are used").
//!
//! The profile is section 5 of the system prompt. Where it is kept and how
//! it is rendered are another crate's, which this one does not see: the
//! loop asks a [`ProfileSource`] for the section's text, and the core
//! fills it. Three rules hold here.
//!
//! - **The section is fixed for an epoch.** It is asked for when a prompt
//!   is built, and the prompt is sent from the epoch's record from then
//!   on. Nothing that happens to the profile changes a byte of a request's
//!   prefix.
//! - **A change is told as a notice,** at the start of a turn, as a changed
//!   instruction file is: when the section the source gives now is not the
//!   one the model has, one notice of the source `profile` carries the
//!   whole section as it stands. What the model has is the epoch's
//!   `profile` hash, then the last such notice of the epoch's history. It
//!   is outside the epoch's `base` hash, so a profile that changed between
//!   two processes never makes a resume a new epoch.
//! - **A notice of an older profile is not replayed after a summary.** The
//!   new epoch's prompt holds the profile as it is then; an older notice
//!   after it would put an old profile over the current one.
//! - **A new prompt over a history that goes on says the profile again.**
//!   A resume whose prompt the harness would now build otherwise begins an
//!   epoch and keeps its history, notices of an older profile included.
//!   They cannot be taken out of a history that goes on, so one notice of
//!   the profile as it is follows them (found by the independent read of
//!   the trust path, step 17).

use cortex_base::ContentHash;

/// Where section 5 comes from.
pub trait ProfileSource: Send + Sync {
    /// The project profile as section 5 is to hold it now, every piece of
    /// it already escaped. Empty when there is nothing to say.
    ///
    /// It blocks (it may read the project's files), and is called on the
    /// blocking pool: when a prompt is built, and at the start of each
    /// turn.
    fn section(&self) -> String;
}

/// The first line of a notice that gives the profile as it is now.
pub const PROFILE_CHANGED: &str =
    "The project profile changed. This is all of it now, in place of what you were given before:";

/// The notice that says the profile holds nothing any more.
pub const PROFILE_EMPTY: &str =
    "The project profile has no entries now. Disregard the one you were given before.";

/// The hash of a section, as an epoch records it: `None` for an empty one.
pub(crate) fn hash_of(section: &str) -> Option<ContentHash> {
    (!section.is_empty()).then(|| ContentHash::of(section.as_bytes()))
}

/// The notice that tells the model the profile as it is now.
pub(crate) fn notice_text(section: &str) -> String {
    if section.is_empty() {
        PROFILE_EMPTY.to_owned()
    } else {
        format!("{PROFILE_CHANGED}\n\n{section}")
    }
}

/// What a profile notice on the branch, read back, says the model has: the
/// hash of the section it carried, and `None` for the notice that the
/// profile is empty or for a text this build does not know as one.
pub(crate) fn told(notice: &str) -> Option<ContentHash> {
    notice
        .strip_prefix(PROFILE_CHANGED)
        .and_then(|rest| rest.strip_prefix("\n\n"))
        .and_then(hash_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_read_back_says_which_profile_the_model_has() {
        let section = "Project profile: what is known.\n\nCommands:\n- test: `cargo test`";
        assert_eq!(told(&notice_text(section)), hash_of(section));
        assert!(hash_of(section).is_some());
        // The empty profile is no profile, told or built.
        assert_eq!(notice_text(""), PROFILE_EMPTY);
        assert_eq!(told(PROFILE_EMPTY), None);
        assert_eq!(hash_of(""), None);
        // A text that is not such a notice says nothing.
        assert_eq!(told(section), None);
        assert_eq!(told(&format!("{PROFILE_CHANGED} {section}")), None);
    }
}
