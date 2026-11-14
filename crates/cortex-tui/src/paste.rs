//! What a paste becomes (`docs/architecture/09-tui.md`, "Paste").
//!
//! A bracketed paste reaches the interface as one
//! [`Event::Paste`](cortex_term::Event::Paste), and goes into the editor as
//! text: it never passes through the key map, so nothing in it is taken
//! for a key. Its line breaks are `\n` (`\r\n` and a lone `\r` included);
//! every other character is kept, control characters too, and drawn as a
//! visible symbol.
//!
//! **A long paste is a chip.** A paste of more than [`CHIP_LINES`] lines or
//! more than [`CHIP_CHARS`] characters is held as one [`Chip`] labelled
//! `[pasted 148 lines]`, or `[pasted 2,345 characters]` when it is one
//! line. It is sent whole, and `Ctrl+X` puts its text in place.

use crate::editor::{Chip, Fragment, normalize};

/// A paste of more lines than this is a chip.
pub const CHIP_LINES: usize = 12;

/// A paste of more characters than this is a chip. Characters are Unicode
/// scalar values, line breaks included.
pub const CHIP_CHARS: usize = 2_000;

/// The fragment a pasted text becomes: the text, or a chip.
#[must_use]
pub fn fragment(pasted: &str) -> Fragment {
    let text = normalize(pasted);
    let lines = line_count(&text);
    let chars = char_count(&text, CHIP_CHARS + 1);
    if lines > CHIP_LINES || chars > CHIP_CHARS {
        let label = label(lines, &text);
        Fragment::chip(Chip::new(text.into_owned(), label))
    } else {
        Fragment::text(text.into_owned())
    }
}

/// How many lines `text` has: a final line break does not begin another.
fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.matches('\n').count() + usize::from(!text.ends_with('\n'))
}

/// How many characters `text` has, counting no further than `stop`.
fn char_count(text: &str, stop: usize) -> usize {
    // A character is at most four bytes: a short text needs no count.
    if text.len() <= stop {
        return text.chars().count();
    }
    text.chars().take(stop).count()
}

/// The chip's label: lines when there are several, characters when one.
fn label(lines: usize, text: &str) -> String {
    if lines > 1 {
        format!("[pasted {} lines]", thousands(lines))
    } else {
        format!("[pasted {} characters]", thousands(text.chars().count()))
    }
}

/// `n` with a comma between each group of three digits.
pub(crate) fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_chip(fragment: &Fragment) -> bool {
        !fragment.chips.is_empty() && fragment.chips[0].label().starts_with("[pasted")
    }

    #[test]
    fn twelve_lines_are_text_and_thirteen_a_chip() {
        let twelve = "line\n".repeat(12);
        assert!(!is_chip(&fragment(&twelve)));
        let thirteen = "line\n".repeat(13);
        let chip = fragment(&thirteen);
        assert!(is_chip(&chip));
        assert_eq!(chip.chips[0].label(), "[pasted 13 lines]");
        assert_eq!(chip.expanded(), thirteen);
    }

    #[test]
    fn two_thousand_characters_are_text_and_one_more_a_chip() {
        let at_limit = "é".repeat(2_000);
        assert!(!is_chip(&fragment(&at_limit)));
        let over = "é".repeat(2_001);
        let chip = fragment(&over);
        assert_eq!(chip.chips[0].label(), "[pasted 2,001 characters]");
    }

    #[test]
    fn line_breaks_become_newlines_and_everything_else_is_kept() {
        let pasted = fragment("a\r\nb\rc\x1b[31m\u{7}");
        assert_eq!(pasted.expanded(), "a\nb\nc\x1b[31m\u{7}");
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(148), "148");
        assert_eq!(thousands(2_345), "2,345");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
