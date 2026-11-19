//! The terminal itself: raw mode on the way in, put back on every way out,
//! and events on the way through.
//!
//! [`Terminal::enter`] turns raw mode on (on Windows: virtual-terminal
//! processing and input, and UTF-8 code pages), asks for bracketed paste and
//! focus events, and sends the capability queries. It never waits for a
//! reply. From then on [`Terminal::next`] gives the events, and the terminal
//! is the [`Sink`] the presenter writes frames to.
//!
//! # Every way out
//!
//! - **A normal exit:** [`Terminal::leave`], or dropping the terminal, turns
//!   off what was turned on and puts the modes back.
//! - **A panic on the thread that entered the terminal:** a panic hook,
//!   installed once, does the same before the panic's message is printed,
//!   then calls the hook that was there before. A panic on any other thread
//!   is left to the program: the hook runs before anything can catch the
//!   panic, and the program catches some (a tool's, which becomes that
//!   call's error), so putting the terminal back then would leave a program
//!   that goes on drawing on a terminal it no longer holds. A program whose
//!   other thread dies of a panic nobody catches learns of it from that
//!   thread, and leaves the terminal itself (step 23; this closed
//!   the open item on a caught panic, `docs/steps/23/answered.md`).
//! - **A signal** (SIGTERM, SIGHUP, SIGINT, SIGQUIT, and since step 23
//!   SIGABRT, which an abort raises) **or a console event**
//!   (Ctrl+Break, the console closing, logging off, shutting down): a handler
//!   does the same, and then lets the signal or the event end the process as
//!   it would have. The handlers replace any installed before them. From then
//!   on nothing more is written: a frame would turn modes on in a terminal
//!   that is no longer the program's, so the next write fails.
//! - **Suspending** (SIGTSTP, or [`Terminal::suspend`] on Ctrl+Z): the
//!   terminal is put back while the program is stopped; when it continues,
//!   raw mode and everything that was turned on come back, with an
//!   [`Event::Resumed`].
//!
//! # The device
//!
//! What the operating system does is behind the small [`Device`] trait, so
//! that it can be replaced, and so that this layer's own logic (the escape
//! key's wait, the replies) is tested without a terminal.

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, Once, PoisonError};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use crate::caps::{Capabilities, Overrides, Platform};
use crate::inline::Sink;
use crate::input::{Decoder, Event, KeyCode, Mods, Reply};
use crate::sys::{self, Ready};

/// How long a lone ESC waits for the rest of a sequence before it is taken
/// as the Escape key. Long enough for a sequence split by a slow link, short
/// enough that Escape does not feel late.
pub const ESCAPE_WAIT: Duration = Duration::from_millis(50);

/// How long a lone ESC waits for the rest of a sequence while the replies
/// to the capability queries are due ([`REPLIES_DUE`]), in place of
/// [`ESCAPE_WAIT`]. Through a pseudo-console (ConPTY) a reply comes about a
/// key at a time, and on a loaded machine its ESC can come more than
/// [`ESCAPE_WAIT`] before its `[`, which made it the Escape key and typed
/// the rest (`[?0u`) into the input box. An Escape pressed while the
/// replies are due comes up to this much late, and one followed by a reply
/// within it is taken with the reply (as Alt is, an ESC before a sequence);
/// every dialog at the start waits longer than this before it takes a key.
pub const REPLY_ESCAPE_WAIT: Duration = Duration::from_millis(500);

/// How long after the capability queries their replies are taken to be on
/// their way, when the last of them (the device attributes, which every
/// terminal answers) has not come: after it, a lone ESC waits
/// [`ESCAPE_WAIT`] again, so a terminal that never answers is held to the
/// longer wait for no more than this.
pub const REPLIES_DUE: Duration = Duration::from_secs(3);

/// The keys of one read that are taken for a paste, where pastes are not
/// known to come bracketed (since step 25): at least this many with a line
/// break among them that is not the last. Typing puts no more than a few
/// keys in one read, and never a line break in the middle of them.
pub(crate) const BURST: usize = 8;

/// Or more than this many with no line break at all: a long line pasted,
/// whose rest may come in the next read. A read whose only line break is
/// its last key is keys however long it is: a line typed through a stall,
/// or sent by a program (`tmux send-keys`), and ended by `Enter` (the
/// review of step 25's second pull request).
pub(crate) const LONG_BURST: usize = 64;

/// After a burst taken for a paste, keys that come within this much time
/// spent waiting for input are its rest: a paste's last read may hold a few
/// keys and its line break. Only the time the reader waited counts, not
/// the time between one read's handling and the next's: a frame drawn
/// between two reads of one paste does not cut its tail short (the review
/// of step 25's second pull request).
pub(crate) const PASTE_TAIL: Duration = Duration::from_millis(50);

/// The end of a bracketed paste.
const PASTE_END: &[u8] = b"\x1b[201~";

/// What one event of a read is to a paste of the read (since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    /// A key that types this character: text with at most `Shift`, `Tab`,
    /// or `Enter`, a line break (`\n`).
    Types(char),
    /// `Ctrl+J`, which a line feed (byte 0x0a) reads as: a line break, or
    /// with the `Enter` just before it one line break (`\r\n`).
    LineFeed,
    /// Not a key: a focus report, a reply, the mouse. Passed on beside the
    /// paste, it does not make the keys keys (the review of step 25's second
    /// pull request).
    Beside,
    /// A key no paste holds (one with `Ctrl` or `Alt`, a sequence's, a
    /// repeat), or a paste of its own: the read is keys.
    Keys,
}

fn part(event: &Event) -> Part {
    match event {
        Event::Key(key) => {
            let plain = !key.mods.contains(Mods::CTRL)
                && !key.mods.contains(Mods::ALT)
                && !key.mods.contains(Mods::SUPER);
            match key.code {
                KeyCode::Char(c) if plain => Part::Types(c),
                KeyCode::Enter if plain => Part::Types('\n'),
                KeyCode::Tab if key.mods.is_empty() => Part::Types('\t'),
                KeyCode::Char('j') if key.mods == Mods::CTRL => Part::LineFeed,
                _ => Part::Keys,
            }
        }
        Event::Focus(_) | Event::Reply(_) | Event::Mouse(_) => Part::Beside,
        Event::Repeat(_) | Event::Paste(_) | Event::Resize { .. } | Event::Resumed => Part::Keys,
    }
}

/// What the keys of one read type, as a paste of them would hold it.
#[derive(Debug, Default)]
struct Typed {
    /// The text, each line break `\n`.
    text: String,
    /// How many characters it holds: none when the read had no key.
    keys: usize,
    /// Whether its last key is an `Enter` no line feed has followed yet:
    /// a line feed that opens the next read of the paste is that `Enter`'s.
    enter_last: bool,
}

impl Typed {
    /// Whether a line break has more text after it.
    fn break_inside(&self) -> bool {
        self.text
            .char_indices()
            .any(|(at, c)| c == '\n' && at + 1 < self.text.len())
    }

    /// Whether it holds a line break.
    fn breaks(&self) -> bool {
        self.text.contains('\n')
    }
}

/// What the keys of `found` type, when every key of it types (what is
/// beside them, a focus report or a reply, does not count): what a paste of
/// it would hold. `Enter` then `Ctrl+J` is one line break. `None` when a
/// key no paste holds is among them.
fn typed(found: &[Event]) -> Option<Typed> {
    let mut typed = Typed::default();
    let mut after_enter = false;
    for event in found {
        match part(event) {
            Part::Types(c) => {
                typed.text.push(c);
                typed.keys += 1;
                after_enter = c == '\n';
            }
            Part::LineFeed if after_enter => after_enter = false,
            Part::LineFeed => {
                typed.text.push('\n');
                typed.keys += 1;
            }
            Part::Beside => {}
            Part::Keys => return None,
        }
    }
    typed.enter_last = after_enter;
    Some(typed)
}

/// Takes out of `found` its first key when that is a line feed (read as
/// `Ctrl+J`): the second half of a `\r\n` whose `Enter` ended the read
/// before.
fn drop_joined_line_feed(found: &mut Vec<Event>) {
    let first_key = found.iter().position(|event| part(event) != Part::Beside);
    if let Some(at) = first_key
        && part(&found[at]) == Part::LineFeed
    {
        found.remove(at);
    }
}

/// `found` with its keys made one paste of `text`, where its first key
/// was; what is beside them is kept, in its order.
fn as_paste(found: Vec<Event>, text: String) -> Vec<Event> {
    let mut text = Some(text);
    let mut events = Vec::new();
    for event in found {
        if part(&event) == Part::Beside {
            events.push(event);
        } else if let Some(text) = text.take() {
            events.push(Event::Paste(text));
        }
    }
    events
}

/// The start sequence for `caps`: [`START`], without bracketed paste where
/// the override says not to ask for it (since step 25).
fn start_sequence(caps: &Capabilities) -> Vec<u8> {
    if caps.overrides().bracketed_paste == Some(false) {
        START_UNBRACKETED.to_vec()
    } else {
        START.to_vec()
    }
}

/// [`START`] without bracketed paste.
const START_UNBRACKETED: &[u8] = b"\x1b[?1004h\x1b[?7l";

/// Turned on at the start, and again after a suspension: bracketed paste and
/// focus events, both harmless where they are not known; and automatic wrap
/// turned off, so that a character the terminal draws wider than it is
/// counted can never push a row onto the next and throw the presenter's
/// count of rows out.
const START: &[u8] = b"\x1b[?2004h\x1b[?1004h\x1b[?7l";
/// Turned off at the end, and automatic wrap turned back on. The enhanced
/// keyboard protocol and the win32 input mode, if they were turned on, are
/// turned off first.
const END: &[u8] = b"\x1b[0m\x1b[?1004l\x1b[?2004l\x1b[?7h\x1b[?25h";
/// How long leaving waits for the terminal's last reply to the capability
/// queries, when it has not come yet, so that the reply does not arrive at
/// the shell's prompt as typed text.
pub const REPLY_WAIT: Duration = Duration::from_millis(100);

/// How long a paste whose end has not come, or a sequence too long to keep,
/// may go with no input before it is given up on: the paste ends with what
/// it holds, given as a paste, and keys are keys again. A terminal sends a
/// paste in one burst; this is long enough for a slow connection's pauses,
/// and short enough that a lost end marker does not swallow `Ctrl+C` for
/// long. A paste that pauses longer than this is ended there, and what
/// follows of it arrives as keys.
pub(crate) const OPEN_WAIT: Duration = Duration::from_secs(2);
/// Turns the enhanced keyboard protocol on, with the flag that tells apart
/// keys that are otherwise the same bytes (1), and the one that reports a
/// key's repeats and releases (2): a repeat is [`Event::Repeat`], which a
/// dialog never takes as its answer, and a release is dropped (since the
/// review of step 23). A key that types text still comes as its text,
/// repeats included.
const KEYBOARD_ON: &[u8] = b"\x1b[>3u";
const KEYBOARD_OFF: &[u8] = b"\x1b[<u";
/// Turns Windows Terminal's win32 input mode on, and off.
const WIN32_ON: &[u8] = b"\x1b[?9001h";
const WIN32_OFF: &[u8] = b"\x1b[?9001l";

/// The alternate screen on, and off: the main screen comes back as it was.
const ALTERNATE_ON: &[u8] = b"\x1b[?1049h";
const ALTERNATE_OFF: &[u8] = b"\x1b[?1049l";
/// Mouse reporting on, presses and releases with the wheel (1000), in the
/// SGR form (1006); and off, the second first (since step 25).
const MOUSE_ON: &[u8] = b"\x1b[?1000h\x1b[?1006h";
const MOUSE_OFF: &[u8] = b"\x1b[?1006l\x1b[?1000l";

/// Wakes a terminal that is waiting in [`Terminal::next`], from any thread.
#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    /// A waker that calls `wake`.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(wake))
    }

    /// Wakes the terminal: its [`next`](Terminal::next) returns `None`.
    pub fn wake(&self) {
        (self.0)();
    }
}

impl fmt::Debug for Waker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Waker")
    }
}

/// What the terminal layer needs from the operating system.
pub trait Device {
    /// The window's size: columns, then rows.
    ///
    /// # Errors
    ///
    /// When the system cannot say.
    fn size(&self) -> io::Result<(u16, u16)>;

    /// Writes `bytes` to the terminal, in one call where the system allows.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written to.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Waits for at most `timeout` (`None`: as long as it takes) for input,
    /// which it adds to `into`, or for something else to happen.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read, or has gone.
    fn wait(&mut self, timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready>;

    /// Something that wakes a [`wait`](Self::wait) from another thread.
    fn waker(&self) -> Waker;

    /// Stops the program as Ctrl+Z would, where the system can.
    fn suspend(&mut self);

    /// Puts the terminal's modes back. Called once, last.
    fn leave(&mut self);

    /// Whether the terminal is still the program's: `false` once it has
    /// been put back in an emergency (a panic, a signal), when nothing more
    /// should be read or written.
    fn active(&self) -> bool {
        true
    }

    /// The time now, on a clock that only goes forward. A device for tests
    /// gives a time of its own, so that waits are tested without waiting.
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// The real terminal, through the operating system.
#[derive(Debug)]
pub struct System {
    _private: (),
}

impl Device for System {
    fn size(&self) -> io::Result<(u16, u16)> {
        sys::size()
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        sys::write(bytes)
    }

    fn wait(&mut self, timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
        sys::wait(timeout, into)
    }

    fn waker(&self) -> Waker {
        Waker::new(sys::wake)
    }

    fn suspend(&mut self) {
        sys::suspend();
    }

    fn leave(&mut self) {
        sys::leave();
    }

    fn active(&self) -> bool {
        sys::active()
    }
}

/// Whether standard input and standard output are both a terminal, which
/// the interactive interface needs.
#[must_use]
pub fn is_terminal() -> bool {
    sys::is_terminal()
}

static PANIC_HOOK: Once = Once::new();

/// The thread that entered the terminal last: the one whose panic puts it
/// back.
static ENTERED_ON: Mutex<Option<ThreadId>> = Mutex::new(None);

/// Whether a panic on the calling thread puts the terminal back: whether it
/// is the thread that entered the terminal.
fn restores_on_panic() -> bool {
    *ENTERED_ON.lock().unwrap_or_else(PoisonError::into_inner) == Some(std::thread::current().id())
}

/// While this lives, the keys that interrupt a program at the terminal do
/// nothing to this process: SIGINT and SIGQUIT on Unix (`Ctrl+C`,
/// `Ctrl+\`), and `Ctrl+C` on Windows. For a child that has been handed the
/// terminal, such as the user's own editor: the keys reach the child, and
/// do not end the program waiting for it. Dropping it, a panic's unwinding
/// included, puts back what was there before.
///
/// **Take it before the terminal is left, and drop it after the terminal
/// is entered again,** so that no key can fall in between: leaving keeps
/// the guard's handler, the terminal's own handlers pass the keys over
/// while a guard lives and for a second after, and a signal still pending
/// when it is dropped is taken then. One guard at a time. A child keeps its own default: on Unix
/// the signals are caught by a handler that does nothing, not ignored, and
/// `exec` resets a caught signal (it would keep an ignored one); on Windows
/// the terminal layer's own console handler answers `Ctrl+C` as handled,
/// and a handler is not inherited (the console's "ignore" attribute would
/// be). `Ctrl+Break` still ends the process on Windows.
#[must_use = "interrupts are ignored only while the guard lives"]
pub struct InterruptsIgnored {
    previous: sys::Interrupts,
}

impl fmt::Debug for InterruptsIgnored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InterruptsIgnored").finish_non_exhaustive()
    }
}

/// Ignores the interrupt keys until the guard is dropped. See
/// [`InterruptsIgnored`].
///
/// # Errors
///
/// When the system refuses. Nothing is changed then.
pub fn ignore_interrupts() -> io::Result<InterruptsIgnored> {
    Ok(InterruptsIgnored {
        previous: sys::ignore_interrupts()?,
    })
}

impl Drop for InterruptsIgnored {
    fn drop(&mut self) {
        sys::restore_interrupts(&self.previous);
    }
}

/// A terminal in raw mode. See the [module documentation](self).
#[derive(Debug)]
pub struct Terminal<D: Device = System> {
    device: D,
    caps: Capabilities,
    decoder: Decoder,
    events: VecDeque<Event>,
    bytes: Vec<u8>,
    keyboard: bool,
    win32: bool,
    active: bool,
    /// When the decoder began to keep bytes that more input could complete
    /// (a lone ESC): they are decoded as they stand once [`ESCAPE_WAIT`] has
    /// passed, however often the wait is cut short meanwhile.
    pending_since: Option<Instant>,
    /// Whether the reply that ends the capability queries has come.
    answered: bool,
    /// When the capability queries were sent: a lone ESC that comes within
    /// [`REPLIES_DUE`] of it, while `answered` is not yet true, waits
    /// [`REPLY_ESCAPE_WAIT`].
    queried: Instant,
    /// When the decoder last had input while inside a paste or a sequence
    /// being dropped: it gives up on it once [`OPEN_WAIT`] has passed.
    open_since: Option<Instant>,
    /// Whether the alternate screen is on (the viewer, the full-screen
    /// presenter).
    alternate: bool,
    /// Whether the alternate screen reports the mouse (since step 25).
    mouse: bool,
    /// How long a paste or a sequence may go with no input before it is
    /// given up on: [`OPEN_WAIT`], or `terminal.paste_wait`.
    open_wait: Duration,
    /// After a paste taken from a burst of keys, how long the reader has
    /// waited for input since: keys that come within [`PASTE_TAIL`] of
    /// waiting are its rest (since step 25).
    paste_tail: Option<Duration>,
    /// The last read taken for a paste ended with an `Enter`: a line feed
    /// that opens its rest is the same line break, a `\r\n` cut between
    /// two reads (the review of the fixes of step 25's second pull
    /// request).
    paste_enter: bool,
    /// A bracketed paste was given up on: when its tail last had input.
    /// What follows of it, to its end marker, is still the paste's (since
    /// step 25).
    stalled: Option<Instant>,
}

impl Terminal<System> {
    /// Takes over the terminal on standard input and output, with its
    /// capabilities detected from the environment and `overrides` applied.
    ///
    /// # Errors
    ///
    /// When standard input or output is not a terminal, when the terminal is
    /// `TERM=dumb`, or when it cannot be put in raw mode. On Windows, also
    /// when the console cannot process escape sequences, as consoles before
    /// Windows 10 cannot.
    pub fn enter(overrides: Overrides) -> io::Result<Self> {
        let caps = Capabilities::detect(
            |name| std::env::var(name).ok(),
            Platform::current(),
            overrides,
        );
        if caps.dumb {
            return Err(io::Error::other(
                "this terminal (TERM=dumb) cannot show the interactive interface",
            ));
        }
        *ENTERED_ON.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(std::thread::current().id());
        PANIC_HOOK.call_once(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if restores_on_panic() {
                    sys::emergency();
                }
                previous(info);
            }));
        });
        sys::enter()?;
        let mut caps = caps;
        // `^H` as the erase character: Backspace sends byte 0x08 (step 25).
        if overrides.backspace_ctrl_h.is_none() {
            caps.backspace_ctrl_h = sys::erase_is_ctrl_h();
        }
        Self::with_device(System { _private: () }, caps)
    }
}

impl<D: Device> Terminal<D> {
    /// A terminal on `device`, already in raw mode, with `caps` as the
    /// starting capabilities. Writes the start sequences and the queries.
    ///
    /// # Errors
    ///
    /// When the device cannot be written to. The device is put back first.
    pub fn with_device(mut device: D, caps: Capabilities) -> io::Result<Self> {
        let mut start = start_sequence(&caps);
        start.extend_from_slice(Capabilities::QUERIES);
        if let Err(error) = device.write(&start) {
            device.leave();
            return Err(error);
        }
        let mut decoder = Decoder::new();
        decoder.set_ctrl_h_backspace(caps.backspace_ctrl_h);
        let open_wait = caps.overrides().paste_wait.unwrap_or(OPEN_WAIT);
        let queried = device.now();
        Ok(Self {
            device,
            caps,
            decoder,
            events: VecDeque::new(),
            bytes: Vec::new(),
            keyboard: false,
            win32: false,
            active: true,
            pending_since: None,
            answered: false,
            queried,
            open_since: None,
            alternate: false,
            mouse: false,
            open_wait,
            paste_tail: None,
            paste_enter: false,
            stalled: None,
        })
    }

    /// What the terminal can do, as known now. Replies to the queries
    /// update it as [`next`](Self::next) reads them.
    #[must_use]
    pub fn caps(&self) -> &Capabilities {
        &self.caps
    }

    /// The window's size: columns, then rows.
    ///
    /// # Errors
    ///
    /// When the system cannot say.
    pub fn size(&self) -> io::Result<(u16, u16)> {
        self.device.size()
    }

    /// Something that wakes [`next`](Self::next) from another thread.
    #[must_use]
    pub fn waker(&self) -> Waker {
        self.device.waker()
    }

    /// Hands the signals that end the program to it from now on (since step
    /// 23): SIGTERM, SIGHUP, SIGINT and SIGQUIT on Unix; on Windows
    /// `Ctrl+Break`, `Ctrl+C` as a console event, and the console's close,
    /// log-off and shutdown. The terminal is put back at once, as it always
    /// is, and the program is then woken to end itself instead of being
    /// ended: [`ending`](Self::ending) says the exit code to end with, and
    /// [`signal_ended`] that it has ended. A watchdog ends the process if
    /// it has not within 3 s (`alarm` on Unix; on Windows the handler's
    /// thread waits that long, since Windows ends the process when it
    /// returns from a close). A second signal ends the process at once.
    pub fn hand_signals_over(&mut self) {
        #[cfg(windows)]
        sys::prepare_hand_off();
        sys::set_hand_off(true);
    }

    /// The exit code a signal handed over asks the program to end with:
    /// 143 for SIGTERM and `Ctrl+Break`, 129 for SIGHUP and the console's
    /// close, 130 for SIGINT, 131 for SIGQUIT. `None` while none has come.
    #[must_use]
    pub fn ending(&self) -> Option<u8> {
        sys::ended_by()
    }

    /// Turns the alternate screen on or off: the viewer's full screen, and
    /// the full-screen presenter's, which leave the main screen, its
    /// scrollback and the live region as they were (since step 23). While
    /// it is on, an emergency restore leaves it too, and so does putting the
    /// terminal back. With it the mouse is reported, if the program asked
    /// for that ([`set_mouse`](Self::set_mouse)), and never without it.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written to.
    pub fn alternate(&mut self, on: bool) -> io::Result<()> {
        if on == self.alternate {
            return Ok(());
        }
        self.alternate = on;
        // The emergency restore's record says on before the screen is, and
        // off only once it is not: a signal in between finds it on and
        // leaves it, and turns the mouse off with it. On the main screen
        // that is not quite nothing (xterm and VTE move the cursor to the
        // position saved last), but it never leaves the terminal on the
        // alternate screen, which the other order could.
        if on {
            sys::set_alternate(true);
            let mut bytes = ALTERNATE_ON.to_vec();
            if self.mouse {
                bytes.extend_from_slice(self.mouse_sequence(true));
                self.decoder.set_mouse(true);
            }
            self.device.write(&bytes)
        } else {
            let mut bytes = Vec::new();
            if self.mouse {
                bytes.extend_from_slice(self.mouse_sequence(false));
                self.decoder.set_mouse(false);
            }
            bytes.extend_from_slice(ALTERNATE_OFF);
            let written = self.device.write(&bytes);
            sys::set_alternate(false);
            written
        }
    }

    /// Whether the alternate screen reports the mouse (since step 25): modes
    /// 1000 and 1006 while the alternate screen is on, where the terminal is
    /// asked for the mouse by its sequences (not on Windows, nor while the
    /// win32 input mode is on: [`Capabilities::vt_mouse`]), and never on the
    /// main screen, where the terminal's own wheel and selection must stay
    /// the person's. `Shift`
    /// and a drag stay the terminal's selection in the terminals that
    /// report the mouse. Off at the start. Taking effect at once when the
    /// alternate screen is on.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written to.
    pub fn set_mouse(&mut self, on: bool) -> io::Result<()> {
        if on == self.mouse {
            return Ok(());
        }
        self.mouse = on;
        if !self.alternate {
            return Ok(());
        }
        self.decoder.set_mouse(on);
        let sequence = self.mouse_sequence(on);
        if sequence.is_empty() {
            return Ok(());
        }
        self.device.write(sequence)
    }

    /// The mouse's sequence, on or off (since step 25): on only where the
    /// terminal is asked for the mouse by its sequences
    /// ([`Self::asks_vt_mouse`]); off wherever it might have been
    /// ([`Capabilities::vt_mouse`]), harmless if it was not, so that a
    /// failed write can never leave it on.
    fn mouse_sequence(&self, on: bool) -> &'static [u8] {
        match on {
            true if self.asks_vt_mouse() => MOUSE_ON,
            false if self.caps.vt_mouse => MOUSE_OFF,
            _ => b"",
        }
    }

    /// Whether the terminal is asked for the mouse by its sequences: where
    /// it can be ([`Capabilities::vt_mouse`]: not on Windows), and not while
    /// the win32 input mode is on, whatever the system: through a
    /// pseudo-console in that mode the terminal's reports come as typed keys
    /// (measured in step 25), as they would for a Linux build under WSL or
    /// over ssh in Windows Terminal.
    fn asks_vt_mouse(&self) -> bool {
        self.caps.vt_mouse && !self.win32
    }

    /// Whether the mouse is wanted now: the alternate screen is on, and
    /// the program asked for the mouse. Whether the terminal is asked for
    /// its reports depends on the terminal as well ([`Self::set_mouse`]).
    #[must_use]
    pub fn mouse(&self) -> bool {
        self.mouse && self.alternate
    }

    /// Whether the alternate screen is on.
    #[must_use]
    pub fn is_alternate(&self) -> bool {
        self.alternate
    }

    /// Puts `text` on the clipboard of the terminal the program runs in,
    /// through its clipboard sequence (OSC 52), which reaches it over SSH
    /// too (since step 23, `/copy`). A terminal that does not take the
    /// sequence ignores it.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written to.
    pub fn copy(&mut self, text: &str) -> io::Result<()> {
        let mut bytes = b"\x1b]52;c;".to_vec();
        bytes.extend_from_slice(&base64(text.as_bytes()));
        bytes.push(0x07);
        self.device.write(&bytes)
    }

    /// Whether events already read wait to be given: a key with another
    /// behind it came in the same read, typed in one burst.
    #[must_use]
    pub fn queued(&self) -> bool {
        !self.events.is_empty()
    }

    /// The time now, on the device's clock: a device for tests gives a
    /// time of its own, so that a loop over the terminal can be tested
    /// without waiting.
    #[must_use]
    pub fn now(&self) -> Instant {
        self.device.now()
    }

    /// The next event, waiting at most `timeout` for one (`None`: as long as
    /// it takes). `Ok(None)` when the time ran out or a [`Waker`] woke it.
    ///
    /// A reply to a capability query updates [`caps`](Self::caps) before it
    /// is returned. A reply that shows the enhanced keyboard protocol turns
    /// it on, unless a setting says not to.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read, or has gone.
    pub fn next(&mut self, timeout: Option<Duration>) -> io::Result<Option<Event>> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(Some(event));
            }
            let mut found = Vec::new();
            // What is kept is decoded as it stands once it has waited long
            // enough, whatever cut the earlier waits short: `ESCAPE_WAIT`
            // after it came, or `REPLY_ESCAPE_WAIT` while the capability
            // replies are due.
            let left = self.pending_since.map(|since| {
                self.escape_deadline(since)
                    .saturating_duration_since(self.device.now())
            });
            if left == Some(Duration::ZERO) {
                self.decoder.flush(&mut found);
                self.pending_since = None;
                self.take(found)?;
                continue;
            }
            // A paste whose end has not come, or a sequence being dropped,
            // is given up on after a time with no input.
            let open = self.open_since.map(|since| {
                self.open_wait
                    .saturating_sub(self.device.now().saturating_duration_since(since))
            });
            if open == Some(Duration::ZERO) {
                let pasting = self.decoder.open();
                self.decoder.give_up(&mut found);
                self.open_since = None;
                // A paste given up on: what follows of it is its tail.
                if pasting && found.iter().any(|event| matches!(event, Event::Paste(_))) {
                    self.stalled = Some(self.device.now());
                }
                self.take(found)?;
                continue;
            }
            let own = match (left, open) {
                (Some(left), Some(open)) => Some(left.min(open)),
                (left, open) => left.or(open),
            };
            let wait = match (own, timeout) {
                (Some(own), Some(timeout)) => Some(own.min(timeout)),
                (Some(own), None) => Some(own),
                (None, timeout) => timeout,
            };
            self.bytes.clear();
            let before = self.device.now();
            let ready = self.device.wait(wait, &mut self.bytes)?;
            // A paste's tail counts the time spent waiting for input, and
            // only that (the review of step 25's second pull request).
            if let Some(waited) = self.paste_tail.as_mut() {
                *waited += self.device.now().saturating_duration_since(before);
            }
            match ready {
                Ready::Input => {
                    self.decoder.feed(&self.bytes, &mut found);
                    found = self.pastes(found);
                    // What is kept now waits its own time: from its arrival,
                    // not from that of something the same read finished.
                    if !self.decoder.pending() {
                        self.pending_since = None;
                    } else if self.pending_since.is_none() || !found.is_empty() {
                        self.pending_since = Some(self.device.now());
                    }
                    self.open_since = self.decoder.open().then(|| self.device.now());
                }
                Ready::Resize => {
                    let (cols, rows) = self.device.size()?;
                    found.push(Event::Resize { cols, rows });
                }
                Ready::Resumed => {
                    self.resume()?;
                    found.push(Event::Resumed);
                }
                Ready::Woken => return Ok(None),
                // The escape's own wait, or the open paste's: the next
                // pass decodes it, or gives up on it.
                Ready::Timeout if own.is_some() && wait == own => {}
                Ready::Timeout => return Ok(None),
            }
            self.take(found)?;
        }
    }

    /// What one read gave, with the keys that were a paste made into one
    /// (since step 25): the rest of a bracketed paste given up on, up to
    /// its end marker; and, where pastes are not known to come bracketed,
    /// a burst of keys no one types.
    fn pastes(&mut self, mut found: Vec<Event>) -> Vec<Event> {
        let ended = self
            .bytes
            .windows(PASTE_END.len())
            .any(|window| window == PASTE_END);
        let now = self.device.now();
        let in_tail = self.paste_tail.is_some_and(|waited| waited <= PASTE_TAIL);
        let in_stall = self
            .stalled
            .is_some_and(|since| now.saturating_duration_since(since) <= self.open_wait);
        // A paste's `\r\n` cut between two reads is one line break, as it
        // is inside one: the line feed that opens the rest is dropped. Each
        // read takes the flag; one taken for a paste sets it again.
        if std::mem::take(&mut self.paste_enter) && (in_tail || in_stall) {
            drop_joined_line_feed(&mut found);
        }
        let typed = typed(&found);
        if let Some(since) = self.stalled {
            let tail = now.saturating_duration_since(since) <= self.open_wait;
            match &typed {
                // Nothing typed (a focus report, a reply): the tail goes
                // on, unless the end marker came.
                Some(typed) if typed.keys == 0 => {
                    if ended {
                        self.stalled = None;
                    }
                }
                // A lone key after the give-up is a key: a person's.
                Some(typed) if typed.keys == 1 && !ended => self.stalled = None,
                Some(typed) if tail => {
                    self.stalled = (!ended).then_some(now);
                    self.paste_enter = typed.enter_last;
                    return as_paste(found, typed.text.clone());
                }
                // The end marker alone ends it; anything else does too.
                _ => self.stalled = None,
            }
        }
        if !self.caps.rates_pastes() {
            return found;
        }
        let Some(typed) = typed else {
            self.paste_tail = None;
            return found;
        };
        // Nothing typed: the tail is left as it is.
        if typed.keys == 0 {
            return found;
        }
        let burst = typed.keys >= BURST && typed.break_inside();
        let long = typed.keys > LONG_BURST && !typed.breaks();
        if in_tail || burst || long {
            self.paste_tail = Some(Duration::ZERO);
            self.paste_enter = typed.enter_last;
            return as_paste(found, typed.text);
        }
        self.paste_tail = None;
        found
    }

    /// When what the decoder has kept since `since` (a lone ESC) is decoded
    /// as it stands: [`ESCAPE_WAIT`] after it came, or
    /// [`REPLY_ESCAPE_WAIT`] after it came when it came while the capability
    /// replies were due. Bounded both ways: the last reply ends the longer
    /// wait (the deadline is worked out again on each pass), and a terminal
    /// that never sends it is held to it only for an ESC that came within
    /// [`REPLIES_DUE`] of the queries.
    fn escape_deadline(&self, since: Instant) -> Instant {
        let due = !self.answered && since < self.queried + REPLIES_DUE;
        since + if due { REPLY_ESCAPE_WAIT } else { ESCAPE_WAIT }
    }

    /// Queues decoded events, taking in the replies first.
    fn take(&mut self, found: Vec<Event>) -> io::Result<()> {
        for event in found {
            if let Event::Reply(reply) = &event {
                self.reply(*reply)?;
            }
            self.events.push_back(event);
        }
        Ok(())
    }

    /// After a suspension, which turned everything off: turns back on what
    /// was on.
    fn resume(&mut self) -> io::Result<()> {
        let mut again = start_sequence(&self.caps);
        // The stop's restore left the alternate screen, which the program
        // still has on (since the review of step 23), and turned the mouse
        // off with it.
        if self.alternate {
            again.extend_from_slice(ALTERNATE_ON);
            if self.mouse {
                again.extend_from_slice(self.mouse_sequence(true));
            }
        }
        if self.keyboard {
            again.extend_from_slice(KEYBOARD_ON);
        }
        if self.win32 {
            again.extend_from_slice(WIN32_ON);
        }
        self.device.write(&again)
    }

    fn reply(&mut self, reply: Reply) -> io::Result<()> {
        if reply == Reply::DeviceAttributes {
            self.answered = true;
        }
        self.caps.apply(&reply);
        if self.caps.keyboard && !self.keyboard {
            self.keyboard = true;
            sys::set_keyboard(true);
            self.device.write(KEYBOARD_ON)?;
        }
        if self.caps.win32_input && !self.win32 {
            // A mouse asked for by its sequences goes off first: through a
            // pseudo-console in the win32 input mode its reports come as
            // typed keys (step 25), so it is not asked for while the mode is
            // on.
            let mut bytes = Vec::new();
            if self.alternate && self.mouse {
                bytes.extend_from_slice(self.mouse_sequence(false));
            }
            self.win32 = true;
            sys::set_win32(true);
            bytes.extend_from_slice(WIN32_ON);
            self.device.write(&bytes)?;
        }
        Ok(())
    }

    /// Stops the program as Ctrl+Z would (Unix only; nothing happens on
    /// Windows). The terminal is put back while it is stopped. When it
    /// continues, [`next`](Self::next) gives [`Event::Resumed`]: the shell
    /// may have written below the live region, so the presenter starts again
    /// where the cursor is ([`Inline::restart`](crate::Inline::restart)).
    pub fn suspend(&mut self) {
        self.device.suspend();
    }

    /// Turns off what was turned on and puts the terminal's modes back. The
    /// presenter should have [finished](crate::Inline::finish) first, so
    /// that the live region is gone.
    ///
    /// # Errors
    ///
    /// When the end sequences cannot be written. The modes are put back
    /// whatever happens.
    pub fn leave(mut self) -> io::Result<()> {
        self.end()
    }

    fn end(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.active = false;
        // After an emergency the terminal is in its ordinary mode already:
        // reading it now would take what the user typed for the shell.
        if self.device.active() {
            self.drain_replies();
        }
        let mut end = Vec::new();
        let alternate = std::mem::take(&mut self.alternate);
        if alternate {
            if self.mouse {
                end.extend_from_slice(self.mouse_sequence(false));
            }
            end.extend_from_slice(ALTERNATE_OFF);
        }
        if self.keyboard {
            end.extend_from_slice(KEYBOARD_OFF);
        }
        if self.win32 {
            end.extend_from_slice(WIN32_OFF);
        }
        end.extend_from_slice(END);
        let written = self.device.write(&end);
        if alternate {
            sys::set_alternate(false);
        }
        self.device.leave();
        if self.keyboard {
            sys::set_keyboard(false);
        }
        if self.win32 {
            sys::set_win32(false);
        }
        written
    }
}

impl<D: Device> Terminal<D> {
    /// Reads, and drops, input until the last reply to the capability
    /// queries has come, for at most [`REPLY_WAIT`]. Once the terminal is
    /// back in its ordinary mode, a late reply would reach the shell as
    /// typed text.
    fn drain_replies(&mut self) {
        let deadline = self.device.now() + REPLY_WAIT;
        let mut found = Vec::new();
        while !self.answered {
            let left = deadline.saturating_duration_since(self.device.now());
            if left.is_zero() {
                return;
            }
            self.bytes.clear();
            match self.device.wait(Some(left), &mut self.bytes) {
                Ok(Ready::Input) => {
                    found.clear();
                    self.decoder.feed(&self.bytes, &mut found);
                    self.answered |= found.contains(&Event::Reply(Reply::DeviceAttributes));
                }
                Ok(Ready::Timeout) | Err(_) => return,
                Ok(_) => {}
            }
        }
    }
}

/// The program has ended what a signal handed to it asked (see
/// [`Terminal::hand_signals_over`]): on Windows the console's handler stops
/// waiting for it. Called last, just before the program exits.
pub fn signal_ended() {
    sys::mark_ended();
    #[cfg(windows)]
    sys::signal_ended();
    // The handlers kept past leaving the terminal go: the program has
    // ended, and exits now.
    sys::release_signals();
}

/// The exit code of a signal handed to the program ([`Terminal::ending`]),
/// asked once the terminal is left: a signal can come while the program
/// ends, after its loop (since the review of step 23). `None` while none
/// has come.
#[must_use]
pub fn signalled() -> Option<u8> {
    sys::ended_by()
}

/// `bytes` in the standard base64 alphabet, padded: what the clipboard
/// sequence carries.
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63]);
        out.push(ALPHABET[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63]
        } else {
            b'='
        });
    }
    out
}

impl<D: Device> Sink for Terminal<D> {
    fn write_frame(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.device.write(bytes)
    }

    fn region(&mut self, rows_above: usize) {
        sys::set_rows_above(rows_above);
    }
}

impl<D: Device> Drop for Terminal<D> {
    fn drop(&mut self) {
        let _ = self.end();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::input::{Key, KeyCode, Mods};

    /// A device that plays back what a test gives it.
    #[derive(Debug, Default)]
    struct Script {
        steps: VecDeque<(Ready, Vec<u8>)>,
        written: Arc<Mutex<Vec<u8>>>,
        waits: Arc<Mutex<Vec<Option<Duration>>>>,
        left: Arc<Mutex<bool>>,
        /// The script's own clock: how far it is past `base`. A wait that
        /// times out moves it by the wait's length; any other step by
        /// [`STEP`].
        base: Option<Instant>,
        elapsed: Arc<Mutex<Duration>>,
    }

    /// How long a step of a script that is not a timeout takes.
    const STEP: Duration = Duration::from_millis(20);

    impl Device for Script {
        fn size(&self) -> io::Result<(u16, u16)> {
            Ok((100, 30))
        }
        fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.written.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }
        fn wait(&mut self, timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
            self.waits.lock().unwrap().push(timeout);
            let (ready, bytes) = self
                .steps
                .pop_front()
                .unwrap_or((Ready::Timeout, Vec::new()));
            *self.elapsed.lock().unwrap() += match ready {
                Ready::Timeout => timeout.unwrap_or(Duration::ZERO),
                _ => STEP,
            };
            into.extend_from_slice(&bytes);
            Ok(ready)
        }
        fn now(&self) -> Instant {
            self.base.unwrap() + *self.elapsed.lock().unwrap()
        }
        fn waker(&self) -> Waker {
            Waker::new(|| {})
        }
        fn suspend(&mut self) {}
        fn leave(&mut self) {
            *self.left.lock().unwrap() = true;
        }
    }

    fn caps() -> Capabilities {
        Capabilities::detect(|_| None, Platform::Unix, Overrides::default())
    }

    /// Capabilities that trust the terminal's brackets: no burst is weighed.
    fn trusted() -> Capabilities {
        Capabilities::detect(
            |_| None,
            Platform::Unix,
            Overrides {
                bracketed_paste: Some(true),
                ..Overrides::default()
            },
        )
    }

    fn script(steps: &[(Ready, &[u8])]) -> Script {
        Script {
            steps: steps.iter().map(|(r, b)| (*r, b.to_vec())).collect(),
            base: Some(Instant::now()),
            ..Script::default()
        }
    }

    /// A stop's restore left the alternate screen; continued, the terminal
    /// turns it on again while the program still has it on (the viewer),
    /// and not otherwise (the review of step 23, step 25's note).
    #[test]
    fn a_resume_enters_the_alternate_screen_again_if_it_was_on() {
        for on in [true, false] {
            let device = script(&[(Ready::Resumed, b"")]);
            let written = Arc::clone(&device.written);
            let mut terminal = Terminal::with_device(device, caps()).unwrap();
            terminal.alternate(on).unwrap();
            written.lock().unwrap().clear();
            assert_eq!(terminal.next(None).unwrap(), Some(Event::Resumed));
            let again = written.lock().unwrap().clone();
            assert_eq!(
                again.windows(ALTERNATE_ON.len()).any(|w| w == ALTERNATE_ON),
                on,
                "{on}: {again:?}"
            );
            terminal.alternate(false).unwrap();
        }
    }

    /// `CSI > flags u`: the enhanced keyboard protocol is asked to tell
    /// apart keys that are otherwise the same bytes (1) and to report a
    /// key's repeats (2), which a dialog never takes as its answer (the
    /// review of step 23).
    #[test]
    fn the_keyboard_protocol_is_asked_for_its_event_types() {
        let flags: u32 = std::str::from_utf8(&KEYBOARD_ON[3..KEYBOARD_ON.len() - 1])
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(flags & 0b11, 0b11, "{KEYBOARD_ON:?}");
        assert!(KEYBOARD_ON.starts_with(b"\x1b[>") && KEYBOARD_ON.ends_with(b"u"));
    }

    #[test]
    fn copy_sends_the_clipboard_sequence_with_the_text_in_base64() {
        let device = script(&[]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        written.lock().unwrap().clear();
        terminal.copy("hi there").unwrap();
        assert_eq!(&*written.lock().unwrap(), b"\x1b]52;c;aGkgdGhlcmU=\x07");
        assert_eq!(base64(b"a"), b"YQ==");
        assert_eq!(base64(b"ab"), b"YWI=");
        assert_eq!(base64(b"abc"), b"YWJj");
        assert_eq!(base64("é".as_bytes()), b"w6k=");
    }

    /// The viewer's alternate screen is left when the terminal is put back,
    /// and by the emergency restore while it is on, and only then.
    #[test]
    fn the_alternate_screen_is_left_by_the_end_and_by_an_emergency() {
        let leaves = |bytes: &[u8]| {
            bytes
                .windows(ALTERNATE_OFF.len())
                .any(|w| w == ALTERNATE_OFF)
        };
        let device = script(&[]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert!(!leaves(sys::restore_sequence()));
        terminal.alternate(true).unwrap();
        assert!(terminal.is_alternate());
        assert!(written.lock().unwrap().ends_with(ALTERNATE_ON));
        assert!(leaves(sys::restore_sequence()));
        terminal.leave().unwrap();
        assert!(
            written
                .lock()
                .unwrap()
                .ends_with(&[ALTERNATE_OFF, END].concat())
        );
        assert!(!leaves(sys::restore_sequence()));
    }

    /// On Windows the terminal is asked for no mouse report (a
    /// pseudo-console would give the reports to the program as typed
    /// keys): the alternate screen and the program's wish for the mouse
    /// write no mouse sequence, and the decoder still reads the reports the
    /// console's own mouse records are written as (step 25).
    #[test]
    fn on_windows_the_terminal_is_asked_for_no_mouse_reports() {
        let device = script(&[(Ready::Input, b"\x1b[<64;10;5M")]);
        let written = device.written.clone();
        let windows = Capabilities::detect(|_| None, Platform::Windows, Overrides::default());
        assert!(!windows.vt_mouse);
        assert!(caps().vt_mouse, "Unix asks for them");
        let mut terminal = Terminal::with_device(device, windows).unwrap();
        terminal.set_mouse(true).unwrap();
        terminal.alternate(true).unwrap();
        let out = written.lock().unwrap().clone();
        assert!(holds(&out, ALTERNATE_ON));
        assert!(!holds(&out, b"\x1b[?1000h") && !holds(&out, b"\x1b[?1006h"));
        assert!(matches!(
            terminal.next(None).unwrap(),
            Some(Event::Mouse(crate::Mouse {
                kind: crate::MouseKind::Wheel(crate::Wheel::Up),
                ..
            }))
        ));
        // Every way the mouse could be asked for or let go, the end too
        // (the review of step 25's follow-up).
        terminal.set_mouse(false).unwrap();
        terminal.set_mouse(true).unwrap();
        terminal.alternate(false).unwrap();
        terminal.alternate(true).unwrap();
        terminal.leave().unwrap();
        let out = written.lock().unwrap().clone();
        assert!(holds(&out, ALTERNATE_OFF));
        assert!(!holds(&out, b"\x1b[?1000") && !holds(&out, b"\x1b[?1006"));
    }

    /// While the win32 input mode is on the terminal is not asked for the
    /// mouse, whatever the system: through a pseudo-console in that mode
    /// its reports come as typed keys (a Linux build under WSL or over ssh
    /// in Windows Terminal). The mode's reply after the mouse was asked for
    /// turns it off first (step 25's follow-up).
    #[test]
    fn the_win32_input_mode_takes_the_mouse_off_and_keeps_it_off() {
        let device = script(&[(Ready::Input, b"\x1b[?9001;2$y"), (Ready::Resumed, b"")]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        terminal.set_mouse(true).unwrap();
        terminal.alternate(true).unwrap();
        assert!(holds(&written.lock().unwrap(), MOUSE_ON));
        assert!(matches!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::Mode { mode: 9001, .. }))
        ));
        assert!(
            written
                .lock()
                .unwrap()
                .ends_with(&[MOUSE_OFF, WIN32_ON].concat()),
            "the mouse off before the mode on"
        );
        let mark = written.lock().unwrap().len();
        // Continued after a stop (a Linux build under WSL, `fg`), and every
        // other way the mouse could be asked for: never on again. Turning
        // it off stays harmless, so only a mode set is looked for.
        assert_eq!(terminal.next(None).unwrap(), Some(Event::Resumed));
        terminal.set_mouse(false).unwrap();
        terminal.set_mouse(true).unwrap();
        terminal.alternate(false).unwrap();
        terminal.alternate(true).unwrap();
        terminal.leave().unwrap();
        let after = written.lock().unwrap()[mark..].to_vec();
        assert!(
            holds(&after, WIN32_ON),
            "the resumption turns the mode on again"
        );
        assert!(holds(&after, WIN32_OFF));
        assert!(
            holds(&after, MOUSE_OFF),
            "off is still written, as harmless"
        );
        assert!(!holds(&after, b"\x1b[?1000h") && !holds(&after, b"\x1b[?1006h"));
    }

    /// Where pastes are not known to come bracketed, keys that come in one
    /// read many at a time with a line break inside are a paste, and the
    /// keys of the reads just after it are its rest: a pasted line break
    /// never sends the prompt (step 25, `pastes-without-bracketed-paste`).
    /// Typing, a line ended by `Enter`, and keys with `Ctrl` are keys.
    #[test]
    fn a_burst_of_keys_with_a_line_break_inside_is_a_paste() {
        let device = script(&[
            (Ready::Input, b"first line\rsecond li"),
            (Ready::Input, b"ne\r"),
            (Ready::Timeout, b""),
            (Ready::Input, b"a line typed\r"),
            (Ready::Input, b"abc\x03def\rgh"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first line\nsecond li".to_owned()))
        );
        // Its rest, a read later: still the paste's.
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("ne\n".to_owned()))
        );
        // A moment later: a line typed, many keys in one read as over a slow
        // connection, its `Enter` the last: keys, the `Enter` sends.
        assert_eq!(
            terminal.next(Some(Duration::from_millis(200))).unwrap(),
            None
        );
        let mut keys = Vec::new();
        for _ in 0..13 {
            keys.push(terminal.next(None).unwrap());
        }
        assert_eq!(
            keys.last(),
            Some(&Some(Event::Key(Key::plain(KeyCode::Enter))))
        );
        // A burst with a control key in it is keys.
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('a'))))
        );
    }

    /// A terminal that says it knows bracketed paste is trusted to bracket
    /// its pastes: no burst is taken for one. The override can trust it, or
    /// not ask for brackets at all.
    #[test]
    fn a_terminal_that_knows_bracketed_paste_has_no_burst_taken_for_one() {
        let device = script(&[
            (Ready::Input, b"\x1b[?2004;1$y"),
            (Ready::Input, b"ab\rcdefgh\r"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::Mode {
                mode: 2004,
                state: 1
            }))
        );
        assert!(!terminal.caps().rates_pastes());
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('a'))))
        );
        // Not asked for: the start leaves bracketed paste off, and every
        // terminal is rate-detected.
        let device = script(&[]);
        let written = device.written.clone();
        let unbracketed = Capabilities::detect(
            |_| None,
            Platform::Unix,
            Overrides {
                bracketed_paste: Some(false),
                ..Overrides::default()
            },
        );
        let terminal = Terminal::with_device(device, unbracketed).unwrap();
        assert!(!holds(&written.lock().unwrap(), b"\x1b[?2004h"));
        assert!(terminal.caps().rates_pastes());
    }

    /// A bracketed paste that paused past the wait is given up on, and what
    /// follows of it, up to its end marker, is its rest, line breaks and
    /// all: nothing is sent (step 25, `a-paste-that-stalls`). A lone key
    /// after the give-up is a key.
    #[test]
    fn the_rest_of_a_paste_that_stalled_is_still_the_paste() {
        let device = script(&[
            (Ready::Input, b"\x1b[200~first part\r"),
            (Ready::Timeout, b""),
            (Ready::Input, b"second\rpart\x1b[201~"),
            (Ready::Input, b"ok\r"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first part\n".to_owned()))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("second\npart".to_owned()))
        );
        // The end marker ended it: what is typed next is keys, and its
        // `Enter` sends.
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('o'))))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('k'))))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Enter)))
        );
    }

    /// A paste's line breaks as each terminal sends them: `\r`, `\n` (read
    /// as `Ctrl+J`), and `\r\n` (one line break), in a burst, in its tail,
    /// and in the rest of a bracketed paste that stalled: none of them an
    /// `Enter` that sends (the review of step 25's second pull request).
    #[test]
    fn line_feeds_and_crlf_are_a_pastes_line_breaks() {
        for (first, rest) in [
            (&b"first line\nsecond li"[..], &b"ne\n"[..]),
            (b"first line\r\nsecond li", b"ne\r\n"),
        ] {
            let device = script(&[(Ready::Input, first), (Ready::Input, rest)]);
            let mut terminal = Terminal::with_device(device, caps()).unwrap();
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Paste("first line\nsecond li".to_owned())),
                "{first:?}"
            );
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Paste("ne\n".to_owned())),
                "{rest:?}"
            );
        }
        // A stalled paste's rest, where bursts are not weighed.
        let device = script(&[
            (Ready::Input, b"\x1b[200~first part\r\n"),
            (Ready::Timeout, b""),
            (Ready::Input, b"second\r\npart\nend\x1b[201~"),
        ]);
        let mut terminal = Terminal::with_device(device, trusted()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first part\n".to_owned()))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("second\npart\nend".to_owned()))
        );
    }

    /// A paste's `\r\n` cut between two reads, at the `\r`, is one line
    /// break, as inside one read: in a burst's tail, a read of the line
    /// feed alone, and the rest of a bracketed paste that stalled (the
    /// review of the fixes of step 25's second pull request: the line feed
    /// was a second line break, a blank line in the prompt).
    #[test]
    fn a_crlf_cut_between_two_reads_of_a_paste_is_one_line_break() {
        let device = script(&[
            (Ready::Input, b"first line\r\nsecond line\r"),
            (Ready::Input, b"\nthird\r"),
            (Ready::Input, b"\n"),
            (Ready::Input, b"fourth"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        for piece in ["first line\nsecond line\n", "third\n", "fourth"] {
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Paste(piece.to_owned()))
            );
        }
        assert_eq!(terminal.next(None).unwrap(), None, "nothing more");
        // A line feed the read before did not end with `Enter` for is a
        // line break of its own.
        let device = script(&[
            (Ready::Input, b"first line\r\nsecond line"),
            (Ready::Input, b"\nthird"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        for piece in ["first line\nsecond line", "\nthird"] {
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Paste(piece.to_owned()))
            );
        }
        // The rest of a stalled bracketed paste.
        let device = script(&[
            (Ready::Input, b"\x1b[200~first part\r\n"),
            (Ready::Timeout, b""),
            (Ready::Input, b"second\r"),
            (Ready::Input, b"\npart\x1b[201~"),
        ]);
        let mut terminal = Terminal::with_device(device, trusted()).unwrap();
        for piece in ["first part\n", "second\n", "part"] {
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Paste(piece.to_owned()))
            );
        }
    }

    /// A focus report or a reply that comes in the same read as a burst's
    /// keys is passed on beside the paste, and does not make the keys keys
    /// (the review of step 25's second pull request).
    #[test]
    fn a_focus_report_inside_a_burst_is_passed_on_beside_the_paste() {
        let device = script(&[(Ready::Input, b"first line\r\x1b[Isecond line")]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first line\nsecond line".to_owned()))
        );
        assert_eq!(terminal.next(None).unwrap(), Some(Event::Focus(true)));
    }

    /// A line ended by `Enter` in one read is keys however long it is, and
    /// its `Enter` sends: typing through a stall, or a program's
    /// `send-keys`. Fewer than eight keys with a line break inside are keys
    /// too. A long read with no line break at all is still taken for a
    /// paste, whose rest the next read is (the review of step 25's second
    /// pull request).
    #[test]
    fn a_long_line_ended_by_enter_is_keys_and_so_is_a_short_burst() {
        let long = format!("{}\r", "x".repeat(100));
        let unbroken = "y".repeat(100);
        let device = script(&[
            (Ready::Input, long.as_bytes()),
            (Ready::Timeout, b""),
            (Ready::Input, b"ab\rcd"),
            (Ready::Timeout, b""),
            (Ready::Input, unbroken.as_bytes()),
            (Ready::Input, b"z\r"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        for _ in 0..100 {
            assert_eq!(
                terminal.next(None).unwrap(),
                Some(Event::Key(Key::plain(KeyCode::Char('x'))))
            );
        }
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Enter)))
        );
        assert_eq!(
            terminal.next(Some(Duration::from_millis(200))).unwrap(),
            None
        );
        let short: Vec<_> = (0..5).map(|_| terminal.next(None).unwrap()).collect();
        assert_eq!(
            short[2],
            Some(Event::Key(Key::plain(KeyCode::Enter))),
            "{short:?}"
        );
        assert_eq!(
            terminal.next(Some(Duration::from_millis(200))).unwrap(),
            None
        );
        assert_eq!(terminal.next(None).unwrap(), Some(Event::Paste(unbroken)));
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("z\n".to_owned()))
        );
    }

    /// A paste's tail counts the time the reader waited for input, not the
    /// time between handling one read and the next: a frame drawn after a
    /// burst, while the paste's rest waited, does not make the rest keys;
    /// time waited with nothing does (the review of step 25's second pull
    /// request).
    #[test]
    fn a_frame_drawn_between_a_pastes_reads_does_not_end_its_tail() {
        let device = script(&[
            (Ready::Input, b"first line\rsecond li"),
            (Ready::Input, b"ne\r"),
            (Ready::Timeout, b""),
            (Ready::Input, b"x\r"),
        ]);
        let elapsed = Arc::clone(&device.elapsed);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first line\nsecond li".to_owned()))
        );
        // A frame that took longer than the tail, the paste's rest waiting.
        *elapsed.lock().unwrap() += PASTE_TAIL * 2;
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("ne\n".to_owned()))
        );
        // Longer than the tail waited with nothing: keys again.
        assert_eq!(terminal.next(Some(PASTE_TAIL)).unwrap(), None);
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('x'))))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Enter)))
        );
    }

    /// After a bracketed paste is given up on, a lone key is a person's: an
    /// `Enter` alone sends, and what follows is keys. And the rest that
    /// comes after more than the wait with no input is not the paste's
    /// (the review of step 25's second pull request).
    #[test]
    fn a_lone_key_after_a_give_up_and_a_late_rest_are_keys() {
        let device = script(&[
            (Ready::Input, b"\x1b[200~first"),
            (Ready::Timeout, b""),
            (Ready::Input, b"\r"),
            (Ready::Input, b"second\rpart"),
        ]);
        let mut terminal = Terminal::with_device(device, trusted()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first".to_owned()))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Enter)))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('s'))))
        );

        let device = script(&[
            (Ready::Input, b"\x1b[200~first"),
            (Ready::Timeout, b""),
            (Ready::Timeout, b""),
            (Ready::Input, b"second\rpart"),
        ]);
        let mut terminal = Terminal::with_device(device, trusted()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("first".to_owned()))
        );
        assert_eq!(
            terminal.next(Some(OPEN_WAIT + STEP)).unwrap(),
            None,
            "past the wait with nothing"
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Char('s'))))
        );
    }

    /// The erase character's `^H` makes byte 0x08 `Backspace` (step 25,
    /// `backspace-sent-as-ctrl-h`); otherwise it is `Ctrl+Backspace`.
    #[test]
    fn byte_eight_is_backspace_where_the_terminal_erases_with_it() {
        for (ctrl_h, want) in [
            (true, Key::plain(KeyCode::Backspace)),
            (false, Key::with(KeyCode::Backspace, Mods::CTRL)),
        ] {
            let device = script(&[(Ready::Input, b"\x08")]);
            let mut with = caps();
            with.backspace_ctrl_h = ctrl_h;
            let mut terminal = Terminal::with_device(device, with).unwrap();
            assert_eq!(terminal.next(None).unwrap(), Some(Event::Key(want)));
        }
    }

    fn holds(bytes: &[u8], part: &[u8]) -> bool {
        bytes.windows(part.len()).any(|w| w == part)
    }

    /// The mouse is reported only with the alternate screen, once the
    /// program has asked for it (step 25): turned on with the screen, off
    /// before it is left, at the end, and by every emergency restore while
    /// the alternate screen is on; never on the main screen.
    #[test]
    fn the_mouse_is_on_only_with_the_alternate_screen_and_off_on_every_way_out() {
        let device = script(&[]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        // Asked for on the main screen: nothing is turned on there.
        written.lock().unwrap().clear();
        terminal.set_mouse(true).unwrap();
        assert!(written.lock().unwrap().is_empty());
        assert!(!terminal.mouse());
        terminal.alternate(true).unwrap();
        assert_eq!(
            &*written.lock().unwrap(),
            &[ALTERNATE_ON, MOUSE_ON].concat()
        );
        assert!(terminal.mouse());
        // Every restore with the alternate screen on turns the mouse off,
        // before it leaves the screen.
        let restore = sys::restore_sequence();
        let off = |part: &[u8]| restore.windows(part.len()).position(|w| w == part);
        assert!(off(b"\x1b[?1000l") < off(ALTERNATE_OFF), "{restore:?}");
        assert!(off(b"\x1b[?1006l").is_some(), "{restore:?}");
        // Off with the screen: the mouse first.
        written.lock().unwrap().clear();
        terminal.alternate(false).unwrap();
        assert_eq!(
            &*written.lock().unwrap(),
            &[MOUSE_OFF, ALTERNATE_OFF].concat()
        );
        assert!(!terminal.mouse());
        // Turned off while the screen is on: at once.
        terminal.alternate(true).unwrap();
        written.lock().unwrap().clear();
        terminal.set_mouse(false).unwrap();
        assert_eq!(&*written.lock().unwrap(), MOUSE_OFF);
        terminal.set_mouse(true).unwrap();
        // The end: the mouse off, then the screen, then the rest.
        written.lock().unwrap().clear();
        terminal.leave().unwrap();
        let end = written.lock().unwrap().clone();
        assert!(
            end.starts_with(&[MOUSE_OFF, ALTERNATE_OFF].concat()),
            "{end:?}"
        );
        assert!(end.ends_with(END), "{end:?}");
    }

    /// A stop's restore turned the mouse off with the alternate screen;
    /// continued, both come back, the mouse only if it was asked for.
    #[test]
    fn a_resume_turns_the_mouse_on_again_with_the_alternate_screen() {
        for mouse in [true, false] {
            let device = script(&[(Ready::Resumed, b"")]);
            let written = Arc::clone(&device.written);
            let mut terminal = Terminal::with_device(device, caps()).unwrap();
            terminal.set_mouse(mouse).unwrap();
            terminal.alternate(true).unwrap();
            written.lock().unwrap().clear();
            assert_eq!(terminal.next(None).unwrap(), Some(Event::Resumed));
            let again = written.lock().unwrap().clone();
            assert!(holds(&again, ALTERNATE_ON), "{again:?}");
            assert_eq!(holds(&again, MOUSE_ON), mouse, "{again:?}");
            terminal.alternate(false).unwrap();
        }
    }

    /// While the mouse is reported, mode 1000's own report is read whole;
    /// once the alternate screen is left, `CSI M` is a sequence with no
    /// meaning again, and what follows it is typed.
    #[test]
    fn the_older_mouse_report_is_read_only_while_the_mouse_is_on() {
        let device = script(&[(Ready::Input, b"\x1b[M !!"), (Ready::Input, b"\x1b[Mab")]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        terminal.set_mouse(true).unwrap();
        terminal.alternate(true).unwrap();
        assert!(matches!(
            terminal.next(None).unwrap(),
            Some(Event::Mouse(crate::Mouse {
                kind: crate::MouseKind::Press(crate::Button::Left),
                col: 0,
                row: 0,
                ..
            }))
        ));
        terminal.alternate(false).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(crate::Key::plain(crate::KeyCode::Char('a'))))
        );
    }

    #[test]
    fn it_starts_with_the_modes_and_the_queries_and_ends_by_turning_them_off() {
        let device = script(&[]);
        let (written, left) = (device.written.clone(), device.left.clone());
        let terminal = Terminal::with_device(device, caps()).unwrap();
        let start = written.lock().unwrap().clone();
        assert_eq!(start, [START, Capabilities::QUERIES].concat());
        terminal.leave().unwrap();
        assert!(*left.lock().unwrap());
        assert!(written.lock().unwrap().ends_with(END));
    }

    #[test]
    fn dropping_it_puts_the_terminal_back_too() {
        let device = script(&[]);
        let left = device.left.clone();
        drop(Terminal::with_device(device, caps()).unwrap());
        assert!(*left.lock().unwrap());
    }

    #[test]
    fn a_paste_whose_end_never_comes_is_given_up_on_and_ctrl_c_works_after() {
        let device = script(&[
            (Ready::Input, b"\x1b[200~hello\r"),
            (Ready::Input, b"world"),
            (Ready::Timeout, b""),
            (Ready::Input, b"\x03"),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Paste("hello\nworld".to_owned())),
            "what came, as a paste, never as keys"
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::with(KeyCode::Char('c'), Mods::CTRL)))
        );
        let waits = waits.lock().unwrap().clone();
        // The wait for more of the paste counts from its last input.
        assert_eq!(waits[0], None);
        assert!(waits[1].is_some_and(|wait| wait <= OPEN_WAIT && wait > OPEN_WAIT / 2));
        assert!(waits[2].is_some_and(|wait| wait <= OPEN_WAIT && wait > OPEN_WAIT / 2));
    }

    /// The device attributes, the last reply to the capability queries:
    /// after it a lone ESC waits only [`ESCAPE_WAIT`].
    const LAST_REPLY: &[u8] = b"\x1b[?62c";

    /// Reads the last reply, which a script gives first.
    fn answered(terminal: &mut Terminal<Script>) {
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::DeviceAttributes))
        );
    }

    #[test]
    fn a_lone_escape_is_the_escape_key_after_a_short_wait() {
        let device = script(&[
            (Ready::Input, LAST_REPLY),
            (Ready::Input, b"\x1b"),
            (Ready::Timeout, b""),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        answered(&mut terminal);
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
        assert_eq!(*waits.lock().unwrap(), vec![None, None, Some(ESCAPE_WAIT)]);
    }

    #[test]
    fn the_rest_of_a_sequence_within_the_wait_is_one_key() {
        let device = script(&[(Ready::Input, b"\x1b"), (Ready::Input, b"[1;5A")]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::with(KeyCode::Up, Mods::CTRL)))
        );
    }

    #[test]
    fn a_shorter_timeout_of_the_caller_returns_and_the_escape_still_comes() {
        let device = script(&[
            (Ready::Input, LAST_REPLY),
            (Ready::Input, b"\x1b"),
            (Ready::Timeout, b""),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        answered(&mut terminal);
        assert_eq!(terminal.next(Some(Duration::from_millis(5))).unwrap(), None);
        // A longer wait of the caller's is cut to what is left of the
        // escape's, and gives the escape.
        assert_eq!(
            terminal.next(Some(Duration::from_millis(100))).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
    }

    #[test]
    fn a_pending_escape_is_the_escape_key_however_often_the_wait_is_woken() {
        // While a stream runs, the loop is woken every few milliseconds. The
        // escape comes once 50 ms have passed since it arrived.
        let device = script(&[
            (Ready::Input, LAST_REPLY),
            (Ready::Input, b"\x1b"),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        answered(&mut terminal);
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            None
        );
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            None
        );
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            None,
            "40 ms after the escape"
        );
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc))),
            "60 ms after the escape"
        );
        // Each wait was cut to what was left of the escape's.
        let waits = waits.lock().unwrap().clone();
        assert_eq!(waits[2], Some(Duration::from_millis(16)));
        assert_eq!(waits[4], Some(Duration::from_millis(10)));
    }

    /// The reply to the keyboard query split by a pseudo-console on a loaded
    /// machine: its ESC, then nothing for longer than [`ESCAPE_WAIT`], then
    /// the rest. While the replies are due it is the reply, and nothing is
    /// typed (the flaky trust test of `cineris-cli`, `docs/steps/ci-budgets`).
    #[test]
    fn a_reply_split_after_its_escape_while_the_replies_are_due_types_nothing() {
        let device = script(&[
            (Ready::Input, b"\x1b"),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Input, b"[?0u"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(terminal.next(Some(Duration::from_millis(16))).unwrap());
        }
        // 100 ms after the ESC, twice its own wait, it is still kept.
        assert_eq!(events, []);
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            Some(Event::Reply(Reply::Keyboard(0)))
        );
    }

    /// A terminal entered again, as the external editor gives it back (a new
    /// `Terminal`, `Terminal::enter`), sends the queries again and waits for
    /// its own replies, whatever the first one's did: the split reply types
    /// nothing there either (`cineris-tui`'s failing-editor test, whose
    /// child ended at the first key it read after the editor).
    #[test]
    fn a_terminal_entered_again_waits_for_its_own_replies() {
        let first = script(&[(Ready::Input, LAST_REPLY)]);
        let mut terminal = Terminal::with_device(first, caps()).unwrap();
        answered(&mut terminal);
        terminal.leave().unwrap();
        let again = script(&[
            (Ready::Input, b"\x1b"),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Input, b"[?0u"),
        ]);
        let written = Arc::clone(&again.written);
        let mut terminal = Terminal::with_device(again, caps()).unwrap();
        assert!(
            written
                .lock()
                .unwrap()
                .windows(Capabilities::QUERIES.len())
                .any(|w| w == Capabilities::QUERIES),
            "the queries are sent again"
        );
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(terminal.next(Some(Duration::from_millis(16))).unwrap());
        }
        assert_eq!(events, []);
        assert_eq!(
            terminal.next(Some(Duration::from_millis(16))).unwrap(),
            Some(Event::Reply(Reply::Keyboard(0)))
        );
    }

    /// Once the last reply has come, the same split is the Escape key and
    /// typed text, as it always was: the longer wait ends with the replies.
    #[test]
    fn after_the_last_reply_a_split_escape_waits_only_its_own_time() {
        let device = script(&[
            (Ready::Input, LAST_REPLY),
            (Ready::Input, b"\x1b"),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Woken, b""),
            (Ready::Input, b"[?0u"),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        answered(&mut terminal);
        let mut events = Vec::new();
        for _ in 0..6 {
            events.extend(terminal.next(Some(Duration::from_millis(16))).unwrap());
        }
        while let Some(event) = terminal.next(Some(Duration::ZERO)).unwrap() {
            events.push(event);
        }
        assert_eq!(
            events[0],
            Event::Key(Key::plain(KeyCode::Esc)),
            "{events:?}"
        );
        assert_eq!(
            events[1..],
            "[?0u"
                .chars()
                .map(|c| Event::Key(Key::plain(KeyCode::Char(c))))
                .collect::<Vec<_>>()
        );
    }

    /// The longer wait is the ESC's own, counted from when it came, and
    /// bounded both ways: a reply that never comes holds an ESC no more than
    /// [`REPLY_ESCAPE_WAIT`]; an ESC that comes just before [`REPLIES_DUE`]
    /// ends still waits it whole; and once [`REPLIES_DUE`] has passed with
    /// no last reply (a terminal that never sends it), an ESC waits only
    /// [`ESCAPE_WAIT`].
    #[test]
    fn with_no_last_reply_the_longer_wait_ends_when_the_replies_are_no_longer_due() {
        let device = script(&[(Ready::Input, b"\x1b"), (Ready::Timeout, b"")]);
        let (waits, elapsed) = (device.waits.clone(), device.elapsed.clone());
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
        // The ESC came one step after the queries, and waited its own
        // longer wait from then.
        assert_eq!(*waits.lock().unwrap(), vec![None, Some(REPLY_ESCAPE_WAIT)]);
        assert_eq!(*elapsed.lock().unwrap(), STEP + REPLY_ESCAPE_WAIT);

        // Just before the replies stop being due: the whole longer wait.
        let device = script(&[
            (Ready::Timeout, b""),
            (Ready::Input, b"\x1b"),
            (Ready::Timeout, b""),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        let before_the_end = REPLIES_DUE - STEP * 2;
        assert_eq!(terminal.next(Some(before_the_end)).unwrap(), None);
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
        assert_eq!(
            *waits.lock().unwrap(),
            vec![Some(before_the_end), None, Some(REPLY_ESCAPE_WAIT)]
        );

        // After it: the ESC's own short wait.
        let device = script(&[
            (Ready::Timeout, b""),
            (Ready::Input, b"\x1b"),
            (Ready::Timeout, b""),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(terminal.next(Some(REPLIES_DUE)).unwrap(), None);
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
        assert_eq!(
            *waits.lock().unwrap(),
            vec![Some(REPLIES_DUE), None, Some(ESCAPE_WAIT)]
        );
    }

    /// The last reply ends the longer wait. An ESC held when the reply comes
    /// is taken with it, as an ESC before any sequence is (Alt); the next
    /// ESC waits only [`ESCAPE_WAIT`].
    #[test]
    fn the_last_reply_ends_the_longer_wait_of_the_next_escape() {
        let device = script(&[
            (Ready::Input, b"\x1b"),
            (Ready::Input, LAST_REPLY),
            (Ready::Input, b"\x1b"),
            (Ready::Timeout, b""),
        ]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::DeviceAttributes))
        );
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::plain(KeyCode::Esc)))
        );
        let waits = waits.lock().unwrap().clone();
        assert_eq!(waits.last(), Some(&Some(ESCAPE_WAIT)), "{waits:?}");
    }

    #[test]
    fn after_a_suspension_what_was_on_is_turned_on_again() {
        let device = script(&[
            (Ready::Input, b"\x1b[?1u\x1b[?9001;2$y"),
            (Ready::Resumed, b""),
        ]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        terminal.next(None).unwrap();
        terminal.next(None).unwrap();
        written.lock().unwrap().clear();
        assert_eq!(terminal.next(None).unwrap(), Some(Event::Resumed));
        assert_eq!(
            *written.lock().unwrap(),
            [START, KEYBOARD_ON, WIN32_ON].concat()
        );
    }

    #[test]
    fn leaving_waits_a_little_for_the_last_reply() {
        let device = script(&[
            (Ready::Input, b"\x1b[?2026;2$y"),
            (Ready::Input, b"x\x1b[?62;22c"),
        ]);
        let (waits, written) = (device.waits.clone(), device.written.clone());
        let terminal = Terminal::with_device(device, caps()).unwrap();
        terminal.leave().unwrap();
        // Two reads, then no more: the reply had come.
        assert_eq!(waits.lock().unwrap().len(), 2);
        assert!(written.lock().unwrap().ends_with(END));
        // With no reply at all, it gives up after the bound.
        let device = script(&[]);
        let waits = device.waits.clone();
        Terminal::with_device(device, caps())
            .unwrap()
            .leave()
            .unwrap();
        assert_eq!(*waits.lock().unwrap(), [Some(REPLY_WAIT)]);
        // Once the reply has come, leaving reads nothing.
        let device = script(&[(Ready::Input, b"\x1b[?62c")]);
        let waits = device.waits.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        terminal.next(None).unwrap();
        terminal.leave().unwrap();
        assert_eq!(waits.lock().unwrap().len(), 1);
    }

    #[test]
    fn replies_update_the_capabilities_and_turn_on_the_keyboard_protocol() {
        let device = script(&[(Ready::Input, b"\x1b[?1u\x1b[?2026;4$y")]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::Keyboard(1)))
        );
        assert!(terminal.caps().keyboard);
        assert!(written.lock().unwrap().ends_with(KEYBOARD_ON));
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::Mode {
                mode: 2026,
                state: 4
            }))
        );
        assert!(!terminal.caps().sync);
        terminal.leave().unwrap();
        let all = written.lock().unwrap().clone();
        assert!(all.ends_with(&[KEYBOARD_OFF, END].concat()));
        assert!(
            !sys::restore_sequence()
                .windows(4)
                .any(|w| w == KEYBOARD_OFF)
        );
    }

    #[test]
    fn a_terminal_that_has_the_win32_input_mode_gets_it_until_the_end() {
        let device = script(&[(Ready::Input, b"\x1b[?9001;2$y\x1b[13;28;13;1;16;1_")]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Reply(Reply::Mode {
                mode: 9001,
                state: 2
            }))
        );
        assert!(written.lock().unwrap().ends_with(WIN32_ON));
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Key(Key::with(KeyCode::Enter, Mods::SHIFT)))
        );
        terminal.leave().unwrap();
        assert!(
            written
                .lock()
                .unwrap()
                .ends_with(&[WIN32_OFF, END].concat())
        );
    }

    #[test]
    fn a_setting_keeps_the_keyboard_protocol_off() {
        let overrides = Overrides {
            keyboard: Some(false),
            ..Overrides::default()
        };
        let caps = Capabilities::detect(|_| None, Platform::Unix, overrides);
        let device = script(&[(Ready::Input, b"\x1b[?1u\x1b[?9001;2$y")]);
        let written = device.written.clone();
        let mut terminal = Terminal::with_device(device, caps).unwrap();
        terminal.next(None).unwrap();
        terminal.next(None).unwrap();
        assert!(!written.lock().unwrap().windows(5).any(|w| w == KEYBOARD_ON));
        assert!(!written.lock().unwrap().windows(8).any(|w| w == WIN32_ON));
    }

    #[test]
    fn resizes_resumptions_and_wakes() {
        let device = script(&[
            (Ready::Resize, b""),
            (Ready::Resumed, b""),
            (Ready::Woken, b""),
        ]);
        let mut terminal = Terminal::with_device(device, caps()).unwrap();
        assert_eq!(
            terminal.next(None).unwrap(),
            Some(Event::Resize {
                cols: 100,
                rows: 30
            })
        );
        assert_eq!(terminal.next(None).unwrap(), Some(Event::Resumed));
        assert_eq!(terminal.next(None).unwrap(), None);
    }
}
