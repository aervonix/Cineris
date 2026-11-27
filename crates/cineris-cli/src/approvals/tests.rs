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

/// A password is never asked while a debugger is attached: the seam the
/// process hardening's check goes in refuses with words that say why.
#[test]
fn no_password_is_asked_while_a_debugger_is_attached() {
    assert_eq!(untraced(|| false), Ok(()));
    let refused = untraced(|| true).unwrap_err();
    assert!(refused.contains("debugger"), "{refused}");
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
