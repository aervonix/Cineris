//! What `cortex project` prints, and how it finds the entry an id names
//! (`docs/architecture/14-adaptation.md`, "Project knowledge: profile and
//! memory").
//!
//! The listing is for a person who has to decide: whether to accept a
//! proposal, and what an entry lets the harness do. So a command's line
//! and directory are printed in full, never cut, and each entry says where
//! it came from.
//!
//! **An entry is two lines, whatever it holds.** Its text, its evidence
//! and its check come from a file anyone can write, or from the model, so
//! each is escaped here, a line break included (`cortex_base::visible`):
//! a text cannot begin a line that reads as another entry, or push the
//! entry above it off the screen. The caller prints the whole through the
//! binary's own `visible`, which keeps the listing's own line breaks.

use std::fmt::Write as _;

use cortex_base::visible;
use cortex_protocol::ids::EntryId;
use cortex_protocol::project::{
    EntryBasis, EntryCheckKind, EntryKind, EntryScope, EntrySourceKind, EntryStatus, ProjectEntry,
};

/// Whether the user accepted the entry, or wrote it in their own store:
/// for a command, what lets it run without asking once the project is
/// trusted.
fn accepted(entry: &ProjectEntry) -> bool {
    entry.accepted_at.is_some()
        || (entry.scope == EntryScope::Private && entry.source.kind == EntrySourceKind::User)
}

/// Whether the entry is past its expiry at `now`, both written as the
/// protocol writes a time (to the second: a fraction sorts just before
/// its own second, which is near enough for a day's precision).
fn expired(entry: &ProjectEntry, now: &str) -> bool {
    entry
        .expires_at
        .as_ref()
        .is_some_and(|expires| expires.as_str() <= now)
}

/// An entry's standing, in a few words.
fn standing(entry: &ProjectEntry, now: &str) -> String {
    let mut words = match &entry.status {
        EntryStatus::Proposed => "proposed".to_owned(),
        EntryStatus::Disabled => "turned off".to_owned(),
        // Out of use by the clock, and not marked yet.
        EntryStatus::Active if expired(entry, now) => "expired".to_owned(),
        EntryStatus::Active if entry.kind != EntryKind::Command => "active".to_owned(),
        // A command in the profile is one thing; one that runs without
        // asking is another.
        EntryStatus::Active if accepted(entry) => "active, accepted".to_owned(),
        EntryStatus::Active => "active, asks".to_owned(),
        other => visible(other.as_str()).into_owned(),
    };
    if entry.scope == EntryScope::Shared {
        words.push_str(", shared");
    }
    words
}

/// What the entry says, on one line: a command's line and directory in
/// full, then its text.
fn said(entry: &ProjectEntry) -> String {
    let label = match (&entry.command, &entry.kind) {
        (Some(command), EntryKind::Command) => command.purpose.as_str(),
        _ => entry.kind.as_str(),
    };
    let mut said = format!("{}:", visible(label));
    if let Some(command) = &entry.command {
        let _ = write!(said, " `{}`", visible(&command.command));
        if let Some(cwd) = &command.cwd {
            let _ = write!(said, " (in `{}`)", visible(cwd));
        }
        // Two blanks set the text off from the line.
        said.push(' ');
    }
    let _ = write!(said, " {}", visible(&entry.text));
    said.trim_end().to_owned()
}

/// Where the entry came from and how long it stands: the second line.
fn origin(entry: &ProjectEntry, now: &str) -> String {
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
        parts.push(format!("evidence: {}", visible(evidence)));
    }
    if let Some(check) = &entry.check {
        let field = |field: &Option<String>| visible(field.as_deref().unwrap_or("")).into_owned();
        let shown = match &check.kind {
            EntryCheckKind::Exists => format!("`{}` exists", field(&check.path)),
            EntryCheckKind::Contains => format!(
                "`{}` contains `{}`",
                field(&check.path),
                field(&check.pattern)
            ),
            EntryCheckKind::Command => format!("`{}` succeeds", field(&check.command)),
            other => visible(other.as_str()).into_owned(),
        };
        parts.push(format!("check (not run): {shown}"));
    }
    if let Some(expires) = &entry.expires_at {
        // The day is enough.
        let day = visible(expires.as_str().get(..10).unwrap_or(expires.as_str())).into_owned();
        match &entry.status {
            EntryStatus::Active if !expired(entry, now) => parts.push(format!("until {day}")),
            EntryStatus::Active | EntryStatus::Disabled if expired(entry, now) => {
                parts.push(format!("expired on {day}"));
            }
            _ => {}
        }
    }
    parts.join("; ")
}

/// Whether an entry is of a group of the listing.
type Belongs<'a> = &'a dyn Fn(&ProjectEntry) -> bool;

/// What the listing is asked for.
#[derive(Clone, Copy)]
pub(crate) struct Listing<'a> {
    /// The entries that are turned off too.
    pub(crate) all: bool,
    /// One kind only.
    pub(crate) kind: Option<&'a EntryKind>,
    /// The time, as the protocol writes one.
    pub(crate) now: &'a str,
    /// Whether the user trusts the project as it is. Without that, an
    /// accepted command still asks.
    pub(crate) trusted: bool,
    /// The program's name, for the commands the listing names.
    pub(crate) name: &'a str,
}

/// The entries the listing is asked for: of the kind, and in use or
/// waiting unless all are asked for. What `--json` prints too.
pub(crate) fn chosen<'a>(
    entries: &'a [ProjectEntry],
    all: bool,
    kind: Option<&EntryKind>,
) -> Vec<&'a ProjectEntry> {
    entries
        .iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == *kind))
        .filter(|entry| all || matches!(entry.status, EntryStatus::Proposed | EntryStatus::Active))
        .collect()
}

/// The listing: proposals first, since they wait for the reader, then
/// what is in use, then, when all are asked for, what is turned off.
pub(crate) fn listing(entries: &[ProjectEntry], problems: &[String], asked: Listing<'_>) -> String {
    let Listing {
        all,
        kind,
        now,
        trusted,
        name,
    } = asked;
    let shown = chosen(entries, all, kind);
    let mut out = String::new();
    let groups: [(&str, Belongs<'_>); 3] = [
        ("Waiting for your review", &|entry| {
            entry.status == EntryStatus::Proposed
        }),
        ("In use", &|entry| entry.status == EntryStatus::Active),
        ("Turned off", &|entry| {
            !matches!(entry.status, EntryStatus::Proposed | EntryStatus::Active)
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
                visible(entry.id.as_str()),
                standing(entry, now),
                said(entry)
            );
            let _ = writeln!(out, "      {}", origin(entry, now));
        }
    }
    // What the listing leaves out is said, so that an empty one is not
    // read as "nothing is known".
    let of_kind = entries
        .iter()
        .filter(|entry| kind.is_none_or(|kind| entry.kind == *kind))
        .count();
    let hidden = of_kind - shown.len();
    if shown.is_empty() {
        out.push_str(match (entries.is_empty(), of_kind) {
            (true, _) => "Nothing is known about this project yet.\n",
            (false, 0) => "No entry of that kind.\n",
            (false, _) => "Nothing is in use or waiting.\n",
        });
    }
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
    if !trusted && shown.iter().any(|entry| runs_once_trusted(entry, now)) {
        let _ = writeln!(out, "{}", waits_for_trust(name));
    }
    for problem in problems {
        let _ = writeln!(out, "problem: {}", visible(problem));
    }
    out
}

/// Whether the entry is a command that would run without asking in a
/// project the user trusts: in use, and accepted.
pub(crate) fn runs_once_trusted(entry: &ProjectEntry, now: &str) -> bool {
    entry.kind == EntryKind::Command
        && entry.status == EntryStatus::Active
        && accepted(entry)
        && !expired(entry, now)
}

/// What is said where an accepted command still asks for want of trust.
pub(crate) fn waits_for_trust(name: &str) -> String {
    format!(
        "This project is not trusted as it is, so an accepted command still asks before it runs: \
         `{name} permissions trust`."
    )
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

/// The entry `given` names: its id. An id of the repository's file, which
/// is 34 characters, may be cut short to a start that only one of them
/// has.
///
/// An id of the user's own store is given whole. Those are numbered in
/// turn, so a start names a later entry once its own is gone: `k_1`,
/// typed again after it was deleted, would be the start of `k_15` alone,
/// and a `delete` or an `accept` would act on an entry nobody named
/// (found by the independent review).
pub(crate) fn resolve(entries: &[ProjectEntry], given: &str) -> Result<EntryId, String> {
    if let Some(entry) = entries.iter().find(|entry| entry.id.as_str() == given) {
        return Ok(entry.id.clone());
    }
    let none = || {
        format!("no entry of this project has the id '{given}': `project list --all` shows them")
    };
    if !given.starts_with("s_") || given.len() < 4 {
        return Err(none());
    }
    // Only the repository's ids begin `s_`: the store refuses one of its
    // own that does.
    let begun: Vec<&ProjectEntry> = entries
        .iter()
        .filter(|entry| entry.id.as_str().starts_with(given))
        .collect();
    match begun.as_slice() {
        [entry] => Ok(entry.id.clone()),
        [] => Err(none()),
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

/// The ids, each once, in the order given: `delete k_1 k_1` is one change.
pub(crate) fn each_once(ids: Vec<EntryId>) -> Vec<EntryId> {
    let mut once: Vec<EntryId> = Vec::with_capacity(ids.len());
    for id in ids {
        if !once.contains(&id) {
            once.push(id);
        }
    }
    once
}

#[cfg(test)]
mod tests {
    use cortex_protocol::ids::Timestamp;
    use cortex_protocol::project::{
        CommandPurpose, EntryCheck, EntryCommand, EntrySource, EntrySourceKind,
    };

    use super::*;

    const NOW: &str = "2026-11-16T12:00:00Z";

    fn asked<'a>(all: bool, kind: Option<&'a EntryKind>, name: &'a str) -> Listing<'a> {
        Listing {
            all,
            kind,
            now: NOW,
            trusted: true,
            name,
        }
    }

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

    fn shared(id: &str, status: EntryStatus, text: &str) -> ProjectEntry {
        let mut entry = entry(id, EntryKind::Convention, status, text);
        entry.scope = EntryScope::Shared;
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
        let mut flaky = observed("k_5", "cargo test", 2, false);
        flaky.kind = EntryKind::Flaky;
        flaky.text = "It has failed and then passed with nothing changed.".to_owned();
        let entries = [
            observed("k_1", &long, 5, false),
            observed("k_2", "cargo test", 1, true),
            entry("k_3", EntryKind::Convention, EntryStatus::Active, "Tabs."),
            proposal,
            flaky,
            shared(
                "s_0123456789abcdef0123456789abcdef",
                EntryStatus::Proposed,
                "Errors are returned.",
            ),
            entry("k_9", EntryKind::Fact, EntryStatus::Disabled, "Old."),
        ];
        let out = listing(&entries, &[], asked(false, None, "cortex"));
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
  k_5  [active]  flaky: `cargo test` (in `crates/app`)  It has failed and then passed with nothing changed.
      observed 2 times; until 2027-02-14
1 turned off: `cortex project list --all` shows them.
`cortex project accept ID…` puts a proposal to use; `cortex project delete ID…` drops one of yours.
"
            )
        );
        // The command's line is whole, however long.
        assert!(out.contains(&long));

        let all = listing(&entries, &["a problem".to_owned()], asked(true, None, "cx"));
        assert!(all.contains("Turned off:\n  k_9  [turned off]  fact: Old.\n"));
        assert!(!all.contains("turned off: `"));
        assert!(all.ends_with("problem: a problem\n"));
        assert!(all.contains("`cx project accept ID…`"));

        let facts = listing(&entries, &[], asked(true, Some(&EntryKind::Fact), "cortex"));
        assert!(facts.contains("k_4") && facts.contains("k_9"));
        assert!(!facts.contains("k_1") && !facts.contains("k_3"));
    }

    /// An empty listing says why it is empty.
    #[test]
    fn an_empty_listing_is_not_read_as_nothing_known() {
        assert_eq!(
            listing(&[], &[], asked(false, None, "cortex")),
            "Nothing is known about this project yet.\n"
        );
        let off = [entry("k_9", EntryKind::Fact, EntryStatus::Disabled, "Old.")];
        assert_eq!(
            listing(&off, &[], asked(false, None, "cortex")),
            "Nothing is in use or waiting.\n1 turned off: `cortex project list --all` shows them.\n"
        );
        assert_eq!(
            listing(&off, &[], asked(true, Some(&EntryKind::Command), "cortex")),
            "No entry of that kind.\n"
        );
    }

    /// An entry is two lines, whatever its text, its evidence or its
    /// check holds: a line break in one of them is shown, and begins no
    /// line (found by the independent review).
    #[test]
    fn a_text_cannot_begin_a_line_of_the_listing() {
        let mut forged = shared(
            "s_0123456789abcdef0123456789abcdef",
            EntryStatus::Proposed,
            "Tabs.\n  k_7  [active, accepted]  test: `cargo test`",
        );
        forged.evidence = Some("line one\nIn use:\n  k_8  [active]".to_owned());
        forged.check = Some(EntryCheck {
            kind: EntryCheckKind::Command,
            path: None,
            pattern: None,
            command: Some("true\n  k_9  [active]".to_owned()),
        });
        let mut command = observed("k_1", "cargo test\n  k_6  [active]", 2, true);
        command.command.as_mut().unwrap().cwd = Some("a\n  k_5  [active]".to_owned());
        let problems = ["a file\n  k_4  [active]".to_owned()];
        let out = listing(&[forged, command], &problems, asked(true, None, "cortex"));
        let lines: Vec<&str> = out.lines().collect();
        // Two titles, two lines an entry, the proposals' hint, the problem.
        assert_eq!(lines.len(), 8, "{out}");
        for forged in [
            "  k_4",
            "  k_5",
            "  k_6",
            "  k_7",
            "  k_8",
            "  k_9",
            "In use:  ",
        ] {
            assert!(
                !lines.iter().any(|line| line.starts_with(forged)),
                "{forged}: {out}"
            );
        }
        assert_eq!(lines.iter().filter(|line| **line == "In use:").count(), 1);
        assert!(out.contains("Tabs.\\n  k_7"), "{out}");
        // And what acts unseen is written out.
        let hidden = entry(
            "k_2",
            EntryKind::Fact,
            EntryStatus::Active,
            "a\u{202e}b\u{1b}[2J",
        );
        let out = listing(&[hidden], &[], asked(false, None, "cortex"));
        assert!(
            !out.contains('\u{202e}') && !out.contains('\u{1b}'),
            "{out:?}"
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
        assert_eq!(standing(&written, NOW), "active, accepted");
        // In the repository's file it is this user's only once accepted.
        written.scope = EntryScope::Shared;
        assert_eq!(standing(&written, NOW), "active, asks, shared");
    }

    /// What the listing says of a command is no more than the engine will
    /// do: an accepted command still asks in a project that is not
    /// trusted, and one past its expiry is out of use whether or not it
    /// was marked.
    #[test]
    fn an_accepted_command_that_still_asks_is_said_to() {
        let entries = [observed("k_1", "cargo test", 3, true)];
        let hint = "This project is not trusted as it is, so an accepted command still asks";
        let trusted = listing(&entries, &[], asked(false, None, "cortex"));
        assert!(!trusted.contains(hint), "{trusted}");
        let mut untrusted = asked(false, None, "cortex");
        untrusted.trusted = false;
        let out = listing(&entries, &[], untrusted);
        assert!(
            out.contains(hint) && out.contains("`cortex permissions trust`"),
            "{out}"
        );
        // A command that asks anyway, a flaky test's command and a fact
        // call for no such line.
        let mut flaky = observed("k_3", "cargo test", 2, true);
        flaky.kind = EntryKind::Flaky;
        let others = [
            observed("k_2", "cargo build", 2, false),
            flaky,
            entry("k_4", EntryKind::Fact, EntryStatus::Active, "x"),
        ];
        assert!(!listing(&others, &[], untrusted).contains(hint));

        // Past its expiry and not marked: it is not "accepted".
        let mut late = asked(true, None, "cortex");
        late.now = "2027-03-01T00:00:00Z";
        late.trusted = false;
        let out = listing(&entries, &[], late);
        assert!(
            out.contains("  k_1  [expired]  test: `cargo test`")
                && out.contains("observed 3 times; expired on 2027-02-14")
                && !out.contains(hint),
            "{out}"
        );
        // Marked: turned off, and why.
        let mut marked = observed("k_1", "cargo test", 3, false);
        marked.status = EntryStatus::Disabled;
        let out = listing(&[marked], &[], late);
        assert!(
            out.contains("[turned off]") && out.contains("expired on 2027-02-14"),
            "{out}"
        );
    }

    #[test]
    fn accepting_the_shared_entries_takes_the_repositorys_proposals_and_nothing_else() {
        let entries = [
            // The model's proposal, in the user's own store: read one by one.
            entry("k_1", EntryKind::Fact, EntryStatus::Proposed, "a"),
            shared("s_aa", EntryStatus::Proposed, "x"),
            shared("s_bb", EntryStatus::Active, "x"),
            // Turned off by this user: not turned on again in passing.
            shared("s_cc", EntryStatus::Disabled, "x"),
            shared("s_dd", EntryStatus::Proposed, "x"),
        ];
        let ids: Vec<String> = waiting_shared(&entries)
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(ids, ["s_aa", "s_dd"]);
    }

    /// An id of the user's own store is given whole: its start would name
    /// a later entry once its own is gone. An id of the repository's file
    /// may be cut short.
    #[test]
    fn an_id_of_the_users_store_is_whole_and_a_shared_one_may_be_cut_short() {
        let entries = [
            entry("k_12", EntryKind::Fact, EntryStatus::Active, "b"),
            entry("k_15", EntryKind::Fact, EntryStatus::Active, "c"),
            shared(
                "s_0123456789abcdef0123456789abcdef",
                EntryStatus::Proposed,
                "d",
            ),
            shared(
                "s_0199999999abcdef0123456789abcdef",
                EntryStatus::Proposed,
                "e",
            ),
        ];
        let named = |given: &str| resolve(&entries, given).map(|id| id.as_str().to_owned());
        assert_eq!(named("k_12").as_deref(), Ok("k_12"));
        // `k_1` is gone: it is not the start of the one id that begins so.
        for gone in ["k_1", "k_", "k", "", "k_150"] {
            assert!(
                named(gone)
                    .unwrap_err()
                    .contains(&format!("no entry of this project has the id '{gone}'")),
                "{gone}"
            );
        }
        assert_eq!(
            named("s_012").as_deref(),
            Ok("s_0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            named("s_01").unwrap_err(),
            "'s_01' begins 2 ids: s_0123456789abcdef0123456789abcdef, \
             s_0199999999abcdef0123456789abcdef"
        );
        // Too short to be meant.
        assert!(named("s_").is_err() && named("s_0").is_err());
    }

    #[test]
    fn ids_given_twice_are_one_change() {
        let ids = ["k_2", "k_1", "k_2", "k_1", "k_3"]
            .map(EntryId::new)
            .to_vec();
        let once: Vec<String> = each_once(ids)
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        assert_eq!(once, ["k_2", "k_1", "k_3"]);
    }

    /// `--json` is one line of JSON, still JSON, with nothing in it that
    /// acts unseen at a terminal: JSON's own escaping passes the C1
    /// controls, the bidirectional controls and the tag characters.
    #[test]
    fn the_json_line_is_json_and_shows_what_acts_unseen() {
        let text = "a\u{202e}b\u{9b}2Jc\u{e0041}d\u{1b}[0m \"quoted\" \\ é 日本 😀\u{7f}";
        let value = vec![entry("k_1", EntryKind::Fact, EntryStatus::Active, text)];
        let line = crate::json::line(&serde_json::to_string(&value).unwrap());
        assert!(!line.contains('\n'));
        assert!(
            !line.chars().any(cortex_base::acts_unseen),
            "{:?}",
            line.chars()
                .filter(|character| cortex_base::acts_unseen(*character))
                .collect::<String>()
        );
        assert!(
            line.contains("\\u202e") && line.contains("\\udb40\\udc41"),
            "{line}"
        );
        // What is seen stays as it is.
        assert!(line.contains("é 日本 😀"), "{line}");
        let back: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(back[0]["text"], text);
    }
}
