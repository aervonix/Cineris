//! Text from the workspace as the harness shows it: every character that
//! would act unseen written as an escape (`docs/architecture/05-tools.md`,
//! "Names and lines from the workspace").
//!
//! A file's name, a line of a file, a heading: the harness shows the model
//! text it did not write, inside its own results and notices (decision
//! 0034), and shows a client the same text in its prompts. Written as it
//! is, such text could start lines of its own (a file named with a line
//! break makes a listing's next entry, or a line that looks like the
//! harness's notice), reorder what is around it (the bidirectional
//! controls), or hold text a person cannot see and a model reads (the tag
//! characters, U+E0020 to U+E007F, which shadow ASCII). [`visible`] is the
//! one function every tool and the repository map pass such text through,
//! since the independent review of step 18.
//!
//! **What acts unseen** ([`acts_unseen`]):
//! - the control characters (Unicode's `Cc`): a line break, a carriage
//!   return, a tab, an escape, NEL;
//! - the line and paragraph separators, U+2028 and U+2029 (`Zl`, `Zp`);
//! - the format characters (`Cf`), from a table of Unicode
//!   [`UNICODE_VERSION`]: the bidirectional controls, the characters of no
//!   width (U+200B to U+200D, U+2060 to U+2064, U+FEFF), the soft hyphen
//!   U+00AD, the tag characters, U+180E, U+FFF9 to U+FFFB, U+1D173 to
//!   U+1D17A, and the rest of the category. The table is checked against
//!   Unicode's own data for that version by this module's tests.
//!
//! **How it is written:** a line break, a carriage return and a tab as
//! `\n`, `\r` and `\t`, any other as `\u{…}` with its code point in hex.
//! A backslash is left as it is: a name holding `\n` as two characters
//! reads the same as an escaped line break, which is harmless, since
//! neither starts a line.
//!
//! Not escaped: the variation selectors (`Mn`, U+FE00 to U+FE0F and
//! U+E0100 to U+E01EF), which follow emoji and ideographs in ordinary text,
//! and no model reads as letters.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt::Write as _;

/// The version of Unicode whose format characters [`FORMAT`] lists. Its
/// `Cf` category is the same in 16.0.
pub const UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0);

/// The format characters (general category `Cf`) of Unicode
/// [`UNICODE_VERSION`], as ranges of code points, first and last, in
/// order. A failing test of this module prints the table to put here.
const FORMAT: [(u32, u32); 21] = [
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Whether a character is a format character (`Cf`).
fn is_format(character: char) -> bool {
    let code = u32::from(character);
    // Nothing below the soft hyphen is one: most text costs a compare.
    if code < 0xAD {
        return false;
    }
    FORMAT
        .binary_search_by(|&(first, last)| {
            if last < code {
                Ordering::Less
            } else if first > code {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// Whether a character would change how the text around it reads without
/// being seen: a control character, a line or paragraph separator, or a
/// format character.
#[must_use]
pub fn acts_unseen(character: char) -> bool {
    character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') || is_format(character)
}

/// `text` with every character that [acts unseen](acts_unseen) written as
/// an escape: what a name from the workspace (a file's path, a directory's,
/// a definition's, a heading) is shown as, to the model and to a client.
#[must_use]
pub fn visible(text: &str) -> Cow<'_, str> {
    visible_keeping(text, &[])
}

/// [`visible`], but every character of `keep` left as it is: a line of a
/// file's content keeps its tabs (`&['\t']`), which are its indentation and
/// start no line.
#[must_use]
pub fn visible_keeping<'a>(text: &'a str, keep: &[char]) -> Cow<'a, str> {
    let escaped = |character: char| acts_unseen(character) && !keep.contains(&character);
    let Some(first) = text.find(escaped) else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..first]);
    for character in text[first..].chars() {
        match character {
            _ if !escaped(character) => out.push(character),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(other));
            }
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ranges of `Cf` in Unicode's `DerivedGeneralCategory.txt` of the
    /// pinned version: the lines of the category, kept as they are.
    const DATA: &str = include_str!("../fixtures/DerivedGeneralCategory-17.0.0-Cf.txt");

    fn from_data() -> Vec<(u32, u32)> {
        DATA.lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| {
                let (range, rest) = line.split_once(';').expect("a range and a category");
                assert!(rest.trim_start().starts_with("Cf "), "{line}");
                let range = range.trim();
                let (first, last) = range.split_once("..").unwrap_or((range, range));
                let hex = |text: &str| u32::from_str_radix(text, 16).expect("hex");
                (hex(first), hex(last))
            })
            .collect()
    }

    /// The table is Unicode's `Cf`, for the version it names; its failure
    /// prints the table to put in its place.
    #[test]
    fn the_table_of_format_characters_is_unicodes_for_its_version() {
        assert!(
            DATA.starts_with(&format!(
                "# DerivedGeneralCategory-{}.{}.{}.txt",
                UNICODE_VERSION.0, UNICODE_VERSION.1, UNICODE_VERSION.2
            )),
            "the data is not of Unicode {UNICODE_VERSION:?}"
        );
        let expected = from_data();
        let table: String = expected
            .iter()
            .map(|(first, last)| format!("    (0x{first:04X}, 0x{last:04X}),\n"))
            .collect();
        assert_eq!(
            FORMAT.to_vec(),
            expected,
            "the table should be:\nconst FORMAT: [(u32, u32); {}] = [\n{table}];",
            expected.len()
        );
        let total: u32 = FORMAT.iter().map(|(first, last)| last - first + 1).sum();
        assert!(DATA.contains(&format!("# Total code points: {total}")));
    }

    /// Every code point is a format character exactly when the data says
    /// so: the search through the table is checked, not only the table.
    #[test]
    fn a_character_is_a_format_character_exactly_when_unicode_says() {
        let ranges = from_data();
        for code in 0..=0x10_FFFF_u32 {
            let Some(character) = char::from_u32(code) else {
                continue;
            };
            let listed = ranges
                .iter()
                .any(|&(first, last)| (first..=last).contains(&code));
            assert_eq!(is_format(character), listed, "U+{code:04X}");
        }
    }

    #[test]
    fn what_would_act_unseen_is_written_as_an_escape() {
        assert_eq!(visible("plain é ü 中文 😀"), "plain é ü 中文 😀");
        assert!(matches!(visible("plain"), Cow::Borrowed(_)));
        assert_eq!(visible("a\nb\tc\r\u{1b}[0m"), "a\\nb\\tc\\r\\u{1b}[0m");
        assert_eq!(visible("x\u{202E}y\u{2066}z"), "x\\u{202e}y\\u{2066}z");
        assert_eq!(
            visible("a\u{2028}b\u{2029}c\u{200B}d\u{FEFF}e\u{85}"),
            "a\\u{2028}b\\u{2029}c\\u{200b}d\\u{feff}e\\u{85}"
        );
        // The ones the review of step 18 found passing.
        assert_eq!(
            visible("s\u{AD}h\u{180E}i\u{FFF9}j\u{1D173}k"),
            "s\\u{ad}h\\u{180e}i\\u{fff9}j\\u{1d173}k"
        );
    }

    /// "IGNORE" spelled in tag characters, which a person does not see and
    /// a model reads as the ASCII they shadow.
    #[test]
    fn tag_characters_are_written_as_escapes() {
        let hidden: String = "IGNORE"
            .chars()
            .map(|c| char::from_u32(0xE0000 + u32::from(c)).expect("a tag"))
            .collect();
        let name = format!("a{hidden}.rs\u{E0001}");
        let shown = visible(&name);
        assert!(!shown.chars().any(acts_unseen), "{shown}");
        assert_eq!(
            shown,
            "a\\u{e0049}\\u{e0047}\\u{e004e}\\u{e004f}\\u{e0052}\\u{e0045}.rs\\u{e0001}"
        );
    }

    #[test]
    fn a_line_of_content_keeps_its_tabs() {
        assert_eq!(
            visible_keeping("\tif x {\u{202E}\r", &['\t']),
            "\tif x {\\u{202e}\\r"
        );
        assert!(matches!(
            visible_keeping("\tplain", &['\t']),
            Cow::Borrowed(_)
        ));
    }

    /// Nothing escaped acts unseen, whatever the text: the property every
    /// caller relies on.
    #[test]
    fn property_nothing_that_acts_unseen_survives() {
        let mut rng = crate::SeededRng::new(18);
        let pieces = [
            "a",
            "é",
            "\n",
            "\r",
            "\t",
            "\u{1b}",
            "\u{202E}",
            "\u{2028}",
            "\u{E0041}",
            "\u{AD}",
            "\u{FEFF}",
            "\\",
            "u{",
            "}",
            "😀",
            "\u{7F}",
            "\u{9B}",
            "\u{110BD}",
        ];
        for _ in 0..2_000 {
            let len = crate::Rng::below(&mut rng, 12);
            let text: String = (0..len)
                .map(|_| pieces[crate::Rng::below(&mut rng, pieces.len() as u64) as usize])
                .collect();
            let shown = visible(&text);
            assert!(!shown.chars().any(acts_unseen), "{text:?} -> {shown:?}");
            assert!(!shown.contains('\n'));
        }
    }
}
