//! The pseudo-terminal driver: a program run in a terminal of its own,
//! its output recorded with the time each piece arrived and parsed by a
//! [`VTerm`], keys typed into it, and waits on what its screen shows.
//!
//! The operating system's half (opening the pseudo-terminal, ConPTY on
//! Windows) is `cineris_term::pty`, where the crate's `unsafe` code is.
//!
//! # Waiting
//!
//! A test never sleeps. [`Session::wait_for`] reads the output as it
//! arrives and returns at the first piece after which the screen shows what
//! the test is waiting for, with the time that piece arrived. Its limit is a
//! bound that turns a mistake into a failed test, and the failure shows the
//! screen.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support: it starts a program and reads and writes its terminal from threads"
)]

use std::io::{Read, Write};
use std::process::ExitStatus;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use cineris_term::pty::{Pty, is_end};

use crate::vterm::{VTerm, answer};

mod close;

/// One piece of output, as one read returned it.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// When the read returned.
    pub at: Instant,
    /// What it returned.
    pub bytes: Vec<u8>,
}

/// A program running in a pseudo-terminal, and what it has shown.
pub struct Session {
    pty: Pty,
    writer: Box<dyn Write + Send>,
    chunks: Receiver<Option<Chunk>>,
    reader: Option<JoinHandle<()>>,
    term: VTerm,
    recording: Vec<Chunk>,
    started: Instant,
    ended: bool,
    answering: bool,
    /// Whether the terminal answering knows the win32 input mode, as
    /// Windows Terminal does.
    win32_input: bool,
    exit: Option<ExitStatus>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("pid", &self.pty.pid())
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts `program` with `arguments` in a terminal of `cols` by `rows`.
    ///
    /// # Panics
    ///
    /// When it cannot be started.
    #[must_use]
    pub fn start(program: &str, arguments: &[String], cols: u16, rows: u16) -> Self {
        let started = Instant::now();
        let pty = Pty::spawn(program, arguments, &[], cols, rows)
            .unwrap_or_else(|error| panic!("start {program} in a pseudo-terminal: {error}"));
        let mut reader = pty.reader().expect("the pseudo-terminal's output");
        let writer = pty.writer().expect("the pseudo-terminal's input");
        let (send, chunks) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buffer = vec![0u8; 65536];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let chunk = Chunk {
                            at: Instant::now(),
                            bytes: buffer[..count].to_vec(),
                        };
                        if send.send(Some(chunk)).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    // The other end has gone (`is_end`), or the read
                    // failed: either way the output is over.
                    Err(error) => {
                        let _ = is_end(&error);
                        break;
                    }
                }
            }
            let _ = send.send(None);
        });
        Self {
            pty,
            writer,
            chunks,
            reader: Some(reader),
            term: VTerm::new(cols, rows),
            recording: Vec::new(),
            started,
            ended: false,
            answering: false,
            win32_input: true,
            exit: None,
        }
    }

    /// Answers the program's queries as a modern terminal would
    /// ([`answer`]), from now on.
    #[must_use]
    pub fn answering(mut self) -> Self {
        self.answering = true;
        self
    }

    /// Answers as a terminal that does not know Windows Terminal's win32
    /// input mode (9001), as xterm and most Unix terminals (since step 25):
    /// a program asks such a terminal for the mouse by its sequences.
    #[must_use]
    pub fn without_win32_input(mut self) -> Self {
        self.win32_input = false;
        self
    }

    /// When the program was started.
    #[must_use]
    pub fn started(&self) -> Instant {
        self.started
    }

    /// The program's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pty.pid()
    }

    /// The screen as parsed so far.
    #[must_use]
    pub fn screen(&self) -> &VTerm {
        &self.term
    }

    /// Everything the program wrote, as it arrived.
    #[must_use]
    pub fn recording(&self) -> &[Chunk] {
        &self.recording
    }

    /// Types `bytes`, and returns when they were handed to the terminal.
    ///
    /// # Panics
    ///
    /// When the terminal cannot be written to.
    pub fn send(&mut self, bytes: &[u8]) -> Instant {
        let at = Instant::now();
        self.writer
            .write_all(bytes)
            .and_then(|()| self.writer.flush())
            .expect("type into the pseudo-terminal");
        at
    }

    /// Sends the key named `name` as the terminal would (since step 25):
    /// on Windows, the win32 input records, a press and a release, that
    /// Windows Terminal sends a pseudo-console, which asks for that mode and
    /// misreads a plain sequence after an escape (`docs/conpty.md`);
    /// elsewhere, the xterm sequence. Names: `f1` to `f12`, `up`, `down`,
    /// `left`, `right`, `pageup`, `pagedown` and `escape`, each with any of
    /// `shift+`, `ctrl+` and `alt+` before it ([`key_bytes`]). A pty test
    /// sends every key through here: `cargo xtask rules` fails on a raw
    /// escape sequence for a key.
    ///
    /// # Panics
    ///
    /// For a name it does not know, or when the terminal cannot be written.
    pub fn send_key(&mut self, name: &str) -> Instant {
        let bytes =
            key_bytes(name, cfg!(windows)).unwrap_or_else(|| panic!("no key named `{name}`"));
        self.send(&bytes)
    }

    /// Resizes the terminal, as a window being resized.
    ///
    /// # Panics
    ///
    /// When the system refuses.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.pty
            .resize(cols, rows)
            .expect("resize the pseudo-terminal");
        self.term.resize(cols, rows);
    }

    fn take(&mut self, chunk: Chunk) {
        self.term.feed(&chunk.bytes);
        self.recording.push(chunk);
        if self.answering {
            for query in self.term.take_queries() {
                let reply = match query {
                    crate::vterm::Query::Mode(9001) if !self.win32_input => {
                        b"\x1b[?9001;0$y".to_vec()
                    }
                    query => answer(query, &self.term),
                };
                let _ = self.writer.write_all(&reply);
            }
        }
    }

    /// Reads output for at most `limit`, until `condition` holds of the
    /// screen, and returns when the piece arrived after which it held. A
    /// condition that already holds returns the time of the last piece read.
    ///
    /// # Panics
    ///
    /// When the time runs out or the output ends first, with the screen.
    pub fn wait_for(
        &mut self,
        limit: Duration,
        what: &str,
        mut condition: impl FnMut(&VTerm) -> bool,
    ) -> Instant {
        if condition(&self.term) {
            return self.recording.last().map_or(self.started, |chunk| chunk.at);
        }
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.chunks.recv_timeout(left) {
                Ok(Some(chunk)) => {
                    let at = chunk.at;
                    self.take(chunk);
                    if condition(&self.term) {
                        return at;
                    }
                }
                Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    panic!(
                        "the output ended before {what}; the screen:\n{}",
                        self.term.dump()
                    );
                }
                Err(RecvTimeoutError::Timeout) => panic!(
                    "waited {limit:?} for {what}, and it did not happen; the screen:\n{}",
                    self.term.dump()
                ),
            }
        }
    }

    /// Answers a dialog of the interactive application with `key`, as a
    /// person does, and returns when the dialog had taken it.
    ///
    /// It waits for the dialog's row of answers (`answers`: text of that
    /// row, which the dialog draws only once it takes keys, at the end of
    /// its grace), types `key` alone, and waits until the row has left the
    /// screen and `answered` holds of it: something only the answer can
    /// bring (the session's header after trust, the tool's result after an
    /// approval). The row alone is not enough: a frame can reach the
    /// terminal in pieces, and one piece can lack the row while the dialog
    /// is still up.
    ///
    /// The application reads keys that arrive in one read as typing, and a
    /// dialog takes none of them as an answer (`cineris_tui`'s `App::burst`,
    /// `docs/architecture/09-tui.md`). So a test that types its next keys
    /// before the dialog has gone can have them read together with the
    /// answer, which then goes to the input box and the dialog stays. On a
    /// fast build that is most runs: a wait that passed too soon (a header
    /// "seen" in a path on the screen) made the test type `go` and Enter a
    /// few microseconds after its `y` (the eighth tooling step).
    ///
    /// # Panics
    ///
    /// When the answers are not shown within `limit`, or the dialog does not
    /// go within `limit` of the key, with the screen.
    pub fn answer_dialog(
        &mut self,
        limit: Duration,
        answers: &str,
        key: &[u8],
        mut answered: impl FnMut(&VTerm) -> bool,
    ) -> Instant {
        self.wait_for(limit, &format!("the dialog's answers `{answers}`"), |t| {
            t.shows(answers)
        });
        self.send(key);
        self.wait_for(
            limit,
            &format!(
                "the dialog to take the key {:?}, go, and be answered (its answers `{answers}` \
                 still shown, or what the answer brings not yet)",
                String::from_utf8_lossy(key)
            ),
            |t| !t.shows(answers) && answered(t),
        )
    }

    /// Reads the next piece of output, waiting at most `limit` for it, and
    /// returns when it arrived; `None` when the time ran out or the output
    /// has ended ([`ended`](Self::ended)).
    pub fn step(&mut self, limit: Duration) -> Option<Instant> {
        if self.ended {
            return None;
        }
        match self.chunks.recv_timeout(limit) {
            Ok(Some(chunk)) => {
                let at = chunk.at;
                self.take(chunk);
                Some(at)
            }
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                self.ended = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => None,
        }
    }

    /// Whether the output has ended: everything in the terminal closed it.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Reads whatever output has arrived, without waiting.
    pub fn pump(&mut self) {
        while let Ok(message) = self.chunks.try_recv() {
            match message {
                Some(chunk) => self.take(chunk),
                None => self.ended = true,
            }
        }
    }

    /// Waits at most `limit` for the program to end, reading its output
    /// meanwhile, and returns how it ended.
    ///
    /// # Panics
    ///
    /// When it does not end in time.
    pub fn wait_exit(&mut self, limit: Duration) -> ExitStatus {
        let deadline = Instant::now() + limit;
        loop {
            self.pump();
            if let Some(status) = self.pty.try_wait().expect("ask how the program ended") {
                self.exit = Some(status);
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "waited {limit:?} for the program to end; the screen:\n{}",
                self.term.dump()
            );
            let step = deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5));
            if let Ok(Some(chunk)) = self.chunks.recv_timeout(step) {
                self.take(chunk);
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.exit.is_none() && !matches!(self.pty.try_wait(), Ok(Some(_))) {
            let _ = self.pty.kill();
        }
        // The reader ends when the terminal closes, which on Windows is when
        // the pseudo-console is dropped: it is not waited for here.
        drop(self.reader.take());
    }
}

/// The bytes of the key named `name` as a terminal sends it: with `win32`,
/// a press and a release as Windows Terminal's win32 input records
/// (`CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`: the virtual key, the scan code, the
/// character (Escape's own, none for the others), down or up, the control
/// keys' state, one repeat), the arrows and the page keys marked as the
/// enhanced keys they are; otherwise the xterm sequence. `None` for a name
/// it does not know.
///
/// A name may start with modifiers, each followed by `+`, in any order:
/// `shift`, `ctrl` and `alt` (since the twelfth tooling step: `ctrl+up`,
/// `shift+f5`). In an xterm sequence they are its modifier parameter
/// (`CSI 1 ; 5 A`), and Escape takes only `alt` (`ESC ESC`); in the win32
/// records they are the control keys' state (the left Ctrl and Alt).
#[must_use]
pub fn key_bytes(name: &str, win32: bool) -> Option<Vec<u8>> {
    // The name, its virtual key, its scan code, its character, whether it
    // is an enhanced key, and its xterm sequence.
    const KEYS: [(&str, u16, u16, u16, bool, &str); 19] = [
        ("f1", 0x70, 0x3b, 0, false, "\x1bOP"),
        ("f2", 0x71, 0x3c, 0, false, "\x1bOQ"),
        ("f3", 0x72, 0x3d, 0, false, "\x1bOR"),
        ("f4", 0x73, 0x3e, 0, false, "\x1bOS"),
        ("f5", 0x74, 0x3f, 0, false, "\x1b[15~"),
        ("f6", 0x75, 0x40, 0, false, "\x1b[17~"),
        ("f7", 0x76, 0x41, 0, false, "\x1b[18~"),
        ("f8", 0x77, 0x42, 0, false, "\x1b[19~"),
        ("f9", 0x78, 0x43, 0, false, "\x1b[20~"),
        ("f10", 0x79, 0x44, 0, false, "\x1b[21~"),
        ("f11", 0x7a, 0x57, 0, false, "\x1b[23~"),
        ("f12", 0x7b, 0x58, 0, false, "\x1b[24~"),
        ("left", 0x25, 0x4b, 0, true, "\x1b[D"),
        ("up", 0x26, 0x48, 0, true, "\x1b[A"),
        ("right", 0x27, 0x4d, 0, true, "\x1b[C"),
        ("down", 0x28, 0x50, 0, true, "\x1b[B"),
        ("pageup", 0x21, 0x49, 0, true, "\x1b[5~"),
        ("pagedown", 0x22, 0x51, 0, true, "\x1b[6~"),
        ("escape", 0x1b, 0x01, 27, false, "\x1b"),
    ];
    /// The control keys' states: an enhanced key (`ENHANCED_KEY`), Shift,
    /// the left Ctrl and the left Alt.
    const ENHANCED: u32 = 0x100;
    const SHIFT: u32 = 0x10;
    const LEFT_CTRL: u32 = 0x08;
    const LEFT_ALT: u32 = 0x02;

    let mut key = name;
    let (mut shift, mut ctrl, mut alt) = (false, false, false);
    while let Some((modifier, rest)) = key.split_once('+') {
        let held = match modifier {
            "shift" => &mut shift,
            "ctrl" => &mut ctrl,
            "alt" => &mut alt,
            _ => return None,
        };
        if *held {
            return None;
        }
        *held = true;
        key = rest;
    }
    let &(_, vk, scan, character, enhanced, sequence) = KEYS.iter().find(|row| row.0 == key)?;
    if !win32 {
        // xterm's modifier parameter: 1, and 1 more for Shift, 2 for Alt,
        // 4 for Ctrl.
        let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
        if modifier == 1 {
            return Some(sequence.as_bytes().to_vec());
        }
        let body = sequence.strip_prefix('\x1b')?;
        let modified = if body.is_empty() {
            // Escape: Alt alone, as an Escape before it.
            (modifier == 3).then(|| "\x1b\x1b".to_owned())?
        } else if let Some(number) = body.strip_prefix('[').and_then(|b| b.strip_suffix('~')) {
            format!("\x1b[{number};{modifier}~")
        } else {
            // `SS3 X` or `CSI X`: the final letter after `CSI 1 ; m`.
            let last = body.chars().next_back()?;
            format!("\x1b[1;{modifier}{last}")
        };
        return Some(modified.into_bytes());
    }
    let state = [
        (enhanced, ENHANCED),
        (shift, SHIFT),
        (ctrl, LEFT_CTRL),
        (alt, LEFT_ALT),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(0, |state, (_, bit)| state | bit);
    let record = |down: u8| format!("\x1b[{vk};{scan};{character};{down};{state};1_");
    Some(format!("{}{}", record(1), record(0)).into_bytes())
}

#[cfg(test)]
mod tests {
    /// A named key's bytes: the win32 records' fields and the xterm
    /// sequence, an arrow marked enhanced; a name it does not know is none.
    #[test]
    fn a_named_key_is_sent_as_its_win32_records_or_its_sequence() {
        assert_eq!(
            super::key_bytes("f5", true).unwrap(),
            b"\x1b[116;63;0;1;0;1_\x1b[116;63;0;0;0;1_"
        );
        assert_eq!(super::key_bytes("f5", false).unwrap(), b"\x1b[15~");
        assert_eq!(
            super::key_bytes("up", true).unwrap(),
            b"\x1b[38;72;0;1;256;1_\x1b[38;72;0;0;256;1_"
        );
        assert_eq!(super::key_bytes("pageup", false).unwrap(), b"\x1b[5~");
        assert_eq!(super::key_bytes("f1", false).unwrap(), b"\x1bOP");
        assert!(super::key_bytes("f13", true).is_none());
    }

    /// Escape and the modifiers (since the twelfth tooling step): Escape
    /// carries its character in a record; a modifier is xterm's parameter
    /// or a record's control keys' state; a name it cannot send is none.
    #[test]
    fn escape_and_the_modifiers_are_sent_as_a_terminal_sends_them() {
        let key = |name, win32| super::key_bytes(name, win32);
        assert_eq!(key("escape", false).unwrap(), b"\x1b");
        assert_eq!(
            key("escape", true).unwrap(),
            b"\x1b[27;1;27;1;0;1_\x1b[27;1;27;0;0;1_"
        );
        assert_eq!(key("alt+escape", false).unwrap(), b"\x1b\x1b");
        assert!(key("ctrl+escape", false).is_none());
        assert_eq!(key("ctrl+up", false).unwrap(), b"\x1b[1;5A");
        assert_eq!(key("shift+f1", false).unwrap(), b"\x1b[1;2P");
        assert_eq!(key("ctrl+shift+f5", false).unwrap(), b"\x1b[15;6~");
        assert_eq!(key("alt+pagedown", false).unwrap(), b"\x1b[6;3~");
        assert_eq!(
            key("ctrl+up", true).unwrap(),
            b"\x1b[38;72;0;1;264;1_\x1b[38;72;0;0;264;1_"
        );
        assert_eq!(
            key("shift+alt+f5", true).unwrap(),
            b"\x1b[116;63;0;1;18;1_\x1b[116;63;0;0;18;1_"
        );
        assert!(key("ctrl+ctrl+up", false).is_none());
        assert!(key("meta+up", false).is_none());
        assert!(key("ctrl+", false).is_none());
    }

    use super::*;

    #[test]
    fn answers_go_back_to_the_program() {
        // Exercised end to end by `cineris-term`'s system tests; here only the
        // pieces that need no process.
        let mut term = VTerm::new(10, 2);
        term.feed(b"\x1b[c");
        let queries = term.take_queries();
        assert_eq!(answer(queries[0], &term), b"\x1b[?62;22c");
    }
}
