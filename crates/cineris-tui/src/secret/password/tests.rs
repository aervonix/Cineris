use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use cineris_term::Ready;

use super::*;

fn char_key(character: char) -> Key {
    Key::plain(KeyCode::Char(character))
}

fn ctrl(character: char) -> Key {
    Key {
        code: KeyCode::Char(character),
        mods: Mods::CTRL,
    }
}

fn look() -> Look {
    Look::new(
        crate::theme::Theme::new(cineris_term::ColorDepth::TrueColor, None),
        &crate::glyphs::Glyphs::UNICODE,
    )
}

fn sign() -> PasswordAsk {
    PasswordAsk::new(
        Purpose::Sign,
        vec!["the extension `fmt-helper`, as it is now".to_owned()],
    )
}

fn choose() -> PasswordAsk {
    PasswordAsk::new(Purpose::Choose, Vec::new())
}

/// Types `text` into `prompt`, a key a character.
fn type_into(prompt: &mut PasswordPrompt, text: &str) {
    for character in text.chars() {
        assert!(
            matches!(prompt.key(char_key(character)), Prompted::Changed),
            "{character:?}"
        );
    }
}

/// The password `prompt` gives for `Enter`, as text.
fn entered(prompt: &mut PasswordPrompt) -> Option<String> {
    match prompt.key(Key::plain(KeyCode::Enter)) {
        Prompted::Done(Some(password)) => {
            Some(String::from_utf8(password.expose_bytes().to_vec()).expect("the password's text"))
        }
        Prompted::Done(None) => panic!("answered with nothing"),
        Prompted::Changed | Prompted::Nothing => None,
    }
}

/// Everything `prompt` draws, as text.
fn drawn(prompt: &PasswordPrompt) -> String {
    prompt
        .rows(70, 40, true, &look())
        .lines
        .iter()
        .map(|line| line.text().into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn typing_backspace_and_ctrl_u_change_it_and_nothing_else_does() {
    let mut input = PasswordInput::new();
    for character in "pw-1".chars() {
        assert_eq!(input.key(char_key(character)), KeyEdit::Changed);
    }
    assert_eq!(input.chars(), 4);
    assert_eq!(input.key(Key::plain(KeyCode::Backspace)), KeyEdit::Changed);
    assert_eq!(input.chars(), 3);
    for nothing in [
        ctrl('g'),
        ctrl('y'),
        ctrl('w'),
        Key {
            code: KeyCode::Char('b'),
            mods: Mods::ALT,
        },
        Key::plain(KeyCode::Up),
        Key::plain(KeyCode::Left),
        Key::plain(KeyCode::Tab),
        Key::plain(KeyCode::Delete),
        Key {
            code: KeyCode::Enter,
            mods: Mods::SHIFT,
        },
    ] {
        assert_eq!(input.key(nothing), KeyEdit::Nothing, "{nothing:?}");
    }
    assert_eq!(input.chars(), 3);
    assert_eq!(input.key(ctrl('u')), KeyEdit::Changed);
    assert_eq!(input.chars(), 0);
    assert!(input.wiped());
    assert_eq!(input.key(Key::plain(KeyCode::Enter)), KeyEdit::Submit);
    assert_eq!(input.key(Key::plain(KeyCode::Esc)), KeyEdit::Cancel);
}

/// A password is any text but control characters, typed or pasted, and
/// it is handed on as it was typed: spaces at its ends and inside it are
/// its own.
#[test]
fn a_password_is_not_trimmed_and_may_be_any_text_but_control_characters() {
    let mut prompt = PasswordPrompt::new(sign());
    type_into(&mut prompt, " Ж é 🙂 ");
    assert_eq!(prompt.chars(), 7);
    assert_eq!(entered(&mut prompt).as_deref(), Some(" Ж é 🙂 "));
    assert!(prompt.wiped(), "taken, the field is written over");
    assert!(matches!(prompt.paste("  two  words\t"), Prompted::Changed));
    assert_eq!(prompt.chars(), 0, "a tab refuses it whole");
    assert!(drawn(&prompt).contains("control character"));
    assert!(matches!(prompt.paste("  two  words  "), Prompted::Changed));
    assert!(
        !drawn(&prompt).contains("control character"),
        "a change clears it"
    );
    assert_eq!(entered(&mut prompt).as_deref(), Some("  two  words  "));
}

/// One line break at the very end of a paste is dropped, whichever
/// system's it is; one anywhere else, or a second at the end, refuses it
/// whole.
#[test]
fn a_pasted_line_loses_its_one_line_break_at_the_end_and_one_inside_refuses_it() {
    let mut input = PasswordInput::new();
    for (pasted, kept) in [
        ("pw-a\n", "pw-a"),
        ("pw-b\r\n", "pw-b"),
        ("pw-c\r", "pw-c"),
        (" pw d \n", " pw d "),
    ] {
        assert_eq!(input.paste(pasted), KeyEdit::Changed, "{pasted:?}");
        let password = input.take().expect("a password");
        assert_eq!(password.expose_bytes(), kept.as_bytes(), "{pasted:?}");
    }
    assert_eq!(input.paste("pw-"), KeyEdit::Changed);
    for refused in ["e\n\n", "e\r\n\r\n", "\ne", "e\nf", "e\r\nf\n"] {
        assert_eq!(input.paste(refused), KeyEdit::Refused, "{refused:?}");
        assert!(
            input
                .refusal()
                .is_some_and(|why| why.contains("line break")),
            "{refused:?}"
        );
    }
    for refused in ["a\tb", "\u{1b}[2J", "a\u{7f}", "\u{9b}1m", "a\0"] {
        assert_eq!(input.paste(refused), KeyEdit::Refused, "{refused:?}");
        assert!(
            input
                .refusal()
                .is_some_and(|why| why.contains("control character")),
            "{refused:?}"
        );
    }
    assert_eq!(input.chars(), 3, "nothing of what was refused was kept");
}

#[test]
fn a_paste_past_the_capacity_is_refused_whole() {
    let mut input = PasswordInput::with_capacity(16);
    assert_eq!(input.paste("0123456789abcdefg"), KeyEdit::Refused);
    assert!(input.refusal().is_some_and(|why| why.contains("longer")));
    assert_eq!(input.chars(), 0);
    assert!(input.wiped());
    assert_eq!(input.paste("0123456789abcdef"), KeyEdit::Changed);
    assert_eq!(input.key(char_key('x')), KeyEdit::Refused);
    assert_eq!(input.chars(), 16);
    let password = input.take().expect("a password");
    assert_eq!(password.chars(), 16);
    assert!(input.wiped());
    assert_eq!(
        input.paste("0123456789abcdef"),
        KeyEdit::Changed,
        "the same room"
    );
}

/// `Enter` hands the password on as it is, and the field it was typed in
/// is left with every byte zero; with nothing typed it says so and goes
/// on.
#[test]
fn enter_hands_the_password_on_and_leaves_the_field_wiped() {
    let mut prompt = PasswordPrompt::new(sign());
    assert_eq!(entered(&mut prompt), None);
    assert!(drawn(&prompt).contains("Nothing was typed"));
    type_into(&mut prompt, "correct horse");
    assert!(
        !drawn(&prompt).contains("Nothing was typed"),
        "a change clears it"
    );
    assert!(!prompt.wiped());
    assert_eq!(entered(&mut prompt).as_deref(), Some("correct horse"));
    assert!(prompt.wiped());
}

/// `Esc` and `Ctrl+C` answer with nothing and leave nothing: the field
/// and a chosen password's first typing are wiped, at either typing.
#[test]
fn esc_and_ctrl_c_answer_with_nothing_and_wipe_everything() {
    for way in [Key::plain(KeyCode::Esc), ctrl('c')] {
        let mut prompt = PasswordPrompt::new(sign());
        type_into(&mut prompt, "half typed");
        assert!(matches!(prompt.key(way), Prompted::Done(None)), "{way:?}");
        assert!(prompt.wiped(), "{way:?}");

        let mut prompt = PasswordPrompt::new(choose());
        type_into(&mut prompt, "the first typing");
        assert_eq!(entered(&mut prompt), None);
        assert!(prompt.again());
        type_into(&mut prompt, "second");
        assert!(matches!(prompt.key(way), Prompted::Done(None)), "{way:?}");
        assert!(prompt.wiped(), "{way:?}: the first typing is dropped too");
        assert!(!prompt.again());
    }
}

/// A password chosen has at least twelve characters, counted as Unicode
/// scalar values, not bytes: eleven are refused, twelve are kept, whatever
/// their script.
#[test]
fn a_chosen_password_has_at_least_twelve_characters() {
    assert_eq!(Password::LEAST, 12);
    let mut prompt = PasswordPrompt::new(choose());
    // It says why, before and after a short one.
    assert!(drawn(&prompt).contains("at least 12 characters"));
    assert!(drawn(&prompt).contains("offline"));
    for short in ["abcdefghijk", "🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂🙂"] {
        type_into(&mut prompt, short);
        assert_eq!(entered(&mut prompt), None, "{short}");
        assert!(!prompt.again(), "{short}: asked again");
        let shown = drawn(&prompt);
        assert!(
            shown.contains("at least 12 characters, and that one"),
            "{shown}"
        );
        assert!(shown.contains("offline"), "{shown}");
        assert!(prompt.wiped(), "{short}");
    }
    for long in ["abcdefghijkl", "äöüßéèêëîïôû"] {
        type_into(&mut prompt, long);
        assert_eq!(entered(&mut prompt), None, "{long}: typed once");
        assert!(prompt.again(), "{long}");
        assert!(drawn(&prompt).contains("Type the same password again"));
        type_into(&mut prompt, long);
        assert_eq!(entered(&mut prompt).as_deref(), Some(long));
        assert!(prompt.wiped());
        prompt = PasswordPrompt::new(choose());
    }
    // Unlocking and signing take what is typed: it is the one chosen, or
    // it is wrong.
    let mut prompt = PasswordPrompt::new(sign());
    type_into(&mut prompt, "short");
    assert_eq!(entered(&mut prompt).as_deref(), Some("short"));
}

/// The second typing of a password chosen must be the first: when it is
/// not, both are dropped, it says so, and the choice starts again.
#[test]
fn a_chosen_password_typed_differently_the_second_time_is_asked_again() {
    let mut prompt = PasswordPrompt::new(choose());
    type_into(&mut prompt, "abcdefghijkl");
    assert_eq!(entered(&mut prompt), None);
    assert!(prompt.again());
    type_into(&mut prompt, "abcdefghijkX");
    assert_eq!(entered(&mut prompt), None);
    assert!(!prompt.again(), "back to the first typing");
    assert!(prompt.wiped(), "both typings are dropped");
    let shown = drawn(&prompt);
    assert!(shown.contains("were not the same"), "{shown}");
    assert!(shown.contains("Choose a password"), "{shown}");
    type_into(&mut prompt, "abcdefghijkX");
    assert_eq!(entered(&mut prompt), None);
    type_into(&mut prompt, "abcdefghijkX");
    assert_eq!(entered(&mut prompt).as_deref(), Some("abcdefghijkX"));
}

/// A key held down types and erases, as a key pressed does; held, `Enter`
/// and `Esc` answer nothing.
#[test]
fn a_held_key_types_and_erases_and_answers_nothing() {
    let mut prompt = PasswordPrompt::new(sign());
    type_into(&mut prompt, "ab");
    assert!(matches!(prompt.repeat(char_key('b')), Prompted::Changed));
    assert_eq!(prompt.chars(), 3);
    assert!(matches!(
        prompt.repeat(Key::plain(KeyCode::Backspace)),
        Prompted::Changed
    ));
    assert_eq!(prompt.chars(), 2);
    for held in [
        Key::plain(KeyCode::Enter),
        Key::plain(KeyCode::Esc),
        ctrl('c'),
    ] {
        assert!(matches!(prompt.repeat(held), Prompted::Nothing), "{held:?}");
    }
    assert_eq!(prompt.chars(), 2);
}

/// Its rows, its `Debug` and the password's say how many characters it
/// has and never one of them; the caller's lines are drawn whole, made
/// safe; the field is drawn ready only when shown and when every row fits.
#[test]
fn its_rows_hold_no_character_of_it_and_the_callers_lines_whole() {
    let ask = PasswordAsk::new(
        Purpose::Sign,
        vec![
            "the project `/work/app`\u{1b}[2J, trusted with its files' rules and its commands \
             as they are now, to be asked again when any of them changes"
                .to_owned(),
        ],
    )
    .warning("That was not the approvals password.");
    let mut prompt = PasswordPrompt::new(ask);
    type_into(&mut prompt, "QQSECRETQQ");
    let rows = prompt.rows(60, 40, true, &look());
    assert!(rows.field);
    let shown: String = rows
        .lines
        .iter()
        .map(|line| line.text().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !shown.contains("SECRET") && !shown.contains("QQ"),
        "{shown}"
    );
    assert!(shown.contains("10 characters, hidden"), "{shown}");
    assert!(shown.contains("to approve:"), "{shown}");
    assert!(shown.contains("changes"), "the line is never cut: {shown}");
    assert!(shown.contains("not the approvals password"), "{shown}");
    assert!(!shown.contains('\u{1b}'), "{shown}");
    assert_eq!(
        crate::testing::problems(&rows.lines, 60),
        Vec::<String>::new()
    );
    let printed = format!("{prompt:?}");
    assert!(!printed.contains("SECRET"), "{printed}");

    // Not shown yet: no count, no keys.
    let hidden = prompt.rows(60, 40, false, &look());
    assert!(!hidden.field);
    assert!(prompt.keys(false).is_empty());
    assert!(
        !hidden
            .lines
            .iter()
            .any(|line| line.text().contains("characters"))
    );
    // Too few rows for it all: a sentence says so, and no key is taken.
    let small = prompt.rows(60, 5, true, &look());
    assert!(!small.field);
    assert!(small.lines.len() <= 5);
    assert!(
        small
            .lines
            .iter()
            .any(|line| line.text().contains("too small"))
    );
}

/// The sentence and the keys name what is asked, for each purpose.
#[test]
fn each_purpose_says_what_it_asks_and_what_its_keys_do() {
    let look = look();
    let first = |ask: PasswordAsk| {
        let prompt = PasswordPrompt::new(ask);
        let rows = prompt.rows(80, 40, true, &look);
        (
            rows.lines[0].text().trim().to_owned(),
            prompt.keys(rows.field).join(", "),
        )
    };
    let (unlock, keys) = first(PasswordAsk::new(Purpose::Unlock, Vec::new()));
    assert!(
        unlock.contains("to use this session's approvals"),
        "{unlock}"
    );
    assert!(keys.contains("esc goes on without them"), "{keys}");
    let (signing, keys) = first(sign());
    assert!(signing.contains("to approve:"), "{signing}");
    assert!(keys.contains("enter approves"), "{keys}");
    let (choosing, keys) = first(choose().warning("Every approval will be dropped."));
    assert!(choosing.contains("Choose a password"), "{choosing}");
    assert!(keys.contains("esc keeps nothing"), "{keys}");
}

// ------------------------------------------------------------ at a terminal

/// What the person does at a [`Typed`] terminal, in order.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// Typed once the prompt waits for nothing but input: the person
    /// waited for the field. Until then each wait with an end runs out.
    Typed(&'static [u8]),
    /// Typed at once, whatever the prompt is waiting for: too early.
    Hasty(&'static [u8]),
    /// The window's size changed (to the size the device says).
    Resized,
}

const ENTER: Step = Step::Typed(b"\r");

/// A terminal for [`read_password_on`]'s tests, on a clock of its own.
/// What was typed before the prompt has arrived already: any wait takes
/// it, one of no time included. Then the person's steps, each to a wait
/// that may take time: a wait with an end that a step does not answer
/// runs out, the clock moved by its length. It keeps what was written,
/// how much of it there was at each step, and the terminal's exclusive
/// mode as each step found it.
struct Typed {
    size: (u16, u16),
    ahead: VecDeque<Vec<u8>>,
    later: VecDeque<Step>,
    seen: Rc<RefCell<Seen>>,
    exclusive: bool,
    base: Instant,
    elapsed: Duration,
}

/// What a [`Typed`] terminal saw.
#[derive(Default)]
struct Seen {
    written: Vec<u8>,
    /// How much had been written when each step was taken.
    steps: Vec<usize>,
    /// Whether the exclusive mode was on when each step was taken.
    exclusive: Vec<bool>,
    /// Every change of the exclusive mode, in order.
    modes: Vec<bool>,
}

impl Device for Typed {
    fn size(&self) -> io::Result<(u16, u16)> {
        Ok(self.size)
    }
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.seen.borrow_mut().written.extend_from_slice(bytes);
        Ok(())
    }
    fn wait(&mut self, timeout: Option<Duration>, into: &mut Vec<u8>) -> io::Result<Ready> {
        if let Some(bytes) = self.ahead.pop_front() {
            self.elapsed += Duration::from_millis(1);
            into.extend_from_slice(&bytes);
            return Ok(Ready::Input);
        }
        // Nothing more has arrived: a wait of no time ends at once.
        if timeout == Some(Duration::ZERO) {
            return Ok(Ready::Timeout);
        }
        let step = match (self.later.front(), timeout) {
            (Some(Step::Typed(_)) | None, Some(timeout)) => {
                self.elapsed += timeout;
                return Ok(Ready::Timeout);
            }
            (None, None) => return Err(io::Error::other("the script has ended")),
            (Some(_), _) => self.later.pop_front(),
        };
        let mut seen = self.seen.borrow_mut();
        let written = seen.written.len();
        seen.steps.push(written);
        seen.exclusive.push(self.exclusive);
        self.elapsed += Duration::from_millis(1);
        match step {
            Some(Step::Typed(bytes) | Step::Hasty(bytes)) => {
                into.extend_from_slice(bytes);
                Ok(Ready::Input)
            }
            Some(Step::Resized) => Ok(Ready::Resize),
            None => Err(io::Error::other("the script has ended")),
        }
    }
    fn waker(&self) -> cineris_term::Waker {
        cineris_term::Waker::new(|| {})
    }
    fn suspend(&mut self) {}
    fn leave(&mut self) {}
    fn now(&self) -> Instant {
        self.base + self.elapsed
    }
    fn exclusive(&mut self, on: bool) -> io::Result<()> {
        self.exclusive = on;
        self.seen.borrow_mut().modes.push(on);
        Ok(())
    }
}

/// What the prompt read, and what the terminal saw.
struct Read {
    read: io::Result<Option<Password>>,
    seen: Seen,
    size: (u16, u16),
    /// Whether the terminal was still closed to others once it was put
    /// back.
    exclusive_after: bool,
}

impl Read {
    /// The password read, as text.
    fn password(&self) -> Option<String> {
        match &self.read {
            Ok(Some(password)) => Some(
                String::from_utf8(password.expose_bytes().to_vec()).expect("the password's text"),
            ),
            Ok(None) => None,
            Err(error) => panic!("the read failed: {error}"),
        }
    }

    /// The screen the person saw as they took step `n`.
    fn before(&self, n: usize) -> cineris_testkit::VTerm {
        shown(&self.seen.written[..self.seen.steps[n]], self.size)
    }

    /// The screen once the prompt ended.
    fn after(&self) -> cineris_testkit::VTerm {
        shown(&self.seen.written, self.size)
    }
}

/// Asks `ask` at an 80 by 24 terminal, with `ahead` typed before the
/// prompt, and `later` done once it is there.
fn asked(ask: &PasswordAsk, ahead: &[&[u8]], later: &[Step]) -> Read {
    asked_on((80, 24), ask, ahead, later)
}

fn asked_on(size: (u16, u16), ask: &PasswordAsk, ahead: &[&[u8]], later: &[Step]) -> Read {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let device = Typed {
        size,
        ahead: ahead.iter().map(|bytes| bytes.to_vec()).collect(),
        later: later.iter().copied().collect(),
        seen: Rc::clone(&seen),
        exclusive: false,
        base: Instant::now(),
        elapsed: Duration::ZERO,
    };
    let mut terminal = Terminal::with_device(device, crate::testkit::caps()).expect("a terminal");
    let read = read_password_on(&mut terminal, ask);
    let exclusive_after = terminal.is_exclusive();
    // Taken before the terminal is dropped, which writes its end.
    let seen = std::mem::take(&mut *seen.borrow_mut());
    drop(terminal);
    Read {
        read,
        seen,
        size,
        exclusive_after,
    }
}

/// The screen `bytes` leave on a terminal of `size`.
fn shown(bytes: &[u8], size: (u16, u16)) -> cineris_testkit::VTerm {
    let mut term = cineris_testkit::VTerm::new(size.0, size.1);
    term.feed(bytes);
    term
}

/// Whatever was typed or pasted before the prompt was drawn does not
/// count: typing and its `Enter`, a paste, `Ctrl+C`, and an `Esc` the
/// terminal held for what might follow it. Only what is typed once the
/// field is there is the password.
#[test]
fn at_a_terminal_type_ahead_before_the_prompt_is_dropped() {
    let read = asked(
        &sign(),
        &[
            b"hunter22",
            b"\r",
            b"\x1b[200~pasted-ahead\x1b[201~",
            b"\x03",
            b"\x1b",
        ],
        &[Step::Typed(b"s3cret-pw"), ENTER],
    );
    assert_eq!(read.password().as_deref(), Some("s3cret-pw"));
    let before = read.before(0);
    assert!(before.shows("0 characters, hidden"), "{}", before.dump());
    let typed = read.before(1);
    assert!(typed.shows("9 characters, hidden"), "{}", typed.dump());
}

/// A paste whose first bytes came before the prompt was drawn is
/// type-ahead, however late its end comes.
#[test]
fn at_a_terminal_a_paste_begun_before_the_prompt_is_dropped_whole() {
    let read = asked(
        &sign(),
        &[b"\x1b[200~half of a pas"],
        &[Step::Hasty(b"te\x1b[201~"), Step::Typed(b"xyz12345"), ENTER],
    );
    assert_eq!(read.password().as_deref(), Some("xyz12345"));
}

/// The field takes no key until its grace after the first frame has
/// passed and the typing has paused: keys on their way when the prompt was
/// drawn, and typing that runs on into it, are dropped, and nothing of the
/// field is shown until it takes keys.
#[test]
fn at_a_terminal_keys_before_the_field_is_shown_are_dropped() {
    let read = asked(
        &sign(),
        &[],
        &[
            Step::Hasty(b"in-fl"),
            Step::Hasty(b"ight"),
            Step::Typed(b"after-pw"),
            ENTER,
        ],
    );
    let early = read.before(1);
    assert!(!early.shows("characters, hidden"), "{}", early.dump());
    assert!(early.shows("to approve:"), "{}", early.dump());
    let ready = read.before(2);
    assert!(ready.shows("0 characters, hidden"), "{}", ready.dump());
    assert!(ready.shows("enter approves"), "{}", ready.dump());
    assert_eq!(read.password().as_deref(), Some("after-pw"));
}

/// No frame ever holds a character of the password: each is a character
/// the prompt draws nowhere else, and not one of them, in any encoding of
/// it, is ever written to the terminal. Non-ASCII is a password's.
#[test]
fn at_a_terminal_no_frame_holds_a_character_of_the_password() {
    let password = "Ж🙂ЩЖ🙂ЩЖЩ";
    let read = asked(
        &PasswordAsk::new(Purpose::Unlock, Vec::new()),
        &[],
        &[Step::Typed(password.as_bytes()), ENTER],
    );
    assert_eq!(read.password().as_deref(), Some(password));
    for character in password.chars() {
        let mut encoded = [0; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        assert!(
            !read
                .seen
                .written
                .windows(bytes.len())
                .any(|window| window == bytes),
            "{character} was written"
        );
    }
    let typed = read.before(1);
    assert!(typed.shows("8 characters, hidden"), "{}", typed.dump());
    assert!(
        typed.shows("Type your approvals password to use this session's approvals."),
        "{}",
        typed.dump()
    );
    let after = read.after();
    assert!(
        !after.shows("hidden"),
        "the rows are erased: {}",
        after.dump()
    );
}

/// The terminal is closed to other programs from before the prompt's
/// first frame until its rows are gone, whichever way it ends, and open
/// again after.
#[test]
fn at_a_terminal_the_terminal_is_closed_to_others_while_the_prompt_is_up() {
    for (way, steps) in [
        ("enter", &[Step::Typed(b"pw"), ENTER][..]),
        ("ctrl+c", &[Step::Typed(b"pw"), Step::Typed(b"\x03")][..]),
    ] {
        let read = asked(&sign(), &[], steps);
        assert_eq!(read.seen.modes, [true, false], "{way}");
        assert!(read.seen.exclusive.iter().all(|on| *on), "{way}");
        assert!(!read.exclusive_after, "{way}");
    }
}

/// A paste is taken, untrimmed and without its one line break at the end;
/// a paste with a line break inside is refused whole and says so.
#[test]
fn at_a_terminal_a_paste_is_taken_and_one_with_a_line_break_inside_is_refused() {
    let read = asked(
        &sign(),
        &[],
        &[
            Step::Typed(b"\x1b[200~ab\ncd\x1b[201~"),
            Step::Typed(b"\x1b[200~  pass word 1  \n\x1b[201~"),
            ENTER,
        ],
    );
    let refused = read.before(1);
    assert!(refused.shows("0 characters, hidden"), "{}", refused.dump());
    assert!(refused.shows("holds a line break"), "{}", refused.dump());
    let taken = read.before(2);
    assert!(taken.shows("15 characters, hidden"), "{}", taken.dump());
    assert!(!taken.shows("line break"), "{}", taken.dump());
    assert_eq!(read.password().as_deref(), Some("  pass word 1  "));
}

/// `Esc` and `Ctrl+C` give nothing, and the rows are gone once the prompt
/// ends. `Ctrl+C` does before the field is shown too.
#[test]
fn at_a_terminal_esc_and_ctrl_c_give_nothing() {
    let ways: [(&str, &[Step]); 3] = [
        ("esc", &[Step::Typed(b"zq7w"), Step::Typed(b"\x1b")]),
        ("ctrl+c", &[Step::Typed(b"zq7w"), Step::Typed(b"\x03")]),
        ("early ctrl+c", &[Step::Hasty(b"\x03")]),
    ];
    for (way, steps) in ways {
        let read = asked(&sign(), &[], steps);
        assert!(matches!(read.read, Ok(None)), "{way}");
        let after = read.after();
        assert!(!after.shows("hidden"), "{way}: {}", after.dump());
        assert!(!after.shows("to approve"), "{way}: {}", after.dump());
    }
}

/// Choosing: fewer than twelve characters is refused, the second typing
/// must be the first, and each says so and asks again.
#[test]
fn at_a_terminal_a_password_is_chosen_by_typing_it_twice() {
    let read = asked(
        &choose().warning("Every approval stored so far will be dropped."),
        &[],
        &[
            Step::Typed(b"abcdefghijk"),
            ENTER,
            Step::Typed(b"abcdefghijkl"),
            ENTER,
            Step::Typed(b"abcdefghijkX"),
            ENTER,
            Step::Typed(b"abcdefghijkl"),
            ENTER,
            Step::Typed(b"abcdefghijkl"),
            ENTER,
        ],
    );
    let first = read.before(0);
    assert!(first.shows("Choose a password"), "{}", first.dump());
    assert!(
        first.shows("Every approval stored so far will be dropped."),
        "{}",
        first.dump()
    );
    let short = read.before(2);
    assert!(
        short.shows("at least 12 characters, and that one"),
        "{}",
        short.dump()
    );
    let again = read.before(4);
    assert!(
        again.shows("Type the same password again."),
        "{}",
        again.dump()
    );
    let mismatch = read.before(6);
    assert!(mismatch.shows("were not the same"), "{}", mismatch.dump());
    assert!(mismatch.shows("Choose a password"), "{}", mismatch.dump());
    assert_eq!(read.password().as_deref(), Some("abcdefghijkl"));
}

/// The caller's lines, what is approved, are drawn whole; in a window too
/// small for them and the field, no key is taken, and `Ctrl+C` still goes
/// back.
#[test]
fn at_a_terminal_a_window_too_small_for_what_is_approved_takes_no_key() {
    let ask = PasswordAsk::new(
        Purpose::Sign,
        vec![
            "the extension `fmt-helper`".to_owned(),
            "its tools `fmt` and `check`".to_owned(),
            "its hook before each command".to_owned(),
        ],
    );
    let read = asked_on(
        (80, 6),
        &ask,
        &[],
        &[Step::Typed(b"ignored"), Step::Resized, Step::Typed(b"\x03")],
    );
    let small = read.before(1);
    assert!(small.shows("too small"), "{}", small.dump());
    assert!(!small.shows("hidden"), "{}", small.dump());
    assert!(matches!(read.read, Ok(None)));

    let read = asked(&ask, &[], &[Step::Typed(b"taken-pw"), ENTER]);
    let whole = read.before(1);
    for line in ["fmt-helper", "`fmt` and `check`", "before each command"] {
        assert!(whole.shows(line), "{line}: {}", whole.dump());
    }
    assert_eq!(read.password().as_deref(), Some("taken-pw"));
}

/// How many pastes were written over on this thread, after `run`.
fn pastes_wiped_by(run: impl FnOnce()) -> usize {
    let before = super::PASTES_WIPED.with(std::cell::Cell::get);
    run();
    super::PASTES_WIPED.with(std::cell::Cell::get) - before
}

/// Each paste the prompt reads is written over when it is done with: one
/// thrown away as type-ahead, and one taken into the field.
#[test]
fn at_a_terminal_each_paste_is_written_over_once_read() {
    let wiped = pastes_wiped_by(|| {
        let read = asked(
            &sign(),
            &[b"\x1b[200~pasted-ahead\x1b[201~"],
            &[Step::Typed(b"\x1b[200~s3cret-pw\x1b[201~"), ENTER],
        );
        assert_eq!(read.password().as_deref(), Some("s3cret-pw"));
    });
    assert_eq!(wiped, 2);
}
