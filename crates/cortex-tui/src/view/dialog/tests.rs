//! The approval dialog's rows, as screens 2a and 2b draw them.

use cortex_protocol::lease::Effect;
use cortex_protocol::methods::{
    AddedRule, ApprovalChoice, ApprovalDetail, ApprovalRequest, CommandDetail, DiffDetail,
};
use cortex_protocol::{CallId, SessionId, TurnId};
use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::theme::Theme;
use crate::view::Change;

fn look() -> Look {
    Look::new(Theme::new(ColorDepth::Mono, None), &Glyphs::UNICODE)
}

fn request(detail: ApprovalDetail, summary: &str) -> ApprovalRequest {
    ApprovalRequest {
        session_id: SessionId::from("s_1"),
        turn_id: TurnId::from("t_1"),
        call_id: CallId::from("c_1"),
        tool: "edit".to_owned(),
        summary: summary.to_owned(),
        detail,
        choices: vec![ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
        reasons: vec!["CI configuration is protected, so a change to it always asks.".to_owned()],
        adds: Vec::new(),
    }
}

fn command(text: &str, outside_sandbox: bool) -> ApprovalDetail {
    ApprovalDetail::Command(CommandDetail {
        command: text.to_owned(),
        cwd: "crates/parser".to_owned(),
        outside_sandbox,
    })
}

fn answers() -> Vec<Answer> {
    vec![
        Answer {
            key: "y".to_owned(),
            label: "Allow once".to_owned(),
        },
        Answer {
            key: "n".to_owned(),
            label: "Deny".to_owned(),
        },
    ]
}

fn view(detail_rows: usize) -> ApprovalView<'static> {
    ApprovalView {
        answers: answers(),
        detail_rows,
        ..ApprovalView::default()
    }
}

fn text(drawn: &Drawn) -> Vec<String> {
    drawn
        .lines
        .iter()
        .map(|line| line.text().trim_end().to_owned())
        .collect()
}

#[test]
fn an_approval_with_a_diff_shows_the_question_its_reason_the_change_and_the_answers() {
    let request = request(
        ApprovalDetail::Diff(DiffDetail {
            path: ".github/workflows/ci.yml".to_owned(),
            diff: String::new(),
            omitted: None,
        }),
        "Edit .github/workflows/ci.yml",
    );
    let diff = vec![
        DiffLine {
            number: 29,
            change: Change::Same,
            text: "    - uses: x".to_owned(),
            emphasis: Vec::new(),
        },
        DiffLine {
            number: 30,
            change: Change::Removed,
            text: "      with: a".to_owned(),
            emphasis: Vec::new(),
        },
        DiffLine {
            number: 30,
            change: Change::Added,
            text: "      with: b".to_owned(),
            emphasis: Vec::new(),
        },
    ];
    let drawn = approval(
        &request,
        &ApprovalView {
            diff: &diff,
            ..view(10)
        },
        99,
        &look(),
    );
    assert!(!drawn.clipped);
    let rows = text(&drawn);
    assert_eq!(rows[0], "? Edit .github/workflows/ci.yml?");
    assert_eq!(
        rows[1],
        "  CI configuration is protected, so a change to it always asks."
    );
    assert_eq!(rows[2], "");
    assert_eq!(rows[3], "     29       - uses: x");
    assert_eq!(rows[4], "     30 −       with: a");
    assert_eq!(rows[5], "     30 +       with: b");
    assert_eq!(
        rows.last().map(String::as_str),
        Some("  y Allow once   n Deny")
    );
    assert!(drawn.lines.iter().all(|line| line.width() <= 99));
}

#[test]
fn leaving_the_sandbox_asks_its_own_question_and_says_what_it_costs() {
    let request = request(
        command("cargo publish --dry-run -p parser", true),
        "Run cargo publish",
    );
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: vec![Answer {
                key: "y".to_owned(),
                label: "Run once outside the sandbox".to_owned(),
            }],
            leaving: true,
            ..view(10)
        },
        80,
        &look(),
    ));
    assert_eq!(rows[0], "? Run outside the sandbox?");
    assert!(
        rows.contains(&"    cargo publish --dry-run -p parser".to_owned()),
        "{rows:#?}"
    );
    assert!(
        rows.contains(&"    in crates/parser".to_owned()),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("full rights")),
        "{rows:#?}"
    );
    // Its reasons are not repeated: the question says it all.
    assert!(
        !rows.iter().any(|row| row.contains("protected")),
        "{rows:#?}"
    );
}

/// A session whose sandbox is off runs every command outside it: that is
/// no request to leave it, and the ordinary question is asked.
#[test]
fn a_command_outside_a_sandbox_that_is_off_is_asked_as_any_other() {
    let request = request(command("cargo build", true), "Run cargo build");
    let rows = text(&approval(&request, &view(10), 80, &look()));
    assert_eq!(rows[0], "? Run cargo build?");
    assert!(
        !rows.iter().any(|row| row.contains("full rights")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().any(|row| row.contains("protected")),
        "{rows:#?}"
    );
}

#[test]
fn a_command_is_shown_whole_and_what_acts_unseen_is_escaped() {
    let request = request(command("echo one\necho two\u{202e}x", false), "Run echo");
    let rows = text(&approval(&request, &view(10), 80, &look()));
    assert!(rows.contains(&"    echo one".to_owned()), "{rows:#?}");
    assert!(
        rows.contains(&"    echo two\\u{202e}x".to_owned()),
        "{rows:#?}"
    );
}

/// A command taller than its room is cut with a line that says so, and the
/// drawing says it was clipped, so that no lasting answer is offered.
#[test]
fn a_tall_command_is_clipped_and_says_so() {
    let script: Vec<String> = (1..=60).map(|line| format!("echo {line}")).collect();
    let request = request(command(&script.join("\n"), false), "Run a script");
    let drawn = approval(&request, &view(8), 80, &look());
    assert!(drawn.clipped);
    let rows = text(&drawn);
    assert_eq!(rows[0], "? Run a script?", "the question is never cut");
    assert!(rows.contains(&"    echo 1".to_owned()), "{rows:#?}");
    assert!(
        rows.iter()
            .any(|row| row.contains("53 more lines are not shown")),
        "{rows:#?}"
    );
    assert!(
        rows.iter().filter(|row| row.contains("echo ")).count() <= 7,
        "{rows:#?}"
    );
}

#[test]
fn a_tall_diff_is_clipped_with_how_much_more_there_is() {
    let request = request(
        ApprovalDetail::Diff(DiffDetail {
            path: "a.rs".to_owned(),
            diff: String::new(),
            omitted: None,
        }),
        "Edit a.rs",
    );
    let diff: Vec<DiffLine> = (1..=40)
        .map(|number| DiffLine {
            number,
            change: Change::Added,
            text: format!("line {number}"),
            emphasis: Vec::new(),
        })
        .collect();
    let drawn = approval(
        &request,
        &ApprovalView {
            diff: &diff,
            ..view(6)
        },
        80,
        &look(),
    );
    assert!(drawn.clipped);
    let rows = text(&drawn);
    assert!(
        rows.iter().any(|row| row.contains("35 more lines")),
        "{rows:#?}"
    );
    assert!(rows.iter().filter(|row| row.contains("line ")).count() <= 6);
}

#[test]
fn what_a_lasting_answer_would_allow_is_said_in_full() {
    let mut request = request(command("cargo build", false), "Run cargo build");
    request.adds = vec![AddedRule {
        effect: Effect::Exec,
        pattern: "cargo build *".to_owned(),
    }];
    let rows = text(&approval(&request, &view(10), 80, &look()));
    assert!(
        rows.iter().any(|row| row.contains("running cargo build *")),
        "{rows:#?}"
    );
}

#[test]
fn a_note_being_written_takes_the_answers_place() {
    let request = request(command("make", false), "Run make");
    let rows = text(&approval(
        &request,
        &ApprovalView {
            note: Some("use cargo"),
            ..view(10)
        },
        80,
        &look(),
    ));
    assert_eq!(
        rows.last().map(String::as_str),
        Some("  Note for the model: use cargo")
    );
}

/// A diff's line wider than the window loses its end: that is clipped
/// too, so that no lasting answer is offered for it.
#[test]
fn a_diff_line_wider_than_the_window_counts_as_clipped() {
    let request = request(
        ApprovalDetail::Diff(DiffDetail {
            path: "a.rs".to_owned(),
            diff: String::new(),
            omitted: None,
        }),
        "Edit a.rs",
    );
    let line = |text: String| DiffLine {
        number: 1,
        change: Change::Added,
        text,
        emphasis: Vec::new(),
    };
    let short = [line("let x = 1;".to_owned())];
    let drawn = approval(
        &request,
        &ApprovalView {
            diff: &short,
            ..view(10)
        },
        80,
        &look(),
    );
    assert!(!drawn.clipped);
    let long = [line(format!("let x = \"{}\";", "y".repeat(200)))];
    let drawn = approval(
        &request,
        &ApprovalView {
            diff: &long,
            ..view(10)
        },
        80,
        &look(),
    );
    assert!(drawn.clipped);
    assert!(drawn.lines.iter().all(|row| row.width() <= 80));
}
