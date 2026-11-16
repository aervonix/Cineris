//! The external editor in a pseudo-terminal (ConPTY on Windows): the
//! terminal is in the shell's modes while the editor runs, the prompt's
//! file and folder are private and gone afterwards, the result is read
//! back, a Ctrl+C reaches the editor and not the application, and the
//! terminal is taken back (raw again, a key reaches the application)
//! whatever the editor did. See [`crate::children`].

use std::time::Duration;

use cortex_testkit::pty::Session;

use crate::children::{
    EDITOR_WAITING, LOST, NOT_RESTORED, RESIZED, RESTORED, SHELL, WAITING, this_binary,
};

/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(20);

/// What the test types while the editor waits for a line.
enum Editor {
    /// No editor runs.
    None,
    /// A line, which it reads.
    Line,
    /// Ctrl+C, which ends it.
    Interrupt,
}

/// Runs `scenario` and returns the report, after checking what holds in
/// every scenario: the editor had the terminal as the shell has it, the
/// application got a key in raw mode after it, and the shell found its
/// modes put back.
fn run(scenario: &str, test: &str, editor: &Editor) -> String {
    let scratch = cortex_testkit::Scratch::outside("cortex-tui-external", test);
    let options = format!("{scenario}|{}", scratch.path().display());
    let mut session =
        Session::start(&this_binary(), &SHELL.arguments(&options), 100, 30).answering();
    if !matches!(editor, Editor::None) {
        session.wait_for(LIMIT, "the editor to wait for a line", |t| {
            t.shows(EDITOR_WAITING)
        });
        // ConPTY renders the console itself: the modes it shows are its own.
        if cfg!(unix) {
            let modes = session.screen().modes();
            assert!(
                !modes.bracketed_paste,
                "bracketed paste is on for the editor"
            );
            assert!(!modes.focus, "focus reports are on for the editor");
            assert!(modes.autowrap, "automatic wrap is off for the editor");
            assert!(modes.cursor_visible, "the cursor is hidden for the editor");
            assert!(
                session.screen().keyboard().is_empty(),
                "a keyboard protocol is on for the editor"
            );
        }
        session.send(match editor {
            Editor::Interrupt => b"\x03",
            _ => b"\r",
        });
    }
    session.wait_for(LIMIT, "the application to wait for a key", |t| {
        t.shows(WAITING) || t.shows(LOST)
    });
    assert!(
        !session.screen().shows(LOST),
        "{scenario}: the terminal was not taken back:\n{}",
        session.screen().dump()
    );
    if cfg!(unix) {
        let modes = session.screen().modes();
        assert!(modes.bracketed_paste, "bracketed paste is not on again");
        assert!(
            !modes.alternate,
            "the editor's alternate screen is still shown"
        );
        assert!(
            session.screen().keyboard().len() <= 1,
            "an editor's keyboard flags are left: {:?}",
            session.screen().keyboard()
        );
    }
    // The terminal's handlers are all in place again: a resize reaches the
    // application (the review of step 23: the terminal entered again with
    // the hand-off's handlers kept installs the rest).
    if cfg!(unix) {
        session.resize(110, 30);
        session.wait_for(LIMIT, "the resize to reach the application", |t| {
            t.shows(RESIZED)
        });
    }
    session.send(b"x");
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        t.shows(RESTORED) || t.shows(NOT_RESTORED)
    });
    let screen = session.screen().dump();
    assert!(
        session.screen().shows(RESTORED),
        "the terminal was not put back:\n{screen}"
    );
    session.wait_exit(LIMIT);
    let report = scratch.read_text("report.txt");
    let resized: &[&str] = if cfg!(unix) { &["resized: yes"] } else { &[] };
    for line in [
        "raw again: yes",
        "key: Char('x')",
        "temporary folder empty: yes",
    ]
    .iter()
    .chain(resized)
    {
        assert!(
            report.contains(line) || scenario == "nottext" && line.starts_with("temporary"),
            "{scenario}: no `{line}` in\n{report}"
        );
    }
    if !matches!(editor, Editor::None) {
        for line in [
            "editor modes: yes",
            "private file: yes",
            "private folder: yes",
            "editor folder: yes",
        ] {
            assert!(
                report.contains(line),
                "{scenario}: no `{line}` in\n{report}"
            );
        }
    }
    report
}

#[test]
fn sys_an_edit_comes_back_and_the_terminal_is_handed_over_and_back() {
    let report = run("change", "change", &Editor::Line);
    assert!(report.contains("file had: \"hello\""), "{report}");
    assert!(
        report.contains("editor read: \""),
        "the editor read a line: {report}"
    );
    assert!(
        report.contains("result: changed \"hello edited\" with 0 chips"),
        "{report}"
    );
}

#[test]
fn sys_a_chip_goes_out_as_its_label_and_comes_back_as_the_chip() {
    let report = run("chip", "chip", &Editor::Line);
    assert!(
        report.contains("file had: \"see [pasted 13 lines]\""),
        "{report}"
    );
    let expected = format!("look at {}", "l\n".repeat(13));
    assert!(
        report.contains(&format!("result: changed {expected:?} with 1 chips")),
        "{report}"
    );
}

#[test]
fn sys_a_file_left_as_it_was_changes_nothing() {
    let report = run("same", "same", &Editor::Line);
    assert!(report.contains("result: Unchanged"), "{report}");
    let report = run("newline", "newline", &Editor::Line);
    assert!(
        report.contains("result: Unchanged"),
        "the editor's final line break: {report}"
    );
}

#[test]
fn sys_a_file_left_empty_keeps_the_prompt() {
    let report = run("empty", "empty", &Editor::Line);
    assert!(report.contains("result: Empty"), "{report}");
}

#[test]
fn sys_a_byte_order_mark_and_crlf_are_not_text() {
    let report = run("bom", "bom", &Editor::Line);
    assert!(
        report.contains("result: changed \"bonjour\" with 0 chips"),
        "{report}"
    );
}

#[test]
fn sys_a_file_that_is_not_utf8_is_kept_where_the_user_can_find_it() {
    let report = run("nottext", "nottext", &Editor::Line);
    assert!(report.contains("result: error: NotText("), "{report}");
    assert!(report.contains("temporary folder empty: NO"), "{report}");
}

#[test]
fn sys_a_failing_editor_keeps_the_prompt_and_gives_the_terminal_back() {
    let report = run("fail", "fail", &Editor::Line);
    assert!(
        report.contains("result: error: Failed(Some(3))"),
        "{report}"
    );
}

#[test]
fn sys_ctrl_c_ends_the_editor_and_not_the_application() {
    let report = run("same", "interrupt", &Editor::Interrupt);
    if cfg!(unix) {
        assert!(report.contains("result: error: Killed(2)"), "{report}");
    } else {
        assert!(report.contains("result: error: Failed("), "{report}");
    }
    assert!(
        !report.contains("editor read"),
        "the editor read on: {report}"
    );
}

#[cfg(unix)]
#[test]
fn sys_a_killed_editor_keeps_the_prompt_and_its_modes_are_undone() {
    let report = run("abort", "abort", &Editor::Line);
    assert!(report.contains("result: error: Killed(6)"), "{report}");
}

#[test]
fn sys_an_editor_that_cannot_start_gives_the_terminal_back() {
    let report = run("broken", "broken", &Editor::None);
    assert!(report.contains("result: error: Start("), "{report}");
    assert!(!report.contains("editor modes"), "{report}");
}

#[test]
fn sys_a_missing_editor_is_not_found_and_never_hands_the_terminal_over() {
    let report = run("missing", "missing", &Editor::None);
    assert!(report.contains("result: error: NotFound("), "{report}");
    assert!(!report.contains("editor modes"), "{report}");
}

/// A signal that comes while the editor has the terminal is the
/// application's to end (its handlers stay past leaving the terminal): the
/// editor finishes, the terminal is not taken back, and the program ends as
/// the signal asks, its prompt's folder removed (the review of step 23).
#[cfg(unix)]
#[test]
fn sys_a_signal_while_the_editor_runs_leaves_the_terminal_to_the_shell() {
    let scratch = cortex_testkit::Scratch::outside("cortex-tui-external", "signal");
    let options = format!("signal|{}", scratch.path().display());
    let mut session =
        Session::start(&this_binary(), &SHELL.arguments(&options), 100, 30).answering();
    session.wait_for(LIMIT, "the editor to wait for a line", |t| {
        t.shows(EDITOR_WAITING)
    });
    session.send(b"\r");
    session.wait_for(LIMIT, "the shell's verdict", |t| {
        t.shows(RESTORED) || t.shows(NOT_RESTORED)
    });
    assert!(
        session.screen().shows(RESTORED),
        "the terminal was not put back:\n{}",
        session.screen().dump()
    );
    session.wait_exit(LIMIT);
    let report = scratch.read_text("report.txt");
    for line in [
        "signal sent: yes",
        "taken back: NO",
        "ending: Some(143)",
        "temporary folder empty: yes",
    ] {
        assert!(report.contains(line), "no `{line}` in\n{report}");
    }
}
