//! The approval dialog's rows, as screens 2a and 2b draw them.

use cineris_protocol::lease::Effect;
use cineris_protocol::methods::{
    AddedRule, ApprovalChoice, ApprovalDetail, ApprovalRequest, CommandDetail, DiffDetail,
    QuestionRequest, TargetDetail, ToolDetail, TrustRequest, TrustResource, TrustResourceKind,
};
use cineris_protocol::{CallId, SessionId, TurnId};
use cineris_term::ColorDepth;

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
        hooks: Vec::new(),
        replaced: None,
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

/// What acts unseen and is not a control character: a bidirectional
/// override, a zero-width space and a tag character.
const UNSEEN: &str = "\u{202e}\u{200b}\u{e0041}";

/// `UNSEEN` as a field holding it after `name` is drawn.
fn escaped(name: &str) -> String {
    format!("{name}\\u{{202e}}\\u{{200b}}\\u{{e0041}}")
}

/// Every row of `drawn` is safe to show, and each of `fields` is drawn
/// with what acts unseen written as its escape.
fn shows_escaped(drawn: &Drawn, width: usize, fields: &[&str]) {
    let found = crate::testing::problems(&drawn.lines, width);
    assert!(found.is_empty(), "{found:#?}");
    let all = text(drawn).join("\n");
    for field in fields {
        assert!(all.contains(&escaped(field)), "{field}: {all}");
    }
}

/// Each field of an approval that comes from outside, the request's
/// summary, its reasons, a command's folder, a lasting rule's pattern, a
/// tool's arguments, a target and a diff's path and why it is left out,
/// is drawn with what acts unseen escaped (the review of step 23: the
/// hostile tests held only control characters, which the renderer shows
/// as symbols anyway).
#[test]
fn every_field_of_an_approval_escapes_what_acts_unseen() {
    let look = look();
    let mut asked = request(command("make", false), &format!("summary{UNSEEN}"));
    asked.reasons = vec![format!("reason{UNSEEN}")];
    asked.adds = vec![AddedRule {
        effect: Effect::Exec,
        pattern: format!("pattern{UNSEEN}"),
    }];
    if let ApprovalDetail::Command(detail) = &mut asked.detail {
        detail.cwd = format!("cwd{UNSEEN}");
    }
    let width = 120;
    shows_escaped(
        &approval(&asked, &view(10), width, &look),
        width,
        &["summary", "reason", "pattern", "in cwd"],
    );
    asked.detail = ApprovalDetail::Tool(ToolDetail {
        arguments: serde_json::json!({ "path": format!("arguments{UNSEEN}") }),
    });
    shows_escaped(
        &approval(&asked, &view(10), width, &look),
        width,
        &["arguments"],
    );
    asked.detail = ApprovalDetail::Target(TargetDetail {
        effect: Effect::Read,
        target: format!("target{UNSEEN}"),
    });
    shows_escaped(
        &approval(&asked, &view(10), width, &look),
        width,
        &["Read target"],
    );
    asked.detail = ApprovalDetail::Diff(DiffDetail {
        path: format!("path{UNSEEN}"),
        diff: String::new(),
        omitted: Some(format!("omitted{UNSEEN}")),
    });
    shows_escaped(
        &approval(&asked, &view(10), width, &look),
        width,
        &["path", "omitted"],
    );
}

/// A call whose arguments a hook replaced (step 28): the dialog says which
/// hook changed it and why, in the warning colour, and its detail begins
/// with both sets of arguments, the model's and the hook's, before what
/// would run. The line `reasons` holds for a client that does not read
/// `replaced` is not drawn twice, and nor is a hook's that asked.
#[test]
fn a_replacement_shows_the_hook_and_both_sets_of_arguments() {
    use cineris_protocol::item::{HookReason, ReplacedArguments};
    let mut asked = request(
        command("cargo test --offline", false),
        "Run cargo test --offline",
    );
    let replaced = ReplacedArguments {
        hook: "offline".to_owned(),
        arguments: serde_json::json!({"command": "cargo test --offline"}),
        original: Some(serde_json::json!({"command": "cargo test"})),
        reason: Some("Tests run offline here.".to_owned()),
    };
    let slow = HookReason {
        hook: "slow".to_owned(),
        reason: Some("the suite takes ten minutes".to_owned()),
    };
    // As the core gives it: a line for each, for older clients.
    asked.reasons = vec![slow.asking_line(), replaced.reason_line()];
    asked.hooks = vec![slow];
    asked.replaced = Some(Box::new(replaced));
    let drawn = approval(&asked, &view(20), 99, &look());
    let rows = text(&drawn);
    assert_eq!(rows[0], "? Run cargo test --offline?");
    assert_eq!(
        rows[1],
        "  The hook `slow` asks: the suite takes ten minutes"
    );
    let said = rows[2..]
        .iter()
        .take_while(|row| !row.is_empty())
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        said,
        "The hook `offline` changed the arguments the model wrote: the call would run with the \
         hook's. Its reason: Tests run offline here."
    );
    let from = rows
        .iter()
        .position(|row| row == "    The model wrote:")
        .expect("the model's arguments");
    assert_eq!(
        rows[from..from + 5],
        [
            "    The model wrote:",
            r#"      {"command":"cargo test"}"#,
            "    The hook's, which would run:",
            r#"      {"command":"cargo test --offline"}"#,
            "",
        ]
        .map(str::to_owned),
        "{rows:#?}"
    );
    assert_eq!(
        rows[from + 5],
        "    cargo test --offline",
        "then what would run"
    );
    let all = rows.join("\n");
    assert_eq!(all.matches("The hook `slow`").count(), 1, "{all}");
    assert_eq!(all.matches("The hook `offline`").count(), 1, "{all}");
}

/// A hook that asked is named from `hooks` when `reasons` does not say it.
#[test]
fn a_hook_that_asked_is_named_when_the_reasons_do_not_say_it() {
    let mut asked = request(command("make", false), "Run make");
    asked.hooks = vec![cineris_protocol::item::HookReason {
        hook: "look".to_owned(),
        reason: None,
    }];
    let rows = text(&approval(&asked, &view(10), 99, &look()));
    assert_eq!(
        rows[2], "  The hook `look` asks about this call.",
        "{rows:#?}"
    );
}

/// What a hook gave (its name, its reason, both sets of arguments) is
/// drawn with what acts unseen escaped.
#[test]
fn what_a_hook_gave_is_drawn_escaped() {
    let mut asked = request(command("make", false), "Run make");
    asked.reasons.clear();
    asked.hooks = vec![cineris_protocol::item::HookReason {
        hook: format!("asker{UNSEEN}"),
        reason: Some(format!("because{UNSEEN}")),
    }];
    asked.replaced = Some(Box::new(cineris_protocol::item::ReplacedArguments {
        hook: format!("swap{UNSEEN}"),
        arguments: serde_json::json!({ "path": format!("theirs{UNSEEN}") }),
        original: Some(serde_json::json!({ "path": format!("mine{UNSEEN}") })),
        reason: Some(format!("why{UNSEEN}")),
    }));
    let width = 140;
    shows_escaped(
        &approval(&asked, &view(10), width, &look()),
        width,
        &["asker", "because", "swap", "why", "theirs", "mine"],
    );
}

/// The trust question's workspace, each resource's detail and a rule's
/// profile are drawn with what acts unseen escaped.
#[test]
fn every_field_of_the_trust_question_escapes_what_acts_unseen() {
    let request = TrustRequest {
        session_id: SessionId::from("s_1"),
        trusted: false,
        workspace: format!("workspace{UNSEEN}"),
        resources: vec![
            TrustResource {
                kind: TrustResourceKind::Repository,
                name: "repository".to_owned(),
                detail: format!("detail{UNSEEN}"),
                changed: false,
            },
            TrustResource {
                kind: TrustResourceKind::Rules,
                name: format!("profiles.ci{UNSEEN}.permissions.rules"),
                detail: format!("rule{UNSEEN}"),
                changed: false,
            },
        ],
    };
    let view = TrustView {
        answers: answers(),
        answers_hidden: false,
        detail_rows: 10,
        unsaid: false,
    };
    let width = 120;
    shows_escaped(
        &trust(&request, &view, width, &look()),
        width,
        &["workspace", "detail", "rule", "(profile ci"],
    );
}

/// Each line of a resource's detail is a row of its own: an extension's
/// author's words, which the core puts on a line apart, begin a row, so
/// that no quote in them can seem to close what the harness says before
/// them (the full review of step 28).
#[test]
fn each_line_of_a_trust_resources_detail_begins_a_row() {
    let request = TrustRequest {
        session_id: SessionId::from("s_1"),
        trusted: false,
        workspace: "app".to_owned(),
        resources: vec![TrustResource {
            kind: TrustResourceKind::Extension,
            name: "probe".to_owned(),
            detail: "`probe` in `.cineris/extensions/probe`. It runs no program.\nIts author \
                     describes it, in their words:\n\"Tracks issues\""
                .to_owned(),
            changed: false,
        }],
    };
    let view = TrustView {
        answers: answers(),
        answers_hidden: false,
        detail_rows: 10,
        unsaid: false,
    };
    let rows = text(&trust(&request, &view, 120, &look()));
    let at = |words: &str| {
        rows.iter()
            .position(|row| row.contains(words))
            .unwrap_or_else(|| panic!("{words:?} is not drawn: {rows:#?}"))
    };
    let harness = at("It runs no program.");
    let label = at("Its author describes it, in their words:");
    let quoted = at("\"Tracks issues\"");
    assert!(harness < label && label < quoted, "{rows:#?}");
    // Nothing of the harness's words before the label on its row.
    let before = &rows[label][..rows[label].find("Its author").unwrap()];
    assert!(
        !before.chars().any(char::is_alphanumeric),
        "{:?}",
        rows[label]
    );
}

/// A question of the core's is drawn with what acts unseen escaped.
/// What a rewind does to a file is said in words only where this build
/// knows both what the file goes back to and whether it can: a target or a
/// status it does not know is said to be unknown, whatever the other is
/// (the review of step 25's second pull request found an unknown target
/// worded as going back).
#[test]
fn a_rewinds_file_this_build_cannot_read_is_not_promised_anything() {
    use cineris_protocol::methods::{RewindFile, RewindFileStatus, RewindTarget};
    let file = |to: RewindTarget, status: RewindFileStatus| RewindFile {
        path: "a.txt".to_owned(),
        to,
        status,
        why_not: None,
    };
    let unknown = RewindTarget::Unknown("moved".to_owned());
    for status in [
        RewindFileStatus::AsLeft,
        RewindFileStatus::Already,
        RewindFileStatus::EditedSince,
        RewindFileStatus::Unknown("held".to_owned()),
    ] {
        assert_eq!(
            what_happens(&file(unknown.clone(), status.clone())),
            "cannot be told: this build does not know what the plan says",
            "{status:?}"
        );
    }
    assert_eq!(
        what_happens(&file(RewindTarget::Content, RewindFileStatus::AsLeft)),
        "goes back to what it was before"
    );
    assert!(
        what_happens(&file(unknown, RewindFileStatus::Cannot)).starts_with("stays as it is"),
        "a file that stays, stays"
    );
}

#[test]
fn a_question_escapes_what_acts_unseen() {
    let request = QuestionRequest {
        session_id: SessionId::from("s_1"),
        turn_id: None,
        question: format!("question{UNSEEN}"),
        choices: Vec::new(),
        extension: None,
    };
    let view = QuestionView {
        answers: answers(),
        ..QuestionView::default()
    };
    let width = 120;
    shows_escaped(
        &question(&request, &view, width, &look()),
        width,
        &["question"],
    );
}
