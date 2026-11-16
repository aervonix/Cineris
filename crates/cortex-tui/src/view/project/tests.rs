use cortex_protocol::Timestamp;
use cortex_protocol::ids::EntryId;
use cortex_protocol::project::{
    CommandPurpose, EntryCheck, EntryCommand, EntrySource, EntrySourceKind,
};
use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

/// 2026-11-16T12:00:00Z.
pub(crate) fn now() -> UtcTime {
    UtcTime::parse("2026-11-16T12:00:00Z").expect("a time")
}

/// An entry of the private store, learned in session `s_1`, as these tests
/// start from.
pub(crate) fn entry(id: &str, kind: EntryKind, text: &str) -> ProjectEntry {
    ProjectEntry {
        id: EntryId::from(id),
        kind,
        text: text.to_owned(),
        command: None,
        evidence: None,
        source: EntrySource {
            kind: EntrySourceKind::Learned,
            session_id: Some("s_1".into()),
            item_id: None,
            at: Some(Timestamp::from("2026-11-14T09:30:00Z")),
        },
        basis: Some(EntryBasis::Observed),
        status: EntryStatus::Active,
        scope: EntryScope::Private,
        confirmed_at: None,
        expires_at: None,
        check: None,
        accepted_at: None,
        observations: Some(3),
    }
}

/// An observed command that became active by itself: it waits for review,
/// and still asks before it runs.
pub(crate) fn observed_command(id: &str, command: &str) -> ProjectEntry {
    ProjectEntry {
        command: Some(EntryCommand {
            purpose: CommandPurpose::Test,
            command: command.to_owned(),
            cwd: Some("crates/parser".to_owned()),
            usual_duration_ms: Some(3_400),
            read_only: false,
        }),
        check: Some(EntryCheck {
            kind: EntryCheckKind::Exists,
            path: Some("Cargo.toml".to_owned()),
            pattern: None,
            command: None,
        }),
        evidence: Some("ran 3 times and passed".to_owned()),
        expires_at: Some(Timestamp::from("2026-12-14T09:30:00Z")),
        ..entry(id, EntryKind::Command, "the parser's tests")
    }
}

/// The entries the snapshots draw: a command waiting, a proposal, a fact
/// in use, accepted, and one of the repository's file.
pub(crate) fn entries() -> Vec<ProjectEntry> {
    let mut proposed = entry(
        "k_2",
        EntryKind::Convention,
        "Errors are written as sentences that say what to do next.",
    );
    proposed.status = EntryStatus::Proposed;
    proposed.basis = Some(EntryBasis::Proposed);
    let mut fact = entry("k_3", EntryKind::Fact, "The parser is hand-written.");
    fact.accepted_at = Some(Timestamp::from("2026-11-15T10:00:00Z"));
    let mut shared = entry(
        "k_4",
        EntryKind::Flaky,
        "lex::numbers::huge times out on CI",
    );
    shared.scope = EntryScope::Shared;
    shared.basis = None;
    shared.source = EntrySource {
        kind: EntrySourceKind::User,
        session_id: None,
        item_id: None,
        at: None,
    };
    vec![
        observed_command(
            "k_1",
            "cargo test -p parser --all-features -- --test-threads 1 --include-ignored",
        ),
        proposed,
        fact,
        shared,
    ]
}

fn view(entries: &[ProjectEntry]) -> ProjectView<'_> {
    ProjectView {
        entries,
        now: now(),
        asking: None,
        reading: false,
        problems: &[],
    }
}

#[test]
fn the_project_in_every_variant() {
    let entries = entries();
    let problems = vec!["`.cortex/profile.toml` is not TOML: the file is not used.".to_owned()];
    let states: Vec<(&str, ProjectView<'_>, Option<usize>)> = vec![
        (
            "reading",
            ProjectView {
                reading: true,
                ..view(&[])
            },
            None,
        ),
        ("nothing kept", view(&[]), None),
        ("a command waiting, focused", view(&entries), Some(0)),
        (
            "its deletion asked",
            ProjectView {
                asking: Some(0),
                ..view(&entries)
            },
            Some(0),
        ),
        ("a proposal focused", view(&entries), Some(1)),
        (
            "a fact in use focused, a problem",
            ProjectView {
                problems: &problems,
                ..view(&entries)
            },
            Some(2),
        ),
    ];
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let width = variant.width();
        for (name, state, focused) in &states {
            let collapsed = project_collapsed(state, width, &look);
            let rows: Vec<Line> = project_rows(state, *focused, width, &look)
                .into_iter()
                .map(|row| row.line)
                .collect();
            for (form, lines) in [("collapsed", collapsed), ("focused", rows)] {
                let problems = testing::problems(&lines, width);
                assert!(problems.is_empty(), "{name}, {form}: {problems:?}");
                parts.push((format!("{} — {name}, {form}", variant.title()), lines));
            }
        }
    }
    testing::assert_snapshot("project", &testing::sections(&parts));
}

#[test]
fn the_first_row_counts_what_waits_and_what_is_in_use() {
    let look = look();
    let entries = entries();
    let header = |entries: &[ProjectEntry]| {
        project_header(&view(entries), 80, &look)
            .text()
            .into_owned()
    };
    assert_eq!(
        header(&entries),
        "◇ Project  3 entries wait for review · 1 in use"
    );
    assert_eq!(header(&entries[2..3]), "◇ Project  1 in use");
    assert_eq!(header(&entries[..1]), "◇ Project  1 entry waits for review");
    assert_eq!(header(&[]), "◇ Project  0 in use");
}

#[test]
fn what_waits_for_review_is_a_proposal_or_an_entry_never_accepted() {
    let entries = entries();
    // An observed command active by itself, a proposal, a shared entry.
    assert!(waits(&entries[0]));
    assert!(waits(&entries[1]));
    assert!(!waits(&entries[2]), "accepted");
    assert!(waits(&entries[3]), "the repository's, not accepted here");
    let mut yours = entries[2].clone();
    yours.accepted_at = None;
    yours.basis = None;
    yours.source.kind = EntrySourceKind::User;
    assert!(!waits(&yours), "written by you");
    let mut off = entries[0].clone();
    off.status = EntryStatus::Disabled;
    assert!(!waits(&off), "turned off");
}

#[test]
fn the_focused_commands_detail_is_whole_and_wrapped_never_cut() {
    let look = look();
    let entries = entries();
    let command = entries[0]
        .command
        .as_ref()
        .map(|command| command.command.clone())
        .unwrap_or_default();
    for width in [24, 40, 60, 120] {
        let rows = project_rows(&view(&entries), Some(0), width, &look);
        let detail: Vec<String> = rows
            .iter()
            .filter(|row| row.item == Some(0))
            .skip(1)
            .map(|row| row.line.text().into_owned())
            .collect();
        for row in &detail {
            assert!(
                !row.contains(look.glyphs.ellipsis),
                "nothing cut at {width}: {detail:#?}"
            );
            assert!(row.starts_with("    "), "four columns in: {row:?}");
        }
        // The command line whole, across its rows.
        let joined: String = detail
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join(" ");
        let words: Vec<&str> = command.split(' ').collect();
        for word in words {
            assert!(joined.contains(word), "{word} at {width}: {detail:#?}");
        }
        assert!(
            joined.contains("in crates/parser · may change files"),
            "{detail:#?}"
        );
        assert!(
            joined.contains("observed 3 times, learned in session s_1 on 2026-11-14"),
            "{detail:#?}"
        );
        assert!(
            joined.contains("in use, and still asks before it runs, until 2026-12-14"),
            "{detail:#?}"
        );
        assert!(
            joined.contains("`Cargo.toml` exists, not run in this build"),
            "{detail:#?}"
        );
    }
}

#[test]
fn each_origin_and_standing_is_said_as_cortex_project_says_it() {
    let look = look();
    let said = |entry: &ProjectEntry| -> String {
        detail(entry, false, now(), 200, &look)
            .iter()
            .map(|line| line.text().trim().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let entries = entries();
    assert!(said(&entries[1]).contains("proposed by the model in session s_1 on 2026-11-14"));
    assert!(said(&entries[1]).contains("waits for your review: not in use"));
    assert!(said(&entries[3]).contains("from the repository's file"));
    let mut yours = entries[2].clone();
    yours.basis = None;
    yours.source.kind = EntrySourceKind::User;
    assert!(said(&yours).contains("you wrote it"));
    let mut accepted = entries[0].clone();
    accepted.accepted_at = Some(Timestamp::from("2026-11-15T10:00:00Z"));
    assert!(said(&accepted).contains("in use, and accepted, until 2026-12-14"));
    let mut old = entries[0].clone();
    old.expires_at = Some(Timestamp::from("2026-11-01T00:00:00Z"));
    assert!(said(&old).contains("expired on 2026-11-01: not in use"));
    assert!(said(&entries[0]).contains("evidence  ran 3 times and passed"));
    let asked = detail(&entries[0], true, now(), 200, &look);
    assert_eq!(
        asked.last().map(|line| line.text().into_owned()).as_deref(),
        Some("    delete this entry? y deletes · any other key keeps it")
    );
}

#[test]
fn an_entrys_age_is_said_in_a_few_columns() {
    let now = now();
    assert_eq!(age("2026-11-16T11:59:30Z", now).as_deref(), Some("now"));
    assert_eq!(age("2026-11-16T11:55:00Z", now).as_deref(), Some("5m"));
    assert_eq!(age("2026-11-16T09:00:00Z", now).as_deref(), Some("3h"));
    assert_eq!(age("2026-11-14T09:30:00Z", now).as_deref(), Some("2d"));
    assert_eq!(age("2026-11-17T00:00:00Z", now), None, "after now");
    assert_eq!(age("yesterday", now), None);
}

#[test]
fn hostile_texts_commands_folders_checks_evidence_and_problems_are_drawn_visible() {
    const HOSTILE: [&str; 5] = [
        "\u{1b}]8;;http://x\u{7}evil\u{1b}]8;;\u{7}",
        "a\u{202e}b",
        "zero\u{200b}width",
        "line\nbreak",
        "\u{9b}2J\u{e0041}",
    ];
    let look = look();
    for hostile in HOSTILE {
        let mut command = observed_command("k_1", hostile);
        command.text = hostile.to_owned();
        command.evidence = Some(hostile.to_owned());
        if let Some(inner) = &mut command.command {
            inner.cwd = Some(hostile.to_owned());
        }
        command.check = Some(EntryCheck {
            kind: EntryCheckKind::Contains,
            path: Some(hostile.to_owned()),
            pattern: Some(hostile.to_owned()),
            command: None,
        });
        command.source.session_id = Some(hostile.into());
        command.source.at = Some(Timestamp::from(hostile));
        command.expires_at = Some(Timestamp::from(hostile));
        let mut other = entry("k_2", EntryKind::from_wire(hostile), hostile);
        other.status = EntryStatus::from_wire(hostile);
        let entries = vec![command, other];
        let problems = vec![hostile.to_owned()];
        for width in [1, 10, 40, 120] {
            for focused in [0, 1] {
                let state = ProjectView {
                    asking: Some(0),
                    problems: &problems,
                    ..view(&entries)
                };
                let mut lines: Vec<Line> = project_rows(&state, Some(focused), width, &look)
                    .into_iter()
                    .map(|row| row.line)
                    .collect();
                lines.extend(project_collapsed(&state, width, &look));
                let problems = testing::problems(&lines, width);
                assert!(problems.is_empty(), "{hostile:?} at {width}: {problems:#?}");
            }
        }
    }
}
