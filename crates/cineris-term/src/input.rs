//! Input decoding: the bytes a terminal sends, as keys, pastes, focus
//! changes and replies to queries.
//!
//! One decoder serves both systems. On Windows the terminal layer turns on
//! the console's virtual-terminal input, so a Windows terminal sends the
//! same sequences as a Linux one (`docs/architecture/09-tui.md`, "Windows").
//!
//! # Bytes arrive in pieces
//!
//! A read can end anywhere: inside a UTF-8 character, inside an escape
//! sequence, inside a paste. The [`Decoder`] keeps what it cannot decode yet
//! and waits for more, so the same bytes decode the same way however they are
//! split. One thing cannot be told from the bytes alone: a lone ESC is either
//! the Escape key or the start of a sequence whose rest is still on its way.
//! The decoder keeps it, says so with [`Decoder::pending`], and the reader
//! calls [`Decoder::flush`] when nothing more arrives within a short time:
//! then it is the Escape key.
//!
//! # Windows Terminal's win32 input mode
//!
//! Virtual-terminal input cannot tell some keys apart: `Shift+Enter` is the
//! same byte as `Enter`, `Ctrl+Tab` as `Tab`, `Ctrl+Shift+A` as `Ctrl+A`.
//! Windows Terminal can instead report every key as a console input record
//! in a sequence of its own, `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _` (the virtual
//! key, the scan code, the character as a UTF-16 unit, down or up, the
//! modifier state, the repeat count). The terminal layer turns that on where
//! the terminal says it has it, and this decoder reads those sequences beside
//! the others: a key with all its modifiers, a release dropped, the two
//! halves of a character outside the Basic Multilingual Plane joined, and the
//! modifiers that only produced a character (Shift for `A`, AltGr for `@`)
//! not reported as modifiers.
//!
//! # What is left out
//!
//! - **Key releases**, which a terminal reports only under the enhanced
//!   keyboard protocol when asked for them, are dropped.
//! - **Sequences the decoder does not know** are dropped whole. None of their
//!   bytes becomes a key, so an unknown sequence never types text.

/// A key with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// Which key.
    pub code: KeyCode,
    /// The modifiers held with it.
    pub mods: Mods,
}

impl Key {
    /// A key with no modifier.
    #[must_use]
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: Mods::NONE,
        }
    }

    /// A key with modifiers.
    #[must_use]
    pub const fn with(code: KeyCode, mods: Mods) -> Self {
        Self { code, mods }
    }
}

/// Which key was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// A character. With Ctrl held, the letter itself: Ctrl+A is
    /// `Char('a')` with [`Mods::CTRL`].
    Char(char),
    /// Enter or Return.
    Enter,
    /// Tab. Shift+Tab is `Tab` with [`Mods::SHIFT`].
    Tab,
    /// Backspace.
    Backspace,
    /// Escape.
    Esc,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// A function key, F1 to F24.
    F(u8),
}

/// Modifier keys, as a set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    /// No modifier.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1);
    /// Alt, or Option.
    pub const ALT: Self = Self(2);
    /// Control.
    pub const CTRL: Self = Self(4);
    /// The Windows, Command or Super key.
    pub const SUPER: Self = Self(8);

    /// Whether every modifier of `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Both sets together.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether no modifier is held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The modifiers of an xterm-style parameter: 1 + the sum of the bits.
    fn from_param(param: u32) -> Self {
        let bits = u8::try_from(param.saturating_sub(1) & 0x0f).unwrap_or(0);
        Self(bits)
    }
}

impl std::ops::BitOr for Mods {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

/// A mouse button (since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    /// The left button.
    Left,
    /// The middle button, or the wheel pressed.
    Middle,
    /// The right button.
    Right,
}

/// Which way the wheel turned (since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Wheel {
    /// Up: away from the person.
    Up,
    /// Down: towards the person.
    Down,
    /// Left, on a wheel or a pad that has it.
    Left,
    /// Right.
    Right,
}

/// What the mouse did (since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseKind {
    /// A button went down.
    Press(Button),
    /// A button came up. The older report (mode 1000 alone) does not say
    /// which.
    Release(Option<Button>),
    /// The pointer moved with a button down.
    Drag(Button),
    /// The pointer moved with no button down (only where the terminal
    /// reports that, which the layer does not ask for).
    Moved,
    /// The wheel turned, a notch.
    Wheel(Wheel),
}

/// A report of the mouse, while the program asked for them
/// ([`Terminal::set_mouse`](crate::Terminal::set_mouse), since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mouse {
    /// What it did.
    pub kind: MouseKind,
    /// The column, counted from 0 at the left edge.
    pub col: u16,
    /// The row, counted from 0 at the top of the window.
    pub row: u16,
    /// The modifiers held. A terminal keeps `Shift` and a drag for its own
    /// selection, so `Shift` is seldom reported.
    pub mods: Mods,
}

/// A terminal's answer to one of the capability queries
/// ([`Capabilities::QUERIES`](crate::Capabilities::QUERIES)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reply {
    /// The state of a private mode (`CSI ? mode ; state $ y`): 0 not known,
    /// 1 set, 2 reset, 3 always set, 4 always reset.
    Mode {
        /// The mode asked about.
        mode: u32,
        /// Its state.
        state: u32,
    },
    /// The enhanced keyboard protocol's current flags (`CSI ? flags u`): the
    /// terminal has the protocol.
    Keyboard(u32),
    /// The background colour, as red, green and blue.
    Background(u8, u8, u8),
    /// The primary device attributes (`CSI ? … c`). Every terminal answers
    /// this, so it marks the end of the replies to a batch of queries.
    DeviceAttributes,
}

/// Something the terminal sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// A key was pressed (or repeated, where the terminal does not say
    /// which).
    Key(Key),
    /// A key held down, repeated: only where the terminal says so (the
    /// enhanced keyboard protocol's event types, Windows Terminal's win32
    /// input mode), since the review of step 23. It types as a key does;
    /// a dialog never takes one as its answer.
    Repeat(Key),
    /// Text was pasted, with line breaks as `\n`. Control characters are
    /// kept: whoever shows it sanitises it like any other content. A paste
    /// of more than 4 MiB arrives as several of these, one after another,
    /// with nothing between them: append them until something else comes.
    Paste(String),
    /// The terminal window gained (`true`) or lost focus.
    Focus(bool),
    /// The mouse, while its reports are on (since step 25): in a terminal
    /// that turned modes 1000 and 1006 on for the program.
    Mouse(Mouse),
    /// An answer to a capability query.
    Reply(Reply),
    /// The window changed size. Not from the decoder: the
    /// [`Terminal`](crate::Terminal) reports it.
    Resize {
        /// The new width, in columns.
        cols: u16,
        /// The new height, in rows.
        rows: u16,
    },
    /// The program was stopped and has been continued. Not from the
    /// decoder: the [`Terminal`](crate::Terminal) reports it.
    Resumed,
}

/// The longest escape sequence kept while waiting for its end. A longer one
/// is not a sequence any terminal sends, and is dropped.
const LONGEST_SEQUENCE: usize = 256;

/// The end of a bracketed paste.
const PASTE_END: &[u8] = b"\x1b[201~";

/// Turns the bytes a terminal sends into [`Event`]s.
#[derive(Debug, Default)]
pub struct Decoder {
    /// Bytes not decoded yet: the beginning of something whose end has not
    /// arrived.
    buffer: Vec<u8>,
    /// Inside a bracketed paste: what has been pasted so far.
    paste: Option<Vec<u8>>,
    /// The first half of a character outside the Basic Multilingual Plane,
    /// from a win32 input record, waiting for its second half.
    high_surrogate: Option<u32>,
    /// Inside a sequence longer than [`LONGEST_SEQUENCE`], which is being
    /// dropped to its end.
    skipping: Option<Skip>,
    /// The virtual-key code of the key a win32 input record last said went
    /// down, until a record says it went up: another down of it is the
    /// key held, repeating.
    held: Option<u32>,
    /// Whether the program asked for the mouse (since step 25): only then is
    /// `CSI M` the start of mode 1000's own report, three bytes long.
    mouse: bool,
    /// Whether byte 0x08 is `Backspace`, in a terminal whose Backspace sends
    /// `^H`, rather than `Ctrl+Backspace` (since step 25).
    ctrl_h_backspace: bool,
    /// Whether the replies to the capability queries are due: then a reply
    /// whose ESC was taken on its own is still the reply, never typed text
    /// ([`set_replies_due`](Self::set_replies_due)).
    replies: bool,
}

/// What some bytes are to the replies the capability queries ask for
/// ([`Capabilities::QUERIES`](crate::Capabilities::QUERIES)), read as what
/// follows a reply's ESC: `CSI ? … $ y`, `CSI ? … u`, `CSI ? … c`, and
/// `OSC 11 ; … ST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplyShape {
    /// A whole reply, this many bytes long.
    Whole(usize),
    /// The beginning of one: more input decides.
    Part,
    /// No reply, nor the beginning of one.
    Not,
}

/// What `bytes`, which follow a reply's ESC (or stand where it was), are to
/// the replies the capability queries ask for. See [`ReplyShape`].
pub(crate) fn reply_shape(bytes: &[u8]) -> ReplyShape {
    let Some(&first) = bytes.first() else {
        return ReplyShape::Part;
    };
    match first {
        // `[ ?`, digits and `;`, then `u` or `c`, or `$ y`.
        b'[' => {
            match bytes.get(1) {
                None => return ReplyShape::Part,
                Some(b'?') => {}
                Some(_) => return ReplyShape::Not,
            }
            let mut dollar = false;
            for (at, &byte) in bytes.iter().enumerate().skip(2) {
                if at >= LONGEST_SEQUENCE {
                    return ReplyShape::Not;
                }
                match byte {
                    b'0'..=b'9' | b';' if !dollar => {}
                    b'$' if !dollar => dollar = true,
                    b'y' if dollar => return ReplyShape::Whole(at + 1),
                    b'u' | b'c' if !dollar => return ReplyShape::Whole(at + 1),
                    _ => return ReplyShape::Not,
                }
            }
            ReplyShape::Part
        }
        // `] 1 1 ;`, printable text, then BEL or ESC `\`.
        b']' => {
            const START: &[u8] = b"]11;";
            let head = bytes.len().min(START.len());
            if bytes[..head] != START[..head] {
                return ReplyShape::Not;
            }
            let mut escape = false;
            for (at, &byte) in bytes.iter().enumerate().skip(START.len()) {
                if at >= LONGEST_SEQUENCE {
                    return ReplyShape::Not;
                }
                match byte {
                    b'\\' if escape => return ReplyShape::Whole(at + 1),
                    _ if escape => return ReplyShape::Not,
                    0x07 => return ReplyShape::Whole(at + 1),
                    0x1b => escape = true,
                    0x20..=0x7e => {}
                    _ => return ReplyShape::Not,
                }
            }
            ReplyShape::Part
        }
        _ => ReplyShape::Not,
    }
}

/// A sequence too long to keep, being dropped until it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    /// A control sequence: it ends at its final byte, or before any byte
    /// that cannot be in one.
    Control,
    /// A string (OSC, DCS and their kind): it ends at BEL or ESC `\`.
    /// `escape`: the last byte dropped was ESC.
    Text { escape: bool },
}

/// One key as Windows Terminal's win32 input mode reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    /// The virtual-key code.
    vk: u32,
    /// The character, as one UTF-16 unit; 0 for none.
    unit: u32,
    /// Pressed (`true`) or released.
    down: bool,
    /// The modifier and lock state.
    state: u32,
}

/// The bits of a record's modifier state.
const RIGHT_ALT: u32 = 0x01;
const LEFT_ALT: u32 = 0x02;
const RIGHT_CTRL: u32 = 0x04;
const LEFT_CTRL: u32 = 0x08;
const SHIFT: u32 = 0x10;

/// What decoding one item from the front of the buffer gave.
enum Step {
    /// This many bytes were used, and gave this event, or nothing.
    Done(usize, Option<Event>),
    /// This many bytes were used, and were a win32 input record.
    Record(usize, Record),
    /// The bytes are the beginning of something longer.
    Incomplete,
    /// This many bytes were used, and began a sequence longer than any a
    /// terminal sends, which is dropped to its end.
    Skip(usize, Skip),
}

impl Decoder {
    /// A decoder that has seen nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes `bytes`, which follow whatever came before, and adds what
    /// they complete to `events`.
    pub fn feed(&mut self, bytes: &[u8], events: &mut Vec<Event>) {
        self.buffer.extend_from_slice(bytes);
        self.decode(events, false);
    }

    /// Reads `CSI M` as mode 1000's own mouse report, its three bytes with
    /// it, while `on` (since step 25): while the program has mouse
    /// reporting on. Off, as at the start, `CSI M` is a sequence with no
    /// meaning and only its three bytes are dropped, so that `Alt+[` and
    /// `M` typed in a row never take the keys after them. The SGR form
    /// (mode 1006) is read either way: no key sends it.
    pub fn set_mouse(&mut self, on: bool) {
        self.mouse = on;
    }

    /// Reads byte 0x08 as `Backspace` while `on` (since step 25): in a
    /// terminal set to send `^H` for Backspace (`stty erase ^H`, an option of
    /// PuTTY and of macOS Terminal). Off, as at the start, it is
    /// `Ctrl+Backspace`, which most terminals send it for. A win32 input
    /// record says which key it was, either way.
    pub fn set_ctrl_h_backspace(&mut self, on: bool) {
        self.ctrl_h_backspace = on;
    }

    /// Reads a reply to the capability queries that comes without its ESC
    /// as the reply, while `due`: while the replies are due, which the
    /// [`Terminal`](crate::Terminal) says (since small fixes II). Through a
    /// pseudo-console a reply comes about a key at a time, and its ESC can
    /// be taken on its own (the Escape key, or `Alt` with a key typed in
    /// between); what follows it, `[?0u` or `[?62c`, is then still the
    /// reply, and is never typed. A `[` or a `]` that may begin one is kept,
    /// as a lone ESC is, until what follows it says whether it does (the
    /// reader waits for it as for a lone ESC); it is a key when it does not.
    /// Off, as at the start, they are keys at once.
    pub(crate) fn set_replies_due(&mut self, due: bool) {
        self.replies = due;
    }

    /// Whether what is kept begins with an ESC that `next`, input that came
    /// a while after the last byte kept (the reader says how long), may
    /// still go on from: `None` when what is kept does not begin with an
    /// ESC. After a lone ESC, only a byte that may begin a sequence (an
    /// introducer: `[`, `]`, `O`, `P`, `_`, `^`, `X`) goes on, so that a
    /// focus report, a key's sequence or a paste's mark split after its ESC
    /// is still what it is; a character, a control byte or a second ESC
    /// does not. After more than the ESC, only the rest of a reply to the
    /// capability queries goes on ([`reply_shape`]): a reply comes a key at
    /// a time through a pseudo-console, and a person's keys a while apart
    /// are keys (the reviews of small fixes II).
    pub(crate) fn kept_escape_goes_on(&self, next: &[u8]) -> Option<bool> {
        if self.paste.is_some() || self.skipping.is_some() || self.buffer.first() != Some(&0x1b) {
            return None;
        }
        if self.buffer.len() == 1 {
            return Some(matches!(
                next.first(),
                None | Some(b'[' | b']' | b'O' | b'P' | b'_' | b'^' | b'X')
            ));
        }
        let mut after = self.buffer[1..].to_vec();
        after.extend_from_slice(next);
        Some(reply_shape(&after) != ReplyShape::Not)
    }

    /// The kept ESC is the Escape key, given now; what was kept after it is
    /// decoded on its own, as it would have been had the ESC been given
    /// when it came. Nothing happens when what is kept does not begin with
    /// an ESC.
    pub(crate) fn release_escape(&mut self, events: &mut Vec<Event>) {
        if self.paste.is_some() || self.skipping.is_some() || self.buffer.first() != Some(&0x1b) {
            return;
        }
        self.buffer.remove(0);
        events.push(key(KeyCode::Esc));
        self.decode(events, false);
    }

    /// Whether bytes are kept that more input would complete: the reader
    /// waits a short time and then calls [`flush`](Self::flush).
    #[must_use]
    pub fn pending(&self) -> bool {
        !self.buffer.is_empty() && self.paste.is_none()
    }

    /// Nothing more arrived: decodes what was kept as it stands. A lone ESC
    /// is the Escape key, and ESC followed by the start of a sequence that
    /// never finished is Alt with the next character. A paste is never cut
    /// short by this: it lasts until its end arrives.
    pub fn flush(&mut self, events: &mut Vec<Event>) {
        if self.paste.is_none() {
            self.decode(events, true);
        }
    }

    /// Whether it is inside something whose end may never come: a paste, or
    /// a sequence being dropped. The reader gives up on it after a time
    /// with no input ([`give_up`](Self::give_up)).
    pub(crate) fn open(&self) -> bool {
        self.paste.is_some() || self.skipping.is_some()
    }

    /// Nothing has come for a long time inside a paste or a sequence being
    /// dropped: the paste ends with what it holds, given as a paste and
    /// never as keys, and the sequence is over. What follows is decoded as
    /// it comes.
    pub(crate) fn give_up(&mut self, events: &mut Vec<Event>) {
        self.skipping = None;
        if let Some(paste) = self.paste.take() {
            // Bytes kept as the possible start of the end marker were not
            // text.
            self.buffer.clear();
            events.push(Event::Paste(paste_text(&paste)));
        }
    }

    fn decode(&mut self, events: &mut Vec<Event>, flushing: bool) {
        let mut start = 0;
        loop {
            if let Some(skip) = self.skipping {
                let (used, ended) = skip_to_end(&self.buffer[start..], skip);
                start += used;
                match ended {
                    Ok(()) => self.skipping = None,
                    Err(still) => {
                        self.skipping = Some(still);
                        break;
                    }
                }
                continue;
            }
            if let Some(paste) = &mut self.paste {
                let rest = &self.buffer[start..];
                if let Some(end) = find(rest, PASTE_END) {
                    paste.extend_from_slice(&rest[..end]);
                    start += end + PASTE_END.len();
                    let text = paste_text(paste);
                    self.paste = None;
                    events.push(Event::Paste(text));
                    continue;
                }
                // Keep any tail that could be the start of the end marker.
                let keep = (1..PASTE_END.len())
                    .rev()
                    .find(|&n| rest.len() >= n && rest.ends_with(&PASTE_END[..n]))
                    .unwrap_or(0);
                paste.extend_from_slice(&rest[..rest.len() - keep]);
                start = self.buffer.len() - keep;
                if paste.len() >= PASTE_PIECE {
                    events.push(Event::Paste(paste_piece(paste)));
                }
                break;
            }
            if start >= self.buffer.len() {
                break;
            }
            if !self.mouse && self.buffer[start..].starts_with(b"\x1b[M") {
                start += 3;
                continue;
            }
            if self.ctrl_h_backspace && self.buffer[start] == 0x08 {
                events.push(Event::Key(Key::plain(KeyCode::Backspace)));
                start += 1;
                continue;
            }
            // And after ESC, `Alt+Backspace`, which deletes a word (the
            // review of step 25's second pull request).
            if self.ctrl_h_backspace && self.buffer[start..].starts_with(b"\x1b\x08") {
                events.push(Event::Key(Key::with(KeyCode::Backspace, Mods::ALT)));
                start += 2;
                continue;
            }
            // A reply whose ESC was taken on its own, while the replies are
            // due: the reply, never typed text (since small fixes II).
            if self.replies && matches!(self.buffer[start], b'[' | b']') {
                match reply_shape(&self.buffer[start..]) {
                    ReplyShape::Whole(used) => {
                        let mut sequence = vec![0x1b];
                        sequence.extend_from_slice(&self.buffer[start..start + used]);
                        if let Step::Done(_, Some(event)) = escape(&sequence, true) {
                            events.push(event);
                        }
                        start += used;
                        continue;
                    }
                    ReplyShape::Part if !flushing => break,
                    ReplyShape::Part | ReplyShape::Not => {}
                }
            }
            match step(&self.buffer[start..], flushing) {
                Step::Done(used, event) => {
                    start += used;
                    match event {
                        Some(Event::Paste(_)) => self.paste = Some(Vec::new()),
                        Some(event) => events.push(event),
                        None => {}
                    }
                }
                Step::Record(used, record) => {
                    start += used;
                    if let Some(event) = self.record(record) {
                        events.push(event);
                    }
                }
                Step::Incomplete => break,
                Step::Skip(used, skip) => {
                    start += used;
                    self.skipping = Some(skip);
                }
            }
        }
        self.buffer.drain(..start);
    }

    /// The key a win32 input record stands for: `None` for a release, for a
    /// key that is only a modifier, and for the first half of a character.
    fn record(&mut self, record: Record) -> Option<Event> {
        if !record.down {
            if self.held == Some(record.vk) {
                self.held = None;
            }
            return None;
        }
        let event = self.record_down(record)?;
        // A modifier is held under other keys, and says nothing of them.
        let repeat = self.held == Some(record.vk);
        self.held = Some(record.vk);
        Some(match event {
            Event::Key(key) if repeat => Event::Repeat(key),
            event => event,
        })
    }

    /// The key a win32 input record of a key going down stands for.
    fn record_down(&mut self, record: Record) -> Option<Event> {
        let code = match record.vk {
            0x08 => KeyCode::Backspace,
            0x09 => KeyCode::Tab,
            0x0d => KeyCode::Enter,
            0x1b => KeyCode::Esc,
            0x21 => KeyCode::PageUp,
            0x22 => KeyCode::PageDown,
            0x23 => KeyCode::End,
            0x24 => KeyCode::Home,
            0x25 => KeyCode::Left,
            0x26 => KeyCode::Up,
            0x27 => KeyCode::Right,
            0x28 => KeyCode::Down,
            0x2d => KeyCode::Insert,
            0x2e => KeyCode::Delete,
            0x70..=0x87 => KeyCode::F(u8::try_from(record.vk - 0x6f).ok()?),
            // Shift, Ctrl, Alt, Caps Lock, the Windows keys, Num Lock,
            // Scroll Lock, and the left and right halves of the first three.
            0x10..=0x12 | 0x14 | 0x5b | 0x5c | 0x90 | 0x91 | 0xa0..=0xa5 => return None,
            _ => return self.record_char(record),
        };
        Some(Event::Key(Key::with(code, record_mods(record.state))))
    }

    /// A record of a key that types a character, or of Ctrl with one.
    fn record_char(&mut self, record: Record) -> Option<Event> {
        let unit = record.unit;
        let typed = match unit {
            0xd800..=0xdbff => {
                self.high_surrogate = Some(unit);
                return None;
            }
            0xdc00..=0xdfff => {
                let high = self.high_surrogate.take()?;
                char::from_u32(0x10000 + ((high - 0xd800) << 10) + (unit - 0xdc00))
            }
            _ => char::from_u32(unit),
        }
        .filter(|c| !c.is_control() && *c != '\0');
        let mut state = record.state;
        let c = if let Some(c) = typed {
            // AltGr is Right Alt with Left Ctrl: it made the character.
            if state & RIGHT_ALT != 0 && state & LEFT_CTRL != 0 {
                state &= !(RIGHT_ALT | LEFT_CTRL);
            }
            // Shift alone is in the character itself.
            if state & (RIGHT_ALT | LEFT_ALT | RIGHT_CTRL | LEFT_CTRL) == 0 {
                state &= !SHIFT;
            }
            c
        } else {
            // Ctrl with a letter or a digit types a control character or
            // none: the key itself says which.
            match record.vk {
                0x41..=0x5a => char::from_u32(record.vk + 0x20)?,
                0x30..=0x39 | 0x20 => char::from_u32(record.vk)?,
                _ => return None,
            }
        };
        Some(Event::Key(Key::with(KeyCode::Char(c), record_mods(state))))
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// How much of a paste is kept before it is handed on: a paste larger than
/// this arrives as several [`Event::Paste`]s, in order, so that a paste
/// whose end never comes cannot take all the memory there is.
const PASTE_PIECE: usize = 1 << 22;

/// The front of a long paste as text, leaving in `paste` what may be the
/// first half of a character or of a line break.
fn paste_piece(paste: &mut Vec<u8>) -> String {
    // A character cut at the end starts in the last three bytes.
    let mut cut = paste.len();
    for back in 1..=3.min(paste.len()) {
        let index = paste.len() - back;
        let byte = paste[index];
        if byte & 0xc0 == 0x80 {
            continue;
        }
        let length = match byte {
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        if length > back {
            cut = index;
        }
        break;
    }
    if cut > 0 && paste[cut - 1] == b'\r' {
        cut -= 1;
    }
    let rest = paste.split_off(cut);
    let piece = paste_text(paste);
    *paste = rest;
    piece
}

/// Pasted bytes as text, with each line break as `\n`.
fn paste_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Decodes one item from the front of `bytes`, which is not empty.
fn step(bytes: &[u8], flushing: bool) -> Step {
    match bytes[0] {
        0x1b => escape(bytes, flushing),
        byte @ (0..0x20 | 0x7f) => Step::Done(1, Some(Event::Key(control(byte)))),
        0x20..0x7f => Step::Done(1, Some(key(KeyCode::Char(char::from(bytes[0]))))),
        _ => match utf8(bytes) {
            Some((c, used)) => Step::Done(used, Some(key(KeyCode::Char(c)))),
            None if flushing => Step::Done(bytes.len(), Some(key(KeyCode::Char('\u{fffd}')))),
            None => Step::Incomplete,
        },
    }
}

/// The modifiers of a record's state.
fn record_mods(state: u32) -> Mods {
    let mut mods = Mods::NONE;
    if state & SHIFT != 0 {
        mods = mods | Mods::SHIFT;
    }
    if state & (LEFT_ALT | RIGHT_ALT) != 0 {
        mods = mods | Mods::ALT;
    }
    if state & (LEFT_CTRL | RIGHT_CTRL) != 0 {
        mods = mods | Mods::CTRL;
    }
    mods
}

/// A win32 input record (`CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`) at the front of
/// `bytes`, with the bytes it takes; `None` when `bytes` do not start with a
/// whole one.
fn win32_record(bytes: &[u8]) -> Option<(usize, Record)> {
    let body = bytes.get(2..)?;
    let end = body.iter().position(|byte| (0x40..=0x7e).contains(byte))?;
    if body[end] != b'_' || !body[..end].iter().all(|b| b.is_ascii_digit() || *b == b';') {
        return None;
    }
    let groups = params(&body[..end]);
    let number = |index: usize, default: u32| -> u32 {
        groups
            .get(index)
            .and_then(|group| group.first())
            .copied()
            .flatten()
            .unwrap_or(default)
    };
    Some((
        2 + end + 1,
        Record {
            vk: number(0, 0),
            unit: number(2, 0),
            down: number(3, 0) == 1,
            state: number(4, 0),
        },
    ))
}

fn key(code: KeyCode) -> Event {
    Event::Key(Key::plain(code))
}

/// The key a control byte stands for.
fn control(byte: u8) -> Key {
    match byte {
        b'\r' => Key::plain(KeyCode::Enter),
        b'\t' => Key::plain(KeyCode::Tab),
        0x7f => Key::plain(KeyCode::Backspace),
        // Ctrl+Backspace in most terminals, and Ctrl+H in the rest.
        0x08 => Key::with(KeyCode::Backspace, Mods::CTRL),
        0 => Key::with(KeyCode::Char(' '), Mods::CTRL),
        0x1b => Key::plain(KeyCode::Esc),
        // Ctrl+A to Ctrl+Z, Ctrl+J (line feed) among them.
        0x01..=0x1a => Key::with(KeyCode::Char(char::from(b'a' + byte - 1)), Mods::CTRL),
        // Ctrl+\, Ctrl+], Ctrl+^ and Ctrl+_.
        _ => Key::with(KeyCode::Char(char::from(byte + 0x40)), Mods::CTRL),
    }
}

/// The first character of `bytes`, which starts with a byte above ASCII,
/// and its length; `None` when it is not complete yet. A byte that cannot
/// start a character, or a character that is not valid, is one U+FFFD.
fn utf8(bytes: &[u8]) -> Option<(char, usize)> {
    let length = match bytes[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Some(('\u{fffd}', 1)),
    };
    // Every byte after the first must be a continuation byte. A wrong one is
    // known before the character is complete.
    for (index, byte) in bytes.iter().enumerate().take(length).skip(1) {
        if byte & 0xc0 != 0x80 {
            return Some(('\u{fffd}', index));
        }
    }
    if bytes.len() < length {
        return None;
    }
    match std::str::from_utf8(&bytes[..length]) {
        Ok(text) => text.chars().next().map(|c| (c, length)),
        Err(_) => Some(('\u{fffd}', length)),
    }
}

/// Decodes what starts with ESC.
fn escape(bytes: &[u8], flushing: bool) -> Step {
    let Some(&second) = bytes.get(1) else {
        return if flushing {
            Step::Done(1, Some(key(KeyCode::Esc)))
        } else {
            Step::Incomplete
        };
    };
    let sequence = match second {
        b'[' => {
            if let Some((used, record)) = win32_record(bytes) {
                return Step::Record(used, record);
            }
            csi(bytes)
        }
        b'O' => ss3(bytes),
        b']' => string(bytes, 2).map(|(used, body)| (used, osc(body))),
        b'P' | b'_' | b'^' | b'X' => string(bytes, 2).map(|(used, _)| (used, None)),
        _ => {
            // Alt with the key that follows: ESC then a character or a
            // control byte. ESC ESC is Alt+Escape.
            return match step(&bytes[1..], flushing) {
                Step::Done(used, Some(Event::Key(key))) => Step::Done(
                    1 + used,
                    Some(Event::Key(Key::with(key.code, key.mods | Mods::ALT))),
                ),
                Step::Done(used, other) => Step::Done(1 + used, other),
                Step::Record(used, record) => Step::Record(1 + used, record),
                Step::Incomplete => Step::Incomplete,
                Step::Skip(used, skip) => Step::Skip(1 + used, skip),
            };
        }
    };
    match sequence {
        Some((used, event)) => Step::Done(used, event),
        None if bytes.len() > LONGEST_SEQUENCE => {
            // Not a sequence any terminal sends. It is not kept growing:
            // what has arrived of it is dropped, and the rest as it comes,
            // to its end, so that none of it becomes keys.
            let skip = if second == b'[' {
                Skip::Control
            } else {
                Skip::Text {
                    escape: bytes.last() == Some(&0x1b),
                }
            };
            Step::Skip(bytes.len(), skip)
        }
        None if flushing => {
            // It never finished: Alt with the introducer, and the rest is
            // decoded on its own.
            Step::Done(
                2,
                Some(Event::Key(Key::with(
                    KeyCode::Char(char::from(second)),
                    Mods::ALT,
                ))),
            )
        }
        None => Step::Incomplete,
    }
}

/// Drops what belongs to the sequence `skip` from the front of `bytes`: the
/// bytes used, and `Ok` if it ended there or the state to go on in.
fn skip_to_end(bytes: &[u8], skip: Skip) -> (usize, Result<(), Skip>) {
    match skip {
        Skip::Control => match bytes.iter().position(|byte| !(0x20..=0x3f).contains(byte)) {
            // The final byte ends it, and is the sequence's; anything else
            // was never part of it, and is decoded on its own.
            Some(at) if (0x40..=0x7e).contains(&bytes[at]) => (at + 1, Ok(())),
            Some(at) => (at, Ok(())),
            None => (bytes.len(), Err(skip)),
        },
        Skip::Text { mut escape } => {
            for (at, &byte) in bytes.iter().enumerate() {
                if byte == 0x07 || (escape && byte == b'\\') {
                    return (at + 1, Ok(()));
                }
                escape = byte == 0x1b;
            }
            (bytes.len(), Err(Skip::Text { escape }))
        }
    }
}

/// A string sequence (OSC, DCS and their kind) starting at `from`, ended by
/// BEL or by ESC `\`: the bytes used and the body; `None` until its end has
/// arrived.
fn string(bytes: &[u8], from: usize) -> Option<(usize, &[u8])> {
    let body = &bytes[from..];
    for (index, &byte) in body.iter().enumerate() {
        match byte {
            0x07 => return Some((from + index + 1, &body[..index])),
            0x1b if body.get(index + 1) == Some(&b'\\') => {
                return Some((from + index + 2, &body[..index]));
            }
            _ => {}
        }
    }
    None
}

/// An OSC reply: only the background colour is asked for.
fn osc(body: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(body).ok()?;
    let spec = text.strip_prefix("11;")?;
    let (r, g, b) = parse_rgb(spec)?;
    Some(Event::Reply(Reply::Background(r, g, b)))
}

/// `rgb:RRRR/GGGG/BBBB`, with one to four hexadecimal digits a channel.
fn parse_rgb(spec: &str) -> Option<(u8, u8, u8)> {
    let channels = spec.strip_prefix("rgb:")?;
    let mut parts = channels.split('/');
    let mut channel = || -> Option<u8> {
        let digits = parts.next()?;
        if digits.is_empty() || digits.len() > 4 {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        let max = (1u32 << (4 * digits.len())) - 1;
        u8::try_from(value * 255 / max).ok()
    };
    let rgb = (channel()?, channel()?, channel()?);
    parts.next().is_none().then_some(rgb)
}

/// SS3: ESC O and one byte.
fn ss3(bytes: &[u8]) -> Option<(usize, Option<Event>)> {
    let &last = bytes.get(2)?;
    let code = match last {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'M' => KeyCode::Enter,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return Some((3, None)),
    };
    Some((3, Some(key(code))))
}

/// A control sequence: ESC [, parameters, intermediates, a final byte.
fn csi(bytes: &[u8]) -> Option<(usize, Option<Event>)> {
    let body = &bytes[2..];
    // The older mouse report, `CSI M` and three bytes, each a value plus 32:
    // from a terminal that turned mode 1000 on and does not know 1006. Its
    // bytes are not a sequence's, so they are taken whole here, and never
    // become typed text.
    if body.first() == Some(&b'M') {
        let report = body.get(1..4)?;
        let value = |byte: u8| u32::from(byte.saturating_sub(32));
        return Some((
            6,
            mouse(value(report[0]), value(report[1]), value(report[2]), None),
        ));
    }
    let mut end = None;
    for (index, &byte) in body.iter().enumerate() {
        match byte {
            0x40..=0x7e => {
                end = Some(index);
                break;
            }
            0x20..=0x3f => {}
            // A byte no sequence holds after more than a sequence's length:
            // a sequence too long to keep, dropped up to that byte, which is
            // decoded on its own; none of it becomes keys.
            _ if 2 + index > LONGEST_SEQUENCE => return Some((2 + index, None)),
            // A byte no sequence holds: this was never one, but `Alt+[`
            // followed by what was typed next, which is decoded on its own.
            _ => {
                return Some((
                    2,
                    Some(Event::Key(Key::with(KeyCode::Char('['), Mods::ALT))),
                ));
            }
        }
    }
    let end = end?;
    let used = 2 + end + 1;
    let final_byte = body[end];
    let inner = &body[..end];
    let split = inner
        .iter()
        .position(|byte| (0x20..=0x2f).contains(byte))
        .unwrap_or(inner.len());
    let (params, intermediates) = inner.split_at(split);
    Some((used, decode_csi(params, intermediates, final_byte)))
}

/// The parameters of a sequence: groups separated by `;`, each of numbers
/// separated by `:`. A missing number is `None`.
fn params(text: &[u8]) -> Vec<Vec<Option<u32>>> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split(|&byte| byte == b';')
        .map(|group| {
            group
                .split(|&byte| byte == b':')
                .map(|number| {
                    std::str::from_utf8(number)
                        .ok()
                        .and_then(|number| number.parse().ok())
                })
                .collect()
        })
        .collect()
}

fn decode_csi(raw: &[u8], intermediates: &[u8], final_byte: u8) -> Option<Event> {
    let (prefix, raw) = match raw.first() {
        Some(&byte @ (b'?' | b'<' | b'=' | b'>')) => (Some(byte), &raw[1..]),
        _ => (None, raw),
    };
    let groups = params(raw);
    let number = |group: usize, index: usize| -> Option<u32> {
        groups
            .get(group)
            .and_then(|g| g.get(index))
            .copied()
            .flatten()
    };
    match (prefix, intermediates, final_byte) {
        (Some(b'?'), b"$", b'y') => Some(Event::Reply(Reply::Mode {
            mode: number(0, 0)?,
            state: number(1, 0).unwrap_or(0),
        })),
        (Some(b'?'), b"", b'u') => Some(Event::Reply(Reply::Keyboard(number(0, 0).unwrap_or(0)))),
        (Some(b'?'), b"", b'c') => Some(Event::Reply(Reply::DeviceAttributes)),
        // The mouse, in the form mode 1006 asks for: `M` a press, a drag or
        // the wheel, `m` a release.
        (Some(b'<'), b"", b'M' | b'm') => mouse(
            number(0, 0)?,
            number(1, 0)?,
            number(2, 0)?,
            Some(final_byte == b'm'),
        ),
        (None, b"", b'I') if groups.is_empty() => Some(Event::Focus(true)),
        (None, b"", b'O') if groups.is_empty() => Some(Event::Focus(false)),
        (None, b"", b'u') => kitty(&groups),
        (None, b"", b'~') => tilde(number(0, 0)?, &groups),
        (None, b"", final_byte) => {
            let code = match final_byte {
                b'A' => KeyCode::Up,
                b'B' => KeyCode::Down,
                b'C' => KeyCode::Right,
                b'D' => KeyCode::Left,
                b'H' => KeyCode::Home,
                b'F' => KeyCode::End,
                b'E' => return None,
                b'P' => KeyCode::F(1),
                b'Q' => KeyCode::F(2),
                b'R' => KeyCode::F(3),
                b'S' => KeyCode::F(4),
                b'Z' => return Some(Event::Key(Key::with(KeyCode::Tab, Mods::SHIFT))),
                _ => return None,
            };
            let mods = Mods::from_param(number(1, 0).unwrap_or(1));
            of_kind(Key::with(code, mods), &groups)
        }
        _ => None,
    }
}

/// A mouse report: its button code, and its column and row counted from 1.
/// `released`: the SGR form says whether a button came up (`m`); `None` for
/// the older form, where a button code of 3 says it.
fn mouse(code: u32, col: u32, row: u32, released: Option<bool>) -> Option<Event> {
    let mut mods = Mods::NONE;
    if code & 4 != 0 {
        mods = mods | Mods::SHIFT;
    }
    if code & 8 != 0 {
        mods = mods | Mods::ALT;
    }
    if code & 16 != 0 {
        mods = mods | Mods::CTRL;
    }
    let button = match code & 3 {
        0 => Some(Button::Left),
        1 => Some(Button::Middle),
        2 => Some(Button::Right),
        _ => None,
    };
    let kind = if code & 64 != 0 {
        MouseKind::Wheel(match code & 3 {
            0 => Wheel::Up,
            1 => Wheel::Down,
            2 => Wheel::Left,
            _ => Wheel::Right,
        })
    } else if code & 128 != 0 {
        // The buttons past the third: nothing the interface uses.
        return None;
    } else {
        match (released, code & 32 != 0, button) {
            (Some(true), _, button) => MouseKind::Release(button),
            (None, false, None) => MouseKind::Release(None),
            (_, true, Some(button)) => MouseKind::Drag(button),
            (_, true, None) => MouseKind::Moved,
            (_, false, Some(button)) => MouseKind::Press(button),
            (Some(false), false, None) => return None,
        }
    };
    let cell = |value: u32| u16::try_from(value.saturating_sub(1)).unwrap_or(u16::MAX);
    Some(Event::Mouse(Mouse {
        kind,
        col: cell(col),
        row: cell(row),
        mods,
    }))
}

/// A key with the enhanced protocol's event type, the second part of its
/// modifiers' group (`CSI … ; mods:kind …`), which the protocol's flag 2
/// adds to the legacy forms of the keys with no text too (arrows, `Home`,
/// `Delete`, the function keys): a release (3) is nothing, a repeat (2)
/// is [`Event::Repeat`], and a press (1, or none) is the key (since the
/// review of step 23).
fn of_kind(key: Key, groups: &[Vec<Option<u32>>]) -> Option<Event> {
    match groups
        .get(1)
        .and_then(|group| group.get(1))
        .copied()
        .flatten()
        .unwrap_or(1)
    {
        3 => None,
        2 => Some(Event::Repeat(key)),
        _ => Some(Event::Key(key)),
    }
}

/// `CSI number [; modifiers] ~`: the editing keys, function keys, the start
/// of a paste, and xterm's other keys with modifiers (`CSI 27 ; m ; c ~`).
fn tilde(first: u32, groups: &[Vec<Option<u32>>]) -> Option<Event> {
    let mods = Mods::from_param(
        groups
            .get(1)
            .and_then(|group| group.first())
            .copied()
            .flatten()
            .unwrap_or(1),
    );
    let code = match first {
        200 => return Some(Event::Paste(String::new())),
        // The end of a paste that never began: nothing.
        201 => return None,
        27 => {
            let c = groups.get(2)?.first().copied().flatten()?;
            return of_kind(Key::with(codepoint_key(c)?, mods), groups);
        }
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11..=15 => KeyCode::F(u8::try_from(first - 10).ok()?),
        17..=21 => KeyCode::F(u8::try_from(first - 11).ok()?),
        23..=26 => KeyCode::F(u8::try_from(first - 12).ok()?),
        28 | 29 => KeyCode::F(u8::try_from(first - 13).ok()?),
        31..=34 => KeyCode::F(u8::try_from(first - 14).ok()?),
        _ => return None,
    };
    of_kind(Key::with(code, mods), groups)
}

/// The key a Unicode code point stands for in a reported key.
fn codepoint_key(code: u32) -> Option<KeyCode> {
    Some(match code {
        13 => KeyCode::Enter,
        9 => KeyCode::Tab,
        127 | 8 => KeyCode::Backspace,
        27 => KeyCode::Esc,
        // The enhanced protocol's own numbers for keys that have no
        // character (the private use area) are not decoded.
        0xe000..=0xf8ff => return None,
        _ => KeyCode::Char(char::from_u32(code).filter(|c| !c.is_control())?),
    })
}

/// The enhanced keyboard protocol: `CSI code[:shifted[:base]] ; mods[:kind]
/// [; text] u`. A release (kind 3) is dropped; a repeat (kind 2) is
/// [`Event::Repeat`].
fn kitty(groups: &[Vec<Option<u32>>]) -> Option<Event> {
    let code = groups.first()?.first().copied().flatten()?;
    let modifiers = groups.get(1);
    let mods = Mods::from_param(
        modifiers
            .and_then(|group| group.first())
            .copied()
            .flatten()
            .unwrap_or(1),
    );
    of_kind(Key::with(codepoint_key(code)?, mods), groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(bytes, &mut events);
        decoder.flush(&mut events);
        events
    }

    #[test]
    fn the_shapes_of_the_replies_to_the_queries() {
        use ReplyShape::{Not, Part, Whole};
        let cases: &[(&[u8], ReplyShape)] = &[
            (b"", Part),
            (b"[", Part),
            (b"[?", Part),
            (b"[?0", Part),
            (b"[?0u", Whole(4)),
            (b"[?0ux", Whole(4)),
            (b"[?62;1c", Whole(7)),
            (b"[?2026;2$", Part),
            (b"[?2026;2$y", Whole(10)),
            (b"[A", Not),
            (b"[1;5A", Not),
            (b"[?0x", Not),
            (b"[?0$u", Not),
            (b"[?0y", Not),
            (b"a", Not),
            (b"\x1b[?0u", Not),
            (b"]", Part),
            (b"]1", Part),
            (b"]11;rgb:1e/20/ff", Part),
            (b"]11;rgb:1e/20/ff\x07", Whole(17)),
            (b"]11;x\x1b", Part),
            (b"]11;x\x1b\\", Whole(7)),
            (b"]11;x\x1bA", Not),
            (b"]12;", Not),
            (b"]11;\x01", Not),
        ];
        for &(bytes, want) in cases {
            assert_eq!(reply_shape(bytes), want, "{bytes:?}");
        }
        let long = [b"[?".as_slice(), &[b'1'; LONGEST_SEQUENCE]].concat();
        assert_eq!(reply_shape(&long), Not);
    }

    /// While the replies are due, a reply that lost its ESC is the reply,
    /// and its beginning is kept until what follows decides; otherwise, and
    /// when what follows does not go on as one, they are keys.
    #[test]
    fn a_reply_without_its_escape_is_the_reply_only_while_they_are_due() {
        let mut decoder = Decoder::new();
        decoder.set_replies_due(true);
        let mut events = Vec::new();
        decoder.feed(b"w[?0", &mut events);
        assert_eq!(events, keys(&[(KeyCode::Char('w'), N)]));
        assert!(decoder.pending(), "the start of a reply is kept");
        decoder.feed(b"u[?62c]11;rgb:00/00/00\x07", &mut events);
        assert_eq!(
            events[1..],
            [
                Event::Reply(Reply::Keyboard(0)),
                Event::Reply(Reply::DeviceAttributes),
                Event::Reply(Reply::Background(0, 0, 0)),
            ]
        );
        // Not a reply: keys, at once or when what follows says so.
        let mut events = Vec::new();
        decoder.feed(b"[a]x[", &mut events);
        assert!(decoder.pending());
        decoder.flush(&mut events);
        let chars = |text: &str| {
            text.chars()
                .map(|c| (KeyCode::Char(c), N))
                .collect::<Vec<_>>()
        };
        assert_eq!(events, keys(&chars("[a]x[")));
        // Not due: typed.
        decoder.set_replies_due(false);
        let mut events = Vec::new();
        decoder.feed(b"[?0u", &mut events);
        assert_eq!(events, keys(&chars("[?0u")));
    }

    /// What may go on from a kept ESC after a while: after a lone ESC, a
    /// sequence's introducer; after more, only the rest of a reply. A kept
    /// ESC released is the Escape key, and what was kept after it is
    /// decoded on its own.
    #[test]
    fn what_goes_on_from_a_kept_escape_after_a_while() {
        let mut decoder = Decoder::new();
        decoder.set_replies_due(true);
        let mut events = Vec::new();
        assert_eq!(decoder.kept_escape_goes_on(b"w"), None, "nothing kept");
        decoder.feed(b"\x1b", &mut events);
        let goes_on: [&[u8]; 9] = [
            b"", b"[", b"[?0", b"[A", b"[I", b"[200~", b"]11;", b"]2", b"OP",
        ];
        for bytes in goes_on {
            assert_eq!(decoder.kept_escape_goes_on(bytes), Some(true), "{bytes:?}");
        }
        for bytes in [&b"w"[..], b"\r", b"\x1b", b"\x1b[A", b"\x1b[?62c"] {
            assert_eq!(decoder.kept_escape_goes_on(bytes), Some(false), "{bytes:?}");
        }
        decoder.feed(b"[", &mut events);
        for bytes in [&b"?"[..], b"?0", b"?0u", b"?62;1c", b"?2026;2$y"] {
            assert_eq!(decoder.kept_escape_goes_on(bytes), Some(true), "{bytes:?}");
        }
        for bytes in [&b"x"[..], b"A", b"I", b"\x1b", b"\x1b[?62c"] {
            assert_eq!(decoder.kept_escape_goes_on(bytes), Some(false), "{bytes:?}");
        }
        decoder.release_escape(&mut events);
        assert_eq!(events, keys(&[(KeyCode::Esc, N)]));
        assert!(decoder.pending(), "the `[` after it may begin a reply");
        decoder.feed(b"x", &mut events);
        assert_eq!(
            events,
            keys(&[
                (KeyCode::Esc, N),
                (KeyCode::Char('['), N),
                (KeyCode::Char('x'), N)
            ])
        );
        // Nothing kept, or no ESC: nothing to release.
        assert_eq!(decoder.kept_escape_goes_on(b"w"), None);
        let mut none = Vec::new();
        decoder.release_escape(&mut none);
        assert_eq!(none, []);
    }

    fn keys(list: &[(KeyCode, Mods)]) -> Vec<Event> {
        list.iter()
            .map(|&(code, mods)| Event::Key(Key::with(code, mods)))
            .collect()
    }

    const N: Mods = Mods::NONE;

    #[test]
    fn text_and_control_bytes() {
        assert_eq!(
            decode("aé漢😀".as_bytes()),
            keys(&[
                (KeyCode::Char('a'), N),
                (KeyCode::Char('é'), N),
                (KeyCode::Char('漢'), N),
                (KeyCode::Char('😀'), N),
            ])
        );
        assert_eq!(
            decode(b"\r\n\t\x7f\x08\x00\x03\x1c"),
            keys(&[
                (KeyCode::Enter, N),
                (KeyCode::Char('j'), Mods::CTRL),
                (KeyCode::Tab, N),
                (KeyCode::Backspace, N),
                (KeyCode::Backspace, Mods::CTRL),
                (KeyCode::Char(' '), Mods::CTRL),
                (KeyCode::Char('c'), Mods::CTRL),
                (KeyCode::Char('\\'), Mods::CTRL),
            ])
        );
    }

    #[test]
    fn sequences_for_the_editing_and_function_keys() {
        let cases: &[(&[u8], KeyCode, Mods)] = &[
            (b"\x1b[A", KeyCode::Up, N),
            (b"\x1bOB", KeyCode::Down, N),
            (b"\x1b[1;5C", KeyCode::Right, Mods::CTRL),
            (b"\x1b[1;3D", KeyCode::Left, Mods::ALT),
            (b"\x1b[H", KeyCode::Home, N),
            (b"\x1b[4~", KeyCode::End, N),
            (b"\x1b[3;2~", KeyCode::Delete, Mods::SHIFT),
            (b"\x1b[5~", KeyCode::PageUp, N),
            (b"\x1b[6~", KeyCode::PageDown, N),
            (b"\x1b[2~", KeyCode::Insert, N),
            (b"\x1bOP", KeyCode::F(1), N),
            (b"\x1b[1;2S", KeyCode::F(4), Mods::SHIFT),
            (b"\x1b[15~", KeyCode::F(5), N),
            (b"\x1b[24~", KeyCode::F(12), N),
            (b"\x1b[Z", KeyCode::Tab, Mods::SHIFT),
            (b"\x1b[13;2u", KeyCode::Enter, Mods::SHIFT),
            (b"\x1b[97;5u", KeyCode::Char('a'), Mods::CTRL),
            (b"\x1b[27;2;13~", KeyCode::Enter, Mods::SHIFT),
            (b"\x1b[1;16A", KeyCode::Up, Mods(15)),
        ];
        for (bytes, code, mods) in cases {
            assert_eq!(decode(bytes), keys(&[(*code, *mods)]), "{bytes:?}");
        }
    }

    /// The enhanced protocol's event types (since the review of step 23): a
    /// press (kind 1, or none) is a key, a repeat (2) is a repeat, which a
    /// dialog never takes, and a release (3) is nothing.
    #[test]
    fn a_repeat_is_told_apart_and_a_release_dropped() {
        let ctrl_a = Key::with(KeyCode::Char('a'), Mods::CTRL);
        assert_eq!(decode(b"\x1b[97;5:1u"), [Event::Key(ctrl_a)]);
        assert_eq!(decode(b"\x1b[97;5:2u"), [Event::Repeat(ctrl_a)]);
        assert_eq!(decode(b"\x1b[97;5:3u"), []);
        assert_eq!(
            decode(b"\x1b[27;1:2u\x1b[27;1:3u"),
            [Event::Repeat(Key::plain(KeyCode::Esc))]
        );
    }

    /// With the protocol's flag 2, the keys with no text keep their legacy
    /// forms and carry the event type in them too: a release is not a
    /// second press (the review of these fixes found Up moving two history
    /// entries, Delete taking two characters).
    #[test]
    fn a_legacy_keys_release_is_dropped_and_its_repeat_told_apart() {
        let up = Key::plain(KeyCode::Up);
        let delete = Key::plain(KeyCode::Delete);
        let f5 = Key::with(KeyCode::F(5), Mods::SHIFT);
        for (bytes, events) in [
            (&b"\x1b[1;1:1A\x1b[1;1:3A"[..], vec![Event::Key(up)]),
            (
                b"\x1b[1;1:2A\x1b[1;1:2A\x1b[1;1:3A",
                vec![Event::Repeat(up), Event::Repeat(up)],
            ),
            (b"\x1b[3;1:1~\x1b[3;1:3~", vec![Event::Key(delete)]),
            (b"\x1b[3;1:2~", vec![Event::Repeat(delete)]),
            (b"\x1b[15;2:3~\x1b[15;2~", vec![Event::Key(f5)]),
            (b"\x1b[1;5:3H", vec![]),
        ] {
            assert_eq!(decode(bytes), events, "{bytes:?}");
        }
    }

    /// Win32 input: a key that goes down again with no record of it going
    /// up is held, repeating; a modifier going down and up meanwhile changes
    /// nothing.
    #[test]
    fn a_win32_key_down_again_before_it_went_up_is_a_repeat() {
        let down = |vk: u32, unit: u32| format!("\x1b[{vk};0;{unit};1;0;1_");
        let up = |vk: u32, unit: u32| format!("\x1b[{vk};0;{unit};0;0;1_");
        let a = Key::plain(KeyCode::Char('a'));
        let held = [down(0x41, 97), down(0x41, 97), down(0x41, 97), up(0x41, 97)].concat();
        assert_eq!(
            decode(held.as_bytes()),
            [Event::Key(a), Event::Repeat(a), Event::Repeat(a)]
        );
        let pressed_twice = [down(0x41, 97), up(0x41, 97), down(0x41, 97), up(0x41, 97)].concat();
        assert_eq!(
            decode(pressed_twice.as_bytes()),
            [Event::Key(a), Event::Key(a)]
        );
        let shifted = [down(0x41, 97), down(0x10, 0), up(0x10, 0), down(0x41, 97)].concat();
        assert_eq!(
            decode(shifted.as_bytes()),
            [Event::Key(a), Event::Repeat(a)]
        );
    }

    #[test]
    fn alt_is_an_escape_before_the_key() {
        assert_eq!(
            decode(b"\x1bx\x1b\r\x1b\x7f\x1b\x1b"),
            keys(&[
                (KeyCode::Char('x'), Mods::ALT),
                (KeyCode::Enter, Mods::ALT),
                (KeyCode::Backspace, Mods::ALT),
                (KeyCode::Esc, Mods::ALT),
            ])
        );
        assert_eq!(
            decode("\x1bé".as_bytes()),
            keys(&[(KeyCode::Char('é'), Mods::ALT)])
        );
    }

    #[test]
    fn a_lone_escape_waits_and_is_the_escape_key_when_nothing_follows() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(b"\x1b", &mut events);
        assert!(events.is_empty());
        assert!(decoder.pending());
        decoder.flush(&mut events);
        assert_eq!(events, keys(&[(KeyCode::Esc, N)]));
        assert!(!decoder.pending());
        // A sequence that never ends is Alt with its introducer, and what
        // followed is decoded as it stands.
        assert_eq!(
            decode(b"\x1b[12"),
            keys(&[
                (KeyCode::Char('['), Mods::ALT),
                (KeyCode::Char('1'), N),
                (KeyCode::Char('2'), N)
            ])
        );
    }

    #[test]
    fn releases_and_unknown_sequences_are_dropped_whole() {
        assert_eq!(decode(b"\x1b[97;1:3u"), vec![]);
        assert_eq!(decode(b"\x1b[57441;2u"), vec![]);
        assert_eq!(decode(b"\x1b[?9999z"), vec![]);
        assert_eq!(decode(b"\x1b[5x"), vec![]);
        assert_eq!(decode(b"\x1bP1$r0m\x1b\\"), vec![]);
        assert_eq!(
            decode(b"a\x1b[201~b"),
            keys(&[(KeyCode::Char('a'), N), (KeyCode::Char('b'), N)])
        );
        // A run of parameter bytes longer than any terminal sends is not
        // kept waiting for an end that will not come: the first part of it
        // is dropped.
        let mut long = b"\x1b[".to_vec();
        long.extend(std::iter::repeat_n(b'1', 400));
        let events = decode(&long);
        assert!(events.len() < 200, "{}", events.len());
    }

    #[test]
    fn a_paste_is_one_event_whatever_it_holds() {
        assert_eq!(
            decode(b"\x1b[200~line one\r\nline\x1b[Atwo\r\x1b[201~x"),
            vec![
                Event::Paste("line one\nline\u{1b}[Atwo\n".to_owned()),
                Event::Key(Key::plain(KeyCode::Char('x')))
            ]
        );
        // Waiting for the end of a paste is not something to flush.
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(b"\x1b[200~abc\x1b[20", &mut events);
        decoder.flush(&mut events);
        assert!(events.is_empty());
        assert!(!decoder.pending());
        decoder.feed(b"1~", &mut events);
        assert_eq!(events, vec![Event::Paste("abc".to_owned())]);
    }

    /// `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`, as Windows Terminal sends it.
    fn record(vk: u32, unit: u32, down: bool, state: u32) -> String {
        format!("\x1b[{vk};0;{unit};{};{state};1_", u8::from(down))
    }

    #[test]
    fn win32_input_records_carry_every_modifier() {
        let cases: &[(String, KeyCode, Mods)] = &[
            (record(0x0d, 13, true, SHIFT), KeyCode::Enter, Mods::SHIFT),
            (
                record(0x0d, 13, true, LEFT_CTRL),
                KeyCode::Enter,
                Mods::CTRL,
            ),
            (record(0x09, 9, true, LEFT_CTRL), KeyCode::Tab, Mods::CTRL),
            (record(0x09, 9, true, SHIFT), KeyCode::Tab, Mods::SHIFT),
            (
                record(0x41, 1, true, LEFT_CTRL | SHIFT),
                KeyCode::Char('a'),
                Mods::CTRL | Mods::SHIFT,
            ),
            (
                record(0x41, 1, true, RIGHT_CTRL),
                KeyCode::Char('a'),
                Mods::CTRL,
            ),
            (
                record(0x58, u32::from('x'), true, LEFT_ALT),
                KeyCode::Char('x'),
                Mods::ALT,
            ),
            (
                record(0x41, u32::from('A'), true, SHIFT),
                KeyCode::Char('A'),
                Mods::NONE,
            ),
            // AltGr made the character: no modifier is left.
            (
                record(0x51, u32::from('@'), true, RIGHT_ALT | LEFT_CTRL),
                KeyCode::Char('@'),
                Mods::NONE,
            ),
            (
                record(0x20, 0, true, LEFT_CTRL),
                KeyCode::Char(' '),
                Mods::CTRL,
            ),
            (
                record(0x32, 0, true, LEFT_CTRL | SHIFT),
                KeyCode::Char('2'),
                Mods::CTRL | Mods::SHIFT,
            ),
            (
                record(0x26, 0, true, LEFT_ALT | 0x100),
                KeyCode::Up,
                Mods::ALT,
            ),
            (record(0x71, 0, true, 0), KeyCode::F(2), Mods::NONE),
            (record(0x1b, 27, true, 0), KeyCode::Esc, Mods::NONE),
            (
                record(0x08, 8, true, LEFT_CTRL),
                KeyCode::Backspace,
                Mods::CTRL,
            ),
            // Caps Lock and Num Lock are not modifiers.
            (
                record(0x41, u32::from('A'), true, 0x80 | 0x20),
                KeyCode::Char('A'),
                Mods::NONE,
            ),
        ];
        for (bytes, code, mods) in cases {
            assert_eq!(
                decode(bytes.as_bytes()),
                keys(&[(*code, *mods)]),
                "{bytes:?}"
            );
        }
    }

    /// Where Backspace sends `^H` (step 25, `backspace-sent-as-ctrl-h`),
    /// `ESC ^H` is `Alt+Backspace`, which deletes a word, as `ESC DEL` is
    /// elsewhere; without the choice it stays `Ctrl+Alt+Backspace`. A win32
    /// input record says which key it was, either way: the choice does not
    /// touch one (the review of step 25's second pull request).
    #[test]
    fn with_ctrl_h_as_backspace_esc_ctrl_h_is_alt_backspace_and_records_are_untouched() {
        let with = |on: bool, bytes: &[u8]| {
            let mut decoder = Decoder::new();
            decoder.set_ctrl_h_backspace(on);
            let mut events = Vec::new();
            decoder.feed(bytes, &mut events);
            decoder.flush(&mut events);
            events
        };
        assert_eq!(
            with(true, b"\x1b\x08\x08"),
            keys(&[(KeyCode::Backspace, Mods::ALT), (KeyCode::Backspace, N)])
        );
        assert_eq!(
            with(false, b"\x1b\x08"),
            keys(&[(KeyCode::Backspace, Mods::CTRL | Mods::ALT)])
        );
        for on in [true, false] {
            let records = format!(
                "{}{}{}",
                record(0x08, 8, true, LEFT_CTRL),
                record(0x08, 8, false, LEFT_CTRL),
                record(0x08, 8, true, 0),
            );
            assert_eq!(
                with(on, records.as_bytes()),
                keys(&[(KeyCode::Backspace, Mods::CTRL), (KeyCode::Backspace, N)]),
                "{on}"
            );
        }
    }

    #[test]
    fn win32_input_releases_and_lone_modifiers_are_dropped() {
        let mut bytes = String::new();
        bytes.push_str(&record(0x10, 0, true, SHIFT));
        bytes.push_str(&record(0x41, u32::from('A'), true, SHIFT));
        bytes.push_str(&record(0x41, u32::from('A'), false, SHIFT));
        bytes.push_str(&record(0x10, 0, false, 0));
        bytes.push_str(&record(0x11, 0, true, LEFT_CTRL));
        assert_eq!(decode(bytes.as_bytes()), keys(&[(KeyCode::Char('A'), N)]));
    }

    #[test]
    fn win32_input_joins_the_halves_of_a_character() {
        let mut bytes = String::new();
        for unit in "😀".encode_utf16() {
            bytes.push_str(&record(0, u32::from(unit), true, 0));
        }
        bytes.push('b');
        assert_eq!(
            decode(bytes.as_bytes()),
            keys(&[(KeyCode::Char('😀'), N), (KeyCode::Char('b'), N)])
        );
        // A second half alone is nothing.
        assert_eq!(decode(record(0, 0xdc00, true, 0).as_bytes()), vec![]);
    }

    #[test]
    fn a_long_paste_comes_in_pieces_and_loses_nothing() {
        let line = "a line of pasted text, é\r\n";
        let count = PASTE_PIECE / line.len() * 2 + 7;
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(b"\x1b[200~", &mut events);
        let text = line.repeat(count);
        for piece in text.as_bytes().chunks(65_537) {
            decoder.feed(piece, &mut events);
        }
        assert!(events.len() >= 2, "{}", events.len());
        decoder.feed(b"\x1b[201~x", &mut events);
        let mut pasted = String::new();
        for event in &events[..events.len() - 1] {
            match event {
                Event::Paste(piece) => pasted.push_str(piece),
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(pasted, text.replace("\r\n", "\n"));
        assert_eq!(
            events.last(),
            Some(&Event::Key(Key::plain(KeyCode::Char('x'))))
        );
    }

    #[test]
    fn a_piece_of_a_paste_never_ends_inside_a_character() {
        for tail in ["😀", "é", "漢", "a\r"] {
            // An invalid byte early on does not move the cut.
            let mut paste = b"x\xffy ".to_vec();
            paste.extend_from_slice(tail.as_bytes());
            let whole = paste.clone();
            for keep in 1..tail.len() {
                let mut piece = whole[..whole.len() - keep].to_vec();
                let text = paste_piece(&mut piece);
                assert!(
                    !text.ends_with('\u{fffd}'),
                    "{tail:?} less {keep}: {text:?}"
                );
                assert_eq!(
                    [text.replace('\u{fffd}', "\u{ff}").as_bytes(), &piece]
                        .concat()
                        .len()
                        - usize::from(text.contains('\u{fffd}')),
                    whole.len() - keep,
                    "{tail:?} less {keep}"
                );
            }
            let text = paste_piece(&mut paste);
            if tail == "a\r" {
                assert_eq!(paste, b"\r");
            } else {
                assert!(text.ends_with(tail), "{text:?}");
                assert!(paste.is_empty());
            }
        }
    }

    #[test]
    fn an_escape_bracket_before_a_byte_no_sequence_holds_is_alt_bracket() {
        assert_eq!(
            decode("\x1b[\ré".as_bytes()),
            keys(&[
                (KeyCode::Char('['), Mods::ALT),
                (KeyCode::Enter, N),
                (KeyCode::Char('é'), N)
            ])
        );
    }

    #[test]
    fn focus_and_replies() {
        assert_eq!(
            decode(b"\x1b[I\x1b[O"),
            vec![Event::Focus(true), Event::Focus(false)]
        );
        assert_eq!(
            decode(b"\x1b[?2026;2$y\x1b[?1u\x1b]11;rgb:1e1e/2020/ffff\x1b\\\x1b]11;rgb:ff/0/8\x07\x1b[?62;22c"),
            vec![
                Event::Reply(Reply::Mode { mode: 2026, state: 2 }),
                Event::Reply(Reply::Keyboard(1)),
                Event::Reply(Reply::Background(0x1e, 0x20, 0xff)),
                Event::Reply(Reply::Background(255, 0, 0x88)),
                Event::Reply(Reply::DeviceAttributes),
            ]
        );
        assert_eq!(decode(b"\x1b]11;rgb:zz/0/0\x07"), vec![]);
    }

    #[test]
    fn invalid_utf8_is_a_replacement_character_and_decoding_goes_on() {
        assert_eq!(
            decode(b"\xffa\xc3(b"),
            keys(&[
                (KeyCode::Char('\u{fffd}'), N),
                (KeyCode::Char('a'), N),
                (KeyCode::Char('\u{fffd}'), N),
                (KeyCode::Char('('), N),
                (KeyCode::Char('b'), N),
            ])
        );
    }

    /// The same bytes decode to the same events however a read cuts them,
    /// into two pieces or three: the property behind "a split UTF-8
    /// character or escape sequence".
    #[test]
    fn every_split_decodes_the_same() {
        let input: &[u8] = "a\x1b[1;5Cé\x1b[200~p\r\nq\x1b[201~漢\x1bOP\x1b]11;rgb:01/02/03\x1b\\😀\x1b[13;2u\x1bx\x1b[?2026;1$y\x1b[13;28;13;1;16;1_"
            .as_bytes();
        let whole = decode(input);
        assert_eq!(whole.len(), 12, "{whole:?}");
        for first in 0..=input.len() {
            for second in first..=input.len() {
                // Every third second cut, which reaches every kind of
                // boundary; the whole set in the nightly run.
                if second % 3 != 0 && !exhaustive() {
                    continue;
                }
                let mut decoder = Decoder::new();
                let mut events = Vec::new();
                decoder.feed(&input[..first], &mut events);
                decoder.feed(&input[first..second], &mut events);
                decoder.feed(&input[second..], &mut events);
                decoder.flush(&mut events);
                assert_eq!(events, whole, "cut at {first} and {second}");
            }
        }
    }

    /// Feeds `input` in two reads cut at `cut`, then `after`, and flushes.
    fn cut_in_two(input: &[u8], cut: usize, after: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(&input[..cut], &mut events);
        decoder.feed(&input[cut..], &mut events);
        decoder.feed(after, &mut events);
        decoder.flush(&mut events);
        events
    }

    #[test]
    fn an_over_long_control_sequence_is_dropped_to_its_end_however_it_is_split() {
        let mut long = b"\x1b[".to_vec();
        long.extend(std::iter::repeat_n(b"1;", 200).flatten());
        long.push(b'm');
        let typed = keys(&[
            (KeyCode::Char('a'), Mods::NONE),
            (KeyCode::Char('b'), Mods::NONE),
        ]);
        // At the limit, and around it, and anywhere else.
        for cut in (LONGEST_SEQUENCE - 3..=LONGEST_SEQUENCE + 3).chain([1, 2, 100, long.len()]) {
            assert_eq!(cut_in_two(&long, cut, b"ab"), typed, "cut at {cut}");
        }
        // A byte that cannot be in a control sequence ends it, and is a key.
        let mut broken = long[..300].to_vec();
        broken.push(b'\r');
        assert_eq!(
            cut_in_two(&broken, LONGEST_SEQUENCE, b""),
            keys(&[(KeyCode::Enter, Mods::NONE)])
        );
    }

    #[test]
    fn an_over_long_string_is_dropped_to_its_end_however_it_is_split() {
        for end in [&b"\x07"[..], b"\x1b\\"] {
            let mut long = b"\x1b]0;".to_vec();
            long.extend(std::iter::repeat_n(b'x', 300));
            // An ESC that does not begin the end is part of the string.
            long.extend_from_slice(b"\x1bz");
            long.extend(std::iter::repeat_n(b'x', 100));
            long.extend_from_slice(end);
            for cut in (LONGEST_SEQUENCE - 2..=LONGEST_SEQUENCE + 2).chain([long.len() - 1]) {
                assert_eq!(
                    cut_in_two(&long, cut, b"q"),
                    keys(&[(KeyCode::Char('q'), Mods::NONE)]),
                    "end {end:?}, cut at {cut}"
                );
            }
        }
    }

    #[test]
    fn giving_up_on_a_paste_keeps_what_came_as_a_paste_and_keys_are_keys_again() {
        let mut decoder = Decoder::new();
        let mut events = Vec::new();
        decoder.feed(b"\x1b[200~line one\r\nline two\x1b[20", &mut events);
        assert!(events.is_empty());
        assert!(decoder.open());
        decoder.give_up(&mut events);
        assert_eq!(events, vec![Event::Paste("line one\nline two".to_owned())]);
        assert!(!decoder.open());
        events.clear();
        decoder.feed(b"\x03", &mut events);
        assert_eq!(events, keys(&[(KeyCode::Char('c'), Mods::CTRL)]));
    }

    /// What `bytes` decode to with the mouse on.
    fn decode_mouse(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Decoder::new();
        decoder.set_mouse(true);
        let mut events = Vec::new();
        decoder.feed(bytes, &mut events);
        decoder.flush(&mut events);
        events
    }

    fn mouse(kind: MouseKind, col: u16, row: u16, mods: Mods) -> Event {
        Event::Mouse(Mouse {
            kind,
            col,
            row,
            mods,
        })
    }

    /// Mode 1006's reports (step 25): a press, a release, a drag, the wheel
    /// four ways, the modifiers, counted from the top left as 0; the buttons
    /// past the third are nothing.
    #[test]
    fn mouse_reports_in_the_sgr_form() {
        assert_eq!(
            decode(b"\x1b[<0;1;1M\x1b[<0;1;1m\x1b[<2;80;24M\x1b[<34;81;24M\x1b[<64;5;6M\x1b[<65;5;6M\x1b[<66;5;6M\x1b[<67;5;6M"),
            vec![
                mouse(MouseKind::Press(Button::Left), 0, 0, N),
                mouse(MouseKind::Release(Some(Button::Left)), 0, 0, N),
                mouse(MouseKind::Press(Button::Right), 79, 23, N),
                mouse(MouseKind::Drag(Button::Right), 80, 23, N),
                mouse(MouseKind::Wheel(Wheel::Up), 4, 5, N),
                mouse(MouseKind::Wheel(Wheel::Down), 4, 5, N),
                mouse(MouseKind::Wheel(Wheel::Left), 4, 5, N),
                mouse(MouseKind::Wheel(Wheel::Right), 4, 5, N),
            ]
        );
        assert_eq!(
            decode(b"\x1b[<4;3;2M\x1b[<8;3;2M\x1b[<16;3;2M\x1b[<1;300;400M\x1b[<35;9;9M"),
            vec![
                mouse(MouseKind::Press(Button::Left), 2, 1, Mods::SHIFT),
                mouse(MouseKind::Press(Button::Left), 2, 1, Mods::ALT),
                mouse(MouseKind::Press(Button::Left), 2, 1, Mods::CTRL),
                mouse(MouseKind::Press(Button::Middle), 299, 399, N),
                mouse(MouseKind::Moved, 8, 8, N),
            ]
        );
        // The eighth button, and a report that names no cell: nothing, and
        // nothing of them is typed.
        assert_eq!(decode(b"\x1b[<128;1;1M\x1b[<0M"), vec![]);
        // A report too large for a cell's number is the last cell.
        assert_eq!(
            decode(b"\x1b[<0;99999;1M"),
            vec![mouse(MouseKind::Press(Button::Left), u16::MAX, 0, N)]
        );
    }

    /// Mode 1000's own form, from a terminal that does not know 1006: taken
    /// whole while the mouse is on, so that none of its bytes is typed, a
    /// release saying no button. With the mouse off `CSI M` is dropped
    /// alone, as before, and what follows it is typed.
    #[test]
    fn mouse_reports_in_the_older_form_are_taken_whole() {
        assert_eq!(
            decode(b"\x1b[Mabc"),
            keys(&[
                (KeyCode::Char('a'), N),
                (KeyCode::Char('b'), N),
                (KeyCode::Char('c'), N)
            ])
        );
        assert_eq!(
            decode_mouse(b"\x1b[M !!\x1b[M#!!\x1b[M`+,a"),
            vec![
                mouse(MouseKind::Press(Button::Left), 0, 0, N),
                mouse(MouseKind::Release(None), 0, 0, N),
                mouse(MouseKind::Wheel(Wheel::Up), 10, 11, N),
                Event::Key(Key::plain(KeyCode::Char('a'))),
            ]
        );
        // Bytes past 127 are values too, not text.
        assert_eq!(
            decode_mouse(b"\x1b[M \xff\xff"),
            vec![mouse(MouseKind::Press(Button::Left), 222, 222, N)]
        );
    }

    /// A mouse report decodes the same however reads split it.
    #[test]
    fn a_mouse_report_decodes_the_same_at_every_split() {
        let input: &[u8] = b"x\x1b[<0;12;7M\x1b[<0;12;7m\x1b[M \"#y";
        let whole = decode_mouse(input);
        assert_eq!(whole.len(), 5, "{whole:?}");
        for cut in 0..=input.len() {
            let mut decoder = Decoder::new();
            decoder.set_mouse(true);
            let mut events = Vec::new();
            decoder.feed(&input[..cut], &mut events);
            decoder.feed(&input[cut..], &mut events);
            decoder.flush(&mut events);
            assert_eq!(events, whole, "cut at {cut}");
        }
    }

    /// Whether the nightly run asked for every case (`CINERIS_PROPERTY_SCALE`
    /// above 1). The test kit has this as `cases::exhaustive`, but unit tests
    /// of this crate cannot depend on it.
    fn exhaustive() -> bool {
        std::env::var("CINERIS_PROPERTY_SCALE")
            .ok()
            .and_then(|scale| scale.parse::<u32>().ok())
            .is_some_and(|scale| scale > 1)
    }
}
