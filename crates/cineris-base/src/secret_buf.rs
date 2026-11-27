//! [`SecretBuf`]: a key being typed, in memory that never moves and is
//! written over with zeros, as far as safe Rust can, when it is dropped
//! (since step 25; `docs/architecture/08-configuration.md`, "Credentials");
//! and [`Password`], the approvals password typed at a prompt, which is one
//! (since the signed approvals).
//!
//! A [`Secret`] is a `String`: it can grow, which moves its text and leaves
//! the old copy behind, and it is not wiped when it is dropped. A key typed
//! or pasted at a terminal is held here instead, until it is handed to the
//! credential store as a `Secret`:
//!
//! - **Fixed capacity.** The buffer is allocated once, at
//!   [`SecretBuf::CAPACITY`] bytes (64 KiB, a key's limit, as `cineris auth
//!   login` reads), and never grows, so no copy of the text is left where an
//!   earlier, smaller allocation was. Text that would not fit is refused
//!   whole: nothing of it is kept.
//! - **One line of text.** Text that holds a line break, or any other
//!   control character (a tab, an escape), is refused whole, on every
//!   system: no key has one.
//! - **Wiped, as a best effort.** Taking a character away, emptying it and
//!   dropping it write zeros over the bytes that held text, with ordinary
//!   writes (`fill(0)`), followed by [`std::hint::black_box`] on the buffer
//!   and a compiler fence, so that the compiler does not drop them as writes
//!   to memory nothing reads again. Both are the standard library's, and
//!   need no `unsafe`, which this crate denies.
//! - **What the wipe cannot promise.** `black_box` is a hint the compiler is
//!   asked to honour, not a guarantee, and there is no volatile write, which
//!   would need `unsafe` or a crate for it. Nor does it reach copies outside
//!   the buffer: a page of it the kernel swapped out, or a core file written,
//!   while the key was in it; and the other copies the key passes through
//!   (below), which the allocator takes back as they were.
//! - **Never printed.** `Debug` says how many characters it holds, never
//!   which. It is not `Clone`, `Display` or `Serialize`.
//! - **Read in one place.** [`SecretBuf::expose_bytes`] gives its bytes to
//!   what must read them, and is named as the place where they can leak.
//!
//! **A password** ([`Password`]) is a buffer that is never trimmed and never
//! becomes a `String` or a [`Secret`]: it gives only its bytes, for the key
//! the approvals are signed with, which is made from them
//! (`cineris_policy::signed`).
//!
//! **What is not wiped** is said in 08's "Credentials": the terminal
//! layer's read buffer and the decoder's pass the key before it reaches
//! this buffer, and the `Secret` given to the store is a `String` of its
//! own. They are the process's memory, short-lived, cleared rather than
//! written over, and never written anywhere. A paste's own string is
//! written over once taken ([`wipe_string`], since the signed approvals).

use crate::Secret;

/// Text refused by [`SecretBuf::push_str`]: nothing of it was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// It would not fit in what is left of the capacity.
    TooLong,
    /// It holds a line break: a key is one line, and a paste with two is
    /// something else.
    LineBreak,
    /// It holds a control character other than a line break (a tab, an
    /// escape): no key has one.
    Control,
}

/// A key being typed. See the [module documentation](self).
pub struct SecretBuf {
    bytes: Box<[u8]>,
    len: usize,
}

impl SecretBuf {
    /// The capacity of [`SecretBuf::new`]: 64 KiB, a key's limit.
    pub const CAPACITY: usize = 64 * 1024;

    /// An empty buffer of [`CAPACITY`](Self::CAPACITY) bytes.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(Self::CAPACITY)
    }

    /// An empty buffer of `capacity` bytes, for tests of its limit.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: vec![0; capacity].into_boxed_slice(),
            len: 0,
        }
    }

    /// Adds `text` at the end, whole, or refuses it whole: a line break
    /// (`\n` or `\r`) anywhere in it, any other control character, or more
    /// than is left of the capacity.
    ///
    /// # Errors
    ///
    /// [`Refused`], when nothing of `text` was kept.
    pub fn push_str(&mut self, text: &str) -> Result<(), Refused> {
        if text.contains(['\n', '\r']) {
            return Err(Refused::LineBreak);
        }
        if text.contains(char::is_control) {
            return Err(Refused::Control);
        }
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(Refused::TooLong);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }

    /// Adds one character at the end, with no copy of it left but this
    /// buffer's: the bytes it is encoded in on the way are written over.
    ///
    /// # Errors
    ///
    /// [`Refused`], when it was not kept.
    pub fn push(&mut self, character: char) -> Result<(), Refused> {
        let mut encoded = [0; 4];
        let result = self.push_str(character.encode_utf8(&mut encoded));
        wipe(&mut encoded);
        result
    }

    /// Takes the last character away, writing zeros over it. Says whether
    /// there was one.
    pub fn pop(&mut self) -> bool {
        let Some(last) = self.text().chars().next_back() else {
            return false;
        };
        let start = self.len - last.len_utf8();
        wipe(&mut self.bytes[start..self.len]);
        self.len = start;
        true
    }

    /// Empties it, writing zeros over what it held.
    pub fn clear(&mut self) {
        wipe(&mut self.bytes[..self.len]);
        self.len = 0;
    }

    /// How many characters it holds: what the key's row shows of it.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.text().chars().count()
    }

    /// Whether it holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many bytes it can hold in all.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.bytes.len()
    }

    /// The key it holds, trimmed as `cineris auth login` trims a key, for
    /// the credential store. `None` when nothing is left after trimming.
    #[must_use]
    pub fn to_secret(&self) -> Option<Secret> {
        let key = self.text().trim();
        (!key.is_empty()).then(|| Secret::new(key))
    }

    /// Whether the key, trimmed, is printable ASCII, as every key and token
    /// is (and as Windows Credential Manager's store requires,
    /// `docs/architecture/08-configuration.md`).
    #[must_use]
    pub fn is_printable_ascii(&self) -> bool {
        self.text()
            .trim()
            .chars()
            .all(|character| character.is_ascii() && !character.is_ascii_control())
    }

    /// Whether every byte of the whole buffer is zero: what it holds once
    /// emptied. For tests.
    #[must_use]
    pub fn all_zero(&self) -> bool {
        self.bytes.iter().all(|byte| *byte == 0)
    }

    /// The text it holds, as its UTF-8 bytes, untrimmed: for what must read
    /// them, such as the derivation of the approvals key from a password,
    /// which takes `&[u8]`.
    ///
    /// **Every call is a place where the text can leak,** as
    /// [`Secret::expose`] is: pass the slice straight to what needs it, and
    /// never to a log, an error, a `String`, or anything that is stored or
    /// sent. A copy made of it is the caller's to write over.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Whether it holds the same text as `other`, byte for byte, with no
    /// copy of either made: a password typed twice. Every byte is looked at,
    /// wherever the first difference is.
    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        self.len == other.len
            && self.bytes[..self.len]
                .iter()
                .zip(&other.bytes[..other.len])
                .fold(0, |differs, (one, two)| differs | (one ^ two))
                == 0
    }

    fn text(&self) -> &str {
        // Only whole characters are ever added and taken away.
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl Default for SecretBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SecretBuf {
    /// Writes zeros over the whole buffer.
    fn drop(&mut self) {
        wipe(&mut self.bytes);
        self.len = 0;
        #[cfg(test)]
        LEFT_BY_DROP.set(Some((self.bytes.len(), self.all_zero())));
    }
}

#[cfg(test)]
thread_local! {
    /// What the last buffer dropped, or string wiped, on this thread left:
    /// how many bytes it had, and whether every one of them was zero. Freed memory cannot be
    /// read without `unsafe`, so in tests the drop says what it left.
    static LEFT_BY_DROP: std::cell::Cell<Option<(usize, bool)>> =
        const { std::cell::Cell::new(None) };
}

impl std::fmt::Debug for SecretBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretBuf({} characters, hidden)", self.chars())
    }
}

/// The approvals password, as typed at a prompt (since the signed
/// approvals; `docs/architecture/09-tui.md`, "The approvals password"): a
/// [`SecretBuf`] that is never trimmed, and gives only its bytes
/// ([`expose_bytes`](Self::expose_bytes)), for the key made from them. It
/// never becomes a `String` or a [`Secret`].
///
/// `Debug` says how many characters it holds, never which. It is not
/// `Clone`, `Display` or `Serialize`. Dropping it writes zeros over its
/// buffer, as [`SecretBuf`]'s drop does: whoever it is handed to drops it
/// once the key is made.
pub struct Password(SecretBuf);

impl Password {
    /// The fewest characters (Unicode scalar values) a password chosen may
    /// have: 12, with nothing else asked of it (settled by the maintainers
    /// after the review of the signed approvals' design: the stores and the
    /// key file, which a copy of the approvals holds, are enough to test a
    /// guess offline). The same value as the signing's own minimum, which
    /// this crate does not depend on. A password typed to unlock or to sign
    /// is not held to it: it is the one chosen, or it is wrong.
    pub const LEAST: usize = 12;

    /// The password held in `buffer`, taken as it is: no copy is made.
    #[must_use]
    pub fn new(buffer: SecretBuf) -> Self {
        Self(buffer)
    }

    /// The password's UTF-8 bytes, untrimmed: what the approvals key is
    /// made from. **Every call is a place where it can leak**
    /// ([`SecretBuf::expose_bytes`]).
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8] {
        self.0.expose_bytes()
    }

    /// How many characters (Unicode scalar values) it has.
    #[must_use]
    pub fn chars(&self) -> usize {
        self.0.chars()
    }

    /// Whether it holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<SecretBuf> for Password {
    fn from(buffer: SecretBuf) -> Self {
        Self::new(buffer)
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Password({} characters, hidden)", self.chars())
    }
}

/// Writes zeros over a string's bytes, all of its allocation, and frees
/// it: a paste that held a key or a password, once it has been taken into
/// a [`SecretBuf`] or thrown away (since the signed approvals). As the
/// buffer's own wipe, a best effort (see the [module documentation](self)),
/// and only of this allocation: the copies a string left behind where it
/// grew from, and those it was made from, are not reached.
pub fn wipe_string(text: String) {
    let mut bytes = text.into_bytes();
    // The spare room too: it may hold what the string held before.
    bytes.resize(bytes.capacity(), 0);
    wipe(&mut bytes);
    #[cfg(test)]
    LEFT_BY_DROP.set(Some((bytes.len(), bytes.iter().all(|byte| *byte == 0))));
}

/// Writes zeros over `bytes`, in a way the compiler is asked to keep (see
/// the [module documentation](self) for what that cannot promise).
fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&mut *bytes);
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-ant-api03-4f9a0c17e2b8";

    #[test]
    fn it_holds_a_key_and_says_only_how_long_it_is() {
        let mut buffer = SecretBuf::new();
        assert_eq!(buffer.capacity(), 64 * 1024);
        buffer.push_str(KEY).unwrap();
        assert_eq!(buffer.chars(), KEY.len());
        let printed = format!("{buffer:?} {buffer:#?}");
        assert!(!printed.contains("sk-"), "{printed}");
        assert!(printed.contains("25 characters"), "{printed}");
        assert_eq!(buffer.to_secret().unwrap().expose(), KEY);
    }

    #[test]
    fn a_line_break_or_too_much_is_refused_whole() {
        let mut buffer = SecretBuf::with_capacity(8);
        buffer.push_str("abc").unwrap();
        assert_eq!(buffer.push_str("de\nf"), Err(Refused::LineBreak));
        assert_eq!(buffer.push_str("g\r"), Err(Refused::LineBreak));
        assert_eq!(buffer.push_str("123456"), Err(Refused::TooLong));
        assert_eq!(buffer.chars(), 3, "nothing of what was refused was kept");
        buffer.push_str("12345").unwrap();
        assert_eq!(buffer.push('x'), Err(Refused::TooLong));
        assert_eq!(buffer.to_secret().unwrap().expose(), "abc12345");
    }

    /// A tab, an escape, a delete, a C1 control and NUL are refused whole
    /// wherever they are, as typed characters and in text, on every system.
    #[test]
    fn a_control_character_is_refused_whole() {
        let mut buffer = SecretBuf::with_capacity(32);
        buffer.push_str("sk-").unwrap();
        for text in ["ab\tcd", "\u{1b}[2J", "ab\u{7f}", "\u{9b}31m", "a\0", "\t"] {
            assert_eq!(buffer.push_str(text), Err(Refused::Control), "{text:?}");
        }
        for character in ['\t', '\u{1b}', '\u{7}', '\u{85}'] {
            assert_eq!(
                buffer.push(character),
                Err(Refused::Control),
                "{character:?}"
            );
        }
        assert_eq!(
            buffer.to_secret().unwrap().expose(),
            "sk-",
            "nothing was kept"
        );
        assert!(
            buffer.bytes[3..].iter().all(|byte| *byte == 0),
            "nothing was written past what was kept"
        );
    }

    #[test]
    fn what_is_taken_away_or_emptied_is_zero() {
        let mut buffer = SecretBuf::with_capacity(16);
        buffer.push_str("ké").unwrap();
        assert!(buffer.pop());
        assert_eq!(buffer.chars(), 1);
        assert!(
            buffer.bytes[1..].iter().all(|byte| *byte == 0),
            "the two bytes of é are zero"
        );
        buffer.push_str(" two  ").unwrap();
        assert_eq!(buffer.to_secret().unwrap().expose(), "k two", "trimmed");
        buffer.clear();
        assert!(buffer.is_empty());
        assert!(buffer.all_zero());
        assert!(!buffer.pop());
        assert!(buffer.to_secret().is_none());
        buffer.push_str("   ").unwrap();
        assert!(buffer.to_secret().is_none(), "only spaces is no key");
    }

    /// The drop itself writes zeros over every byte the key was in: read by
    /// the drop, in tests, since the freed memory cannot be read without
    /// `unsafe`.
    #[test]
    fn a_dropped_buffer_leaves_every_byte_zero() {
        let mut buffer = SecretBuf::with_capacity(32);
        buffer.push_str(KEY).unwrap();
        assert!(!buffer.all_zero());
        LEFT_BY_DROP.set(None);
        drop(buffer);
        assert_eq!(LEFT_BY_DROP.get(), Some((32, true)));
    }

    /// Its bytes are the text as typed, untrimmed, and two buffers are the
    /// same only when every byte is.
    #[test]
    fn its_bytes_are_the_text_untrimmed_and_two_are_compared_whole() {
        let mut one = SecretBuf::with_capacity(32);
        one.push_str(" pässwörd ").unwrap();
        assert_eq!(one.expose_bytes(), " pässwörd ".as_bytes());
        let mut two = SecretBuf::with_capacity(64);
        two.push_str(" pässwörd ").unwrap();
        assert!(one.same_as(&two) && two.same_as(&one));
        two.pop();
        assert!(!one.same_as(&two), "one shorter");
        two.push('x').unwrap();
        assert!(!one.same_as(&two), "the last differs");
        let mut three = SecretBuf::with_capacity(32);
        three.push_str("xpässwörd ").unwrap();
        assert!(!one.same_as(&three), "the first differs");
        assert!(SecretBuf::with_capacity(4).same_as(&SecretBuf::with_capacity(8)));
    }

    /// A password says how long it is and nothing of what, gives its bytes
    /// as typed, and leaves every byte zero when dropped.
    #[test]
    fn a_password_is_hidden_and_wiped_when_dropped() {
        let mut buffer = SecretBuf::with_capacity(32);
        buffer.push_str("correct horse é").unwrap();
        let password = Password::from(buffer);
        assert_eq!(password.chars(), 15);
        assert!(!password.is_empty());
        assert_eq!(password.expose_bytes(), "correct horse é".as_bytes());
        let printed = format!("{password:?} {password:#?}");
        assert!(!printed.contains("horse"), "{printed}");
        assert!(printed.contains("15 characters, hidden"), "{printed}");
        LEFT_BY_DROP.set(None);
        drop(password);
        assert_eq!(LEFT_BY_DROP.get(), Some((32, true)));
    }

    /// A paste's string is written over whole, its spare room included,
    /// before it is freed.
    #[test]
    fn a_wiped_string_leaves_every_byte_of_its_allocation_zero() {
        let mut text = String::with_capacity(64);
        text.push_str("pasted password");
        LEFT_BY_DROP.set(None);
        wipe_string(text);
        assert_eq!(LEFT_BY_DROP.get(), Some((64, true)));
    }

    #[test]
    fn only_printable_ascii_is_a_key_for_windows() {
        let mut buffer = SecretBuf::new();
        buffer.push_str(" sk-plain ").unwrap();
        assert!(buffer.is_printable_ascii());
        buffer.push_str("é").unwrap();
        assert!(!buffer.is_printable_ascii());
    }
}
