//! The test binary run again as a child, in a role (`docs/testing-and-ci.md`,
//! "The test binary in roles"), for tests of a terminal: a shell that checks
//! the terminal after a program, the program, and whatever the program runs
//! in turn (an editor).
//!
//! - **A crate's integration binary has one entry test,** [`ENTRY`], which
//!   passes at once under the test runner. A [`Role`] is a mark, such as
//!   `cxterm-demo=`: [`Role::command`] starts this binary with the entry
//!   test and the mark with the role's options after it, and in the child
//!   [`Role::options`] gives them back, so the entry test knows which part
//!   to play.
//! - **What a child found goes in a [`Report`]:** a file of `name: value`
//!   lines in the test's scratch directory, which the test reads once the
//!   child is gone. A child's standard output is the terminal, which a
//!   pseudo-terminal re-renders on Windows.
//! - **A [`ModeShell`]** records the terminal's modes, runs a child, and
//!   prints [`RESTORED`] when the modes after it are the ones before, or
//!   [`NOT_RESTORED`] with both. It gives its children the modes it saw in
//!   [`SHELL_MODES`], so that a grandchild can tell whether it was handed
//!   the terminal as the shell has it ([`same_modes_as_the_shell`]).
//!
//! Before this module, `cineris-term` and `cineris-tui` each had their own
//! copy of the entry, the arguments and the shell.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support: a child of the test binary, its report file and a shell that waits \
              for its child, each on its own thread"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The name of the entry test every binary that uses roles has.
pub const ENTRY: &str = "child_entry";

/// What a [`ModeShell`] prints when the terminal's modes after its child
/// are the ones before.
pub const RESTORED: &str = "restored: yes";

/// What it prints when they are not, with both after it.
pub const NOT_RESTORED: &str = "restored: NO";

/// The variable a [`ModeShell`] gives its children: the modes it saw before
/// it started them.
pub const SHELL_MODES: &str = "CINERIS_TESTKIT_SHELL_MODES";

/// A part this test binary plays as a child: a mark, ending in `=`, with
/// the role's options after it on the child's command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Role {
    mark: &'static str,
}

impl Role {
    /// The role marked `mark`, such as `cxterm-demo=`.
    #[must_use]
    pub const fn new(mark: &'static str) -> Self {
        Self { mark }
    }

    /// The arguments that start this binary in this role with `options`:
    /// the entry test alone, on one thread, its output not captured.
    #[must_use]
    pub fn arguments(self, options: &str) -> Vec<String> {
        vec![
            "--exact".to_owned(),
            ENTRY.to_owned(),
            "--nocapture".to_owned(),
            "--test-threads=1".to_owned(),
            format!("{}{options}", self.mark),
        ]
    }

    /// This binary, in this role with `options`.
    #[must_use]
    pub fn command(self, options: &str) -> Command {
        let mut command = Command::new(this_binary());
        command.args(self.arguments(options));
        command
    }

    /// The command line that starts this binary in this role, quoted as a
    /// variable such as `$EDITOR` holds one.
    #[must_use]
    pub fn command_line(self, options: &str) -> String {
        format!(
            "\"{}\" {}",
            this_binary(),
            self.arguments(options).join(" ")
        )
    }

    /// The options this process was started in this role with, if it was.
    #[must_use]
    pub fn options(self) -> Option<String> {
        std::env::args().find_map(|argument| argument.strip_prefix(self.mark).map(str::to_owned))
    }
}

/// This test binary's path.
///
/// # Panics
///
/// When the system does not say, or the path is not Unicode.
#[must_use]
pub fn this_binary() -> String {
    std::env::current_exe()
        .expect("the test binary has a path")
        .to_str()
        .expect("the test binary's path is Unicode")
        .to_owned()
}

/// `yes` or `NO`, as a report writes a check.
#[must_use]
pub fn yes(ok: bool) -> &'static str {
    if ok { "yes" } else { "NO" }
}

/// A file of `name: value` lines that children write and a test reads.
#[derive(Debug, Clone)]
pub struct Report {
    path: PathBuf,
}

impl Report {
    /// The report `report.txt` in `dir`.
    #[must_use]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join("report.txt"),
        }
    }

    /// Where it is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Adds `name: value`.
    ///
    /// # Panics
    ///
    /// When the file cannot be written: a child that cannot report has no
    /// other way to say so.
    pub fn say(&self, name: &str, value: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .expect("open the report");
        writeln!(file, "{name}: {value}").expect("write the report");
    }

    /// Every line, in order, as `(name, value)`. None when there is no
    /// report.
    #[must_use]
    pub fn lines(&self) -> Vec<(String, String)> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines()
            .filter_map(|line| line.split_once(": "))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect()
    }

    /// The last value said for `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<String> {
        self.lines()
            .into_iter()
            .rev()
            .find(|(said, _)| said == name)
            .map(|(_, value)| value)
    }
}

/// Whether this process's terminal has the modes the [`ModeShell`] above it
/// saw before it started its child. `None` when no shell said.
///
/// # Panics
///
/// When this process has no terminal.
#[must_use]
pub fn same_modes_as_the_shell() -> Option<bool> {
    let shell = std::env::var(SHELL_MODES).ok()?;
    Some(cineris_term::pty::modes().expect("the terminal's modes") == shell)
}

/// A shell child: records the terminal's modes, runs a child, and says
/// whether the modes after it are the ones before.
#[derive(Debug, Default)]
pub struct ModeShell {
    old_console: bool,
    ignoring_interrupts: bool,
    surviving_hangup: bool,
    announce: Option<String>,
}

impl ModeShell {
    /// A shell that does nothing to the terminal before its child.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Turns virtual-terminal processing off before the child, as the old
    /// console host has it (Windows; nothing on Unix), so that the child
    /// must turn it on itself.
    #[must_use]
    pub fn as_the_old_console(mut self) -> Self {
        self.old_console = true;
        self
    }

    /// Passes the interrupt keys over while the child runs, as a shell
    /// does with its job: a `Ctrl+C` typed for the child is not the
    /// shell's (`cineris_term::ignore_interrupts`).
    #[must_use]
    pub fn ignoring_interrupts(mut self) -> Self {
        self.ignoring_interrupts = true;
        self
    }

    /// Stays through the terminal hanging up while the child runs (Unix:
    /// SIGHUP, with a handler that does nothing, which the child does not
    /// inherit), so that the shell can say how its child ended after
    /// [`Session::close_console`](crate::pty::Session::close_console). Does
    /// nothing on Windows, where the shell goes with the console.
    #[must_use]
    pub fn surviving_hangup(mut self) -> Self {
        self.surviving_hangup = true;
        self
    }

    /// Prints `text` just before the child starts.
    #[must_use]
    pub fn announcing(mut self, text: &str) -> Self {
        self.announce = Some(text.to_owned());
        self
    }

    /// Runs `child` and reports on the terminal, then ends this process.
    ///
    /// # Panics
    ///
    /// When the terminal's modes cannot be read or the child cannot be
    /// started.
    pub fn run(self, mut child: Command) -> ! {
        cineris_term::pty::ignore_control_events();
        if self.old_console {
            cineris_term::pty::set_vt_output(false).expect("turn off virtual-terminal processing");
        }
        if self.surviving_hangup {
            cineris_term::pty::pass_over_hangup().expect("pass SIGHUP over");
        }
        let before = cineris_term::pty::modes().expect("the terminal's modes");
        if let Some(text) = &self.announce {
            write_terminal(&format!("{text}\r\n"));
        }
        let ignored = self
            .ignoring_interrupts
            .then(|| cineris_term::ignore_interrupts().expect("ignore interrupts"));
        let status = child
            .env(SHELL_MODES, &before)
            .status()
            .expect("start the child");
        drop(ignored);
        let after = cineris_term::pty::modes().expect("the terminal's modes");
        // Plain text, which needs no escape sequence: the console may not
        // process them now.
        if before == after {
            write_terminal(&format!("\r\n{RESTORED} ({status})\r\n"));
        } else {
            write_terminal(&format!(
                "\r\n{NOT_RESTORED} ({status})\r\nbefore: {before}\r\nafter:  {after}\r\n"
            ));
        }
        std::process::exit(0);
    }
}

/// Writes `text` to this process's terminal, at once.
///
/// # Panics
///
/// When it cannot: a child's terminal is how it reports.
pub fn write_terminal(text: &str) {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .expect("write to the terminal");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_roles_arguments_run_the_entry_test_alone_with_its_options() {
        let role = Role::new("cin-demo=");
        assert_eq!(
            role.arguments("rate=400,keys"),
            [
                "--exact",
                ENTRY,
                "--nocapture",
                "--test-threads=1",
                "cin-demo=rate=400,keys"
            ]
        );
        // Not started in the role, this process has no options for it.
        assert_eq!(role.options(), None);
        assert!(
            role.command_line("x")
                .ends_with(" --exact child_entry --nocapture --test-threads=1 cin-demo=x")
        );
    }

    #[test]
    fn a_report_keeps_each_line_and_gives_the_last_value_of_a_name() {
        let scratch = crate::Scratch::outside("cineris-testkit-role", "role_report");
        let report = Report::in_dir(scratch.path());
        assert!(report.lines().is_empty());
        report.say("result", "changed");
        report.say("raw again", yes(true));
        report.say("result", "again: with a colon");

        assert_eq!(report.lines().len(), 3);
        assert_eq!(report.get("result").as_deref(), Some("again: with a colon"));
        assert_eq!(report.get("raw again").as_deref(), Some("yes"));
        assert_eq!(report.get("missing"), None);
        assert_eq!(yes(false), "NO");
    }
}
