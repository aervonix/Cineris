//! Section 5 of the system prompt: the project profile
//! (`docs/architecture/07-sessions-and-context.md`, "What goes into a
//! request"; `docs/architecture/14-adaptation.md`, "How entries are used").
//!
//! [`render`] turns a [`Snapshot`] into the section's text. Four rules
//! hold for it:
//!
//! - **Only entries in use are in it:** active, and not past their expiry.
//!   A proposal, a disabled entry, an expired one and a shared entry this
//!   user has not accepted never are.
//! - **The same entries give the same bytes.** The order is fixed
//!   (commands by purpose, then conventions, flaky tests and facts, each in
//!   the order the files hold them), and nothing in the text depends on a
//!   clock, a count or how long a command takes: confirming an entry does
//!   not change the section, so it does not change the prompt.
//! - **Every piece of every entry is written through
//!   [`cortex_base::visible`]**, so an entry is one list line: it cannot
//!   begin a line, a heading or a message of its own, whatever its text
//!   holds.
//! - **It stays inside its budget**, counted at a fixed three bytes a
//!   token, and says so in a last line when entries were left out.

use cortex_base::{ContentHash, UtcTime, visible};

use crate::entry::{Basis, Entry, Kind};
use crate::project::Snapshot;

/// The section's budget in tokens, unless configured otherwise.
pub const DEFAULT_TOKENS: u32 = 1_500;

/// How many bytes the budget counts as one token. Fixed, so that the same
/// entries always give the same section: the ratio the loop measures for a
/// model moves with every request.
pub const BYTES_PER_TOKEN: usize = 3;

/// The section's first line.
pub const HEADING: &str = "Project profile: what is known about this project from earlier \
    sessions, each entry learned by Cortex-Harness or written by the user, and none of it part \
    of the present task.";

/// Room kept for the line that says entries were left out.
const CUT_ROOM: usize = 96;

/// Section 5, rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    /// The text. Empty when no entry is in use: the section is then left
    /// out of the prompt.
    pub text: String,
    /// How many entries it holds.
    pub shown: usize,
    /// How many entries in use the budget left out.
    pub left_out: usize,
    /// How many entries in use were held back by the caller's filter: a
    /// line the session's secret filter would mask.
    pub withheld: usize,
}

impl Section {
    /// Whether there is nothing to put in the prompt.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The hash of the text: what an epoch records of the profile its
    /// prompt holds. `None` for an empty section.
    #[must_use]
    pub fn hash(&self) -> Option<ContentHash> {
        (!self.text.is_empty()).then(|| ContentHash::of(self.text.as_bytes()))
    }
}

fn group_title(kind: Kind) -> &'static str {
    match kind {
        Kind::Command => "Commands:",
        Kind::Convention => "Conventions:",
        Kind::Flaky => "Flaky tests:",
        Kind::Fact => "Facts:",
    }
}

/// One entry as its list line.
fn line(entry: &Entry) -> String {
    let text = visible(&entry.text);
    match (&entry.kind, &entry.command) {
        (Kind::Command, Some(command)) => {
            let mut out = format!(
                "- {}: `{}`",
                command.purpose.as_str(),
                visible(&command.run)
            );
            if let Some(cwd) = &command.cwd {
                out.push_str(&format!(" (in `{}`)", visible(cwd)));
            }
            // What an observed command is, its purpose already says.
            if entry.basis != Some(Basis::Observed) {
                out.push_str(&format!(". {text}"));
            }
            out
        }
        _ => format!("- {text}"),
    }
}

/// Renders the entries of `snapshot` that are in use at `now` into section
/// 5, within `tokens`.
#[must_use]
pub fn render(snapshot: &Snapshot, now: UtcTime, tokens: u32) -> Section {
    render_with(snapshot, now, tokens, &|_| true)
}

/// [`render`], leaving out every entry whose line `keep` refuses: the core
/// passes the session's secret filter, since a prompt the log would mask
/// cannot be gone on with after a resume.
#[must_use]
pub fn render_with(
    snapshot: &Snapshot,
    now: UtcTime,
    tokens: u32,
    keep: &dyn Fn(&str) -> bool,
) -> Section {
    let budget = usize::try_from(tokens)
        .unwrap_or(usize::MAX)
        .saturating_mul(BYTES_PER_TOKEN);
    let mut section = Section::default();
    // Commands by purpose, then the other kinds; within each, the order
    // the files hold them in, the private store's first.
    let mut ordered: Vec<(Kind, String)> = Vec::new();
    for kind in Kind::ALL {
        let mut of_kind: Vec<&Entry> = snapshot
            .in_use(now)
            .filter(|entry| entry.kind == kind)
            .collect();
        if kind == Kind::Command {
            of_kind.sort_by_key(|entry| entry.command.as_ref().map(|command| command.purpose));
        }
        for entry in of_kind {
            let line = line(entry);
            if keep(&line) {
                ordered.push((kind, line));
            } else {
                section.withheld += 1;
            }
        }
    }
    if ordered.is_empty() {
        return section;
    }
    let cost = |kind: Kind, line: &str, first: bool| {
        // A blank line and the group's title before its first line.
        let title = if first {
            2 + group_title(kind).len()
        } else {
            0
        };
        title + 1 + line.len()
    };
    let whole: usize = HEADING.len()
        + ordered
            .iter()
            .enumerate()
            .map(|(index, (kind, line))| {
                cost(*kind, line, index == 0 || ordered[index - 1].0 != *kind)
            })
            .sum::<usize>();
    let room = if whole <= budget {
        budget
    } else {
        budget.saturating_sub(CUT_ROOM)
    };
    let mut text = String::from(HEADING);
    let mut last: Option<Kind> = None;
    for (kind, line) in &ordered {
        let first = last != Some(*kind);
        if text.len() + cost(*kind, line, first) > room {
            break;
        }
        if first {
            text.push_str("\n\n");
            text.push_str(group_title(*kind));
        }
        text.push('\n');
        text.push_str(line);
        last = Some(*kind);
        section.shown += 1;
    }
    section.left_out = ordered.len() - section.shown;
    if section.shown == 0 {
        // Not even one entry fits: a heading over nothing says nothing.
        return section;
    }
    if section.left_out > 0 {
        text.push_str(&format!(
            "\n\n({} more {} not shown, to keep this within its size.)",
            section.left_out,
            if section.left_out == 1 {
                "entry is"
            } else {
                "entries are"
            }
        ));
    }
    section.text = text;
    section
}
