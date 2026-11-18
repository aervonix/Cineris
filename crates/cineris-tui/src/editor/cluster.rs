//! Grapheme clusters as the editor counts them.
//!
//! A cluster is what a reader sees as one character
//! ([`cineris_base::text::graphemes`]). The editor adds one rule: a chip's
//! mark ([`CHIP`]) is always a cluster of its own. Unicode would join a
//! combining mark typed after it to it, and a chip is drawn as its label,
//! never as a base for an accent; deleting the accent must not delete the
//! paste.
//!
//! Every position the editor keeps is a byte offset at a cluster boundary
//! of the whole text. A line break is always such a boundary (the editor
//! never holds `\r`), so a boundary is found by reading only the line it is
//! in.

use std::borrow::Cow;

use cineris_base::text;

/// The character that stands for a chip in the text: U+FFFC, the Object
/// Replacement Character, which Unicode keeps for exactly this.
pub(crate) const CHIP: char = '\u{FFFC}';

/// How many columns a tab takes. A tab is kept in the text as it was typed
/// or pasted, and drawn as this many spaces.
pub(crate) const TAB_WIDTH: usize = 4;

/// What kind of cluster, for moving and deleting by word and for grouping
/// undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// A letter, a digit or `_`, in any script.
    Word,
    /// White space, line breaks included.
    Space,
    /// Anything else: punctuation, symbols, emoji, a chip.
    Other,
}

/// The class of a cluster, from its first character.
pub(crate) fn class(cluster: &str) -> Class {
    match cluster.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
        Some(c) if c.is_whitespace() => Class::Space,
        _ => Class::Other,
    }
}

/// The clusters of `text`, each with the byte offset it starts at, a chip's
/// mark always on its own.
pub(crate) fn clusters(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text::graphemes(text).flat_map(|(offset, cluster, _)| Split {
        rest: cluster,
        offset,
    })
}

/// A cluster cut around each chip mark in it. Nearly always yields the
/// cluster whole.
struct Split<'a> {
    rest: &'a str,
    offset: usize,
}

impl<'a> Iterator for Split<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.is_empty() {
            return None;
        }
        let cut = match self.rest.find(CHIP) {
            Some(0) => CHIP.len_utf8(),
            Some(at) => at,
            None => self.rest.len(),
        };
        let (piece, rest) = self.rest.split_at(cut);
        let item = (self.offset, piece);
        self.offset += cut;
        self.rest = rest;
        Some(item)
    }
}

/// The start of the line that holds byte `at`.
pub(crate) fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |found| found + 1)
}

/// The end of the line that holds byte `at`: the offset of its `\n`, or of
/// the end of the text.
pub(crate) fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |found| at + found)
}

/// The cluster boundary before `at`, or 0.
pub(crate) fn prev(text: &str, at: usize) -> usize {
    if at == 0 {
        return 0;
    }
    // The cluster before a line's start is its `\n`.
    let start = line_start(text, at);
    if start == at {
        return at - 1;
    }
    let mut last = start;
    for (offset, cluster) in clusters(&text[start..line_end(text, at)]) {
        let begin = start + offset;
        if begin + cluster.len() >= at {
            return begin;
        }
        last = begin;
    }
    last
}

/// The cluster boundary after `at`, or the end of the text.
pub(crate) fn next(text: &str, at: usize) -> usize {
    if at >= text.len() {
        return text.len();
    }
    if text.as_bytes()[at] == b'\n' {
        return at + 1;
    }
    let start = line_start(text, at);
    for (offset, cluster) in clusters(&text[start..line_end(text, at)]) {
        let end = start + offset + cluster.len();
        if end > at {
            return end;
        }
    }
    line_end(text, at)
}

/// `at` if it is a cluster boundary; otherwise the end of the cluster that
/// holds it. An edit can join two clusters (two halves of a flag), which
/// leaves a position that was a boundary inside one.
pub(crate) fn snap(text: &str, at: usize) -> usize {
    let at = at.min(text.len());
    if at == 0 || at == text.len() || text.as_bytes()[at] == b'\n' {
        return at;
    }
    let start = line_start(text, at);
    if start == at {
        return at;
    }
    for (offset, cluster) in clusters(&text[start..line_end(text, at)]) {
        let begin = start + offset;
        let end = begin + cluster.len();
        if begin == at {
            return at;
        }
        if end > at {
            return end;
        }
    }
    at
}

/// The class of the cluster that ends at `at`, if any.
pub(crate) fn class_before(text: &str, at: usize) -> Option<Class> {
    (at > 0).then(|| class(&text[prev(text, at)..at]))
}

/// The class of the cluster that starts at `at`, if any.
pub(crate) fn class_after(text: &str, at: usize) -> Option<Class> {
    (at < text.len()).then(|| class(&text[at..next(text, at)]))
}

/// The start of the word before `at`: back over anything that is not a
/// word, then over the word.
pub(crate) fn word_start(text: &str, at: usize) -> usize {
    let mut at = at;
    while class_before(text, at).is_some_and(|class| class != Class::Word) {
        at = prev(text, at);
    }
    while class_before(text, at) == Some(Class::Word) {
        at = prev(text, at);
    }
    at
}

/// The end of the word after `at`: forward over anything that is not a
/// word, then over the word.
pub(crate) fn word_end(text: &str, at: usize) -> usize {
    let mut at = at;
    while class_after(text, at).is_some_and(|class| class != Class::Word) {
        at = next(text, at);
    }
    while class_after(text, at) == Some(Class::Word) {
        at = next(text, at);
    }
    at
}

/// A cluster as the input box draws it, made safe to show
/// (`cineris_term::sanitize`) on its own: a control character as its
/// symbol, and what acts unseen (a character of no width, a bidirectional
/// control) as its escape, `\u{200b}`. A tab is the caller's to draw.
pub(crate) fn drawn(cluster: &str) -> Cow<'_, str> {
    cineris_term::sanitize(cluster)
}

/// How many columns a cluster takes in the editor: a tab [`TAB_WIDTH`],
/// a chip its label's width (the caller knows it), anything else the width
/// of what is [`drawn`] (since the review of step 23, which found a
/// zero-width space counted as no columns and drawn as eight, the cursor
/// left inside the escape and the row cut).
pub(crate) fn width(cluster: &str) -> usize {
    if cluster == "\t" {
        return TAB_WIDTH;
    }
    match drawn(cluster) {
        Cow::Borrowed(cluster) => text::grapheme_width(cluster),
        Cow::Owned(shown) => text::width(&shown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every cluster boundary of `text`, by walking `next` from the start.
    fn walk(text: &str) -> Vec<usize> {
        let mut at = 0;
        let mut seen = vec![0];
        while at < text.len() {
            at = next(text, at);
            seen.push(at);
        }
        seen
    }

    #[test]
    fn a_chip_mark_is_a_cluster_of_its_own_even_with_an_accent_after_it() {
        let text = "a\u{FFFC}\u{301}b";
        let found: Vec<&str> = clusters(text).map(|(_, c)| c).collect();
        assert_eq!(found, ["a", "\u{FFFC}", "\u{301}", "b"]);
    }

    #[test]
    fn next_and_prev_step_over_whole_clusters() {
        // e + combining acute, a family emoji, a flag, a keycap, a skin
        // tone, Hangul jamo, a wide character.
        let parts = [
            "e\u{301}",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{1F1EB}\u{1F1F7}",
            "1\u{FE0F}\u{20E3}",
            "\u{1F44D}\u{1F3FD}",
            "\u{1100}\u{1161}\u{11A8}",
            "\u{4E2D}",
            "\n",
            "x",
        ];
        let text: String = parts.concat();
        let mut expected = vec![0];
        for part in parts {
            expected.push(expected.last().unwrap() + part.len());
        }
        assert_eq!(walk(&text), expected);
        let mut back = vec![text.len()];
        let mut at = text.len();
        while at > 0 {
            at = prev(&text, at);
            back.push(at);
        }
        back.reverse();
        assert_eq!(back, expected);
        for &boundary in &expected {
            assert_eq!(snap(&text, boundary), boundary);
        }
        // Inside the family: snapped to its end.
        assert_eq!(snap(&text, expected[1] + 4), expected[2]);
    }

    #[test]
    fn words_are_letters_digits_and_underscores_in_any_script() {
        let text = "fix scan_number, then Ünïcode 中文!";
        assert_eq!(word_start(text, text.len()), text.find('中').unwrap());
        assert_eq!(word_end(text, 0), 3);
        assert_eq!(word_end(text, 3), text.find(',').unwrap());
        assert_eq!(word_start(text, text.find(',').unwrap() + 1), 4);
    }
}
