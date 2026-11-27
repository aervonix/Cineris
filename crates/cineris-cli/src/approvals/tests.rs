use cineris_core::{ApprovalsHeld, HeldKey};
use cineris_protocol::ids::Timestamp;

use super::*;

fn held(key: HeldKey) -> ApprovalsHeld {
    ApprovalsHeld {
        key,
        pins: 0,
        pins_carry_signature: false,
        projects_carrying: 0,
        projects_not_carrying: 0,
        project_carries: false,
        project_pins: Vec::new(),
    }
}

/// The status says what is set up and what carries a signature, in words
/// that say it is not checked, and names no secret.
#[test]
fn the_status_says_what_carries_a_signature_and_that_nothing_is_checked() {
    let none = status_text(&held(HeldKey::None), "cin");
    assert!(
        none.contains("not set") && none.contains("`cin approvals setup`"),
        "{none}"
    );
    assert!(
        none.contains("pins.toml): none") && none.contains("trust.toml): none"),
        "{none}"
    );
    let mut set = held(HeldKey::Made {
        fingerprint: "0123456789abcdef".to_owned(),
        made_at: Timestamp::new("2026-11-27T10:00:00Z"),
    });
    set.pins = 2;
    set.pins_carry_signature = true;
    set.projects_carrying = 3;
    set.projects_not_carrying = 1;
    let text = status_text(&set, "cineris");
    assert!(
        text.contains("key 0123456789abcdef, made 2026-11-27T10:00:00Z"),
        "{text}"
    );
    assert!(
        text.contains("2, carrying a signature, not checked"),
        "{text}"
    );
    assert!(
        text.contains("3 carrying a signature, not checked; 1 carrying none"),
        "{text}"
    );
    assert!(text.contains("checks no signature"), "{text}");
    set.pins_carry_signature = false;
    let text = status_text(&set, "cineris");
    assert!(
        text.contains("2, carrying no signature: each asks again"),
        "{text}"
    );
    let unread = status_text(
        &held(HeldKey::Unreadable("it is not TOML".to_owned())),
        "cin",
    );
    assert!(
        unread.contains("could not be read: it is not TOML"),
        "{unread}"
    );
}

/// A password is never asked while a debugger is attached, nor in a
/// process whose threads are open to the user's other programs (Windows,
/// when the process could not close itself): the seam the process
/// hardening's checks go in refuses with words that say why.
#[test]
fn no_password_is_asked_while_a_debugger_is_attached() {
    assert_eq!(may_ask_with(false, false), Ok(()));
    let refused = may_ask_with(true, false).unwrap_err();
    assert!(refused.contains("debugger"), "{refused}");
    let refused = may_ask_with(true, true).unwrap_err();
    assert!(refused.contains("debugger"), "{refused}");
    let refused = may_ask_with(false, true).unwrap_err();
    assert!(
        refused.contains("could not close its own process")
            && refused.contains("default access list"),
        "{refused}"
    );
    // This test's process is not traced, and on Windows not closed: it is
    // asked of the real checks only where they can say yes.
    if !cfg!(windows) {
        assert_eq!(may_ask(), Ok(()));
    }
}

/// A yes or no read at the terminal is judged in place and written over
/// once judged (`yes_of`): a password typed at the wrong prompt is in it.
#[test]
fn a_yes_or_no_line_is_judged_and_written_over() {
    let before = LINES_WIPED.with(std::cell::Cell::get);
    for (line, yes) in [
        ("y\n", true),
        (" YES \r\n", true),
        ("Yes\n", true),
        ("\n", false),
        ("n\n", false),
        ("Vq7 tidemork 91 Plover\n", false),
        ("yess\n", false),
    ] {
        assert_eq!(yes_of(line.to_owned()), yes, "{line:?}");
    }
    assert_eq!(LINES_WIPED.with(std::cell::Cell::get) - before, 7);
}

/// The reset's warning says what each approval held back, and that what
/// each pinned keeps its gates, until it is approved again: what
/// `docs/architecture/06-permissions-and-sandbox.md` says it warns of.
#[test]
fn the_resets_warning_says_what_keeps_its_gates() {
    assert!(RESET_WARNING.contains("asks again"), "{RESET_WARNING}");
    assert!(
        RESET_WARNING.contains("stays held back")
            && RESET_WARNING.contains("keeps its gates, until you approve it again"),
        "{RESET_WARNING}"
    );
}

/// With no terminal the refusal names the command to run at one, and says
/// that a hash is no approval on its own.
#[test]
fn with_no_terminal_the_refusal_names_the_command_to_run_at_one() {
    let said = no_terminal("cin", "ext add ./tracker");
    assert!(
        said.contains("Run `cin ext add ./tracker` in your own terminal"),
        "{said}"
    );
    assert!(said.contains("not an approval on its own"), "{said}");
}

/// A reset says each store it left as it was, a line each, its words shown
/// as text (the core's reset leaves a store it cannot read or lock, and
/// says so in `ResetReport::left`).
#[test]
fn a_reset_says_each_store_it_left_as_it_was() {
    let report = cineris_core::ResetReport {
        pins: 2,
        projects: 1,
        left: vec![
            "~/.local/state/cineris/trust.toml could not be read".to_owned(),
            "a\u{1b}[2Jb".to_owned(),
        ],
    };
    let said = reset_text("key 0123", &report);
    assert!(
        said.contains("off 2 of your own pins and 1 projects'"),
        "{said}"
    );
    assert!(
        said.contains("Left as it was: ~/.local/state/cineris/trust.toml could not be read\n"),
        "{said}"
    );
    assert!(!said.contains('\u{1b}'), "{said}");
    let none = cineris_core::ResetReport {
        pins: 0,
        projects: 0,
        left: Vec::new(),
    };
    assert!(!reset_text("key 0123", &none).contains("Left as it was"));
}
