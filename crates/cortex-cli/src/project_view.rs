//! What `cortex project` prints, and how it finds the entry an id names
//! (`docs/architecture/14-adaptation.md`, "Project knowledge: profile and
//! memory").
//!
//! The listing is for a person who has to decide: whether to accept a
//! proposal, and what an entry lets the harness do. So a command's line
//! and directory are printed in full, never cut, and each entry says where
//! it came from. The caller prints the text through `visible`, as every
//! subcommand prints what came from a file.

use std::fmt::Write as _;

use cortex_protocol::ids::EntryId;
use cortex_protocol::project::{
    EntryBasis, EntryCheckKind, EntryKind, EntryScope, EntrySourceKind, EntryStatus, ProjectEntry,
};

/// Whether an entry runs without asking once the project is trusted, and
/// is offered to verification: the user accepted it, or wrote it in their
/// own store.
fn accepted(entry: &ProjectEntry) -> bool {
    entry.accepted_at.is_some()
        || (entry.scope == EntryScope::Private && entry.source.kind == EntrySourceKind::User)
}

/// An entry's standing, in a few words.
fn standing(entry: &ProjectEntry) -> String {
    let mut words = match &entry.status {
        EntryStatus::Proposed => "proposed".to_owned(),
        EntryStatus::Disabled => "disabled".to_owned(),
        EntryStatus::Active if entry.kind != EntryKind::Command => "active".to_owned(),
        // A command in the profile is one thing; one that runs without
        // asking is another.
        EntryStatus::Active if accepted(entry) => "active, accepted".to_owned(),
        EntryStatus::Active => "active, asks".to_owned(),
        other => other.as_str().to_owned(),
    };
    if entry.scope == EntryScope::Shared {
        words.push_str(", shared");
    }
    words
}

/// What the entry says: a command's purpose, line and directory in full,
/// then its text.
fn said(entry: &ProjectEntry) -> String {
    let mut said = String::new();
    match &entry.command {
        Some(command) if entry.kind == EntryKind::Command => {
            let _ = write!(said, "{}: `{}`", command.purpose.as_str(), command.command);
            if let Some(cwd) = &command.cwd {
                let _ = write!(said, " (in `{cwd}`)");
            }
            if !entry.text.is_empty() {
                let _ = write!(said, "  {}", entry.text);
            }
        }
        _ => {
            let _ = write!(said, "{}: {}", entry.kind.as_str(), entry.text);
        }
    }
    said
}

/// Where the entry came from and how long it stands: the second line.
fn origin(entry: &ProjectEntry) -> String {
    let mut parts = Vec::new();
    parts.push(match (&entry.scope, &entry.basis, &entry.source.kind) {
        (EntryScope::Shared, _, _) => "from the repository's .cortex/profile.toml".to_owned(),
        (_, Some(EntryBasis::Observed), _) => match entry.observations {
            Some(1) => "observed once".to_owned(),
            Some(count) => format!("observed {count} times"),
            None => "observed".to_owned(),
        },
        (_, Some(EntryBasis::Proposed), _) => "proposed by the model".to_owned(),
        (_, _, EntrySourceKind::User) => "written by you".to_owned(),
        _ => "learned".to_owned(),
    });
    if let Some(evidence) = &entry.evidence {
        parts.push(format!("evidence: {evidence}"));
    }
    if let Some(check) = &entry.check {
        let shown = match &check.kind {
            EntryCheckKind::Exists => format!("`{}` exists", check.path.as_deref().unwrap_or("")),
            EntryCheckKind::Contains => format!(
                "`{}` contains `{}`",
                check.path.as_deref().unwrap_or(""),
                check.pattern.as_deref().unwrap_or("")
            ),
            EntryCheckKind::Command => {
                format!("`{}` succeeds", check.command.as_deref().unwrap_or(""))
            }
            other => other.as_str().to_owned(),
        };
        parts.push(format!("check (not run): {shown}"));
    }
    if entry.status == EntryStatus::Active
        && let Some(expires) = &entry.expires_at
    {
        // The day is enough.
        let day = expires.as_str().get(..10).unwrap_or(expires.as_str());
        parts.push(format!("until {day}"));
    }
    parts.join("; ")
}

/// Whether an entry is of a group of the listing.
type Belongs<'a> = &'a dyn Fn(&ProjectEntry) -> bool;

/// The listing: proposals first, since they wait for the reader, then
/// what is in use, then, when `all`, what is turned off. `kind` keeps one
/// kind.
pub(crate) fn listing(
    entries: &[ProjectEntry],
    problems: &[String],
    all: bool,
    kind: Option<&EntryKind>,
    name: &str,
) -> String {
    let shown: Vec<&ProjectEntry> = entries
        .iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == *kind))
        .collect();
    let mut out = String::new();
    let groups: [(&str, Belongs<'_>); 3] = [
        ("Waiting for your review", &|entry| {
            entry.status == EntryStatus::Proposed
        }),
        ("In use", &|entry| entry.status == EntryStatus::Active),
        ("Turned off", &|entry| {
            all && !matches!(entry.status, EntryStatus::Proposed | EntryStatus::Active)
        }),
    ];
    for (title, belongs) in groups {
        let group: Vec<&&ProjectEntry> = shown.iter().filter(|entry| belongs(entry)).collect();
        if group.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{title}:");
        for entry in group {
            let _ = writeln!(
                out,
                "  {}  [{}]  {}",
                entry.id,
                standing(entry),
                said(entry)
            );
            let _ = writeln!(out, "      {}", origin(entry));
        }
    }
    if out.is_empty() {
        out.push_str("Nothing is known about this project yet.\n");
    }
    let hidden = shown
        .iter()
        .filter(|entry| !all && entry.status == EntryStatus::Disabled)
        .count();
    if hidden > 0 {
        let _ = writeln!(
            out,
            "{hidden} turned off: `{name} project list --all` shows them."
        );
    }
    if shown
        .iter()
        .any(|entry| entry.status == EntryStatus::Proposed)
    {
        let _ = writeln!(
            out,
            "`{name} project accept ID…` puts a proposal to use; `{name} project delete ID…` \
             drops one of yours."
        );
    }
    for problem in problems {
        let _ = writeln!(out, "problem: {problem}");
    }
    out
}

/// The entries of the repository's file that wait for this user: what
/// `accept --shared` accepts. Not a proposal of the user's own store (the
/// model's, to be read one by one), and not a shared entry the user turned
/// off.
pub(crate) fn waiting_shared(entries: &[ProjectEntry]) -> Vec<EntryId> {
    entries
        .iter()
        .filter(|entry| entry.scope == EntryScope::Shared && entry.status == EntryStatus::Proposed)
        .map(|entry| entry.id.clone())
        .collect()
}

/// The entry `given` names: its id, or the start of exactly one id.
pub(crate) fn resolve(entries: &[ProjectEntry], given: &str) -> Result<EntryId, String> {
    if let Some(entry) = entries.iter().find(|entry| entry.id.as_str() == given) {
        return Ok(entry.id.clone());
    }
    let begun: Vec<&ProjectEntry> = entries
        .iter()
        .filter(|entry| !given.is_empty() && entry.id.as_str().starts_with(given))
        .collect();
    match begun.as_slice() {
        [entry] => Ok(entry.id.clone()),
        [] => Err(format!(
            "no entry of this project has the id '{given}': `project list --all` shows them"
        )),
        several => Err(format!(
            "'{given}' begins {} ids: {}",
            several.len(),
            several
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use cortex_protocol::ids::Timestamp;
    use cortex_protocol::project::{
        CommandPurpose, EntryCheck, EntryCommand, EntrySource, EntrySourceKind,
    };

    use super::*;

    fn entry(id: &str, kind: EntryKind, status: EntryStatus, text: &str) -> ProjectEntry {
        ProjectEntry {
            id: EntryId::new(id),
            kind,
            text: text.to_owned(),
            command: None,
            evidence: None,
            source: EntrySource {
                kind: EntrySourceKind::User,
                session_id: None,
                item_id: None,
                at: None,
            },
            basis: None,
            status,
            scope: EntryScope::Private,
            confirmed_at: None,
            expires_at: None,
            check: None,
            accepted_at: None,
            observations: None,
        }
    }

    fn observed(id: &str, line: &str, count: u32, accepted: bool) -> ProjectEntry {
        let mut entry = entry(
            id,
            EntryKind::Command,
            EntryStatus::Active,
            "The project's test command.",
        );
        entry.command = Some(EntryCommand {
            purpose: CommandPurpose::Test,
            command: line.to_owned(),
            cwd: Some("crates/app".to_owned()),
            usual_duration_ms: Some(4_000),
            read_only: false,
        });
        entry.source.kind = EntrySourceKind::Learned;
        entry.basis = Some(EntryBasis::Observed);
        entry.observations = Some(count);
        entry.expires_at = Some(Timestamp::new("2027-02-14T09:00:00Z"));
        entry.accepted_at = accepted.then(|| Timestamp::new("2026-11-16T09:30:00Z"));
        entry
    }

    /// The listing says of each entry what a person needs to decide: a
    /// command's line and directory in full, whether it asks, where it
    /// came from, and how long it stands.
    #[test]
    fn the_listing_says_what_each_entry_is_and_where_it_came_from() {
        let long = format!("cargo nextest run {}", "--features a,b ".repeat(8).trim());
        let mut proposal = entry(
            "k_4",
            EntryKind::Fact,
            EntryStatus::Proposed,
            "The API is in crates/api.",
        );
        proposal.source.kind = EntrySourceKind::Learned;
        proposal.basis = Some(EntryBasis::Proposed);
        proposal.evidence = Some("crates/api/src/lib.rs, line 1".to_owned());
        proposal.check = Some(EntryCheck {
            kind: EntryCheckKind::Exists,
            path: Some("crates/api".to_owned()),
            pattern: None,
            command: None,
        });
        let mut shared = entry(
            "s_0123456789abcdef0123456789abcdef",
            EntryKind::Convention,
            EntryStatus::Proposed,
            "Errors are returned.",
        );
        shared.scope = EntryScope::Shared;
        let entries = [
            observed("k_1", &long, 5, false),
            observed("k_2", "cargo test", 1, true),
            entry("k_3", EntryKind::Convention, EntryStatus::Active, "Tabs."),
            proposal,
            shared,
            entry("k_9", EntryKind::Fact, EntryStatus::Disabled, "Old."),
        ];
        let out = listing(&entries, &[], false, None, "cortex");
        assert_eq!(
            out,
            format!(
                "\
Waiting for your review:
  k_4  [proposed]  fact: The API is in crates/api.
      proposed by the model; evidence: crates/api/src/lib.rs, line 1; check (not run): `crates/api` exists
  s_0123456789abcdef0123456789abcdef  [proposed, shared]  convention: Errors are returned.
      from the repository's .cortex/profile.toml
In use:
  k_1  [active, asks]  test: `{long}` (in `crates/app`)  The project's test command.
      observed 5 times; until 2027-02-14
  k_2  [active, accepted]  test: `cargo test` (in `crates/app`)  The project's test command.
      observed once; until 2027-02-14
  k_3  [active]  convention: Tabs.
      written by you
1 turned off: `cortex project list --all` shows them.
`cortex project accept ID…` puts a proposal to use; `cortex project delete ID…` drops one of yours.
"
            )
        );
        // The command's line is whole, however long.
        assert!(out.contains(&long));

        let all = listing(&entries, &["a problem".to_owned()], true, None, "cx");
        assert!(all.contains("Turned off:\n  k_9  [disabled]  fact: Old.\n"));
        assert!(!all.contains("turned off: `"));
        assert!(all.ends_with("problem: a problem\n"));
        assert!(all.contains("`cx project accept ID…`"));

        let facts = listing(&entries, &[], true, Some(&EntryKind::Fact), "cortex");
        assert!(facts.contains("k_4") && facts.contains("k_9"));
        assert!(!facts.contains("k_1") && !facts.contains("k_3"));

        assert_eq!(
            listing(&[], &[], false, None, "cortex"),
            "Nothing is known about this project yet.\n"
        );
    }

    /// A user-written command in the user's own store runs without asking
    /// (once the project is trusted): it says so, though nothing recorded
    /// an acceptance.
    #[test]
    fn a_command_the_user_wrote_is_accepted() {
        let mut written = observed("k_1", "make test", 1, false);
        written.source.kind = EntrySourceKind::User;
        written.basis = None;
        written.observations = None;
        assert_eq!(standing(&written), "active, accepted");
        // In the repository's file it is this user's only once accepted.
        written.scope = EntryScope::Shared;
        assert_eq!(standing(&written), "active, asks, shared");
    }

    #[test]
    fn accepting_the_shared_entries_takes_the_repositorys_proposals_and_nothing_else() {
        let shared = |id: &str, status: EntryStatus| {
            let mut entry = entry(id, EntryKind::Convention, status, "x");
            entry.scope = EntryScope::Shared;
            entry
        };
        let entries = [
            // The model's proposal, in the user's own store: read one by one.
            entry("k_1", EntryKind::Fact, EntryStatus::Proposed, "a"),
            shared("s_aa", EntryStatus::Proposed),
            shared("s_bb", EntryStatus::Active),
            // Turned off by this user: not turned on again in passing.
            shared("s_cc", EntryStatus::Disabled),
            shared("s_dd", EntryStatus::Proposed),
        ];
        let ids: Vec<String> = waiting_shared(&entries)
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(ids, ["s_aa", "s_dd"]);
    }

    #[test]
    fn an_id_is_named_whole_or_by_a_start_that_only_one_has() {
        let entries = [
            entry("k_1", EntryKind::Fact, EntryStatus::Active, "a"),
            entry("k_12", EntryKind::Fact, EntryStatus::Active, "b"),
            entry("k_13", EntryKind::Fact, EntryStatus::Active, "c"),
            entry(
                "s_0123456789abcdef0123456789abcdef",
                EntryKind::Fact,
                EntryStatus::Proposed,
                "d",
            ),
        ];
        let named = |given: &str| resolve(&entries, given).map(|id| id.as_str().to_owned());
        // The whole id wins over the ids it begins.
        assert_eq!(named("k_1").as_deref(), Ok("k_1"));
        assert_eq!(named("k_12").as_deref(), Ok("k_12"));
        assert_eq!(
            named("s_01").as_deref(),
            Ok("s_0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            named("k_").unwrap_err(),
            "'k_' begins 3 ids: k_1, k_12, k_13"
        );
        assert!(
            named("k_7")
                .unwrap_err()
                .contains("no entry of this project has the id 'k_7'")
        );
        assert!(named("").is_err());
    }
}
