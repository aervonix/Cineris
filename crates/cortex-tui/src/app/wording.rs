//! What an error says to the person: the core's own sentence, and a dim one
//! on what to do next (`docs/architecture/09-tui.md`, "Elements": Error).

use cortex_protocol::error::code;
use cortex_protocol::{ErrorData, ErrorKind};

/// The error's message and what to do next, when there is something.
#[must_use]
pub(crate) fn error(error: &ErrorData) -> (String, Option<String>) {
    let hint = match error.code.as_str() {
        "day_limit_needs_stats" => Some(
            "A day's limit needs the day's usage, which the statistics keep: turn them on \
             (stats.enabled = true), or take [limits.day] out of the configuration.",
        ),
        "stats_newer" => Some(
            "A newer version of Cortex-Harness wrote the statistics. This one leaves them as \
             they are, and records nothing until it is updated.",
        ),
        code::UNKNOWN_VALUE => Some(
            "This build does not know that value: check its spelling, or update \
             Cortex-Harness.",
        ),
        "compaction_needed" => Some("/compact makes room, and /new starts a new session."),
        code::HISTORY_CHANGING => Some("The session's history was being changed: send it again."),
        code::TURN_RUNNING => Some("Wait for the turn to end, or stop it with esc."),
        code::SESSION_NOT_OPEN | code::UNKNOWN_SESSION => {
            Some("/new starts a new session.")
        }
        "enforced_spend_limit_reached" => {
            Some("The provider's spending limit was reached: it is set in the provider's console.")
        }
        _ => None,
    };
    let hint = hint.map(str::to_owned).or_else(|| {
        match error.kind {
            ErrorKind::Auth => Some(
                "Give the provider's key with `cortex auth login`, or in its environment \
                 variable.",
            ),
            ErrorKind::Config => {
                Some("Fix the file the message names; the configuration in force stays.")
            }
            ErrorKind::Transient => Some("It was tried again and kept failing: try again soon."),
            ErrorKind::Limit => Some("A limit of [limits] was reached; it is the user's to raise."),
            ErrorKind::ContextOverflow => {
                Some("/compact makes room, and /new starts a new session.")
            }
            _ => None,
        }
        .map(str::to_owned)
    });
    (error.message.clone(), hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_this_step_words_have_a_next_step() {
        for code in ["day_limit_needs_stats", "stats_newer", "unknown_value"] {
            let (message, hint) = error(&ErrorData::new(ErrorKind::Usage, code, "It failed."));
            assert_eq!(message, "It failed.");
            assert!(hint.is_some(), "{code} has no next step");
        }
        let (_, hint) = error(&ErrorData::new(ErrorKind::Internal, "x", "A bug."));
        assert_eq!(hint, None);
    }
}
