use cineris_protocol::event::CheckSource;
use cineris_term::ColorDepth;

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

fn file(path: &str, added: Option<u64>, removed: Option<u64>) -> ChangedFile {
    ChangedFile {
        path: path.to_owned(),
        added,
        removed,
    }
}

pub(crate) fn check(
    command: &str,
    cwd: Option<&str>,
    outcome: Option<CheckOutcome>,
    exit_code: Option<i32>,
    duration_ms: Option<u64>,
    note: Option<&str>,
) -> CheckReport {
    CheckReport {
        command: command.to_owned(),
        cwd: cwd.map(str::to_owned),
        source: CheckSource::User,
        entry: None,
        outcome,
        exit_code,
        duration_ms,
        note: note.map(str::to_owned),
        artifact: None,
    }
}

fn notice(phase: VerificationPhase, checks: Vec<CheckReport>) -> VerificationNotice {
    VerificationNotice {
        message: "The checks.".to_owned(),
        phase,
        round: 1,
        rounds: 2,
        checks,
        verdict: None,
    }
}

fn files() -> Vec<ChangedFile> {
    vec![
        file("src/lex.rs", Some(5), Some(1)),
        file(
            "crates/parser/src/lexer/numbers/separators_and_suffixes.rs",
            Some(6),
            Some(3),
        ),
        file("tests/lex.rs", Some(1), Some(0)),
        file("assets/big.bin", None, None),
    ]
}

fn finished() -> Vec<CheckReport> {
    vec![
        check(
            "cargo test -p parser",
            Some("crates/parser"),
            Some(CheckOutcome::Passed),
            Some(0),
            Some(3_400),
            None,
        ),
        check(
            "cargo clippy -- -D warnings",
            None,
            Some(CheckOutcome::Failed),
            Some(101),
            Some(8_100),
            Some("2 warnings"),
        ),
        check(
            "cargo deny check",
            None,
            Some(CheckOutcome::Blocked),
            None,
            Some(200),
            Some("the sandbox has no network"),
        ),
        check(
            "./slow-check",
            None,
            Some(CheckOutcome::TimedOut),
            None,
            Some(60_000),
            None,
        ),
    ]
}

/// The states the panel is drawn in, by name.
fn states() -> Vec<(&'static str, Review)> {
    let started = notice(
        VerificationPhase::Started,
        vec![
            check(
                "cargo test -p parser",
                Some("crates/parser"),
                None,
                None,
                None,
                None,
            ),
            check("cargo clippy -- -D warnings", None, None, None, None, None),
        ],
    );
    let mut ended = notice(VerificationPhase::Finished, finished());
    ended.verdict = Some(Verification::Failed);
    let offered = notice(
        VerificationPhase::Offered,
        vec![check("cargo test", None, None, None, None, None)],
    );
    vec![
        (
            "files, checks not run",
            Review {
                files: files(),
                ..Review::default()
            },
        ),
        (
            "files, a round running",
            Review {
                files: files(),
                checks: Some(started),
                ..Review::default()
            },
        ),
        (
            "files, a round ended, commands ran",
            Review {
                files: files(),
                checks: Some(ended.clone()),
                verdict: Some(Verification::Failed),
                commands: true,
            },
        ),
        (
            "files, checks offered",
            Review {
                files: files()[..1].to_vec(),
                checks: Some(offered),
                verdict: Some(Verification::Skipped),
                commands: false,
            },
        ),
        (
            "checks alone, passed",
            Review {
                checks: Some(VerificationNotice {
                    verdict: Some(Verification::Passed),
                    ..notice(VerificationPhase::Finished, finished()[..1].to_vec())
                }),
                verdict: Some(Verification::Passed),
                ..Review::default()
            },
        ),
    ]
}

#[test]
fn the_review_in_every_variant() {
    let mut parts = Vec::new();
    for variant in testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let width = variant.width();
        for (name, review) in states() {
            let collapsed = review_collapsed(&review, width, &look);
            let focused: Vec<Line> = review_rows(&review, Some(1), width, &look, 0)
                .into_iter()
                .map(|row| row.line)
                .collect();
            for (form, lines) in [("collapsed", collapsed), ("focused", focused)] {
                let problems = testing::problems(&lines, width);
                assert!(problems.is_empty(), "{name}, {form}: {problems:?}");
                parts.push((format!("{} — {name}, {form}", variant.title()), lines));
            }
        }
    }
    testing::assert_snapshot("review", &testing::sections(&parts));
}

#[test]
fn the_first_row_says_the_files_their_counts_and_the_checks() {
    let look = look();
    let text = |review: &Review| review_header(review, 120, &look).text().into_owned();
    let mut review = Review {
        files: vec![
            file("a.rs", Some(5), Some(1)),
            file("b.rs", Some(4), Some(3)),
            file("c.rs", Some(3), Some(0)),
        ],
        verdict: Some(Verification::Passed),
        ..Review::default()
    };
    assert_eq!(text(&review), "◇ Changed 3 files  +12 −4 · checks passed");
    review.verdict = None;
    review.checks = Some(notice(VerificationPhase::Started, Vec::new()));
    assert_eq!(
        text(&review),
        "◇ Changed 3 files  +12 −4 · checks running · round 1 of 2"
    );
    review.checks = Some(notice(VerificationPhase::Offered, Vec::new()));
    assert_eq!(
        text(&review),
        "◇ Changed 3 files  +12 −4 · checks not run: /verify runs them"
    );
    review.files.truncate(1);
    review.checks = None;
    assert_eq!(text(&review), "◇ Changed 1 file  +5 −1");
    // The checks alone.
    let alone = Review {
        verdict: Some(Verification::Failed),
        ..Review::default()
    };
    assert_eq!(text(&alone), "◇ Checks failed");
    // The verdict's colour.
    let header = review_header(
        &Review {
            files: vec![file("a.rs", Some(1), Some(1))],
            verdict: Some(Verification::Failed),
            ..Review::default()
        },
        120,
        &look,
    );
    let verdict = header
        .spans()
        .iter()
        .find(|span| span.text() == "checks failed")
        .unwrap();
    assert_eq!(verdict.style(), look.theme.style(Token::Error));
}

#[test]
fn a_long_path_is_shortened_from_its_start_and_its_counts_stay() {
    let look = look();
    let long = file(
        "crates/parser/src/lexer/numbers/separators_and_suffixes.rs",
        Some(6),
        Some(3),
    );
    for width in 12..=80 {
        let row = file_row(&long, false, width, &look);
        let text = row.text().into_owned();
        assert!(row.width() <= width, "{width}: {text}");
        assert!(text.ends_with("+6 −3"), "{width}: {text}");
        if width < 67 {
            assert!(text.starts_with("  …"), "{width}: {text}");
        }
    }
    let row = file_row(&long, false, 40, &look).text().into_owned();
    assert_eq!(row, "  …ers/separators_and_suffixes.rs  +6 −3");
    // A path that fits is whole; the focused one is the selection.
    let short = file_row(&file("src/lex.rs", Some(5), Some(1)), true, 40, &look);
    assert_eq!(short.text(), "  src/lex.rs  +5 −1");
    assert_eq!(short.spans()[1].style(), look.theme.style(Token::Selected));
}

#[test]
fn a_checks_line_says_its_mark_command_folder_measure_and_note() {
    let look = look();
    let rows: Vec<String> = finished()
        .iter()
        .map(|check| {
            check_row(check, &VerificationPhase::Finished, false, 120, &look, 0)
                .text()
                .into_owned()
        })
        .collect();
    assert_eq!(
        rows,
        [
            "  ✓ cargo test -p parser  crates/parser  3.4s",
            "  ✗ cargo clippy -- -D warnings  exit 101 · 8.1s  2 warnings",
            "  ⊘ cargo deny check  0.2s  the sandbox has no network",
            "  ■ ./slow-check  timed out · 1m 0s",
        ]
    );
    let failed = check_row(
        &finished()[1],
        &VerificationPhase::Finished,
        false,
        120,
        &look,
        0,
    );
    let measure = failed
        .spans()
        .iter()
        .find(|span| span.text().starts_with("exit"))
        .unwrap();
    assert_eq!(measure.style(), look.theme.style(Token::Error));
    // A round running: the spinner, the command alone.
    let running = check_row(
        &check("make check", None, None, None, None, None),
        &VerificationPhase::Started,
        false,
        120,
        &look,
        1,
    );
    assert_eq!(running.text(), format!("  {} make check", look.spinner(1)));
}

#[test]
fn the_rows_are_tagged_files_then_checks() {
    let look = look();
    let mut ended = notice(VerificationPhase::Finished, finished()[..2].to_vec());
    ended.verdict = Some(Verification::Failed);
    let review = Review {
        files: files()[..2].to_vec(),
        checks: Some(ended),
        verdict: None,
        commands: true,
    };
    let rows = review_rows(&review, None, 120, &look, 0);
    let tags: Vec<Option<usize>> = rows.iter().map(|row| row.item).collect();
    assert_eq!(tags, [None, Some(0), Some(1), Some(2), Some(3), None, None]);
    assert_eq!(review.items(), 4);
    let last: Vec<String> = rows[5..]
        .iter()
        .map(|row| row.line.text().into_owned())
        .collect();
    assert_eq!(
        last,
        [
            "  checks failed · round 1 of 2",
            "  commands ran too: /view shows them"
        ]
    );
}

#[test]
fn hostile_paths_commands_folders_and_notes_are_drawn_visible() {
    const HOSTILE: [&str; 5] = [
        "\u{1b}]8;;http://x\u{7}evil\u{1b}]8;;\u{7}",
        "a\u{202e}b",
        "zero\u{200b}width",
        "line\nbreak",
        "\u{9b}2J\u{e0041}",
    ];
    let look = look();
    for hostile in HOSTILE {
        let mut ended = notice(
            VerificationPhase::Finished,
            vec![check(
                hostile,
                Some(hostile),
                Some(CheckOutcome::Failed),
                Some(1),
                Some(10),
                Some(hostile),
            )],
        );
        ended.verdict = Some(Verification::Failed);
        let review = Review {
            files: vec![file(hostile, Some(1), Some(1))],
            checks: Some(ended),
            verdict: Some(Verification::from_wire(hostile)),
            commands: true,
        };
        for width in [1, 10, 40, 120] {
            let mut lines: Vec<Line> = review_rows(&review, Some(0), width, &look, 0)
                .into_iter()
                .map(|row| row.line)
                .collect();
            lines.extend(review_collapsed(&review, width, &look));
            let problems = testing::problems(&lines, width);
            assert!(problems.is_empty(), "{hostile:?} at {width}: {problems:#?}");
        }
    }
}
