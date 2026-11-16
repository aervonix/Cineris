//! How a process ended, in one form on every system: the matcher a test
//! uses so that it does not spell Linux's wording and Windows' side by side.
//!
//! `std::process::ExitStatus` says `exit status: 1` on Unix and
//! `exit code: 1` on Windows, a signal as `signal: 6 (SIGABRT)`, and a fault
//! that Windows ends a process with as `exit code: 0xc0000005`. A program in
//! a pseudo-terminal cannot hand its status to the test as a value (the
//! [`ModeShell`](crate::role::ModeShell) prints it on the screen), so a test
//! reads it as text; this module reads either system's wording into an
//! [`Exit`], and `Exit::of` makes the same from the status itself.
//!
//! `Exit::parse("exit status: 143")` and `Exit::parse("exit code: 143")` are
//! both `Some(Exit::Code(143))`; `Exit::parse("signal: 6 (SIGABRT)")` is
//! `Some(Exit::Signal(6))`.

use std::process::ExitStatus;

use crate::role::{NOT_RESTORED, RESTORED};

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// It ended by itself with this code. The same on both systems.
    Code(i32),
    /// A signal ended it (Unix). The signal's number.
    Signal(i32),
    /// The system ended it for a fault, as a Windows status code with its
    /// high bit set (`0xc000013a` is the console's Ctrl+C). On Unix, a fault
    /// is a [`Signal`](Self::Signal).
    Fault(u32),
}

impl Exit {
    /// How `status` says the process ended, on this system.
    #[must_use]
    pub fn of(status: ExitStatus) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt as _;
            match (status.code(), status.signal()) {
                (Some(code), _) => Self::Code(code),
                (None, Some(signal)) => Self::Signal(signal),
                // Stopped or continued, which is not an end: -1, which no
                // program's own code is.
                (None, None) => Self::Code(-1),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt as _;
            Self::from_windows(status.raw())
        }
    }

    /// A Windows exit code: one with the high bit set is a fault.
    #[must_use]
    pub fn from_windows(raw: u32) -> Self {
        match i32::try_from(raw) {
            Ok(code) => Self::Code(code),
            Err(_) => Self::Fault(raw),
        }
    }

    /// Reads what `ExitStatus`'s `Display` says on either system:
    /// `exit status: N`, `exit code: N` or `exit code: 0xHEX`, and
    /// `signal: N` with its name and `(core dumped)` after it. Surrounding
    /// brackets and white space are ignored. `None` for any other text,
    /// including a stop (`stopped (not terminated) by signal`), which is not
    /// an end.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_matches(['(', ')']).trim();
        if let Some(rest) = text
            .strip_prefix("exit status:")
            .or_else(|| text.strip_prefix("exit code:"))
        {
            let number = rest.split_whitespace().next()?;
            return match number.strip_prefix("0x") {
                Some(hex) => u32::from_str_radix(hex, 16).ok().map(Self::from_windows),
                None => number.parse().ok().map(Self::Code),
            };
        }
        let rest = text.strip_prefix("signal:")?;
        rest.split_whitespace()
            .next()?
            .parse()
            .ok()
            .map(Self::Signal)
    }

    /// Whether the process ended by itself with `code`.
    #[must_use]
    pub fn is_code(self, code: i32) -> bool {
        self == Self::Code(code)
    }
}

/// What a [`ModeShell`](crate::role::ModeShell) said when its child ended:
/// whether the terminal's modes were put back, and how the child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// The modes after the child were the ones before.
    pub restored: bool,
    /// How the child ended.
    pub exit: Exit,
}

impl Verdict {
    /// The last verdict `text` holds: the shell prints
    /// `restored: yes (exit status: 0)` (with `exit code` on Windows) on a
    /// line of its own. `None` when there is none, or when its end is not
    /// on the screen yet.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (restored, rest) = text.lines().rev().find_map(|line| {
            if let Some((_, rest)) = line.split_once(RESTORED) {
                Some((true, rest))
            } else {
                Some((false, line.split_once(NOT_RESTORED)?.1))
            }
        })?;
        // ` (exit status: 143)`: the status's own text may hold brackets
        // (`signal: 6 (SIGABRT)`), so only the last is the verdict's.
        let inside = rest.trim().strip_prefix('(')?.strip_suffix(')')?;
        Some(Self {
            restored,
            exit: Exit::parse(inside)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_systems_wordings_of_a_code_read_the_same() {
        for text in ["exit status: 143", "exit code: 143", " (exit code: 143) "] {
            assert_eq!(Exit::parse(text), Some(Exit::Code(143)), "{text}");
        }
        assert_eq!(Exit::parse("exit status: 0"), Some(Exit::Code(0)));
        assert!(Exit::Code(0).is_code(0));
        assert!(!Exit::Signal(0).is_code(0));
    }

    #[test]
    fn a_signal_and_a_windows_fault_are_not_a_code() {
        assert_eq!(Exit::parse("signal: 6 (SIGABRT)"), Some(Exit::Signal(6)));
        assert_eq!(
            Exit::parse("signal: 11 (SIGSEGV) (core dumped)"),
            Some(Exit::Signal(11))
        );
        // Windows spells a status with the high bit set in hexadecimal.
        assert_eq!(
            Exit::parse("exit code: 0xc000013a"),
            Some(Exit::Fault(0xc000_013a))
        );
        assert_eq!(Exit::from_windows(0xc000_013a), Exit::Fault(0xc000_013a));
        assert_eq!(Exit::from_windows(143), Exit::Code(143));
    }

    #[test]
    fn text_that_is_not_an_ending_is_not_read_as_one() {
        for text in [
            "",
            "exit status:",
            "exit status: many",
            "stopped (not terminated) by signal: 19 (SIGSTOP)",
            "restored: yes",
        ] {
            assert_eq!(Exit::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_status_of_this_system_reads_as_its_own_text_does() {
        #[cfg(unix)]
        let statuses = {
            use std::os::unix::process::ExitStatusExt as _;
            [
                (ExitStatus::from_raw(143 << 8), Exit::Code(143)),
                (ExitStatus::from_raw(0), Exit::Code(0)),
                (ExitStatus::from_raw(6), Exit::Signal(6)),
            ]
        };
        #[cfg(windows)]
        let statuses = {
            use std::os::windows::process::ExitStatusExt as _;
            [
                (ExitStatus::from_raw(143), Exit::Code(143)),
                (ExitStatus::from_raw(0), Exit::Code(0)),
                (ExitStatus::from_raw(0xc000_013a), Exit::Fault(0xc000_013a)),
            ]
        };
        for (status, exit) in statuses {
            assert_eq!(Exit::of(status), exit, "{status}");
            assert_eq!(Exit::parse(&status.to_string()), Some(exit), "{status}");
        }
    }

    #[test]
    fn the_shells_verdict_is_read_from_a_screen_with_either_wording() {
        for wording in ["exit status", "exit code"] {
            let screen = format!("starting cortex\n\nrestored: yes ({wording}: 129)\n");
            assert_eq!(
                Verdict::parse(&screen),
                Some(Verdict {
                    restored: true,
                    exit: Exit::Code(129)
                })
            );
        }
        // A signal's own brackets are inside the verdict's.
        let screen = format!("{NOT_RESTORED} (signal: 6 (SIGABRT))\nbefore: x\nafter:  y");
        assert_eq!(
            Verdict::parse(&screen),
            Some(Verdict {
                restored: false,
                exit: Exit::Signal(6)
            })
        );
        // Cut short on the screen: not a verdict yet.
        assert_eq!(Verdict::parse("restored: yes (exit status: 1"), None);
        assert_eq!(Verdict::parse("no shell here"), None);
    }

    #[test]
    fn the_last_verdict_is_the_one_read() {
        let screen = "restored: yes (exit status: 0)\nagain\nrestored: yes (exit code: 130)";
        assert_eq!(
            Verdict::parse(screen).map(|v| v.exit),
            Some(Exit::Code(130))
        );
    }
}
