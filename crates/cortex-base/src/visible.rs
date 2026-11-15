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
//! **What acts unseen** ([`acts_unseen`]), from tables of Unicode
//! [`UNICODE_VERSION`] that this module's tests check against Unicode's own
//! data:
//! - the control characters (`Cc`): a line break, a carriage return, a
//!   tab, an escape, NEL;
//! - the line and paragraph separators, U+2028 and U+2029 (`Zl`, `Zp`);
//! - the format characters (`Cf`): the bidirectional controls, the
//!   characters of no width (U+200B to U+200D, U+2060 to U+2064, U+FEFF),
//!   the soft hyphen U+00AD, the tag characters, U+180E, U+FFF9 to U+FFFB,
//!   U+1D173 to U+1D17A, and the rest of the category;
//! - the characters Unicode says render as nothing
//!   (`Default_Ignorable_Code_Point`): beside those above, the Hangul
//!   fillers, the combining grapheme joiner, U+2065, U+FFF0 to U+FFF8 and
//!   the whole of U+E0000 to U+E0FFF, assigned or not.
//!
//! **A variation selector** (U+FE00 to U+FE0F, U+E0100 to U+E01EF, and
//! Mongolian's four) is kept right after a character shown as it is, where
//! it picks how that character is drawn (an emoji's colour); anywhere else,
//! one after another or at the start, it is an escape, since a run of them
//! can carry any bytes unseen.
//!
//! **How it is written:** a line break, a carriage return and a tab as
//! `\n`, `\r` and `\t`, any other as `\u{…}` with its code point in hex.
//! A backslash is left as it is: a name holding `\n` as two characters
//! reads the same as an escaped line break, which is harmless, since
//! neither starts a line. [`unescape`] reads the escapes back, for a name
//! the model was shown and gives back.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt::Write as _;

/// The version of Unicode whose characters [`FORMAT`] and [`IGNORABLE`]
/// list. Both are the same in 16.0.
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

/// The code points Unicode [`UNICODE_VERSION`] says render as nothing
/// (`Default_Ignorable_Code_Point`), as [`FORMAT`] is laid out.
const IGNORABLE: [(u32, u32); 27] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180D),
    (0x180E, 0x180E),
    (0x180F, 0x180F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2065, 0x2065),
    (0x2066, 0x206F),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFF8),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0000),
    (0xE0001, 0xE0001),
    (0xE0002, 0xE001F),
    (0xE0020, 0xE007F),
    (0xE0080, 0xE00FF),
    (0xE0100, 0xE01EF),
    (0xE01F0, 0xE0FFF),
];

/// Whether `code` is in one of `ranges`, which are in order.
fn listed(ranges: &[(u32, u32)], code: u32) -> bool {
    // Nothing below the soft hyphen is in either table: most text costs a
    // compare.
    code >= 0xAD
        && ranges
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

/// Whether a character is a format character (`Cf`).
fn is_format(character: char) -> bool {
    listed(&FORMAT, u32::from(character))
}

/// Whether a character renders as nothing (`Default_Ignorable_Code_Point`).
fn is_ignorable(character: char) -> bool {
    listed(&IGNORABLE, u32::from(character))
}

/// Whether a character is a variation selector: it picks how the character
/// before it is drawn.
#[must_use]
pub fn is_selector(character: char) -> bool {
    matches!(
        u32::from(character),
        0x180B..=0x180D | 0x180F | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF
    )
}

/// Whether a character would change how the text around it reads without
/// being seen: a control character, a line or paragraph separator, a
/// format character, or one that renders as nothing. Not a variation
/// selector, which [`visible`] judges by what it follows.
#[must_use]
pub fn acts_unseen(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{2028}' | '\u{2029}')
        || is_format(character)
        || (is_ignorable(character) && !is_selector(character))
}

/// `text` with every character that [acts unseen](acts_unseen) written as
/// an escape, and every variation selector that does not follow a
/// character shown as it is: what a name from the workspace (a file's path,
/// a directory's, a definition's, a heading) is shown as, to the model and
/// to a client.
#[must_use]
pub fn visible(text: &str) -> Cow<'_, str> {
    visible_keeping(text, &[])
}

/// [`visible`], but every character of `keep` left as it is: a line of a
/// file's content keeps its tabs (`&['\t']`), which are its indentation and
/// start no line.
#[must_use]
pub fn visible_keeping<'a>(text: &'a str, keep: &[char]) -> Cow<'a, str> {
    // Whether the character before was shown as it is, and is one a
    // selector may follow.
    let mut base = false;
    let mut escaped = |character: char| {
        let escape = if is_selector(character) {
            !base
        } else {
            acts_unseen(character) && !keep.contains(&character)
        };
        base = !escape && !is_selector(character) && !keep.contains(&character);
        escape
    };
    let Some(first) = text.find(&mut escaped) else {
        return Cow::Borrowed(text);
    };
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(&text[..first]);
    let mut characters = text[first..].chars();
    // The first was judged already.
    if let Some(character) = characters.next() {
        push_escape(&mut out, character);
    }
    for character in characters {
        if escaped(character) {
            push_escape(&mut out, character);
        } else {
            out.push(character);
        }
    }
    Cow::Owned(out)
}

fn push_escape(out: &mut String, character: char) {
    match character {
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        other => {
            let _ = write!(out, "\\u{{{:x}}}", u32::from(other));
        }
    }
}

/// `text` with the escapes [`visible`] writes read back: `\n`, `\r`, `\t`,
/// and `\u{…}` of a character that acts unseen or is a variation selector.
/// Any other backslash is left as it is. For a name the model was shown
/// escaped and gives back (a path a tool is asked for, a key a notice
/// holds): a caller tries the text as it is first, since a name may hold
/// `\n` as two characters.
#[must_use]
pub fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('\\') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (decoded, len) = match after.as_bytes().first() {
            Some(b'n') => (Some('\n'), 1),
            Some(b'r') => (Some('\r'), 1),
            Some(b't') => (Some('\t'), 1),
            Some(b'u') => after
                .strip_prefix("u{")
                .and_then(|hex| hex.split_once('}'))
                .filter(|(hex, _)| (1..=6).contains(&hex.len()))
                .and_then(|(hex, _)| {
                    let character = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                    (acts_unseen(character) || is_selector(character))
                        .then_some((Some(character), hex.len() + 3))
                })
                .unwrap_or((None, 0)),
            _ => (None, 0),
        };
        match decoded {
            Some(character) => {
                out.push(character);
                rest = &after[len..];
            }
            None => {
                out.push('\\');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ranges of `Cf` in Unicode's `DerivedGeneralCategory.txt` of the
    /// pinned version: the lines of the category, kept as they are.
    const DATA: &str = include_str!("../fixtures/DerivedGeneralCategory-17.0.0-Cf.txt");

    /// The ranges of `Default_Ignorable_Code_Point` in Unicode's
    /// `DerivedCoreProperties.txt` of the pinned version, the same way.
    const IGNORABLE_DATA: &str =
        include_str!("../fixtures/DerivedCoreProperties-17.0.0-Default_Ignorable_Code_Point.txt");

    fn ranges_of(data: &str, property: &str) -> Vec<(u32, u32)> {
        data.lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .map(|line| {
                let (range, rest) = line.split_once(';').expect("a range and a property");
                assert!(rest.trim_start().starts_with(property), "{line}");
                let range = range.trim();
                let (first, last) = range.split_once("..").unwrap_or((range, range));
                let hex = |text: &str| u32::from_str_radix(text, 16).expect("hex");
                (hex(first), hex(last))
            })
            .collect()
    }

    fn from_data() -> Vec<(u32, u32)> {
        ranges_of(DATA, "Cf ")
    }

    /// The table of what renders as nothing is Unicode's, for the version
    /// it names; its failure prints the table to put in its place.
    #[test]
    fn the_table_of_characters_that_render_as_nothing_is_unicodes() {
        assert!(IGNORABLE_DATA.starts_with(&format!(
            "# DerivedCoreProperties-{}.{}.{}.txt",
            UNICODE_VERSION.0, UNICODE_VERSION.1, UNICODE_VERSION.2
        )));
        let expected = ranges_of(IGNORABLE_DATA, "Default_Ignorable_Code_Point");
        let table: String = expected
            .iter()
            .map(|(first, last)| format!("    (0x{first:04X}, 0x{last:04X}),\n"))
            .collect();
        assert_eq!(
            IGNORABLE.to_vec(),
            expected,
            "the table should be:\nconst IGNORABLE: [(u32, u32); {}] = [\n{table}];",
            expected.len()
        );
        let total: u32 = IGNORABLE.iter().map(|(first, last)| last - first + 1).sum();
        assert!(IGNORABLE_DATA.contains(&format!("# Total code points: {total}")));
        for code in 0..=0x10_FFFF_u32 {
            let Some(character) = char::from_u32(code) else {
                continue;
            };
            let listed = expected
                .iter()
                .any(|&(first, last)| (first..=last).contains(&code));
            assert_eq!(is_ignorable(character), listed, "U+{code:04X}");
        }
    }

    /// The independent review of step 18's fixes found these passing:
    /// characters that render as nothing and are not format characters.
    #[test]
    fn what_renders_as_nothing_is_written_as_an_escape() {
        assert_eq!(
            visible("a\u{3164}b\u{115F}c\u{34F}d\u{2065}e\u{FFF0}f\u{E0000}g\u{E0FFF}"),
            "a\\u{3164}b\\u{115f}c\\u{34f}d\\u{2065}e\\u{fff0}f\\u{e0000}g\\u{e0fff}"
        );
    }

    /// A variation selector picks how the character before it is drawn,
    /// and is kept after one shown as it is; a run of them, or one at the
    /// start or after an escape, can carry bytes unseen and is an escape.
    #[test]
    fn a_variation_selector_is_kept_after_a_character_and_a_run_is_escaped() {
        assert_eq!(visible("love \u{2764}\u{FE0F}"), "love \u{2764}\u{FE0F}");
        assert!(matches!(visible("\u{2764}\u{FE0F}"), Cow::Borrowed(_)));
        assert_eq!(
            visible("\u{1F600}\u{FE00}\u{FE01}\u{E0100}"),
            "\u{1F600}\u{FE00}\\u{fe01}\\u{e0100}"
        );
        assert_eq!(visible("\u{FE0F}x"), "\\u{fe0f}x");
        assert_eq!(visible("a\u{200B}\u{FE0F}"), "a\\u{200b}\\u{fe0f}");
        assert_eq!(visible_keeping("\t\u{FE0F}", &['\t']), "\t\\u{fe0f}");
    }

    /// What `visible` writes reads back, and nothing else is read: a name
    /// the model was shown escaped is found again by it.
    #[test]
    fn an_escape_reads_back_to_the_character_it_stands_for() {
        for text in [
            "a\nb\tc\r",
            "x\u{202E}y\u{E0041}z\u{3164}",
            "\u{1F600}\u{FE00}\u{FE01}",
            "plain é",
        ] {
            assert_eq!(unescape(&visible(text)), text, "{text:?}");
        }
        // Not escapes `visible` writes: left as they are.
        for text in [
            r"C:\Work\x",
            r"a\u{41}b",
            r"a\u{zz}",
            r"a\u{110000}",
            r"end\",
            r"a\qb",
        ] {
            assert_eq!(unescape(text), text, "{text:?}");
        }
        assert!(matches!(unescape("plain"), Cow::Borrowed(_)));
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

    /// Nothing escaped acts unseen, and a selector survives only right
    /// after a character shown as it is, whatever the text: the property
    /// every caller relies on.
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
            "\u{FE0F}",
            "\u{E0100}",
            "\u{3164}",
            "\u{E0FFF}",
        ];
        for _ in 0..2_000 {
            let len = crate::Rng::below(&mut rng, 12);
            let text: String = (0..len)
                .map(|_| pieces[crate::Rng::below(&mut rng, pieces.len() as u64) as usize])
                .collect();
            let shown = visible(&text);
            assert!(!shown.chars().any(acts_unseen), "{text:?} -> {shown:?}");
            assert!(!shown.contains('\n'));
            let mut before: Option<char> = None;
            for character in shown.chars() {
                if is_selector(character) {
                    assert!(
                        before.is_some_and(|base| !is_selector(base)),
                        "{text:?} -> {shown:?}"
                    );
                }
                before = Some(character);
            }
            // Read back, it is the text again, when the text held no
            // backslash: one that held `\u{a}` as five characters reads
            // back as a line break, which is why a caller tries a name as
            // it is first.
            if !text.contains('\\') {
                assert_eq!(unescape(&shown), text, "{text:?}");
            }
        }
    }
}
