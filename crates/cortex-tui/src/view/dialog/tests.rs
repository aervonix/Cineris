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

fn text(lines: &[cortex_term::Line]) -> Vec<String> {
    lines
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
    let lines = approval(
        &request,
        &ApprovalView {
            answers: answers(),
            diff,
            detail_rows: 10,
            note: None,
        },
        99,
        &look(),
    );
    let rows = text(&lines);
    assert_eq!(rows[0], "? Edit .github/workflows/ci.yml?");
    assert_eq!(
        rows[1],
        "  CI configuration is protected, so a change to it always asks."
    );
    assert_eq!(rows[2], "");
    assert_eq!(rows[3], "     29       - uses: x");
    assert_eq!(rows[4], "     30 −       with: a");
    assert_eq!(rows[5], "     30 +       with: b");
    assert_eq!(rows.last().map(String::as_str), Some("  y Allow once   n Deny"));
    assert!(lines.iter().all(|line| line.width() <= 99));
}

#[test]
fn leaving_the_sandbox_asks_its_own_question_and_says_what_it_costs() {
    let request = request(
        ApprovalDetail::Command(CommandDetail {
            command: "cargo publish --dry-run -p parser".to_owned(),
            cwd: "crates/parser".to_owned(),
            outside_sandbox: true,
        }),
        "Run cargo publish",
    );
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: vec![Answer {
                key: "y".to_owned(),
                label: "Run once outside the sandbox".to_owned(),
            }],
            ..ApprovalView::default()
        },
        80,
        &look(),
    ));
    assert_eq!(rows[0], "? Run outside the sandbox?");
    assert!(rows.contains(&"    cargo publish --dry-run -p parser".to_owned()), "{rows:#?}");
    assert!(rows.contains(&"    in crates/parser".to_owned()), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("full rights")), "{rows:#?}");
    assert!(rows.iter().any(|row| row.contains("asked every time")), "{rows:#?}");
    // Its reasons are not repeated: the question says it all.
    assert!(!rows.iter().any(|row| row.contains("protected")), "{rows:#?}");
}

#[test]
fn a_command_is_shown_whole_and_what_acts_unseen_is_escaped() {
    let request = request(
        ApprovalDetail::Command(CommandDetail {
            command: "echo one\necho two\u{202e}x".to_owned(),
            cwd: "/work".to_owned(),
            outside_sandbox: false,
        }),
        "Run echo",
    );
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: answers(),
            ..ApprovalView::default()
        },
        80,
        &look(),
    ));
    assert!(rows.contains(&"    echo one".to_owned()), "{rows:#?}");
    assert!(rows.contains(&"    echo two\\u{202e}x".to_owned()), "{rows:#?}");
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
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: answers(),
            diff,
            detail_rows: 6,
            note: None,
        },
        80,
        &look(),
    ));
    assert!(rows.iter().any(|row| row.contains("35 more lines")), "{rows:#?}");
    assert!(rows.iter().filter(|row| row.contains("line ")).count() <= 6);
}

#[test]
fn what_a_lasting_answer_would_allow_is_said_in_full() {
    let mut request = request(
        ApprovalDetail::Command(CommandDetail {
            command: "cargo build".to_owned(),
            cwd: "/work".to_owned(),
            outside_sandbox: false,
        }),
        "Run cargo build",
    );
    request.adds = vec![AddedRule {
        effect: Effect::Exec,
        pattern: "cargo build *".to_owned(),
    }];
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: answers(),
            ..ApprovalView::default()
        },
        80,
        &look(),
    ));
    assert!(
        rows.iter().any(|row| row.contains("running cargo build *")),
        "{rows:#?}"
    );
}

#[test]
fn a_note_being_written_takes_the_answers_place() {
    let request = request(
        ApprovalDetail::Command(CommandDetail {
            command: "make".to_owned(),
            cwd: "/work".to_owned(),
            outside_sandbox: false,
        }),
        "Run make",
    );
    let rows = text(&approval(
        &request,
        &ApprovalView {
            answers: answers(),
            note: Some("use cargo"),
            ..ApprovalView::default()
        },
        80,
        &look(),
    ));
    assert_eq!(
        rows.last().map(String::as_str),
        Some("  Note for the model: use cargo")
    );
}
