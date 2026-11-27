//! The approvals password, typed at a terminal and in the application
//! (since the signed approvals; `docs/architecture/09-tui.md`, "The
//! approvals password", and `docs/architecture/08-configuration.md`,
//! "Credentials").
//!
//! The approvals the harness stores for the person are signed with a key
//! made, each time, from a password the person types
//! (`cineris_policy::signed`). This is where it is typed:
//!
//! - [`PasswordInput`]: the field, a sibling of the key's
//!   [`KeyInput`](super::KeyInput) over the same [`SecretBuf`]. It answers
//!   typing, a paste, `Backspace`, `Ctrl+U` (which empties it), `Enter` and
//!   `Esc`, and nothing else does anything there. It is never drawn: its row
//!   says only how many characters it holds.
//! - [`PasswordPrompt`]: what is asked ([`PasswordAsk`]: a [`Purpose`],
//!   the caller's lines and a warning), and, for a password chosen, the
//!   second typing. The application's dialog (`app::state::password`) and
//!   the terminal's prompt are both this prompt.
//! - [`read_password`], [`PasswordTerminal`] and [`read_password_on`]: the
//!   prompt at a terminal, for the subcommands.
//!
//! **Unlike a key,** a password is not trimmed, and is any Unicode text but
//! control characters; a password chosen has at least
//! [`Password::LEAST`] characters (Unicode scalar values); and it is handed
//! on as a [`Password`], which gives only its bytes and never becomes a
//! `String` or a `Secret`. As for a key, one line break at the very end of
//! a paste (a line copied with its end) is dropped, and a paste with a line
//! break anywhere else, any other control character, or more than the
//! field holds is refused whole, with a sentence, and nothing of it is
//! kept.
//!
//! **Unlike a key, type-ahead is dropped.** A password must be typed after
//! the person has seen what it approves. Both the terminal's prompt and the
//! application's dialog hold the dialogs' rule (`app::ready`): the field is
//! drawn, and takes keys, no sooner than 300 ms after the first frame and a
//! quiet second after the last key or paste it did not take, which it
//! drops; so typing that runs on into it, and keys still on their way when
//! it was drawn, never reach it. At a terminal, once the first frame is
//! written, everything that has arrived is also read and thrown away, and
//! so is an event whose first byte was read by then (an `Esc` held for what
//! might follow it, a paste still open). It is read and not flushed
//! (`tcflush`, `FlushConsoleInputBuffer`): the terminal's replies to the
//! queries it was asked when it was entered share the input, and a flush
//! could cut one in two and leave its tail to be typed. `Ctrl+C` answers
//! with nothing whenever it comes after the first frame: a way out is never
//! held back.
//!
//! **Closed to other programs, on Unix.** While the prompt or the dialog is
//! up, the terminal's exclusive mode is on (`TIOCEXCL`,
//! `cineris_term::Terminal::set_exclusive`): a program that was not
//! reading the terminal cannot open it to start. What it does not stop is
//! in 09, "The approvals password".
//!
//! **Nothing of it is kept anywhere else:** the input box, its history,
//! the draft, the scrollback and the log never see a key typed at it, and
//! the field, the first typing of a password chosen and the [`Password`]
//! handed on are written over with zeros when they are emptied, taken or
//! dropped (`Esc` and `Ctrl+C` included). A paste's own string is written
//! over once taken or dropped (`cineris_base::secret_buf::wipe_string`);
//! the copies that remain are in 08, "Credentials".

use std::io;
use std::time::{Duration, Instant};

use cineris_base::secret_buf::Refused;
use cineris_base::{Password, SecretBuf};
use cineris_term::{Device, Event, Frame, Inline, Key, KeyCode, Line, Mods, Terminal};

use super::{KeyEdit, hidden_box, hidden_count, sentence, without_line_end, wrapped};
use crate::app::ready::Readiness;
use crate::theme::Token;
use crate::view::{Hint, HintRow, Look, hint_row, one_line};

/// What a password is asked for: what its sentence says, what its keys
/// say, and whether it is typed once or chosen and typed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// To use the approvals of the session being opened: "type your
    /// approvals password to use this session's approvals". `Esc` goes on
    /// without them.
    Unlock,
    /// To approve what the caller's lines name, exactly: "type your
    /// approvals password to approve:", then the lines.
    Sign,
    /// A new password, typed twice: at set-up, and at a reset, whose caller
    /// passes a warning that every approval will be dropped. A mismatch, or
    /// fewer than [`Password::LEAST`] characters, says so and asks again.
    Choose,
}

/// What a password prompt asks: its purpose, the caller's lines under its
/// sentence, and a warning above the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordAsk {
    /// What it is for.
    pub purpose: Purpose,
    /// The lines under the purpose's sentence, each drawn whole (wrapped,
    /// never cut), four columns in: for [`Purpose::Sign`], what is approved,
    /// exactly. Text from outside is made safe as every view makes it
    /// (`cineris_term::sanitize`, a line break or a tab a space); a name
    /// that may hold a character that shows as nothing is the caller's to
    /// pass through `cineris_base::visible` first.
    pub lines: Vec<String>,
    /// A line in the warning's colour above the field: that the last
    /// password typed was not the one chosen, or, at a reset, that every
    /// approval will be dropped.
    pub warning: Option<String>,
}

impl PasswordAsk {
    /// A prompt for `purpose` with `lines` and no warning.
    #[must_use]
    pub fn new(purpose: Purpose, lines: Vec<String>) -> Self {
        Self {
            purpose,
            lines,
            warning: None,
        }
    }

    /// The same, with `warning` above the field.
    #[must_use]
    pub fn warning(mut self, warning: impl Into<String>) -> Self {
        self.warning = Some(warning.into());
        self
    }
}

/// The password being typed. See the [module documentation](self).
#[derive(Debug)]
pub struct PasswordInput {
    buffer: SecretBuf,
    refusal: Option<&'static str>,
}

impl Default for PasswordInput {
    fn default() -> Self {
        Self::new()
    }
}

impl PasswordInput {
    /// The most bytes a password may have here: 1 KiB, far more than any
    /// passphrase.
    pub const CAPACITY: usize = 1024;

    /// An empty input of [`CAPACITY`](Self::CAPACITY) bytes.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::CAPACITY)
    }

    /// An empty input of `capacity` bytes, for tests of its limit.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: SecretBuf::with_capacity(capacity),
            refusal: None,
        }
    }

    /// A key pressed: what types a character into the prompt's editor
    /// types it here (`crate::keys::typed`), with no copy of its own.
    pub fn key(&mut self, key: Key) -> KeyEdit {
        let ctrl = key.mods.contains(Mods::CTRL);
        let alt = key.mods.contains(Mods::ALT) || key.mods.contains(Mods::SUPER);
        let changed = match key.code {
            KeyCode::Enter if key.mods.is_empty() => return KeyEdit::Submit,
            KeyCode::Esc => return KeyEdit::Cancel,
            KeyCode::Backspace if !ctrl && !alt => self.buffer.pop(),
            KeyCode::Char('u') if ctrl && !alt => {
                let had = !self.buffer.is_empty();
                self.buffer.clear();
                had
            }
            _ => match crate::keys::typed(key) {
                Some(character) => {
                    let pushed = self.buffer.push(character);
                    self.kept(pushed)
                }
                None => return KeyEdit::Nothing,
            },
        };
        if changed {
            self.refusal = None;
            KeyEdit::Changed
        } else if self.refusal.is_some() {
            KeyEdit::Refused
        } else {
            KeyEdit::Nothing
        }
    }

    /// A paste: kept whole, untrimmed, or refused whole. One line break at
    /// its very end is not the password's and is dropped; one anywhere
    /// else refuses it.
    pub fn paste(&mut self, text: &str) -> KeyEdit {
        let pushed = self.buffer.push_str(without_line_end(text));
        if self.kept(pushed) {
            self.refusal = None;
            KeyEdit::Changed
        } else {
            KeyEdit::Refused
        }
    }

    /// Whether what was pushed was kept; if not, says why until the next
    /// change.
    fn kept(&mut self, pushed: Result<(), Refused>) -> bool {
        let why = match pushed {
            Ok(()) => return true,
            Err(Refused::LineBreak) => {
                "That paste holds a line break, and a password is one line: nothing of it was \
                 kept."
            }
            Err(Refused::Control) => {
                "That paste holds a control character, such as a tab or an escape, which a \
                 password cannot hold: nothing of it was kept."
            }
            Err(Refused::TooLong) => {
                "That is longer than a password can be here: nothing of it was kept."
            }
        };
        self.refusal = Some(why);
        false
    }

    /// How many characters it holds.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.buffer.chars()
    }

    /// Why the last paste or key was refused, until the next change.
    #[must_use]
    pub fn refusal(&self) -> Option<&'static str> {
        self.refusal
    }

    /// The password typed, untrimmed; the input is left empty, its buffer a
    /// new one, so that the password's bytes are never copied. `None` when
    /// nothing was typed.
    pub fn take(&mut self) -> Option<Password> {
        self.take_buffer().map(Password::new)
    }

    /// The buffer itself, as [`take`](Self::take) takes it.
    fn take_buffer(&mut self) -> Option<SecretBuf> {
        self.refusal = None;
        if self.buffer.is_empty() {
            return None;
        }
        let capacity = self.buffer.capacity();
        Some(std::mem::replace(
            &mut self.buffer,
            SecretBuf::with_capacity(capacity),
        ))
    }

    /// Empties it, writing over what it held.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.refusal = None;
    }

    /// Whether every byte of its buffer is zero, for tests.
    #[must_use]
    pub fn wiped(&self) -> bool {
        self.buffer.all_zero()
    }
}

/// What a key or a paste did to a [`PasswordPrompt`].
#[derive(Debug)]
pub enum Prompted {
    /// What it shows changed: draw it again.
    Changed,
    /// Nothing.
    Nothing,
    /// It is answered: the password, or `None` for `Esc` or `Ctrl+C`.
    /// Everything it held is wiped.
    Done(Option<Password>),
}

/// What [`PasswordPrompt::rows`] drew.
#[derive(Debug)]
pub struct PromptRows {
    /// The rows, at most the height asked for.
    pub lines: Vec<Line>,
    /// Whether the field was drawn ready to type into: it was shown, and
    /// every row fitted. Keys are taken only after a frame that drew it.
    pub field: bool,
}

/// A password asked of the person: the sentence its [`Purpose`] says, the
/// caller's lines, the field, and for a password chosen the second typing.
/// See the [module documentation](self).
#[derive(Debug)]
pub struct PasswordPrompt {
    ask: PasswordAsk,
    input: PasswordInput,
    /// A password being chosen, as typed the first time, while it is typed
    /// again.
    first: Option<SecretBuf>,
    /// Why the last `Enter` took nothing, until the next change.
    said: Option<String>,
}

impl PasswordPrompt {
    /// A prompt asking `ask`, its field of [`PasswordInput::CAPACITY`].
    #[must_use]
    pub fn new(ask: PasswordAsk) -> Self {
        Self::with_input(ask, PasswordInput::new())
    }

    /// A prompt asking `ask` with `input`, for tests of its limit.
    #[must_use]
    pub fn with_input(ask: PasswordAsk, input: PasswordInput) -> Self {
        Self {
            ask,
            input,
            first: None,
            said: None,
        }
    }

    /// What it asks.
    #[must_use]
    pub fn ask(&self) -> &PasswordAsk {
        &self.ask
    }

    /// Whether a password being chosen is being typed the second time.
    #[must_use]
    pub fn again(&self) -> bool {
        self.first.is_some()
    }

    /// How many characters the field holds.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.input.chars()
    }

    /// A key pressed while the prompt takes keys. `Ctrl+C` and `Esc` answer
    /// it with nothing.
    pub fn key(&mut self, key: Key) -> Prompted {
        if is_ctrl_c(key) {
            return self.cancel();
        }
        match self.input.key(key) {
            KeyEdit::Submit => self.submit(),
            KeyEdit::Cancel => self.cancel(),
            KeyEdit::Changed | KeyEdit::Refused => {
                self.said = None;
                Prompted::Changed
            }
            KeyEdit::Nothing => Prompted::Nothing,
        }
    }

    /// A key held down, which the terminal marks as a repeat: `Backspace`
    /// and typing go on; `Enter`, `Esc`, `Ctrl+C` and the rest do nothing.
    pub fn repeat(&mut self, key: Key) -> Prompted {
        if is_ctrl_c(key) || matches!(key.code, KeyCode::Enter | KeyCode::Esc) {
            return Prompted::Nothing;
        }
        self.key(key)
    }

    /// A paste while the prompt takes keys.
    pub fn paste(&mut self, text: &str) -> Prompted {
        self.said = None;
        match self.input.paste(text) {
            KeyEdit::Nothing => Prompted::Nothing,
            _ => Prompted::Changed,
        }
    }

    /// Answered with nothing: everything it holds is wiped.
    pub fn cancel(&mut self) -> Prompted {
        self.input.clear();
        self.first = None;
        self.said = None;
        Prompted::Done(None)
    }

    /// `Enter`: the password, or, for one chosen, the first typing kept or
    /// the second compared with it.
    fn submit(&mut self) -> Prompted {
        let Some(typed) = self.input.take_buffer() else {
            self.said =
                Some("Nothing was typed: type the password, or press esc to go back.".to_owned());
            return Prompted::Changed;
        };
        if self.ask.purpose != Purpose::Choose {
            self.said = None;
            return Prompted::Done(Some(Password::new(typed)));
        }
        match self.first.take() {
            None if typed.chars() < Password::LEAST => {
                drop(typed);
                self.said = Some(format!(
                    "A password has at least {} characters, and that one has fewer: choose a \
                     longer one. Anyone with a copy of the approvals can test guesses at it \
                     offline.",
                    Password::LEAST
                ));
                Prompted::Changed
            }
            None => {
                self.first = Some(typed);
                self.said = None;
                Prompted::Changed
            }
            Some(first) if first.same_as(&typed) => {
                drop(typed);
                self.said = None;
                Prompted::Done(Some(Password::new(first)))
            }
            Some(first) => {
                drop((first, typed));
                self.said = Some(
                    "The two were not the same: choose the password again, and type it twice."
                        .to_owned(),
                );
                Prompted::Changed
            }
        }
    }

    /// Whether the field is wiped and no first typing is held: what it
    /// leaves once answered. For tests.
    #[must_use]
    pub fn wiped(&self) -> bool {
        self.input.wiped() && self.first.is_none()
    }

    /// The keys under it, as the hint row names them; none until its field
    /// is drawn ready to type into.
    #[must_use]
    pub fn keys(&self, field: bool) -> Vec<String> {
        if !field {
            return Vec::new();
        }
        let (enter, esc) = match (self.ask.purpose, self.again()) {
            (Purpose::Unlock, _) => ("enter opens them", "esc goes on without them"),
            (Purpose::Sign, _) => ("enter approves", "esc does not approve"),
            (Purpose::Choose, false) => ("enter goes on", "esc keeps nothing"),
            (Purpose::Choose, true) => ("enter keeps it", "esc keeps nothing"),
        };
        vec![
            enter.to_owned(),
            "ctrl+u clears it".to_owned(),
            esc.to_owned(),
        ]
    }

    /// Its rows, `width` columns wide and at most `height` rows: the
    /// purpose's sentence, the caller's lines, the warning, and the field,
    /// which says how many characters it holds and none of them. The field
    /// is drawn ready to type into only when `shown` and every row fits;
    /// in a window too small for them all, a sentence says so instead, and
    /// no key is to be taken.
    #[must_use]
    pub fn rows(&self, width: usize, height: usize, shown: bool, look: &Look) -> PromptRows {
        let says = match (self.ask.purpose, self.again()) {
            (Purpose::Unlock, _) => {
                "Type your approvals password to use this session's approvals.".to_owned()
            }
            (Purpose::Sign, _) => "Type your approvals password to approve:".to_owned(),
            (Purpose::Choose, false) => format!(
                "Choose a password for your approvals, of at least {} characters: anyone with a \
                 copy of the approvals can test guesses at it offline.",
                Password::LEAST
            ),
            (Purpose::Choose, true) => "Type the same password again.".to_owned(),
        };
        let mut lines = sentence(&says, width, look);
        for line in &self.ask.lines {
            lines.extend(wrapped(&one_line(line), 4, Token::Text, width, look));
        }
        if let Some(warning) = &self.ask.warning {
            lines.extend(wrapped(&one_line(warning), 2, Token::Warning, width, look));
        }
        lines.push(Line::new());
        let field = if shown {
            hidden_count(self.input.chars())
        } else {
            look.glyphs.ellipsis.to_owned()
        };
        lines.extend(hidden_box(&field, width, look));
        if shown && let Some(why) = self.input.refusal().or(self.said.as_deref()) {
            lines.extend(wrapped(why, 2, Token::Warning, width, look));
        }
        if lines.len() > height {
            let mut small = sentence(
                "The window is too small to show all of what this asks: make it taller, or \
                 press ctrl+c to go back.",
                width,
                look,
            );
            small.truncate(height.max(1));
            return PromptRows {
                lines: small,
                field: false,
            };
        }
        PromptRows {
            lines,
            field: shown,
        }
    }
}

/// Whether `key` is `Ctrl+C`, which answers the prompt with nothing
/// whenever it comes.
pub(crate) fn is_ctrl_c(key: Key) -> bool {
    key.code == KeyCode::Char('c') && key.mods == Mods::CTRL
}

/// The longest the prompt at a terminal spends reading and throwing away
/// what arrived before its first frame: input that keeps coming for longer
/// is no one's type-ahead, and what comes after it counts.
const DROP_MOST: Duration = Duration::from_millis(100);

/// Reads a password at the terminal: enters the terminal inline with
/// `overrides`, asks `ask` ([`read_password_on`]), and puts the terminal
/// back whatever happens. `None` when the person pressed `Esc` or
/// `Ctrl+C`, or a signal came (`cineris_term::signalled` then says which).
///
/// # Errors
///
/// When the terminal cannot be entered, read or written.
pub fn read_password(
    overrides: cineris_term::Overrides,
    ask: &PasswordAsk,
) -> io::Result<Option<Password>> {
    let mut terminal = PasswordTerminal::enter(overrides)?;
    let read = terminal.read(ask);
    let left = terminal.leave();
    let read = read?;
    left?;
    Ok(read)
}

/// The terminal taken for a password, before anything slow is done, as
/// [`KeyTerminal`](super::KeyTerminal) is taken for a key: from
/// [`enter`](Self::enter) on the terminal echoes nothing, signals are
/// handed over, and dropping it, or [`leave`](Self::leave), puts the
/// terminal back. What is typed before a prompt's first frame is thrown
/// away ([`read_password_on`]). It can ask more than once
/// ([`read`](Self::read)): again, with a warning, after a password that was
/// not the one chosen.
pub struct PasswordTerminal(Terminal);

impl std::fmt::Debug for PasswordTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordTerminal").finish_non_exhaustive()
    }
}

impl PasswordTerminal {
    /// Takes the terminal on standard input and output, with `overrides`
    /// (`[terminal]`), and hands the signals that end the program to it.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be entered.
    pub fn enter(overrides: cineris_term::Overrides) -> io::Result<Self> {
        let mut terminal = Terminal::enter(overrides)?;
        terminal.hand_signals_over();
        Ok(Self(terminal))
    }

    /// As [`enter`](Self::enter), with the signals that end the program
    /// left to their own handling: for a program that watches them itself
    /// once the terminal is left (a headless run, since the signed
    /// approvals), whose handlers the terminal's would otherwise outlive.
    /// A signal while it is taken puts the terminal back and ends the
    /// program as the signal's own end does; `Ctrl+C`, a key in raw mode,
    /// answers the prompt with nothing.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be entered.
    pub fn enter_leaving_signals(overrides: cineris_term::Overrides) -> io::Result<Self> {
        Ok(Self(Terminal::enter(overrides)?))
    }

    /// Asks `ask` ([`read_password_on`]). The rows are gone once it ends;
    /// the terminal stays taken. `None` when the person pressed `Esc` or
    /// `Ctrl+C`, or a signal came: [`ending`](Self::ending) says which.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read or written.
    pub fn read(&mut self, ask: &PasswordAsk) -> io::Result<Option<Password>> {
        read_password_on(&mut self.0, ask)
    }

    /// Asks a yes or no while the terminal is taken, for a question that
    /// comes before the password's (`cineris permissions trust`'s, of a
    /// project's extensions): `question` and `[y/N]` written, a line typed,
    /// echoed (it is no secret), and `Enter`. `Some(true)` for `y` or
    /// `yes`, `Some(false)` for anything else, an empty line and `Esc`
    /// included; `None` for `Ctrl+C` or a signal. What was typed before the
    /// question was written does not answer it.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read or written.
    pub fn confirm(&mut self, question: &str) -> io::Result<Option<bool>> {
        confirm_on(&mut self.0, question)
    }

    /// Writes `text` while the terminal is taken, each line break as the
    /// terminal needs it in raw mode: what a subcommand shows before it
    /// asks. Its control characters are the caller's to make visible.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written.
    pub fn show(&mut self, text: &str) -> io::Result<()> {
        use cineris_term::Sink as _;
        self.0
            .write_frame(text.replace("\r\n", "\n").replace('\n', "\r\n").as_bytes())
    }

    /// The exit code a signal handed over asks the program to end with,
    /// once one has come.
    #[must_use]
    pub fn ending(&self) -> Option<u8> {
        self.0.ending()
    }

    /// Puts the terminal back.
    ///
    /// # Errors
    ///
    /// When the end sequences cannot be written. The modes are put back
    /// whatever happens.
    pub fn leave(self) -> io::Result<()> {
        self.0.leave()
    }
}

/// [`read_password`] on a terminal already entered: for tests, on a device
/// of their own.
///
/// - **Closed to other programs** while its rows are up: the terminal's
///   exclusive mode ([`Terminal::set_exclusive`]) is on from before its
///   first frame until its rows are erased, whatever way it ends. Where the
///   system refuses it, the password is asked all the same.
/// - **Type-ahead is dropped.** Once its first frame is written, everything
///   that has arrived is read and thrown away, for at most [`DROP_MOST`],
///   and then every key and paste whose first byte came by then.
/// - **The dialogs' rule** (`app::ready`): its field is drawn, and takes
///   keys, only 300 ms after its first frame and a quiet second after the
///   last input it did not take (what was thrown away counts), and only
///   while every row fits the window. Until then a key or a paste is
///   dropped and holds it; `Ctrl+C` answers it with nothing whenever it
///   comes after the first frame.
/// - A paste's text is written over once taken or dropped.
/// - It erases its rows when it ends. After the program was stopped and
///   continued, it draws them again where the cursor is now, and starts its
///   rule again.
///
/// # Errors
///
/// When the terminal cannot be read or written.
pub fn read_password_on<D: Device>(
    terminal: &mut Terminal<D>,
    ask: &PasswordAsk,
) -> io::Result<Option<Password>> {
    // Refused, it is asked all the same; the terminal's end tries again to
    // turn it off.
    let _ = terminal.set_exclusive(true);
    let read = ask_on(terminal, ask);
    let _ = terminal.set_exclusive(false);
    read
}

/// [`read_password_on`] once the terminal is closed to others.
fn ask_on<D: Device>(
    terminal: &mut Terminal<D>,
    ask: &PasswordAsk,
) -> io::Result<Option<Password>> {
    let (cols, rows) = terminal.size()?;
    let mut presenter = Inline::new(cols, rows);
    let mut height = usize::from(rows);
    let mut prompt = PasswordPrompt::new(ask.clone());
    let mut ready = Readiness::default();
    // Input whose first byte came by this time is type-ahead: set once the
    // first frame is written and what had arrived is thrown away.
    let mut fence: Option<Instant> = None;
    let result = loop {
        if terminal.ending().is_some() {
            break None;
        }
        let caps = *terminal.caps();
        let look = Look::new(
            crate::theme::Theme::for_terminal(&caps),
            crate::glyphs::Glyphs::for_terminal(&caps),
        );
        let width = usize::from(presenter.width());
        let shown = ready.ready(terminal.now());
        let drawn = prompt.rows(width, height.saturating_sub(1), shown, &look);
        let mut lines = drawn.lines;
        lines.push(hint_row(
            &HintRow {
                left: Hint::Keys(prompt.keys(drawn.field)),
                model: String::new(),
                effort: None,
                speed: None,
                context: None,
                budget: None,
                limit: None,
                access: None,
                extras: Vec::new(),
            },
            width,
            &look,
        ));
        let cursor = cineris_term::Cursor {
            row: lines.len().saturating_sub(1),
            col: 0,
        };
        presenter.draw(
            &Frame {
                commit: &[],
                live: &lines,
                cursor: Some(cursor),
            },
            &caps,
            terminal,
        )?;
        ready.drawn(terminal.now());
        ready.set_answers_drawn(drawn.field);
        if fence.is_none() {
            let (end, dropped) = drop_typed_ahead(terminal, &mut presenter, &mut height)?;
            if dropped {
                ready.typed(end);
            }
            fence = Some(end);
            // A resize or the replies read meanwhile: drawn again.
            continue;
        }
        // Woken when the field is due, unless it is shown, or is kept back
        // only by a window too small for it.
        let timeout = if shown {
            None
        } else {
            ready
                .ready_at()
                .map(|at| at.saturating_duration_since(terminal.now()))
        };
        let Some(event) = terminal.next(timeout)? else {
            continue;
        };
        let at = terminal.now();
        let came = terminal.came().min(at);
        let typed_ahead = fence.is_some_and(|fence| came <= fence);
        let takes = !typed_ahead && ready.takes_keys(came);
        let prompted = match &event {
            // Typed or pasted before the prompt was there to read, `Ctrl+C`
            // included: nothing of it counts.
            Event::Key(_) | Event::Repeat(_) | Event::Paste(_) if typed_ahead => Prompted::Nothing,
            Event::Key(key) if is_ctrl_c(*key) => prompt.cancel(),
            Event::Key(key) if takes => prompt.key(*key),
            Event::Repeat(key) if takes => prompt.repeat(*key),
            Event::Paste(text) if takes => prompt.paste(text),
            // Before the field took keys: dropped, and it waits for the
            // typing to stop.
            Event::Key(_) | Event::Repeat(_) | Event::Paste(_) => {
                ready.typed(at);
                Prompted::Nothing
            }
            Event::Resize { cols, rows } => {
                presenter.resize(*cols, *rows);
                height = usize::from(*rows);
                Prompted::Nothing
            }
            Event::Resumed => {
                presenter.restart();
                ready.hide();
                Prompted::Nothing
            }
            _ => Prompted::Nothing,
        };
        forget(event);
        if let Prompted::Done(answer) = prompted {
            break answer;
        }
    };
    let _ = prompt.cancel();
    let caps = *terminal.caps();
    presenter.finish(&caps, terminal)?;
    Ok(result)
}

/// An event read and done with: a paste's text, taken into the field or
/// thrown away, is written over before it is freed. Every paste the
/// password's prompt, the key's prompt and the application's loop read
/// goes through here, which a test counts ([`PASTES_WIPED`]).
pub(crate) fn forget(event: Event) {
    if let Event::Paste(text) = event {
        #[cfg(test)]
        PASTES_WIPED.with(|wiped| wiped.set(wiped.get() + 1));
        cineris_base::secret_buf::wipe_string(text);
    }
}

#[cfg(test)]
thread_local! {
    /// How many pastes [`forget`] wrote over on this thread: for the tests
    /// of each place a paste is read, which a paste left to drop unwiped
    /// fails.
    pub(crate) static PASTES_WIPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`PasswordTerminal::confirm`] on a terminal already entered: for tests,
/// on a device of their own.
///
/// # Errors
///
/// When the terminal cannot be read or written.
pub fn confirm_on<D: Device>(
    terminal: &mut Terminal<D>,
    question: &str,
) -> io::Result<Option<bool>> {
    use cineris_term::Sink as _;
    // What was typed before the question is written does not answer it:
    // thrown away first, so that nothing typed once it is shown is.
    while let Some(event) = terminal.next(Some(Duration::ZERO))? {
        forget(event);
    }
    let question = cineris_term::sanitize(question);
    terminal.write_frame(format!("{question} [y/N] ").as_bytes())?;
    let mut line = String::new();
    let yes = |line: &str| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
    let answer = loop {
        if terminal.ending().is_some() {
            break None;
        }
        let Some(event) = terminal.next(None)? else {
            continue;
        };
        // What the event answers, once it is done with: `Some(None)` for no
        // answer at all.
        let mut answered: Option<Option<bool>> = None;
        let mut echo = String::new();
        match &event {
            Event::Key(key) if is_ctrl_c(*key) => answered = Some(None),
            Event::Key(Key {
                code: KeyCode::Enter,
                ..
            }) => answered = Some(Some(yes(&line))),
            Event::Key(Key {
                code: KeyCode::Esc, ..
            }) => answered = Some(Some(false)),
            Event::Key(Key {
                code: KeyCode::Backspace,
                ..
            }) => {
                if line.pop().is_some() {
                    echo.push_str("\x08 \x08");
                }
            }
            Event::Key(Key {
                code: KeyCode::Char(typed),
                mods,
            }) if *mods == Mods::NONE || *mods == Mods::SHIFT => {
                if !typed.is_control() && line.chars().count() < CONFIRM_MOST {
                    line.push(*typed);
                    echo.push(*typed);
                }
            }
            // Typed quickly, a line can come as one burst a terminal with
            // no bracketed paste gives as a paste: its line break answers.
            Event::Paste(text) => {
                for typed in text.chars() {
                    if typed == '\r' || typed == '\n' {
                        answered = Some(Some(yes(&line)));
                        break;
                    }
                    if !typed.is_control() && line.chars().count() < CONFIRM_MOST {
                        line.push(typed);
                        echo.push(typed);
                    }
                }
            }
            _ => {}
        }
        forget(event);
        if !echo.is_empty() {
            terminal.write_frame(echo.as_bytes())?;
        }
        if let Some(answer) = answered {
            break answer;
        }
    };
    terminal.write_frame(b"\r\n")?;
    Ok(answer)
}

/// The most characters a yes or no is typed with.
const CONFIRM_MOST: usize = 16;

/// Reads, and throws away, every key and paste that has arrived: what was
/// typed or pasted before the prompt's first frame. A resize, a resumption
/// and the replies to the terminal's queries are taken as they come. Gives
/// the time it ended, an event whose first byte came by then being
/// type-ahead too, and whether it threw anything away.
fn drop_typed_ahead<D: Device>(
    terminal: &mut Terminal<D>,
    presenter: &mut Inline,
    height: &mut usize,
) -> io::Result<(Instant, bool)> {
    let began = terminal.now();
    let mut dropped = false;
    while terminal.now().saturating_duration_since(began) < DROP_MOST {
        let Some(event) = terminal.next(Some(Duration::ZERO))? else {
            break;
        };
        match event {
            Event::Resize { cols, rows } => {
                presenter.resize(cols, rows);
                *height = usize::from(rows);
            }
            Event::Resumed => presenter.restart(),
            // Typed or pasted before the prompt was there to read.
            Event::Key(_) | Event::Repeat(_) | Event::Paste(_) => {
                dropped = true;
                forget(event);
            }
            _ => {}
        }
    }
    Ok((terminal.now(), dropped))
}

#[cfg(test)]
mod tests;
