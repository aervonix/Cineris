//! The external editor in a pseudo-terminal (ConPTY on Windows): the
//! terminal is in the shell's modes while the editor runs, the prompt's
//! file and folder are private and gone afterwards, the result is read
//! back, and the terminal is taken back (raw again, a key reaches the
//! application) whatever the editor did. See [`crate::children`].

use std::time::Duration;

use cortex_testkit::pty::Session;

use crate::children::{NOT_RESTORED, RESTORED, SHELL, WAITING, child_arguments, this_binary};

/// A bound on every wait, which only a broken test reaches.
const LIMIT: Duration = Duration::from_secs(20);

/// Runs `scenario` and returns the report, after checking what holds in
/// every scenario: the application got a key in raw mode after the editor,
/// and the shell found its modes put back.
fn run(scenario: &str, test: &str) -> String {
    let scratch = cortex_testkit::Scratch::outside("cortex-tui-external", test);
    let options = format!("{scenario}|{}", scratch.path().display());
    let mut session =
        Session::start(&this_binary(), &child_arguments(SHELL, &options), 100, 30).answering();
    session.wait_for(LIMIT, "the application to wait for a key", |t| {
        t.shows(WAITING)
    });
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
    for line in [
        "raw again: yes",
        "key: Char('x')",
        "temporary folder empty: yes",
    ] {
        assert!(
            report.contains(line),
            "{scenario}: no `{line}` in\n{report}"
        );
    }
    report
}

/// What every scenario that ran the editor reports from inside it.
fn editor_saw_the_shell_and_private_files(report: &str) {
    for line in [
        "editor modes: yes",
        "private file: yes",
        "private folder: yes",
    ] {
        assert!(report.contains(line), "no `{line}` in\n{report}");
    }
}

#[test]
fn sys_an_edit_comes_back_and_the_terminal_is_handed_over_and_back() {
    let report = run("change", "change");
    editor_saw_the_shell_and_private_files(&report);
    assert!(report.contains("file had: \"hello\""), "{report}");
    assert!(
        report.contains("result: changed \"hello edited\" with 0 chips"),
        "{report}"
    );
}

#[test]
fn sys_a_chip_goes_out_as_its_label_and_comes_back_as_the_chip() {
    let report = run("chip", "chip");
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
    let report = run("same", "same");
    assert!(report.contains("result: Unchanged"), "{report}");
    let report = run("newline", "newline");
    assert!(
        report.contains("result: Unchanged"),
        "the editor's final line break: {report}"
    );
}

#[test]
fn sys_a_file_left_empty_keeps_the_prompt() {
    let report = run("empty", "empty");
    assert!(report.contains("result: Empty"), "{report}");
}

#[test]
fn sys_a_failing_editor_keeps_the_prompt_and_gives_the_terminal_back() {
    let report = run("fail", "fail");
    editor_saw_the_shell_and_private_files(&report);
    assert!(
        report.contains("result: error: Failed(Some(3))"),
        "{report}"
    );
}

#[cfg(unix)]
#[test]
fn sys_a_killed_editor_keeps_the_prompt_and_gives_the_terminal_back() {
    let report = run("abort", "abort");
    assert!(report.contains("result: error: Killed(6)"), "{report}");
}

#[test]
fn sys_an_editor_that_cannot_start_gives_the_terminal_back() {
    let report = run("broken", "broken");
    assert!(report.contains("result: error: Start("), "{report}");
    assert!(!report.contains("editor modes"), "{report}");
}

#[test]
fn sys_a_missing_editor_is_not_found_and_the_terminal_is_untouched() {
    let report = run("missing", "missing");
    assert!(report.contains("result: error: NotFound("), "{report}");
}
