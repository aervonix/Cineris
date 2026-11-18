//! The integration tests of `cineris-term`: the presenter against the
//! virtual terminal (tier 2), and the demo in a pseudo-terminal (tier 3,
//! `sys_…`), on both systems.
//!
//! # Children
//!
//! The pseudo-terminal tests run this test binary again, as the demo or as a
//! small shell that runs the demo and checks the terminal after it. The
//! entry test [`child_entry`] becomes that child when its arguments say so,
//! and passes at once otherwise. No other program is built.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "tests: children are processes, the shell child waits for its own and reports on \
              its terminal, and a test fails by panicking"
)]

#[path = "../../examples/demo/app.rs"]
#[allow(
    dead_code,
    unreachable_pub,
    reason = "the demo is the example's, and this binary uses part of it"
)]
mod demo;
mod full;
mod handoff;
mod perf;
mod presenter;
mod signals;
mod system;

/// The name of the entry test.
use cineris_testkit::role::{ModeShell, NOT_RESTORED, RESTORED, Role, this_binary};

/// This binary as the demo, with the demo's options, separated by commas.
const DEMO: Role = Role::new("cxterm-demo=");
/// This binary as a shell that runs the demo with the options given, and
/// reports whether the terminal was put back. On Windows it first turns
/// virtual-terminal processing off, as the old console host has it, so the
/// demo must turn it on itself.
const SHELL: Role = Role::new("cxterm-shell=");
/// What the shell prints just before it starts the demo.
const STARTING: &str = "starting the demo";
/// This binary as a shell that runs [`LEAVES_RAW`], the check that the
/// shell says when the modes were not put back.
const SHELL_OVER_RAW: Role = Role::new("cxterm-shell-over-raw=");
/// This binary as a program that enters the terminal and ends without
/// leaving it: raw mode left on.
const LEAVES_RAW: Role = Role::new("cxterm-leaves-raw=");

#[test]
fn child_entry() {
    if let Some(options) = DEMO.options() {
        let options = demo::Options::parse(options.split(',')).expect("the demo's options");
        demo::run(&options).expect("the demo runs");
        #[expect(clippy::exit, reason = "a child ends when the demo does")]
        std::process::exit(0);
    }
    if let Some(options) = SHELL.options() {
        ModeShell::new()
            .as_the_old_console()
            .announcing(STARTING)
            .run(DEMO.command(&options));
    }
    if SHELL_OVER_RAW.options().is_some() {
        ModeShell::new().run(LEAVES_RAW.command(""));
    }
    if LEAVES_RAW.options().is_some() {
        let terminal =
            cineris_term::Terminal::enter(cineris_term::Overrides::default()).expect("enter");
        std::mem::forget(terminal);
        #[expect(clippy::exit, reason = "the child ends with the terminal left raw")]
        std::process::exit(0);
    }
    signals::enter();
    handoff::enter();
}
