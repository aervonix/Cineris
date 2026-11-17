//! [`SecretBuf`]: a key being typed, in memory that never moves and is
//! written over with zeros when it is dropped (since step 25;
//! `docs/architecture/08-configuration.md`, "Credentials").
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
//! - **Wiped.** Taking a character away, emptying it and dropping it write
//!   zeros over the bytes that held text. The writes are followed by
//!   [`std::hint::black_box`] on the buffer and a compiler fence, so that
//!   the compiler does not drop them as writes to memory nothing reads
//!   again. Both are the standard library's, and need no `unsafe`, which
//!   this crate denies; a volatile write would need it, or a crate for it.
//! - **Never printed.** `Debug` says how many characters it holds, never
//!   which. It is not `Clone`, `Display` or `Serialize`.
//!
//! **What is not wiped** is said in 08's "Credentials": the terminal
//! layer's read buffer, the decoder's, and a paste's string pass the key
//! before it reaches this buffer, and the `Secret` given to the store is a
//! `String` of its own. They are the process's memory, short-lived, and
//! never written anywhere.

use crate::Secret;

/// Text refused by [`SecretBuf::push_str`]: nothing of it was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// It would not fit in what is left of the capacity.
    TooLong,
    /// It holds a line break: a key is one line, and a paste with two is
    /// something else.
    LineBreak,
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
    /// (`\n` or `\r`) anywhere in it, or more than is left of the capacity.
    ///
    /// # Errors
    ///
    /// [`Refused`], when nothing of `text` was kept.
    pub fn push_str(&mut self, text: &str) -> Result<(), Refused> {
        if text.contains(['\n', '\r']) {
            return Err(Refused::LineBreak);
        }
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(Refused::TooLong);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }

    /// Adds one character at the end.
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

    /// Writes zeros over the whole buffer: what the drop runs.
    fn wipe(&mut self) {
        wipe(&mut self.bytes);
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
    fn drop(&mut self) {
        self.wipe();
    }
}

impl std::fmt::Debug for SecretBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretBuf({} characters, hidden)", self.chars())
    }
}

/// Writes zeros over `bytes`, in a way the compiler keeps.
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

    #[test]
    fn what_the_drop_runs_leaves_every_byte_zero() {
        // The drop runs `wipe`, the code below; its memory cannot be read
        // after it without `unsafe`, so the same code is run here on a
        // buffer that is still alive.
        let mut buffer = SecretBuf::with_capacity(32);
        buffer.push_str(KEY).unwrap();
        buffer.wipe();
        assert!(buffer.all_zero());
        assert_eq!(buffer.chars(), 0);
    }

    #[test]
    fn only_printable_ascii_is_a_key_for_windows() {
        let mut buffer = SecretBuf::new();
        buffer.push_str(" sk-plain ").unwrap();
        assert!(buffer.is_printable_ascii());
        buffer.push_str("é").unwrap();
        assert!(!buffer.is_printable_ascii());
        buffer.clear();
        buffer.push('\u{7}').unwrap();
        assert!(!buffer.is_printable_ascii());
    }
}
