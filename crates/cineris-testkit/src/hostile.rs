//! Hostile names: the one set the workspace's tests take their names from
//! (`docs/testing-and-ci.md`, "Hostile names").
//!
//! A name from a workspace (a file's, a directory's, a heading's) is text
//! the harness did not write. It may hold characters that act unseen (a
//! control character, a bidirectional control, a tag character), a line
//! break that starts a line of its own, an escape sequence for the
//! terminal, a length at the edge of what a file system holds, or a word
//! Windows keeps for its devices. Each test used to keep its own few of
//! them, and one kept a constant that was a line feed on one system and
//! U+2028 on the other, so that what it checked depended on where it ran:
//! it hid a bug on Linux.
//!
//! **A name that is hostile is hostile on both systems.** This module has
//! no `cfg`: the entries, their number and their order are the same on
//! Linux and on Windows (a unit test reads the source to check it). What
//! differs is what a *file system* takes as a file name: Windows refuses
//! `CON`, a line feed and a colon. A test that makes files asks the system
//! at run time ([`takes_file_name`], [`accepted`]) and runs the cases that
//! can be made as files; or it tests the text, with no file, and says so.
//! It never keeps a constant per system that changes what is checked.
//!
//! An entry is a [`Name`]: a label a failure message can print, the
//! [`text`](Name::text), how [`cineris_base::visible`] writes it
//! ([`shown`](Name::shown), written here by hand, so that the function is
//! checked against something other than itself), and its [`Kind`]. The text
//! may be used whole as a name, or inside one: `format!("x{}y", name.text)`.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support that makes a real file to ask a file system what it takes, from a test's own thread"
)]

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// What a hostile name is hostile by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A control character (`Cc`) that is not a line break: C0, DEL, C1.
    Control,
    /// A bidirectional control: the embeddings, overrides and isolates and
    /// the marks, alone and in a name that reorders what is around it.
    Bidi,
    /// Another format character (`Cf`): of no width, a soft hyphen, the
    /// byte-order mark, a tag character.
    Format,
    /// A character that renders as nothing and is not `Cf`: a Hangul
    /// filler, the combining grapheme joiner, a run of variation selectors.
    Invisible,
    /// A line break: LF, CR, CRLF, NEL, U+2028, U+2029.
    LineBreak,
    /// An escape sequence for a terminal: ESC and CSI, OSC closed by BEL and
    /// by ST, the C1 forms.
    Escape,
    /// A length at the edge of what a file system or a limit holds, and a
    /// run of characters whose escape is longer than they are.
    Long,
    /// A word Windows keeps for a device, in the forms it still treats as
    /// the device: other case, an extension, trailing dots and spaces.
    Reserved,
    /// A combining mark, alone, stacked and with no base.
    Combining,
    /// A wide character, an emoji, a sequence joined into one.
    Wide,
    /// Letters written right to left, alone and among others.
    RightToLeft,
    /// Punctuation a file system, a shell or a glob gives a meaning: `..`,
    /// a leading `-`, a backslash, a colon, a quote, a trailing dot.
    Punctuation,
    /// The replacement character.
    Replacement,
}

impl Kind {
    /// Every kind, in the order [`all`] lists them.
    pub const ALL: [Kind; 13] = [
        Kind::Control,
        Kind::Bidi,
        Kind::Format,
        Kind::Invisible,
        Kind::LineBreak,
        Kind::Escape,
        Kind::Long,
        Kind::Reserved,
        Kind::Combining,
        Kind::Wide,
        Kind::RightToLeft,
        Kind::Punctuation,
        Kind::Replacement,
    ];
}

/// One hostile name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    /// What it is, for a failure message to print.
    pub label: String,
    /// What it is hostile by.
    pub kind: Kind,
    /// The text: a name by itself, or to put inside one.
    pub text: String,
    /// How [`cineris_base::visible`] shows the text: every character that
    /// acts unseen as an escape, and nothing else changed.
    pub shown: String,
}

impl Name {
    fn new(kind: Kind, label: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            label: label.into(),
            kind,
            shown: text.clone(),
            text,
        }
    }

    /// The same, with `shown` written out.
    fn shown(mut self, shown: impl Into<String>) -> Self {
        self.shown = shown.into();
        self
    }
}

/// How an escape is written: a line feed, a carriage return and a tab as
/// `\n`, `\r` and `\t`, any other as `\u{…}`.
fn escape(character: char) -> String {
    match character {
        '\n' => "\\n".to_owned(),
        '\r' => "\\r".to_owned(),
        '\t' => "\\t".to_owned(),
        other => format!("\\u{{{:x}}}", u32::from(other)),
    }
}

/// An entry of one character that acts unseen, which is shown as its escape.
fn one(kind: Kind, label: impl Into<String>, character: char) -> Name {
    Name::new(kind, label, character.to_string()).shown(escape(character))
}

/// An entry of text in which each of `unseen` is an escape.
fn mixed(kind: Kind, label: &str, text: &str, unseen: &[char]) -> Name {
    let mut shown = String::new();
    for character in text.chars() {
        if unseen.contains(&character) {
            shown.push_str(&escape(character));
        } else {
            shown.push(character);
        }
    }
    Name::new(kind, label, text).shown(shown)
}

/// Every hostile name, the same on every system, in a fixed order.
#[must_use]
pub fn all() -> Vec<Name> {
    let mut names = Vec::new();
    for kind in Kind::ALL {
        names.extend(of(kind));
    }
    names
}

/// The hostile names of one kind.
#[must_use]
pub fn of(kind: Kind) -> Vec<Name> {
    match kind {
        Kind::Control => control(),
        Kind::Bidi => bidi(),
        Kind::Format => format_characters(),
        Kind::Invisible => invisible(),
        Kind::LineBreak => line_breaks(),
        Kind::Escape => escapes(),
        Kind::Long => long(),
        Kind::Reserved => reserved(),
        Kind::Combining => combining(),
        Kind::Wide => wide(),
        Kind::RightToLeft => right_to_left(),
        Kind::Punctuation => punctuation(),
        Kind::Replacement => replacement(),
    }
}

/// The entry that is the one character `character`, for a test that needs
/// that one in particular.
///
/// # Panics
///
/// When the set holds no entry of that character alone.
#[must_use]
pub fn character(character: char) -> Name {
    let text = character.to_string();
    all()
        .into_iter()
        .find(|name| name.text == text)
        .unwrap_or_else(|| panic!("no hostile name is U+{:04X} alone", u32::from(character)))
}

/// The entry with this label.
///
/// # Panics
///
/// When no entry has it.
#[must_use]
pub fn named(label: &str) -> Name {
    all()
        .into_iter()
        .find(|name| name.label == label)
        .unwrap_or_else(|| panic!("no hostile name is called `{label}`"))
}

/// The characters that break a line, as a text may hold them: for a test
/// that asks whether a name started a line of its own.
pub const LINE_BREAK_CHARACTERS: [char; 5] = ['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'];

/// `word` spelled in tag characters (U+E0020 to U+E007E), which a person
/// does not see and a model reads as the ASCII they shadow.
#[must_use]
pub fn tags(word: &str) -> String {
    word.chars()
        .map(|character| {
            char::from_u32(0xE0000 + u32::from(character)).expect("a tag character of ASCII")
        })
        .collect()
}

fn control() -> Vec<Name> {
    let mut names = Vec::new();
    for code in (0x00..=0x1Fu32).chain([0x7F]).chain(0x80..=0x9F) {
        // The line breaks have a kind of their own.
        if matches!(code, 0x0A | 0x0D | 0x85) {
            continue;
        }
        let character = char::from_u32(code).expect("a control character");
        let set = match code {
            0x00..=0x1F => "C0",
            0x7F => "DEL",
            _ => "C1",
        };
        names.push(one(Kind::Control, format!("{set} U+{code:04X}"), character));
    }
    names.push(mixed(Kind::Control, "tab between letters", "a\tb", &['\t']));
    names.push(mixed(
        Kind::Control,
        "NUL inside a name",
        "a\u{0}b",
        &['\u{0}'],
    ));
    names.push(mixed(
        Kind::Control,
        "BEL then DEL then CSI",
        "\u{7}\u{7f}\u{9b}",
        &['\u{7}', '\u{7f}', '\u{9b}'],
    ));
    names
}

fn bidi() -> Vec<Name> {
    let controls = [
        ('\u{202A}', "LRE left-to-right embedding"),
        ('\u{202B}', "RLE right-to-left embedding"),
        ('\u{202C}', "PDF pop directional formatting"),
        ('\u{202D}', "LRO left-to-right override"),
        ('\u{202E}', "RLO right-to-left override"),
        ('\u{2066}', "LRI left-to-right isolate"),
        ('\u{2067}', "RLI right-to-left isolate"),
        ('\u{2068}', "FSI first strong isolate"),
        ('\u{2069}', "PDI pop directional isolate"),
        ('\u{200E}', "LRM left-to-right mark"),
        ('\u{200F}', "RLM right-to-left mark"),
        ('\u{061C}', "ALM arabic letter mark"),
    ];
    let mut names: Vec<Name> = controls
        .iter()
        .map(|&(character, label)| {
            one(
                Kind::Bidi,
                format!("U+{:04X} {label}", u32::from(character)),
                character,
            )
        })
        .collect();
    // The spoof of an extension: a name that reads `invoice_fdp.exe`
    // reversed after the override.
    names.push(mixed(
        Kind::Bidi,
        "an extension spoofed by an override",
        "invoice_\u{202E}fdp.exe",
        &['\u{202E}'],
    ));
    names.push(mixed(
        Kind::Bidi,
        "an isolate opened and never closed",
        "a\u{2067}b\u{2066}c\u{2068}d",
        &['\u{2067}', '\u{2066}', '\u{2068}'],
    ));
    names.push(mixed(
        Kind::Bidi,
        "an embedding closed twice",
        "\u{202B}a\u{202C}\u{202C}b",
        &['\u{202B}', '\u{202C}'],
    ));
    names
}

fn format_characters() -> Vec<Name> {
    let characters = [
        ('\u{200B}', "zero-width space"),
        ('\u{200C}', "zero-width non-joiner"),
        ('\u{200D}', "zero-width joiner"),
        ('\u{2060}', "word joiner"),
        ('\u{2061}', "function application"),
        ('\u{2062}', "invisible times"),
        ('\u{2063}', "invisible separator"),
        ('\u{2064}', "invisible plus"),
        ('\u{FEFF}', "byte-order mark"),
        ('\u{00AD}', "soft hyphen"),
        ('\u{180E}', "mongolian vowel separator"),
        ('\u{0600}', "arabic number sign"),
        ('\u{06DD}', "arabic end of ayah"),
        ('\u{070F}', "syriac abbreviation mark"),
        ('\u{FFF9}', "interlinear annotation anchor"),
        ('\u{FFFB}', "interlinear annotation terminator"),
        ('\u{110BD}', "kaithi number sign"),
        ('\u{1D173}', "musical symbol begin beam"),
        ('\u{E0001}', "language tag"),
        ('\u{E0020}', "tag space"),
        ('\u{E0041}', "tag latin capital letter a"),
        ('\u{E007F}', "cancel tag"),
    ];
    let mut names: Vec<Name> = characters
        .iter()
        .map(|&(character, label)| {
            one(
                Kind::Format,
                format!("U+{:04X} {label}", u32::from(character)),
                character,
            )
        })
        .collect();
    names.push(mixed(
        Kind::Format,
        "a zero-width space in a word",
        "pass\u{200B}word",
        &['\u{200B}'],
    ));
    names.push(mixed(
        Kind::Format,
        "a zero-width non-joiner between letters",
        "a\u{200C}b",
        &['\u{200C}'],
    ));
    names.push(mixed(
        Kind::Format,
        "a byte-order mark before a name",
        "\u{FEFF}name.txt",
        &['\u{FEFF}'],
    ));
    // "IGNORE", spelled in tag characters.
    let hidden = tags("IGNORE");
    let shown: String = hidden.chars().map(escape).collect();
    names.push(Name::new(Kind::Format, "IGNORE in tag characters", hidden.clone()).shown(shown));
    let name = format!("a{hidden}.rs\u{E0001}");
    names.push(mixed(
        Kind::Format,
        "a file name with a hidden word and a language tag",
        &name,
        &name
            .chars()
            .filter(|c| *c >= '\u{E0000}')
            .collect::<Vec<_>>(),
    ));
    names
}

fn invisible() -> Vec<Name> {
    let characters = [
        ('\u{034F}', "combining grapheme joiner"),
        ('\u{115F}', "hangul choseong filler"),
        ('\u{1160}', "hangul jungseong filler"),
        ('\u{3164}', "hangul filler"),
        ('\u{FFA0}', "halfwidth hangul filler"),
        ('\u{17B4}', "khmer vowel inherent aq"),
        ('\u{2065}', "unassigned, ignorable"),
        ('\u{FFF0}', "unassigned, ignorable"),
        ('\u{E0000}', "unassigned tag block"),
        ('\u{E0FFF}', "last of the ignorable tag block"),
    ];
    let mut names: Vec<Name> = characters
        .iter()
        .map(|&(character, label)| {
            one(
                Kind::Invisible,
                format!("U+{:04X} {label}", u32::from(character)),
                character,
            )
        })
        .collect();
    // A variation selector is kept after a character, and is an escape
    // when it follows nothing, or follows another.
    names.push(
        Name::new(
            Kind::Invisible,
            "a variation selector with no base",
            "\u{FE0F}x",
        )
        .shown("\\u{fe0f}x"),
    );
    names.push(
        Name::new(
            Kind::Invisible,
            "a run of variation selectors after an emoji",
            "\u{1F600}\u{FE00}\u{FE01}\u{E0100}",
        )
        .shown("\u{1F600}\u{FE00}\\u{fe01}\\u{e0100}"),
    );
    names
}

fn line_breaks() -> Vec<Name> {
    vec![
        one(Kind::LineBreak, "LF line feed", '\n'),
        one(Kind::LineBreak, "CR carriage return", '\r'),
        Name::new(Kind::LineBreak, "CRLF", "\r\n").shown("\\r\\n"),
        one(Kind::LineBreak, "NEL next line U+0085", '\u{85}'),
        one(Kind::LineBreak, "LS line separator U+2028", '\u{2028}'),
        one(Kind::LineBreak, "PS paragraph separator U+2029", '\u{2029}'),
    ]
}

fn escapes() -> Vec<Name> {
    let esc = '\u{1b}';
    let all_unseen = |text: &str, label: &str| {
        mixed(
            Kind::Escape,
            label,
            text,
            &text
                .chars()
                .filter(|c| c.is_control() || *c == '\u{7}')
                .collect::<Vec<_>>(),
        )
    };
    vec![
        all_unseen("\u{1b}[31mred\u{1b}[0m", "CSI select graphic rendition"),
        all_unseen("a\u{1b}[2Kb", "CSI erase line"),
        all_unseen("\u{1b}[2J\u{1b}[H", "CSI clear the screen and home"),
        all_unseen("\u{1b}[?1049h", "CSI private mode, alternate screen"),
        all_unseen("\u{1b}]0;title\u{7}", "OSC window title closed by BEL"),
        all_unseen(
            "\u{1b}]8;;link.invalid\u{1b}\\link\u{1b}]8;;\u{1b}\\",
            "OSC hyperlink closed by ST",
        ),
        all_unseen("\u{1b}Pq\u{1b}\\", "DCS closed by ST"),
        all_unseen("\u{1b}c", "ESC c reset"),
        one(Kind::Escape, "ESC alone", esc),
        all_unseen("\u{9b}31m", "the C1 form of CSI"),
        all_unseen("\u{9d}0;title\u{9c}", "the C1 forms of OSC and ST"),
        one(Kind::Escape, "BEL", '\u{7}'),
        all_unseen(
            "\u{8}\u{8}\u{8}",
            "backspaces that overwrite what is before",
        ),
    ]
}

/// A name of `count` copies of `unit`, whose label says how long it is.
fn repeated(label: &str, unit: &str, count: usize) -> Name {
    Name::new(Kind::Long, label, unit.repeat(count))
}

fn long() -> Vec<Name> {
    // Each label starts with the length and its unit (`bytes`,
    // `characters` or `UTF-16 units`), which a unit test of this module
    // checks against the text.
    let mut names = vec![
        repeated("254 bytes of ASCII", "a", 254),
        repeated(
            "255 bytes of ASCII, the most a name holds on Linux",
            "a",
            255,
        ),
        repeated("256 bytes of ASCII, one over", "a", 256),
        repeated("260 characters of ASCII", "a", 260),
        repeated("1000 characters of ASCII", "a", 1000),
        repeated("2048 characters of ASCII", "a", 2048),
        repeated("4096 characters of ASCII, a path's limit", "a", 4096),
        repeated("5000 characters of ASCII", "a", 5000),
        repeated("255 characters of two bytes each", "é", 255),
        repeated("260 characters of three bytes each", "中", 260),
        repeated("254 UTF-16 units of surrogate pairs", "😀", 127),
        repeated("256 UTF-16 units of surrogate pairs", "😀", 128),
    ];
    // A name made only of what is escaped: the escape is longer than the
    // text, so a limit on the escaped text is met long before one on the
    // name (an escape-length bug is shown here, and not by a short name).
    let mut overrides = repeated("100 characters of right-to-left overrides", "\u{202E}", 100);
    overrides.shown = "\\u{202e}".repeat(100);
    names.push(overrides);
    let mut feeds = repeated("200 characters of line feeds", "\n", 200);
    feeds.shown = "\\n".repeat(200);
    names.push(feeds);
    let mut tag_run = repeated("128 characters of tag a", "\u{E0041}", 128);
    tag_run.shown = "\\u{e0041}".repeat(128);
    names.push(tag_run);
    // A name at the limit with an escape in the middle of it.
    let mut edge = String::from("a").repeat(250);
    edge.push('\u{200B}');
    edge.push_str("bcd");
    let mut shown = "a".repeat(250);
    shown.push_str("\\u{200b}bcd");
    names.push(
        Name::new(
            Kind::Long,
            "254 characters with a zero-width space at 250",
            edge,
        )
        .shown(shown),
    );
    names
}

fn reserved() -> Vec<Name> {
    let mut names = Vec::new();
    let mut add = |label: String, text: String| {
        names.push(Name::new(Kind::Reserved, label, text));
    };
    for device in ["CON", "PRN", "AUX", "NUL"] {
        add(device.to_owned(), device.to_owned());
        add(format!("{device} in lower case"), device.to_lowercase());
    }
    for number in 1..=9 {
        add(format!("COM{number}"), format!("COM{number}"));
        add(format!("LPT{number}"), format!("LPT{number}"));
    }
    add("COM1 in lower case".into(), "com1".into());
    add("lpt9 in lower case".into(), "lpt9".into());
    add("a mixed case".into(), "Con".into());
    add("a mixed case, two".into(), "cOn".into());
    add("a mixed case of NUL".into(), "nUl".into());
    add("CON with an extension".into(), "con.txt".into());
    add("AUX with an extension".into(), "aux.c".into());
    add("PRN with an extension".into(), "PRN.rs".into());
    add("NUL with two extensions".into(), "NUL.tar.gz".into());
    add("COM1 with an extension".into(), "COM1.log".into());
    add("lpt9 with an extension".into(), "lpt9.log".into());
    add("CON, a trailing dot".into(), "CON.".into());
    add("NUL, a trailing dot and space".into(), "NUL. ".into());
    add("AUX, trailing spaces".into(), "AUX  ".into());
    add(
        "aux with an extension and a trailing dot".into(),
        "aux.c.".into(),
    );
    add(
        "aux, a space before the extension".into(),
        "aux .txt".into(),
    );
    add("CONIN$".into(), "CONIN$".into());
    add("CONOUT$".into(), "CONOUT$".into());
    // Windows treats the superscript digits as a device's number too.
    add("COM with a superscript one".into(), "COM\u{b9}".into());
    add("LPT with a superscript two".into(), "LPT\u{b2}".into());
    names
}

fn combining() -> Vec<Name> {
    vec![
        Name::new(Kind::Combining, "e and a combining acute", "e\u{301}"),
        Name::new(Kind::Combining, "a precomposed e acute", "\u{e9}"),
        Name::new(Kind::Combining, "a combining mark with no base", "\u{301}x"),
        Name::new(
            Kind::Combining,
            "thirty combining marks on one letter",
            format!("a{}", "\u{301}".repeat(30)),
        ),
        Name::new(
            Kind::Combining,
            "combining marks of three scripts",
            "a\u{300}\u{20DD}\u{0E49}\u{093C}",
        ),
        Name::new(Kind::Combining, "an enclosing mark", "1\u{20E3}"),
    ]
}

fn wide() -> Vec<Name> {
    vec![
        Name::new(Kind::Wide, "chinese", "中文"),
        Name::new(Kind::Wide, "japanese", "日本語のファイル"),
        Name::new(Kind::Wide, "korean", "한국어"),
        Name::new(Kind::Wide, "full-width latin", "\u{FF21}\u{FF22}\u{FF23}"),
        Name::new(Kind::Wide, "half-width katakana", "\u{FF71}\u{FF72}"),
        Name::new(Kind::Wide, "an emoji", "\u{1F600}"),
        Name::new(
            Kind::Wide,
            "an emoji family joined by zero-width joiners",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        )
        .shown("\u{1F468}\\u{200d}\u{1F469}\\u{200d}\u{1F467}"),
        Name::new(
            Kind::Wide,
            "a flag, two regional indicators",
            "\u{1F1EF}\u{1F1F5}",
        ),
        Name::new(
            Kind::Wide,
            "an emoji with a skin tone",
            "\u{1F44D}\u{1F3FD}",
        ),
        Name::new(
            Kind::Wide,
            "a heart with an emoji selector",
            "\u{2764}\u{FE0F}",
        ),
        Name::new(Kind::Wide, "a keycap", "1\u{FE0F}\u{20E3}"),
    ]
}

fn right_to_left() -> Vec<Name> {
    vec![
        Name::new(Kind::RightToLeft, "hebrew", "\u{5E9}\u{5DC}\u{5D5}\u{5DD}"),
        Name::new(
            Kind::RightToLeft,
            "arabic",
            "\u{645}\u{631}\u{62D}\u{628}\u{627}",
        ),
        Name::new(
            Kind::RightToLeft,
            "hebrew among latin letters and digits",
            "abc \u{5E9}\u{5DC}\u{5D5}\u{5DD} 123",
        ),
        Name::new(
            Kind::RightToLeft,
            "arabic with latin digits and a dot",
            "\u{645}\u{644}\u{641}.2026.txt",
        ),
        Name::new(
            Kind::RightToLeft,
            "an extension after hebrew",
            "\u{5E9}\u{5DC}\u{5D5}\u{5DD}.rs",
        ),
    ]
}

fn punctuation() -> Vec<Name> {
    let texts: &[(&str, &str)] = &[
        ("one dot", "."),
        ("two dots", ".."),
        ("a name that is a dot and a space", ". "),
        ("a trailing dot", "name."),
        ("trailing dots", "README.md..."),
        ("a trailing space", "name "),
        ("a leading space", " name"),
        ("a leading dash", "-name"),
        ("a name that looks like a flag", "-rf"),
        ("a long flag", "--help"),
        ("two dashes", "--"),
        ("a single dash", "-"),
        ("a backslash", "a\\b"),
        ("a trailing backslash", "dir\\"),
        // An escape written out as text: a name that reads, once the
        // escapes are read back, as another name (or as none).
        ("a line feed escape spelled out", "a\\nb"),
        ("a NUL escape spelled out", "c\\u{0}d"),
        ("a zero-width non-joiner escape spelled out", "a\\u{200c}b"),
        ("a colon", "a:b"),
        ("a drive prefix", "C:x"),
        ("an alternate data stream", "a::$DATA"),
        ("a stream named like a zone mark", "x:Zone.Identifier"),
        ("a double quote", "say\"hi\""),
        ("a single quote", "it's"),
        ("a back-tick", "a`b"),
        ("a dollar sign", "$HOME"),
        ("a command substitution", "$(id)"),
        ("a percent variable", "%PATH%"),
        ("a semicolon", "a;b"),
        ("an ampersand", "a&b"),
        ("a pipe", "a|b"),
        ("a redirect", "a>b<c"),
        ("a glob star", "*.rs"),
        ("a recursive glob", "**"),
        ("a question mark", "what?.txt"),
        ("a brace set", "{a,b}"),
        ("a bracket set", "[ab]"),
        ("a short name", "PROGRA~1"),
        ("a tilde", "~"),
        ("a hash and an exclamation mark", "#!x"),
        ("a path that climbs", "a/../b"),
        ("a network path", "\\\\server\\share"),
    ];
    texts
        .iter()
        .map(|&(label, text)| Name::new(Kind::Punctuation, label, text))
        .collect()
}

fn replacement() -> Vec<Name> {
    vec![
        Name::new(Kind::Replacement, "the replacement character", "\u{FFFD}"),
        Name::new(
            Kind::Replacement,
            "replacement characters among letters",
            "caf\u{FFFD}\u{FFFD}.txt",
        ),
    ]
}

/// Whether the file system that holds `dir` takes `name` as the exact name
/// of an ordinary file there: the question a test that makes files asks
/// before it makes one. Asked of the system at run time, by making the
/// file in a directory of its own inside `dir` and reading back what the
/// directory holds, so that it is true only when the name that comes back
/// is the name that went in. Windows refuses `CON`, a line feed and a
/// colon, and changes `name.` to `name`; Linux refuses only a slash and a
/// NUL and a name over 255 bytes. The directory is removed again.
#[must_use]
pub fn takes_file_name(dir: &Path, name: &str) -> bool {
    static PROBES: AtomicU32 = AtomicU32::new(0);
    let probe = dir.join(format!(
        ".hostile-probe-{}-{}",
        std::process::id(),
        PROBES.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::create_dir_all(&probe).is_err() {
        return false;
    }
    let made = std::fs::File::create_new(probe.join(name)).is_ok();
    let held: Vec<_> = std::fs::read_dir(&probe)
        .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
        .unwrap_or_default();
    let exact = made && held.len() == 1 && held[0].as_os_str() == std::ffi::OsStr::new(name);
    let _ = std::fs::remove_dir_all(&probe);
    exact
}

/// The names of `kind` that the file system holding `dir` takes as file
/// names, used whole: what a test that makes files runs its case for. A
/// test that must check something says so when this is empty.
#[must_use]
pub fn accepted(dir: &Path, kind: Kind) -> Vec<Name> {
    of(kind)
        .into_iter()
        .filter(|name| takes_file_name(dir, &name.text))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The ranges of `Cf` in Unicode's data, as `cineris-base` pins them.
    const FORMAT_DATA: &str =
        include_str!("../../cineris-base/fixtures/DerivedGeneralCategory-17.0.0-Cf.txt");

    fn is_format(character: char) -> bool {
        let code = u32::from(character);
        FORMAT_DATA
            .lines()
            .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
            .any(|line| {
                let range = line.split_once(';').expect("a range").0.trim();
                let (first, last) = range.split_once("..").unwrap_or((range, range));
                let hex = |text: &str| u32::from_str_radix(text, 16).expect("hex");
                (hex(first)..=hex(last)).contains(&code)
            })
    }

    /// The part of this file that is not tests.
    fn source() -> &'static str {
        let all = include_str!("hostile.rs");
        &all[..all.find("#[cfg(test)]").expect("the tests")]
    }

    #[test]
    fn every_kind_has_entries_and_all_is_all_of_them() {
        let mut total = 0;
        for kind in Kind::ALL {
            let names = of(kind);
            assert!(names.len() >= 2, "{kind:?} has {} entries", names.len());
            assert!(names.iter().all(|name| name.kind == kind), "{kind:?}");
            total += names.len();
        }
        assert_eq!(all().len(), total);
        assert_eq!(all(), all(), "the same on every call");
    }

    #[test]
    fn every_label_is_one_and_every_entry_has_text() {
        let mut labels = HashSet::new();
        for name in all() {
            assert!(!name.text.is_empty(), "{}", name.label);
            assert!(!name.label.is_empty());
            assert!(
                labels.insert(name.label.clone()),
                "two entries are called `{}`",
                name.label
            );
        }
    }

    /// No `cfg` in the module that makes the set: an entry and the number
    /// of them are the same on every system, which is the point of it.
    /// A name holds no slash, on either system, but the punctuation that
    /// is about paths on purpose (`a/../b`), so that putting a name in a
    /// path never makes two names of it.
    #[test]
    fn only_a_punctuation_entry_holds_a_slash() {
        for name in all() {
            assert!(
                !name.text.contains('/') || name.kind == Kind::Punctuation,
                "{}: {:?}",
                name.label,
                name.text
            );
        }
    }

    #[test]
    fn the_set_has_no_cfg_that_changes_it() {
        for line in source().lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for word in ["cfg", "target_os", "target_family"] {
                assert!(!code.contains(word), "`{word}` in the set: {line}");
            }
        }
    }

    #[test]
    fn a_control_entry_is_a_control_character_that_is_not_a_line_break() {
        let names = of(Kind::Control);
        for name in &names {
            assert!(
                name.text.chars().any(char::is_control),
                "{}: {:?}",
                name.label,
                name.text
            );
            assert!(!name.text.contains(LINE_BREAK_CHARACTERS), "{}", name.label);
        }
        // All of C0 but the two line breaks, DEL and all of C1 but NEL, each
        // alone, then the three of a name.
        let alone: HashSet<char> = names
            .iter()
            .filter(|name| name.text.chars().count() == 1)
            .map(|name| name.text.chars().next().unwrap())
            .collect();
        for code in (0x00..=0x1Fu32).chain([0x7F]).chain(0x80..=0x9F) {
            let character = char::from_u32(code).unwrap();
            let breaks = matches!(character, '\n' | '\r' | '\u{85}');
            assert_eq!(alone.contains(&character), !breaks, "U+{code:04X}");
        }
    }

    #[test]
    fn a_bidi_entry_holds_a_bidirectional_control() {
        const CONTROLS: [char; 12] = [
            '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}',
        ];
        let names = of(Kind::Bidi);
        for name in &names {
            assert!(
                name.text.chars().any(|c| CONTROLS.contains(&c)),
                "{}: {:?}",
                name.label,
                name.text
            );
        }
        for control in CONTROLS {
            assert!(
                names.iter().any(|name| name.text == control.to_string()),
                "no entry of U+{:04X} alone",
                u32::from(control)
            );
        }
    }

    #[test]
    fn a_format_entry_holds_a_format_character_and_the_tags_are_among_them() {
        let names = of(Kind::Format);
        for name in &names {
            assert!(
                name.text.chars().any(is_format),
                "{} holds no `Cf` character: {:?}",
                name.label,
                name.text
            );
        }
        for wanted in [
            '\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}', '\u{2060}', '\u{AD}',
        ] {
            assert!(
                names.iter().any(|name| name.text == wanted.to_string()),
                "no entry of U+{:04X}",
                u32::from(wanted)
            );
        }
        assert!(names.iter().any(|name| {
            name.text
                .chars()
                .any(|c| ('\u{E0020}'..='\u{E007F}').contains(&c))
        }));
        assert_eq!(tags("IGNORE").chars().count(), 6);
        assert!(tags("IGNORE").chars().all(is_format));
    }

    #[test]
    fn an_invisible_entry_acts_unseen_and_is_not_a_format_character() {
        for name in of(Kind::Invisible) {
            let hidden: Vec<char> = name
                .text
                .chars()
                .filter(|c| cineris_base::acts_unseen(*c) || cineris_base::visible::is_selector(*c))
                .collect();
            assert!(!hidden.is_empty(), "{}", name.label);
            assert!(
                hidden.iter().all(|c| !is_format(*c) && !c.is_control()),
                "{} holds a `Cf` or a control character",
                name.label
            );
        }
    }

    #[test]
    fn the_line_breaks_are_the_six_and_only_the_six() {
        let texts: Vec<String> = of(Kind::LineBreak).into_iter().map(|n| n.text).collect();
        assert_eq!(
            texts,
            ["\n", "\r", "\r\n", "\u{85}", "\u{2028}", "\u{2029}"]
        );
        for text in &texts {
            assert!(
                text.chars().all(|c| LINE_BREAK_CHARACTERS.contains(&c)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_escape_entry_holds_an_introducer() {
        for name in of(Kind::Escape) {
            assert!(
                name.text
                    .chars()
                    .any(|c| matches!(c, '\u{1b}' | '\u{9b}' | '\u{9d}' | '\u{7}' | '\u{8}')),
                "{}: {:?}",
                name.label,
                name.text
            );
        }
        let all: String = of(Kind::Escape).into_iter().map(|n| n.text).collect();
        for part in [
            "\u{1b}[", "\u{1b}]", "\u{7}", "\u{1b}\\", "\u{9b}", "\u{9d}",
        ] {
            assert!(all.contains(part), "no sequence with {part:?}");
        }
    }

    /// A long entry says its length first and its unit second, and the text
    /// has it.
    #[test]
    fn a_long_name_has_the_length_it_claims() {
        let names = of(Kind::Long);
        let mut lengths = HashSet::new();
        for name in &names {
            let mut words = name.label.split_whitespace();
            let count: usize = words.next().unwrap().parse().unwrap_or_else(|_| {
                panic!(
                    "a label of a long name starts with its length: `{}`",
                    name.label
                )
            });
            let unit = words.next().unwrap().trim_end_matches(',');
            let measured = match unit {
                "bytes" => name.text.len(),
                "characters" => name.text.chars().count(),
                "UTF-16" => name.text.encode_utf16().count(),
                other => panic!("`{other}` is not a unit: `{}`", name.label),
            };
            assert_eq!(measured, count, "{}", name.label);
            lengths.insert((unit, count));
        }
        // At and around 255 bytes, 260 characters, and a few thousand.
        for wanted in [
            ("bytes", 254),
            ("bytes", 255),
            ("bytes", 256),
            ("characters", 260),
        ] {
            assert!(lengths.contains(&wanted), "no name of {wanted:?}");
        }
        assert!(lengths.iter().any(|&(_, count)| count >= 4000));
        // One of 255 UTF-16 units either side, and one longer in bytes than
        // a Linux file name holds.
        assert!(
            names
                .iter()
                .any(|name| name.text.encode_utf16().count() == 254)
        );
        assert!(
            names
                .iter()
                .any(|name| name.text.encode_utf16().count() == 256)
        );
    }

    #[test]
    fn a_reserved_entry_is_a_windows_device_in_a_form_windows_still_takes_for_it() {
        const DEVICES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
        let names = of(Kind::Reserved);
        let stems: HashSet<String> = names
            .iter()
            .map(|name| {
                let stem = name.text.trim_end_matches(['.', ' ']);
                let stem = stem.split('.').next().unwrap().trim_end();
                let stem = stem.to_uppercase();
                let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.chars().count() == 4
                    && stem.chars().nth(3).is_some_and(|c| {
                        (c.is_ascii_digit() && c != '0')
                            || matches!(c, '\u{b9}' | '\u{b2}' | '\u{b3}')
                    });
                assert!(
                    numbered || DEVICES.contains(&stem.as_str()),
                    "{}: {:?} is not a device",
                    name.label,
                    name.text
                );
                stem
            })
            .collect();
        for device in DEVICES {
            assert!(stems.contains(device), "no {device}");
        }
        for family in ["COM", "LPT"] {
            for number in 1..=9 {
                assert!(
                    stems.contains(&format!("{family}{number}")),
                    "{family}{number}"
                );
            }
        }
        // The other cases, the extensions and the trailing dots and spaces.
        assert!(names.iter().any(|n| n.text == "nul" || n.text == "con"));
        assert!(
            names
                .iter()
                .any(|n| n.text.contains('.') && !n.text.ends_with('.'))
        );
        assert!(names.iter().any(|n| n.text.ends_with('.')));
        assert!(names.iter().any(|n| n.text.ends_with(' ')));
        assert!(names.iter().any(|n| n.text.contains(char::is_lowercase)));
        assert!(
            names
                .iter()
                .any(|n| n.text.chars().any(|c| c.is_ascii_uppercase()))
        );
    }

    #[test]
    fn a_combining_entry_holds_a_combining_mark() {
        let is_mark = |c: char| {
            matches!(u32::from(c),
                0x0300..=0x036F | 0x20D0..=0x20FF | 0x0E31..=0x0E4E | 0x093C | 0x1AB0..=0x1AFF)
        };
        for name in of(Kind::Combining) {
            if name.text == "\u{e9}" {
                continue;
            }
            assert!(
                name.text.chars().any(is_mark),
                "{}: {:?}",
                name.label,
                name.text
            );
        }
        let names = of(Kind::Combining);
        assert!(
            names.iter().any(|n| n.text.starts_with(is_mark)),
            "no mark without a base"
        );
        assert!(
            names
                .iter()
                .any(|n| n.text.chars().filter(|c| is_mark(*c)).count() >= 30)
        );
    }

    #[test]
    fn a_wide_entry_holds_a_wide_or_emoji_character() {
        let is_wide = |c: char| {
            matches!(u32::from(c),
                0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xFF00..=0xFFEF
                | 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F | 0x20E3)
        };
        for name in of(Kind::Wide) {
            assert!(
                name.text.chars().any(is_wide),
                "{}: {:?}",
                name.label,
                name.text
            );
        }
        assert!(
            of(Kind::Wide).iter().any(|n| n.text.contains('\u{200D}')),
            "a joined sequence"
        );
        assert!(
            of(Kind::Wide).iter().any(|n| n.text.contains('\u{FE0F}')),
            "an emoji selector"
        );
    }

    #[test]
    fn a_right_to_left_entry_holds_letters_that_run_right_to_left() {
        let is_rtl = |c: char| matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF);
        for name in of(Kind::RightToLeft) {
            assert!(name.text.chars().any(is_rtl), "{}", name.label);
        }
        assert!(
            of(Kind::RightToLeft)
                .iter()
                .any(|n| n.text.chars().any(|c| c.is_ascii_alphanumeric())),
            "one among others"
        );
    }

    #[test]
    fn a_punctuation_entry_has_a_character_a_system_gives_a_meaning() {
        let names = of(Kind::Punctuation);
        for wanted in [
            ".",
            "..",
            "-rf",
            "a\\b",
            "a:b",
            "a::$DATA",
            "name.",
            "say\"hi\"",
        ] {
            assert!(names.iter().any(|n| n.text == wanted), "no `{wanted}`");
        }
        assert!(names.iter().any(|n| n.text.starts_with('-')));
        for name in names {
            assert!(
                name.text.chars().any(|c| !c.is_alphanumeric()),
                "{}",
                name.label
            );
        }
    }

    #[test]
    fn the_replacement_entries_are_the_replacement_character() {
        for name in of(Kind::Replacement) {
            assert!(name.text.contains('\u{FFFD}'), "{}", name.label);
        }
    }

    /// What an entry says it is shown as is what the harness shows it as:
    /// the hand-written column and `cineris_base::visible` agree.
    #[test]
    fn every_entry_is_shown_as_it_says() {
        for name in all() {
            assert_eq!(
                cineris_base::visible(&name.text),
                name.shown.as_str(),
                "{}: {:?}",
                name.label,
                name.text
            );
        }
        // A line break, a carriage return and a tab are `\n`, `\r`, `\t`.
        assert_eq!(of(Kind::LineBreak)[0].shown, "\\n");
        assert_eq!(of(Kind::LineBreak)[2].shown, "\\r\\n");
        assert_eq!(of(Kind::LineBreak)[4].shown, "\\u{2028}");
    }

    #[test]
    fn what_acts_unseen_is_escaped_and_what_does_not_is_left_alone() {
        for name in all() {
            let hostile = name
                .text
                .chars()
                .any(|c| cineris_base::acts_unseen(c) || cineris_base::visible::is_selector(c));
            match name.kind {
                Kind::Control
                | Kind::Bidi
                | Kind::Format
                | Kind::Invisible
                | Kind::LineBreak
                | Kind::Escape => assert!(hostile, "{}", name.label),
                Kind::Combining
                | Kind::RightToLeft
                | Kind::Punctuation
                | Kind::Reserved
                | Kind::Replacement => {
                    assert!(!hostile, "{} acts unseen", name.label);
                    assert_eq!(name.shown, name.text, "{}", name.label);
                }
                Kind::Wide | Kind::Long => {}
            }
        }
    }

    #[test]
    fn a_file_system_that_takes_a_name_is_known_by_making_the_file() {
        let scratch = crate::Scratch::outside("cineris-testkit-hostile", "probe");
        let dir = scratch.path();
        assert!(takes_file_name(dir, "plain.txt"));
        assert!(takes_file_name(dir, "caf\u{e9}"));
        // No file system holds a slash in a name, nor a name that is `..`.
        assert!(!takes_file_name(dir, "a/b"));
        assert!(!takes_file_name(dir, ".."));
        assert!(!takes_file_name(dir, ""));
        // Nothing is left behind.
        assert_eq!(std::fs::read_dir(dir).unwrap().count(), 0);
        // What is accepted is a subset of what there is, in its order, and
        // the ordinary kinds are all accepted.
        let wide = accepted(dir, Kind::Wide);
        assert!(!wide.is_empty());
        let every = of(Kind::LineBreak);
        let taken = accepted(dir, Kind::LineBreak);
        assert!(taken.iter().all(|name| every.contains(name)));
        assert!(
            !taken.is_empty(),
            "some line break is a file name on every system"
        );
    }
}
