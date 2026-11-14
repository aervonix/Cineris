//! The text and its chips, and the one way to change them.
//!
//! A chip is a pasted text shown as one short label (`[pasted 148 lines]`).
//! In the text it is one [`CHIP`] character; the chips are kept in a list in
//! the order their marks appear, so the n-th mark is the n-th chip. Every
//! change is [`Buffer::replace`]: a range out, a [`Fragment`] in, and the
//! fragment that came out returned, which is what undo keeps.

use std::sync::Arc;

use super::cluster::CHIP;

/// A pasted text held as one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    /// The text it stands for, as it will be sent.
    content: Arc<str>,
    /// What is drawn in its place.
    label: String,
}

impl Chip {
    /// A chip for `content`, drawn as `label`.
    #[must_use]
    pub fn new(content: impl Into<Arc<str>>, label: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            label: label.into(),
        }
    }

    /// A chip that holds a U+FFFC that came in as text: the editor keeps
    /// every such character as a chip, so none is lost, and draws it as the
    /// character itself.
    pub(crate) fn literal() -> Self {
        Self::new(CHIP.to_string(), CHIP.to_string())
    }

    /// Whether it holds only a U+FFFC that came in as text.
    #[must_use]
    pub fn is_literal(&self) -> bool {
        &*self.content == "\u{FFFC}" && self.label == "\u{FFFC}"
    }

    /// The text it stands for.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// What is drawn in its place.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// A piece of text with the chips its marks stand for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fragment {
    pub(crate) text: String,
    pub(crate) chips: Vec<Chip>,
}

impl Fragment {
    /// Plain text. Any U+FFFC in it becomes a chip that holds that
    /// character, so the text comes out as it went in.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        let chips = text.matches(CHIP).map(|_| Chip::literal()).collect();
        Self { text, chips }
    }

    /// One chip.
    #[must_use]
    pub fn chip(chip: Chip) -> Self {
        Self {
            text: CHIP.to_string(),
            chips: vec![chip],
        }
    }

    /// Whether it holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Adds `other` after this.
    pub(crate) fn push(&mut self, other: Fragment) {
        self.text.push_str(&other.text);
        self.chips.extend(other.chips);
    }

    /// Adds `other` before this.
    pub(crate) fn prepend(&mut self, mut other: Fragment) {
        other.text.push_str(&self.text);
        other.chips.append(&mut self.chips);
        *self = other;
    }

    /// The text with each chip replaced by what it stands for.
    #[must_use]
    pub fn expanded(&self) -> String {
        expand(&self.text, &self.chips)
    }
}

/// `text` with the n-th chip mark replaced by the n-th chip's content.
pub(crate) fn expand(text: &str, chips: &[Chip]) -> String {
    if chips.is_empty() {
        return text.to_owned();
    }
    let extra: usize = chips.iter().map(|chip| chip.content.len()).sum();
    let mut out = String::with_capacity(text.len() + extra);
    let mut chips = chips.iter();
    for piece in text.split_inclusive(CHIP) {
        match piece.strip_suffix(CHIP) {
            Some(before) => {
                out.push_str(before);
                if let Some(chip) = chips.next() {
                    out.push_str(&chip.content);
                }
            }
            None => out.push_str(piece),
        }
    }
    out
}

/// How many chip marks `text` holds.
pub(crate) fn marks(text: &str) -> usize {
    text.matches(CHIP).count()
}

/// The editor's text: a string, and the chips its marks stand for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Buffer {
    text: String,
    chips: Vec<Chip>,
}

impl Buffer {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn chips(&self) -> &[Chip] {
        &self.chips
    }

    /// The chip whose mark is at byte `at`.
    pub(crate) fn chip_at(&self, at: usize) -> Option<&Chip> {
        if !self.text[at..].starts_with(CHIP) {
            return None;
        }
        self.chips.get(marks(&self.text[..at]))
    }

    /// Replaces `range` with `with`, and returns what was there.
    pub(crate) fn replace(&mut self, range: std::ops::Range<usize>, with: Fragment) -> Fragment {
        let first = marks(&self.text[..range.start]);
        let taken = marks(&self.text[range.clone()]);
        let removed_chips: Vec<Chip> = self
            .chips
            .splice(first..first + taken, with.chips)
            .collect();
        let removed_text = self.text[range.clone()].to_owned();
        self.text.replace_range(range, &with.text);
        Fragment {
            text: removed_text,
            chips: removed_chips,
        }
    }

    /// The fragment in `range`.
    pub(crate) fn fragment(&self, range: std::ops::Range<usize>) -> Fragment {
        let first = marks(&self.text[..range.start]);
        let taken = marks(&self.text[range.clone()]);
        Fragment {
            text: self.text[range].to_owned(),
            chips: self.chips[first..first + taken].to_vec(),
        }
    }

    /// The whole text with every chip expanded.
    pub(crate) fn expanded(&self) -> String {
        expand(&self.text, &self.chips)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_keeps_the_chips_in_step_with_their_marks() {
        let mut buffer = Buffer::default();
        buffer.replace(0..0, Fragment::text("ab"));
        buffer.replace(1..1, Fragment::chip(Chip::new("ONE", "[1]")));
        let at = buffer.text().len();
        buffer.replace(at..at, Fragment::chip(Chip::new("TWO", "[2]")));
        assert_eq!(buffer.expanded(), "aONEbTWO");
        let removed = buffer.replace(0..1 + CHIP.len_utf8(), Fragment::text("x"));
        assert_eq!(removed.expanded(), "aONE");
        assert_eq!(buffer.expanded(), "xbTWO");
        assert_eq!(buffer.chip_at(2).map(Chip::label), Some("[2]"));
    }

    #[test]
    fn a_literal_object_replacement_character_survives() {
        let mut buffer = Buffer::default();
        buffer.replace(0..0, Fragment::text("a\u{FFFC}b"));
        buffer.replace(0..0, Fragment::chip(Chip::new("P", "[p]")));
        assert_eq!(buffer.expanded(), "Pa\u{FFFC}b");
    }
}
