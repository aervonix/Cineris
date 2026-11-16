//! The terminal closing under the program: [`Session::close_console`].

use std::time::Instant;

use super::Session;

impl Session {
    /// Closes the terminal under the program while it runs, as a terminal
    /// window that is closed does, and returns when it was closed. The
    /// program's own end is not waited for: [`wait_exit`](Self::wait_exit)
    /// does that.
    ///
    /// - **On Windows** the pseudo-console is closed (`ClosePseudoConsole`):
    ///   the console host sends every process on it `CTRL_CLOSE_EVENT` and
    ///   ends it once its handler has returned, or after the host's own
    ///   time. That is what a closed Windows Terminal tab does. The screen
    ///   is gone with the console: the output ends, nothing the program
    ///   says after the close is seen, and nothing can be typed. A program
    ///   that is a shell of the test's ends with the event too, when its
    ///   handler returns, so how its child ended is not on the screen: a
    ///   test checks what the child leaves outside it (a file, a request
    ///   dropped, a process gone).
    /// - **On Unix** the terminal hangs up: SIGHUP, then SIGCONT, to the
    ///   program's process group, as the kernel sends them when the last
    ///   descriptor of the terminal's master closes. Closing the master
    ///   itself is not possible while the reader thread has it open in a
    ///   read. Unlike a real hang-up, the terminal stays open: the output
    ///   goes on to be read, and a write to it succeeds. A shell that
    ///   should live through it to say how its child ended passes SIGHUP
    ///   over ([`ModeShell::surviving_hangup`](crate::role::ModeShell::surviving_hangup)).
    ///
    /// # Panics
    ///
    /// When the system refuses.
    pub fn close_console(&mut self) -> Instant {
        let at = Instant::now();
        self.pty
            .close_console()
            .expect("close the pseudo-terminal under the program");
        at
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Duration;

    use crate::exit::Exit;

    use super::*;

    /// A shell that says when it is up, and ends with 7 when it is hung up
    /// on. It waits on a child of its own, so that the signal reaches it
    /// while it waits (`wait` is cut short by a trapped signal), and the
    /// hang-up reaches the child too, which ends with it.
    fn hang_up_shell() -> Session {
        let script = "trap 'echo hung up; exit 7' HUP; echo ready; sleep 600 & wait";
        Session::start("sh", &["-c".to_owned(), script.to_owned()], 40, 5)
    }

    #[test]
    fn sys_closing_the_console_hangs_up_on_the_program_which_ends_as_it_asks() {
        let limit = Duration::from_secs(30);
        let mut session = hang_up_shell();
        session.wait_for(limit, "the shell to be up", |t| t.shows("ready"));
        session.close_console();
        let status = session.wait_exit(limit);
        assert_eq!(Exit::of(status), Exit::Code(7), "{status}");
        // What it said as it ended arrived on the terminal.
        session.wait_for(limit, "what the shell said when hung up on", |t| {
            t.shows("hung up")
        });
    }

    #[test]
    fn sys_a_program_that_does_not_trap_the_hang_up_ends_by_the_signal() {
        let limit = Duration::from_secs(30);
        let mut session = Session::start(
            "sh",
            &["-c".to_owned(), "echo ready; sleep 600".to_owned()],
            40,
            5,
        );
        session.wait_for(limit, "the shell to be up", |t| t.shows("ready"));
        session.close_console();
        let status = session.wait_exit(limit);
        assert_eq!(Exit::of(status), Exit::Signal(1), "{status}");
    }

    #[test]
    fn sys_closing_the_console_of_a_program_that_has_ended_is_not_an_error() {
        let limit = Duration::from_secs(30);
        let mut session = Session::start("sh", &["-c".to_owned(), "echo ready".to_owned()], 40, 5);
        session.wait_exit(limit);
        session.close_console();
    }
}
