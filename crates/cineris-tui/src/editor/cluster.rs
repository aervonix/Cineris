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

use crate::glyphs::Glyphs;

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
/// control) as its escape, `\u{200b}`. A run of [`RUN`] or more escapes is
/// written as one mark with its count, so that a row always holds a
/// cluster (since step 25: a letter followed by a long run of tag
/// characters is one cluster, and was wider than the row): a run of one
/// character as its escape, the glyph set's `×` and the count,
/// `\u{e0041}×12`; a run of several (text hidden in tag characters,
/// `\u{e0068}\u{e0065}…`) as the first's escape, `+` and how many more
/// follow, `\u{e0068}+11` (since the review of step 25's second pull
/// request). Only the prompt editor's row counts: everywhere else
/// `sanitize` shows every escape. A tab is the caller's to draw.
pub(crate) fn drawn<'a>(cluster: &'a str, glyphs: &Glyphs) -> Cow<'a, str> {
    match cineris_term::sanitize(cluster) {
        Cow::Borrowed(text) => Cow::Borrowed(text),
        Cow::Owned(shown) => Cow::Owned(counted(&shown, glyphs)),
    }
}

/// How many escapes in a row are written as one mark with their count.
pub(crate) const RUN: usize = 4;

/// `shown` with each run of [`RUN`] or more escapes written as one mark:
/// of one character, its escape, `×` and the count; of several, the
/// first's escape, `+` and how many more follow.
fn counted(shown: &str, glyphs: &Glyphs) -> String {
    let mut out = String::with_capacity(shown.len());
    let mut rest = shown;
    while let Some(c) = rest.chars().next() {
        let Some(first) = escape_at(rest) else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        // The run of escapes from here, whatever their characters.
        let mut run = rest;
        let mut count = 0;
        let mut one = true;
        while let Some(escape) = escape_at(run) {
            count += 1;
            one &= escape == first;
            run = &run[escape.len()..];
        }
        if count < RUN {
            out.push_str(&rest[..rest.len() - run.len()]);
        } else if one {
            out.push_str(first);
            out.push_str(glyphs.times);
            out.push_str(&count.to_string());
        } else {
            out.push_str(first);
            out.push('+');
            out.push_str(&(count - 1).to_string());
        }
        rest = run;
    }
    out
}

/// The escape `sanitize` writes for a character, `\u{…}` with one to six
/// hex digits, when `text` starts with one.
fn escape_at(text: &str) -> Option<&str> {
    let body = text.strip_prefix("\\u{")?;
    let close = body.find('}')?;
    let digits = &body[..close];
    (!digits.is_empty() && digits.len() <= 6 && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| &text[..3 + close + 1])
}

/// How many columns a cluster takes in the editor: a tab [`TAB_WIDTH`],
/// a chip its label's width (the caller knows it), anything else the width
/// of what is [`drawn`] (since the review of step 23, which found a
/// zero-width space counted as no columns and drawn as eight, the cursor
/// left inside the escape and the row cut). The same in either glyph set:
/// a count's `×` is one column in both (a test holds it).
pub(crate) fn width(cluster: &str) -> usize {
    if cluster == "\t" {
        return TAB_WIDTH;
    }
    match drawn(cluster, &Glyphs::UNICODE) {
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

    /// A letter followed by twelve tag characters is one cluster, drawn as
    /// the letter, one escape and its count, so that a row holds it; three
    /// are each written (step 25, `one-character-wider-than-the-prompts-row`).
    /// A run of several characters (text hidden in tags) is one mark too,
    /// the first's escape and how many more follow; the count's `×` is the
    /// ASCII set's `x` where that set is chosen, as wide, so that the width
    /// is what is drawn in either (the review of step 25's second pull
    /// request).
    #[test]
    fn a_long_run_of_one_unseen_character_is_drawn_once_with_its_count() {
        let unicode = &Glyphs::UNICODE;
        let tags: String = std::iter::repeat_n('\u{e0041}', 12).collect();
        let cluster = format!("a{tags}");
        assert_eq!(clusters(&cluster).count(), 1, "one cluster");
        assert_eq!(drawn(&cluster, unicode), "a\\u{e0041}×12");
        assert_eq!(width(&cluster), "a\\u{e0041}×12".chars().count());
        let three = "a\u{e0041}\u{e0041}\u{e0041}";
        assert_eq!(drawn(three, unicode), "a\\u{e0041}\\u{e0041}\\u{e0041}");
        // Plain text, and a text that only looks like an escape, as they are.
        assert_eq!(drawn("é", unicode), "é");
        assert!(matches!(drawn("x", unicode), Cow::Borrowed("x")));
    }

    #[test]
    fn a_run_of_several_unseen_characters_is_one_mark_and_ascii_counts_with_x() {
        let (unicode, ascii) = (&Glyphs::UNICODE, &Glyphs::ASCII);
        // `hello` in tag characters, after a letter: one cluster.
        let hidden: String = "hello"
            .chars()
            .map(|c| char::from_u32(0xe0000 + u32::from(c)).unwrap())
            .collect();
        let cluster = format!("a{hidden}");
        assert_eq!(clusters(&cluster).count(), 1, "one cluster");
        assert_eq!(drawn(&cluster, unicode), "a\\u{e0068}+4");
        assert_eq!(width(&cluster), "a\\u{e0068}+4".chars().count());
        let mixed: String = format!("a{}{}", "\u{e0041}".repeat(5), "\u{e0042}".repeat(4));
        assert_eq!(drawn(&mixed, unicode), "a\\u{e0041}+8");
        // A long run of tags in a cluster, of any kind, fits a narrow row.
        let long: String = (0..200)
            .map(|n| char::from_u32(0xe0020 + n % 0x5f).unwrap())
            .collect();
        assert!(width(&format!("a{long}")) < 20);
        // The ASCII set's count, as wide as the Unicode set's.
        let tags = format!("a{}", "\u{e0041}".repeat(12));
        assert_eq!(drawn(&tags, ascii), "a\\u{e0041}x12");
        assert_eq!(
            text::width(&drawn(&tags, ascii)),
            text::width(&drawn(&tags, unicode))
        );
        assert_eq!(text::width(ascii.times), text::width(unicode.times));
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
