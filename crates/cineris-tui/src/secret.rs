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
//! ([`key_box`]). A paste that holds a line break, or more than the
//! buffer's capacity, is refused whole with a sentence, and nothing of it
//! is kept.
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
                self.push(&character.to_string())
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

    /// A paste: kept whole, or refused whole.
    pub fn paste(&mut self, text: &str) -> KeyEdit {
        if self.push(text) {
            self.refusal = None;
            KeyEdit::Changed
        } else {
            KeyEdit::Refused
        }
    }

    fn push(&mut self, text: &str) -> bool {
        match self.buffer.push_str(text) {
            Ok(()) => true,
            Err(Refused::LineBreak) => {
                self.refusal = Some(
                    "That paste holds a line break, and a key is one line: nothing of it was kept.",
                );
                false
            }
            Err(Refused::TooLong) => {
                self.refusal = Some("That is longer than a key can be: nothing of it was kept.");
                false
            }
        }
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
    let mut terminal = Terminal::enter(overrides)?;
    terminal.hand_signals_over();
    let read = read_key_on(&mut terminal, asking);
    let left = terminal.leave();
    let read = read?;
    left?;
    Ok(read)
}

/// [`read_key`] on a terminal already entered: for tests, on a device of
/// their own.
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
}
