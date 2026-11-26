//! A key read at the terminal without echo (since step 25;
//! `docs/architecture/08-configuration.md`, "Credentials", and
//! `docs/architecture/09-tui.md`, "The first run"): the first run's key and
//! `cineris auth login`'s at a terminal ([`read_key`]).
//!
//! **It is not the prompt's editor.** [`KeyInput`] has no undo, no kill
//! slot, no history, no completion and no hand-off: it answers typing, a
//! paste, `Backspace`, `Ctrl+U` (which empties it), `Enter` and `Esc`, and
//! nothing else does anything there. What it holds is in a
//! [`SecretBuf`], which never moves and is written over with zeros, and
//! **it is never drawn:** its row says only how many characters it holds
//! ([`key_box`]). A typed character goes into the buffer with no copy of
//! its own. One line break at the very end of a paste (`\n`, `\r\n` or
//! `\r`: a line copied with its end) is dropped; a paste that holds a line
//! break anywhere else, any other control character, or more than the
//! buffer's capacity is refused whole with a sentence, and nothing of it is
//! kept.
//!
//! The key is handed on as a [`Secret`], trimmed as `auth login` trims a
//! key from standard input. Where it is stored in Windows Credential
//! Manager, a key that is not printable ASCII is refused before the store
//! is asked (08, "Windows Credential Manager").

use std::io;

use cineris_base::secret_buf::Refused;
use cineris_base::{Secret, SecretBuf};
use cineris_term::{Device, Event, Frame, Inline, Key, KeyCode, Line, Mods, Span, Terminal};

use crate::layout::wrap;
use crate::rounded::RoundedBox;
use crate::theme::Token;
use crate::view::{Hint, HintRow, Look, hint_row};

/// What a key or a paste did to a [`KeyInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdit {
    /// What it holds changed.
    Changed,
    /// A paste was refused whole: [`KeyInput::refusal`] says why.
    Refused,
    /// `Enter`: the key is to be stored.
    Submit,
    /// `Esc`: back, nothing kept.
    Cancel,
    /// The key does nothing here.
    Nothing,
}

/// The key being typed. See the [module documentation](self).
#[derive(Debug, Default)]
pub struct KeyInput {
    buffer: SecretBuf,
    refusal: Option<&'static str>,
}

impl KeyInput {
    /// An empty input of a key's capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty input of `capacity` bytes, for tests of its limit.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: SecretBuf::with_capacity(capacity),
            refusal: None,
        }
    }

    /// A key pressed.
    pub fn key(&mut self, key: Key) -> KeyEdit {
        let ctrl = key.mods.contains(Mods::CTRL);
        let alt = key.mods.contains(Mods::ALT) || key.mods.contains(Mods::SUPER);
        let edit = match key.code {
            KeyCode::Enter if key.mods.is_empty() => return KeyEdit::Submit,
            KeyCode::Esc => return KeyEdit::Cancel,
            KeyCode::Backspace if !ctrl && !alt => self.buffer.pop(),
            KeyCode::Char('u') if ctrl && !alt => {
                let had = !self.buffer.is_empty();
                self.buffer.clear();
                had
            }
            KeyCode::Char(character) if !ctrl && !alt && !character.is_control() => {
                let pushed = self.buffer.push(character);
                self.kept(pushed)
            }
            _ => return KeyEdit::Nothing,
        };
        if edit {
            self.refusal = None;
            KeyEdit::Changed
        } else if self.refusal.is_some() {
            KeyEdit::Refused
        } else {
            KeyEdit::Nothing
        }
    }

    /// A paste: kept whole, or refused whole. One line break at its very
    /// end, which a line copied with its end brings, is not the key's and
    /// is dropped; one anywhere else refuses it.
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
                "That paste holds a line break, and a key is one line: nothing of it was kept."
            }
            Err(Refused::Control) => {
                "That paste holds a control character, such as a tab or an escape, which no key \
                 has: nothing of it was kept."
            }
            Err(Refused::TooLong) => "That is longer than a key can be: nothing of it was kept.",
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

    /// The key, trimmed, for the store; the input is emptied either way.
    /// `ascii_only` where the store is Windows Credential Manager.
    ///
    /// # Errors
    ///
    /// A sentence: nothing was typed, or a key that is not printable ASCII
    /// for a store that keeps only that.
    pub fn take(&mut self, ascii_only: bool) -> Result<Secret, &'static str> {
        let taken = if ascii_only && !self.buffer.is_printable_ascii() {
            Err(
                "A key is letters, digits and punctuation: that one holds other characters, \
                 and was not kept.",
            )
        } else {
            self.buffer
                .to_secret()
                .ok_or("Nothing was typed: paste or type the key, or press esc.")
        };
        self.buffer.clear();
        if let Err(why) = taken {
            self.refusal = Some(why);
        }
        taken
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

/// `text` without the one line break at its very end, if it has one: `\n`,
/// `\r\n` or `\r`.
fn without_line_end(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(['\n', '\r']))
        .unwrap_or(text)
}

/// The key's row: a rounded box holding `◆` and how many characters the
/// key has, never one of them, `width` columns wide.
#[must_use]
pub fn key_box(input: &KeyInput, width: usize, look: &Look) -> Vec<Line> {
    let frame = RoundedBox::new(width, look.glyphs, look.theme.style(Token::Border));
    let count = match input.chars() {
        1 => "1 character, hidden".to_owned(),
        many => format!("{many} characters, hidden"),
    };
    let row = Line::from(Span::new(
        format!("{} ", look.glyphs.user),
        look.theme.style(Token::User),
    ))
    .with(Span::new(count, look.theme.style(Token::Muted)));
    frame.draw(&[(row, cineris_term::Style::PLAIN)], None)
}

/// The keys under the key's row.
pub(crate) const KEYS: [&str; 3] = ["enter stores it", "ctrl+u clears it", "esc goes back"];

/// What `cineris auth login` shows above the key's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asking {
    /// The provider.
    pub provider: String,
    /// Where the key is kept, as a sentence's end: `in Windows Credential
    /// Manager, for this machine only`.
    pub kept: String,
    /// Whether the store keeps only printable ASCII.
    pub ascii_only: bool,
}

/// Reads a key at the terminal for `cineris auth login`: enters the
/// terminal inline with `overrides`, draws the sentence, the key's row and
/// its keys, and puts the terminal back whatever happens. `None` when the
/// person pressed `Esc` or `Ctrl+C`, or a signal came.
///
/// # Errors
///
/// When the terminal cannot be entered, read or written.
pub fn read_key(overrides: cineris_term::Overrides, asking: &Asking) -> io::Result<Option<Secret>> {
    KeyTerminal::enter(overrides)?.read(asking)
}

/// The terminal taken for a key's reading, before anything else is done
/// (since the step 25 fixes): from [`enter`](Self::enter) on the terminal
/// echoes nothing, and what is typed while the program goes on starting
/// waits for the key's row, which takes it as typing. `cineris auth login`
/// takes it before its core starts, which is long enough for a key pasted
/// straight after the command to be echoed, onto the screen and into the
/// scrollback, by a terminal still in its ordinary mode. Signals are
/// handed over as [`read_key`] hands them, and dropping it puts the
/// terminal back.
pub struct KeyTerminal(Terminal);

impl std::fmt::Debug for KeyTerminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyTerminal").finish_non_exhaustive()
    }
}

impl KeyTerminal {
    /// Takes the terminal, as [`read_key`] does.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be entered.
    pub fn enter(overrides: cineris_term::Overrides) -> io::Result<Self> {
        let mut terminal = Terminal::enter(overrides)?;
        terminal.hand_signals_over();
        Ok(Self(terminal))
    }

    /// Reads the key ([`read_key_on`]) and puts the terminal back, whatever
    /// happens. `None` when the person pressed `Esc` or `Ctrl+C`, or a
    /// signal came.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be read or written.
    pub fn read(self, asking: &Asking) -> io::Result<Option<Secret>> {
        let Self(mut terminal) = self;
        let read = read_key_on(&mut terminal, asking);
        let left = terminal.leave();
        let read = read?;
        left?;
        Ok(read)
    }
}

/// [`read_key`] on a terminal already entered: for tests, on a device of
/// their own. After the program was stopped and continued, the stop having
/// erased its rows and the shell perhaps written below them, it draws them
/// again where the cursor is now.
///
/// # Errors
///
/// When the terminal cannot be read or written.
pub fn read_key_on<D: Device>(
    terminal: &mut Terminal<D>,
    asking: &Asking,
) -> io::Result<Option<Secret>> {
    let (cols, rows) = terminal.size()?;
    let mut presenter = Inline::new(cols, rows);
    let mut input = KeyInput::new();
    let result = loop {
        if terminal.ending().is_some() {
            break None;
        }
        let caps = *terminal.caps();
        let look = Look::new(
            crate::theme::Theme::for_terminal(&caps),
            crate::glyphs::Glyphs::for_terminal(&caps),
        );
        let lines = frame(asking, &input, usize::from(presenter.width()), &look);
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
        let Some(event) = terminal.next(None)? else {
            continue;
        };
        let edit = match &event {
            Event::Key(key) if key.code == KeyCode::Char('c') && key.mods == Mods::CTRL => {
                KeyEdit::Cancel
            }
            Event::Key(key) => input.key(*key),
            Event::Paste(text) => input.paste(text),
            Event::Resize { cols, rows } => {
                presenter.resize(*cols, *rows);
                KeyEdit::Nothing
            }
            Event::Resumed => {
                presenter.restart();
                KeyEdit::Nothing
            }
            _ => KeyEdit::Nothing,
        };
        match edit {
            KeyEdit::Submit => match input.take(asking.ascii_only) {
                Ok(key) => break Some(key),
                Err(_) => continue,
            },
            KeyEdit::Cancel => break None,
            KeyEdit::Changed | KeyEdit::Refused | KeyEdit::Nothing => {}
        }
    };
    input.clear();
    let caps = *terminal.caps();
    presenter.finish(&caps, terminal)?;
    Ok(result)
}

/// The rows `auth login` draws.
fn frame(asking: &Asking, input: &KeyInput, width: usize, look: &Look) -> Vec<Line> {
    let mut lines = sentence(
        &format!(
            "Paste or type the key for {}. Nothing of it is shown. It is stored {}, and is \
             never written to a file of settings, the log or the prompt history.",
            crate::view::one_line(&asking.provider),
            asking.kept
        ),
        width,
        look,
    );
    lines.push(Line::new());
    lines.extend(key_box(input, width, look));
    if let Some(why) = input.refusal() {
        lines.push(Line::plain("  ").with(Span::new(why, look.theme.style(Token::Warning))));
    }
    lines.push(hint_row(
        &HintRow {
            left: Hint::Keys(KEYS.iter().map(|key| (*key).to_owned()).collect()),
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
        look,
    ));
    lines
}

/// A sentence wrapped two columns in.
pub(crate) fn sentence(text: &str, width: usize, look: &Look) -> Vec<Line> {
    wrap(
        &[Span::new(text.to_owned(), look.theme.style(Token::Text))],
        width.saturating_sub(2).max(1),
    )
    .into_iter()
    .map(|line| {
        let mut row = Line::plain("  ");
        for span in line.spans() {
            row.push(span.clone());
        }
        row
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use cineris_term::Ready;

    use super::*;

    fn char_key(character: char) -> Key {
        Key::plain(KeyCode::Char(character))
    }

    #[test]
    fn typing_backspace_and_ctrl_u_change_what_it_holds_and_nothing_else_does() {
        let mut input = KeyInput::new();
        for character in "sk-1".chars() {
            assert_eq!(input.key(char_key(character)), KeyEdit::Changed);
        }
        assert_eq!(input.chars(), 4);
        assert_eq!(input.key(Key::plain(KeyCode::Backspace)), KeyEdit::Changed);
        assert_eq!(input.chars(), 3);
        for nothing in [
            Key {
                code: KeyCode::Char('g'),
                mods: Mods::CTRL,
            },
            Key {
                code: KeyCode::Char('y'),
                mods: Mods::CTRL,
            },
            Key {
                code: KeyCode::Char('o'),
                mods: Mods::CTRL,
            },
            Key::plain(KeyCode::Up),
            Key::plain(KeyCode::Tab),
        ] {
            assert_eq!(input.key(nothing), KeyEdit::Nothing, "{nothing:?}");
        }
        assert_eq!(input.chars(), 3);
        assert_eq!(
            input.key(Key {
                code: KeyCode::Char('u'),
                mods: Mods::CTRL,
            }),
            KeyEdit::Changed
        );
        assert_eq!(input.chars(), 0);
        assert!(input.wiped());
        assert_eq!(input.key(Key::plain(KeyCode::Enter)), KeyEdit::Submit);
        assert_eq!(input.key(Key::plain(KeyCode::Esc)), KeyEdit::Cancel);
    }

    #[test]
    fn a_paste_with_a_line_break_or_past_the_capacity_is_refused_whole() {
        let mut input = KeyInput::with_capacity(16);
        assert_eq!(input.paste("sk-abc"), KeyEdit::Changed);
        assert_eq!(input.paste("def\nghi"), KeyEdit::Refused);
        assert!(
            input
                .refusal()
                .is_some_and(|why| why.contains("line break"))
        );
        assert_eq!(input.paste("0123456789abcdef"), KeyEdit::Refused);
        assert!(input.refusal().is_some_and(|why| why.contains("longer")));
        assert_eq!(input.chars(), 6, "nothing of either was kept");
        assert_eq!(input.key(char_key('x')), KeyEdit::Changed);
        assert_eq!(input.refusal(), None, "a change clears what was said");
        assert_eq!(input.take(false).unwrap().expose(), "sk-abcx");
        assert!(input.wiped(), "taken, it is written over");
    }

    /// A line copied with its end: the one line break at its very end is
    /// dropped, whichever system's it is, and one anywhere else, or a
    /// second at the end, refuses the paste whole.
    #[test]
    fn a_pasted_line_loses_its_one_line_break_at_the_end() {
        let mut input = KeyInput::new();
        for (pasted, kept) in [
            ("sk-a\n", "sk-a"),
            ("sk-b\r\n", "sk-b"),
            ("sk-c\r", "sk-c"),
            ("  sk-d  \n", "sk-d"),
        ] {
            assert_eq!(input.paste(pasted), KeyEdit::Changed, "{pasted:?}");
            assert_eq!(input.refusal(), None, "{pasted:?}");
            assert_eq!(input.take(false).unwrap().expose(), kept, "{pasted:?}");
        }
        assert_eq!(input.paste("sk-"), KeyEdit::Changed);
        for refused in ["e\n\n", "e\r\n\r\n", "e\n\r", "\ne", "e\nf\n", "e\r\nf"] {
            assert_eq!(input.paste(refused), KeyEdit::Refused, "{refused:?}");
            assert!(
                input
                    .refusal()
                    .is_some_and(|why| why.contains("line break")),
                "{refused:?}"
            );
        }
        assert_eq!(input.chars(), 3, "nothing of what was refused was kept");
        assert_eq!(input.paste("\n"), KeyEdit::Changed, "a line break alone");
        assert_eq!(input.take(false).unwrap().expose(), "sk-");
    }

    /// A tab, an escape, a delete, a C1 control or NUL refuses a paste
    /// whole, with a sentence, on every system: not only where the store
    /// keeps printable ASCII alone.
    #[test]
    fn a_paste_with_a_control_character_is_refused_whole() {
        let mut input = KeyInput::new();
        assert_eq!(input.paste("sk-"), KeyEdit::Changed);
        for refused in [
            "ab\tcd",
            "\u{1b}[2J",
            "ab\u{7f}",
            "\u{9b}31m",
            "a\0b",
            "\t\n",
        ] {
            assert_eq!(input.paste(refused), KeyEdit::Refused, "{refused:?}");
            assert!(
                input
                    .refusal()
                    .is_some_and(|why| why.contains("control character")),
                "{refused:?}: {:?}",
                input.refusal()
            );
        }
        assert_eq!(input.chars(), 3, "nothing of what was refused was kept");
        assert_eq!(input.take(false).unwrap().expose(), "sk-");
    }

    /// A key's 64 KiB: a paste of that much is kept, and one byte more is
    /// refused whole, as is a key typed past it.
    #[test]
    fn a_paste_past_a_keys_64_kib_is_refused_whole() {
        let mut input = KeyInput::new();
        assert_eq!(input.paste(&"k".repeat(64 * 1024 + 1)), KeyEdit::Refused);
        assert!(input.refusal().is_some_and(|why| why.contains("longer")));
        assert_eq!(input.chars(), 0, "nothing of it was kept");
        assert!(input.wiped());
        assert_eq!(input.paste(&"k".repeat(64 * 1024)), KeyEdit::Changed);
        assert_eq!(input.chars(), 64 * 1024);
        assert_eq!(input.key(char_key('x')), KeyEdit::Refused);
        assert_eq!(input.paste("x"), KeyEdit::Refused);
        assert_eq!(input.chars(), 64 * 1024);
        input.clear();
        assert!(input.wiped());
    }

    #[test]
    fn the_key_is_trimmed_and_a_store_of_ascii_refuses_other_text() {
        let mut input = KeyInput::new();
        input.paste("  sk-é1  ");
        assert!(input.take(true).is_err());
        assert!(input.wiped());
        input.paste("  sk-plain  ");
        assert_eq!(input.take(true).unwrap().expose(), "sk-plain");
        input.paste("   ");
        assert!(input.take(false).is_err(), "only spaces is no key");
    }

    #[test]
    fn the_row_says_how_many_characters_and_none_of_them() {
        let look = Look::new(
            crate::theme::Theme::new(cineris_term::ColorDepth::TrueColor, None),
            &crate::glyphs::Glyphs::UNICODE,
        );
        let mut input = KeyInput::new();
        input.paste("sk-ant-SECRETVALUE");
        let shown: String = key_box(&input, 60, &look)
            .iter()
            .map(|line| line.text().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(shown.contains("18 characters, hidden"), "{shown}");
        assert!(
            !shown.contains("SECRET") && !shown.contains("sk-"),
            "{shown}"
        );
        input.clear();
        input.paste("x");
        let one: String = key_box(&input, 60, &look)
            .iter()
            .map(|line| line.text().into_owned())
            .collect();
        assert!(one.contains("1 character, hidden"), "{one}");
    }

    #[test]
    fn caps_are_not_needed_to_draw_it() {
        let caps = crate::testkit::caps();
        let look = Look::new(
            crate::theme::Theme::for_terminal(&caps),
            crate::glyphs::Glyphs::for_terminal(&caps),
        );
        let asking = Asking {
            provider: "anthropic\u{1b}[2J".to_owned(),
            kept: "in a file".to_owned(),
            ascii_only: false,
        };
        let lines = frame(&asking, &KeyInput::new(), 70, &look);
        assert_eq!(crate::testing::problems(&lines, 70), Vec::<String>::new());
    }

    /// A terminal of 80 by 24 for [`read_key_on`]'s tests: it plays back
    /// what was typed, one step a wait, on a clock of its own, and keeps
    /// what was written, with how much of it there was at each wait.
    struct Played {
        steps: VecDeque<(Ready, Vec<u8>)>,
        written: Rc<RefCell<Vec<u8>>>,
        waits: Rc<RefCell<Vec<usize>>>,
        base: Instant,
        elapsed: Duration,
    }

    impl Device for Played {
        fn size(&self) -> io::Result<(u16, u16)> {
            Ok((80, 24))
        }
        fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.written.borrow_mut().extend_from_slice(bytes);
            Ok(())
        }
        fn wait(&mut self, timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
            self.waits.borrow_mut().push(self.written.borrow().len());
            let Some((ready, bytes)) = self.steps.pop_front() else {
                return Err(io::Error::other("the script has ended"));
            };
            // A timeout is the escape key's wait, run out.
            self.elapsed += match ready {
                Ready::Timeout => timeout.unwrap_or(Duration::from_secs(1)),
                _ => Duration::from_millis(1),
            };
            into.extend_from_slice(&bytes);
            Ok(ready)
        }
        fn waker(&self) -> cineris_term::Waker {
            cineris_term::Waker::new(|| {})
        }
        fn suspend(&mut self) {}
        fn leave(&mut self) {}
        fn now(&self) -> Instant {
            self.base + self.elapsed
        }
    }

    /// One step of a [`Played`] terminal: what a wait ends with, and the
    /// bytes it reads.
    type Step = (Ready, &'static [u8]);

    /// What `auth login` read from `steps`, everything written, and how
    /// much had been written at each wait: the screen after step `n` is
    /// what was written before wait `n + 1`.
    fn login(steps: &[Step]) -> (io::Result<Option<Secret>>, Vec<u8>, Vec<usize>) {
        let written = Rc::new(RefCell::new(Vec::new()));
        let waits = Rc::new(RefCell::new(Vec::new()));
        let device = Played {
            steps: steps
                .iter()
                .map(|(ready, bytes)| (*ready, bytes.to_vec()))
                .collect(),
            written: Rc::clone(&written),
            waits: Rc::clone(&waits),
            base: Instant::now(),
            elapsed: Duration::ZERO,
        };
        let mut terminal =
            Terminal::with_device(device, crate::testkit::caps()).expect("a terminal");
        let asking = Asking {
            provider: "anthropic".to_owned(),
            kept: "in a file of its own".to_owned(),
            ascii_only: false,
        };
        let read = read_key_on(&mut terminal, &asking);
        let waits = waits.borrow().clone();
        // Taken before the terminal is dropped, which writes its end.
        let written = written.borrow().clone();
        drop(terminal);
        (read, written, waits)
    }

    /// The screen `bytes` leave on a terminal of 80 by 24.
    fn shown(bytes: &[u8]) -> cineris_testkit::VTerm {
        let mut term = cineris_testkit::VTerm::new(80, 24);
        term.feed(bytes);
        term
    }

    const ESC: Step = (Ready::Input, b"\x1b");
    /// The escape key's wait, run out: the lone ESC is the key.
    const ESC_WAITED: Step = (Ready::Timeout, b"");

    /// `Esc` and `Ctrl+C` store nothing: the read gives no key, the key
    /// was never written to the terminal, and the rows are gone once it
    /// ends.
    #[test]
    fn auth_login_at_a_terminal_stores_nothing_on_esc_or_ctrl_c() {
        let ways: [(&str, &[Step]); 2] = [
            ("esc", &[(Ready::Input, b"zq7w"), ESC, ESC_WAITED]),
            (
                "ctrl+c",
                &[(Ready::Input, b"zq7w"), (Ready::Input, b"\x03")],
            ),
        ];
        for (way, steps) in ways {
            let (read, written, waits) = login(steps);
            // A key would print as `[redacted]`.
            assert!(matches!(read, Ok(None)), "{way}: {read:?}");
            let typed = shown(&written[..waits[1]]);
            assert!(
                typed.shows("4 characters, hidden"),
                "{way}: {}",
                typed.dump()
            );
            assert!(
                !written.windows(4).any(|window| window == b"zq7w"),
                "{way}: the key is never written"
            );
            let after = shown(&written);
            assert!(!after.shows("hidden"), "{way}: {}", after.dump());
        }
    }

    /// A reply of the terminal's split after its ESC (its rest 600 ms
    /// later, as a pseudo-console splits one on a busy machine) neither
    /// cancels the login nor goes into the key: while the replies are due
    /// the terminal layer holds the ESC and gives the reply (a review of
    /// step 25; small fixes II).
    #[test]
    fn auth_login_is_neither_cancelled_nor_typed_into_by_a_split_reply() {
        let mut steps: Vec<Step> = vec![ESC];
        // The loop's wakes, a millisecond each: 600 of them.
        steps.extend(std::iter::repeat_n((Ready::Woken, &b""[..]), 600));
        steps.extend([
            (Ready::Input, &b"[?0u"[..]),
            (Ready::Input, b"sk-12345"),
            (Ready::Input, b"\r"),
        ]);
        let (read, written, waits) = login(&steps);
        assert!(matches!(read, Ok(Some(_))), "{read:?}");
        let typed = shown(&written[..waits[waits.len() - 1]]);
        assert!(typed.shows("8 characters, hidden"), "{}", typed.dump());
    }

    /// `Ctrl+U` empties the key; a paste refused keeps nothing of it and
    /// says why under the row; `Enter` with nothing typed says so and goes
    /// on; a line pasted with its end is kept without it.
    #[test]
    fn auth_login_at_a_terminal_empties_on_ctrl_u_and_keeps_nothing_of_a_refused_paste() {
        let steps: &[Step] = &[
            (Ready::Input, b"sk-12"),
            // Ctrl+U.
            (Ready::Input, b"\x15"),
            (Ready::Input, b"\r"),
            (Ready::Input, b"\x1b[200~ab\ncd\x1b[201~"),
            (Ready::Input, b"\x1b[200~ab\tcd\x1b[201~"),
            (Ready::Input, b"k1"),
            (Ready::Input, b"\x1b[200~x\r\n\x1b[201~"),
            (Ready::Input, b"\r"),
        ];
        let (read, written, waits) = login(steps);
        let screen = |step: usize| shown(&written[..waits[step + 1]]);
        let emptied = screen(1);
        assert!(emptied.shows("0 characters, hidden"), "{}", emptied.dump());
        let nothing = screen(2);
        assert!(nothing.shows("Nothing was typed"), "{}", nothing.dump());
        let line_break = screen(3);
        assert!(
            line_break.shows("0 characters, hidden"),
            "{}",
            line_break.dump()
        );
        assert!(
            line_break.shows("holds a line break"),
            "{}",
            line_break.dump()
        );
        let control = screen(4);
        assert!(control.shows("0 characters, hidden"), "{}", control.dump());
        assert!(
            control.shows("holds a control character"),
            "{}",
            control.dump()
        );
        let line = screen(6);
        assert!(line.shows("3 characters, hidden"), "{}", line.dump());
        assert!(!line.shows("nothing of it was kept"), "{}", line.dump());
        assert_eq!(read.expect("read").expect("a key").expose(), "k1x");
    }

    /// Stopped and continued (`fg`): the stop erased the rows and the shell
    /// wrote below where they were, so they are drawn again whole where
    /// the cursor is now, before anything more is typed.
    #[test]
    fn auth_login_at_a_terminal_draws_its_rows_again_after_a_resume() {
        let steps: &[Step] = &[
            (Ready::Input, b"zq7w"),
            (Ready::Resumed, b""),
            ESC,
            ESC_WAITED,
        ];
        let (read, written, waits) = login(steps);
        assert!(matches!(read, Ok(None)), "{read:?}");
        let mut term = shown(&written[..waits[1]]);
        assert!(term.shows("4 characters, hidden"), "{}", term.dump());
        // What the stop and the shell left: an empty screen, its prompt and
        // the `fg` typed at it.
        term.feed(b"\x1b[H\x1b[2J[1]+  Stopped\r\n$ fg\r\n");
        term.feed(&written[waits[1]..waits[2]]);
        let screen = term.screen();
        let fg = screen.iter().position(|row| row == "$ fg");
        let sentence = screen
            .iter()
            .position(|row| row.contains("Paste or type the key for anthropic"));
        let count = screen
            .iter()
            .position(|row| row.contains("4 characters, hidden"));
        assert!(
            matches!((fg, sentence, count), (Some(fg), Some(sentence), Some(count)) if fg < sentence && sentence < count),
            "{}",
            term.dump()
        );
    }
}
