//! Pseudo-terminals, for tests: a program started in a terminal of its own
//! whose other end the test reads and writes. On Windows a pseudo-console
//! (ConPTY).
//!
//! Behind the `pty` feature, which only the test kit turns on: the operating
//! system's half is here, beside the rest of this crate's `unsafe` code, and
//! the driver that records and types is `cortex_testkit::pty`. Also here,
//! for the programs such tests start: a description of the terminal's
//! modes, to check they were put back, and the means to end a program the
//! way a signal or a console event would.

use std::io::{self, Read, Write};
use std::process::ExitStatus;

use crate::sys;

/// A program running in a pseudo-terminal of its own.
#[derive(Debug)]
pub struct Pty(sys::pty::Pty);

impl Pty {
    /// Starts `program` with `arguments` in a pseudo-terminal of `cols` by
    /// `rows`. On Unix it gets `TERM=xterm-256color` and `env` added to this
    /// process's environment; on Windows `env` must be empty.
    ///
    /// # Errors
    ///
    /// When the pseudo-terminal cannot be made or the program not started.
    pub fn spawn(
        program: &str,
        arguments: &[String],
        env: &[(String, String)],
        cols: u16,
        rows: u16,
    ) -> io::Result<Self> {
        sys::pty::Pty::spawn(program, arguments, env, cols, rows).map(Self)
    }

    /// What the program writes to its terminal. A read that ends with an
    /// error for which [`is_end`] holds means the program and everything it
    /// started have closed the terminal.
    ///
    /// # Errors
    ///
    /// When the handle cannot be duplicated.
    pub fn reader(&self) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.0.reader()?))
    }

    /// What the program reads from its terminal: keys.
    ///
    /// # Errors
    ///
    /// When the handle cannot be duplicated.
    pub fn writer(&self) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(self.0.writer()?))
    }

    /// Changes the terminal's size, as a window being resized does.
    ///
    /// # Errors
    ///
    /// When the system refuses.
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.0.resize(cols, rows)
    }

    /// The program's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.0.pid()
    }

    /// How the program ended, if it has.
    ///
    /// # Errors
    ///
    /// When the system cannot say.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }

    /// Ends the program at once.
    ///
    /// # Errors
    ///
    /// When the system refuses.
    pub fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }
}

/// Whether a read error from [`Pty::reader`] means the other end has gone,
/// which is how the end of the output shows on some systems.
#[must_use]
pub fn is_end(error: &io::Error) -> bool {
    sys::pty::is_end(error)
}

/// A description of the terminal modes of this process's standard input
/// and output, the same text when the modes are the same: on Unix every
/// field of its `termios`, on Windows the console's two modes and two code
/// pages.
///
/// # Errors
///
/// When they are not a terminal.
pub fn modes() -> io::Result<String> {
    sys::modes()
}

/// Turns virtual-terminal processing of this process's console output off
/// (or on), so that a test can start a program the way the old console host
/// would. Nothing happens on Unix.
///
/// # Errors
///
/// When the console refuses.
pub fn set_vt_output(on: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        sys::set_vt_output(on)
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Ok(())
    }
}

/// Ends this process the way an outside request to stop would: on Unix
/// SIGTERM, on Windows a Ctrl+Break event, which goes to every process on
/// the console.
///
/// # Errors
///
/// When the system refuses.
pub fn terminate_self() -> io::Result<()> {
    #[cfg(unix)]
    {
        sys::pty::signal(std::process::id(), libc::SIGTERM)
    }
    #[cfg(windows)]
    {
        sys::pty::signal(std::process::id(), 0)
    }
}

/// Makes this process ignore console control events (Windows), so that a
/// parent that shares its console with a child the test ends that way stays.
/// Nothing happens on Unix, where a signal goes to one process.
pub fn ignore_control_events() {
    #[cfg(windows)]
    sys::pty::ignore_control_events();
}

/// An interrupt (SIGINT) made pending on the calling thread by
/// [`interrupt_pending_here`]: blocked on the thread, and sent to it.
/// Dropping it unblocks the signal, so one still pending then is delivered
/// as it would have been. Unix only.
#[cfg(unix)]
#[must_use = "the interrupt stays blocked until this is dropped"]
pub struct PendingInterrupt {
    previous: libc::sigset_t,
}

#[cfg(unix)]
impl std::fmt::Debug for PendingInterrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingInterrupt").finish_non_exhaustive()
    }
}

#[cfg(unix)]
impl PendingInterrupt {
    /// Whether the interrupt is still pending: nothing has taken it.
    #[must_use]
    pub fn still_pending(&self) -> bool {
        sys::pty::interrupt_is_pending()
    }
}

#[cfg(unix)]
impl Drop for PendingInterrupt {
    fn drop(&mut self) {
        sys::pty::restore_mask(&self.previous);
    }
}

/// Makes an interrupt pending on the calling thread, deterministically: the
/// state a `Ctrl+C` is in when the kernel has not yet handed it to a thread
/// (step 21's race, in which another thread took it after an interrupt
/// guard was gone). A test of what takes a pending interrupt (the guard of
/// [`crate::ignore_interrupts`], as it ends) makes one with this, and drops
/// the result after: a signal still pending then is delivered. Unix only.
///
/// # Errors
///
/// When the system refuses to block or send the signal.
#[cfg(unix)]
pub fn interrupt_pending_here() -> io::Result<PendingInterrupt> {
    Ok(PendingInterrupt {
        previous: sys::pty::interrupt_pending_here()?,
    })
}

/// Raises an interrupt (SIGINT) on the calling thread: its handler has run
/// when this returns. Unix only.
///
/// # Errors
///
/// When the system refuses.
#[cfg(unix)]
pub fn raise_interrupt() -> io::Result<()> {
    sys::pty::raise_interrupt()
}

/// Lets process `pid` go on after it stopped itself (SIGCONT), as a shell's
/// `fg` does. Unix only.
///
/// # Errors
///
/// When the system refuses.
#[cfg(unix)]
pub fn continue_process(pid: u32) -> io::Result<()> {
    sys::pty::signal(pid, libc::SIGCONT)
}
